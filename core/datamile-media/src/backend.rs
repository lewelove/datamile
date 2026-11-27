use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Context, Result};
use datamile_logic::LogicEngine;
use datamile_server::{
    ActionCtx, DEFAULT_PORT, EntityKey, MediaBackend, PlaybackSlice, ServerConfig,
    ServerState, UpdateRequest,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::deps::{MediaShared, rebuild_external_index};
use crate::schema::{discover_roots, read_media_manifest, validate_media_schema};
use crate::state::{MediaState, open_engine};

/// Schema-driven media engine backend.
pub struct MediaEngineBackend {
    shared: Arc<MediaShared>,
}

impl Default for MediaEngineBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaEngineBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(MediaShared::default()),
        }
    }

    #[must_use]
    pub fn shared(&self) -> Arc<MediaShared> {
        Arc::clone(&self.shared)
    }

    fn current_state(&self) -> Option<MediaState> {
        self.shared
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl MediaBackend for MediaEngineBackend {
    fn scan_library(&self, logic: &mut LogicEngine) {
        let Some(state) = self.current_state() else {
            log::error!("Media state is not loaded; cannot scan");
            return;
        };

        let Ok(engine) = open_engine(&state) else {
            log::error!("Failed to open the media engine for scanning");
            return;
        };

        logic.clear();

        for root in &state.roots {
            let lock_path = root.dir.join(format!("{}.lock.json", root.media_type));
            let Ok(lock_json) = std::fs::read_to_string(&lock_path) else {
                continue;
            };
            let Some(schema) = state.root_schema(&root.media_type) else {
                continue;
            };

            let music_directory = state.mpd_base();
            let Some(parsed) = crate::ingest::parse_lock(
                &root.media_type,
                &lock_json,
                schema,
                music_directory.as_deref(),
            )
            .ok()
            .flatten() else {
                continue;
            };

            let eval = serde_json::from_str::<Value>(&lock_json)
                .ok()
                .and_then(|lock| lock.get(&root.media_type).cloned())
                .and_then(|entity| engine.evaluate_logic(&root.media_type, &entity).ok())
                .unwrap_or_else(|| serde_json::json!({}));

            if let Err(e) =
                logic.upsert_entity(parsed.entity, parsed.playables, parsed.images, eval)
            {
                log::error!("Failed to index {}: {e}", root.dir.display());
            }
        }

        logic.build_cache();
    }

    #[allow(
        clippy::too_many_lines,
        reason = "config load, discovery and state swap read as one sequence"
    )]
    fn reload_config_and_logic(&self, logic: &mut LogicEngine) -> Result<ServerConfig> {
        // Prefer the path already loaded by this backend: a hot reload must
        // re-read the same file, not re-run path discovery.
        let config_path = self
            .current_state()
            .map_or_else(datamile_lua_domain::resolve_config_path, |state| {
                Ok(state.config_path)
            })
            .context("Failed to resolve init.lua")?;
        let config_dir = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let engine = datamile_lua_domain::DomainLuaEngine::new()?;
        engine.load_extension("media.lua", crate::state::MEDIA_LUA)?;
        engine.load_extension("compile_root.lua", crate::state::COMPILE_ROOT_LUA)?;
        crate::lua_bridge::register_native_helpers(&engine.runtime)?;

        let evaluated = engine.evaluate_config(&config_path)?;
        let media = read_media_manifest(&engine)?;
        validate_media_schema(&media, &evaluated.manifest, &evaluated.actions)?;
        let actions = engine.resolve_actions(&evaluated.actions)?;

        let discovery = discover_roots(&engine, &media, &config_dir)?;
        if let Some((media_type, error)) = discovery.failures.first() {
            return Err(anyhow::anyhow!(
                "media.root '{media_type}' discovery failed: {error}"
            ));
        }
        let roots = discovery.roots;

        let cache_root = if evaluated.config.storage.cache.is_empty() {
            crate::state::default_cache_root()
        } else {
            datamile_fs::resolve_path(&evaluated.config.storage.cache, &config_dir)
        };
        let state_root = if evaluated.config.storage.state.is_empty() {
            datamile_fs::expand_path("~/.local/share/datamile")
        } else {
            datamile_fs::resolve_path(&evaluated.config.storage.state, &config_dir)
        };

        let mpd = evaluated.config.mpd.clone();

        let compiler_jobs = evaluated.config.compiler.jobs;

        let mut dependencies = evaluated.dependencies.clone();
        dependencies.push(
            config_path
                .canonicalize()
                .unwrap_or_else(|_| config_path.clone()),
        );

        let port = if evaluated.config.server.port == 0 {
            DEFAULT_PORT
        } else {
            evaluated.config.server.port
        };

        let media_types = media.roots_order.clone();
        let state = MediaState {
            config_path: config_path.clone(),
            config_dir: config_dir.clone(),
            cache_root: cache_root.clone(),
            state_root: state_root.clone(),
            media,
            logic_manifest: evaluated.manifest.clone(),
            roots,
            dependencies: dependencies.clone(),
            interfaces: evaluated.interfaces.clone(),
            mpd: mpd.clone(),
            compiler_jobs,
            host: evaluated.config.server.host.clone(),
            port,
        };

        // No persisted dependency graph: the reverse index is rebuilt from the
        // state files, which already hold the external entries.
        *self
            .shared
            .external_index
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            rebuild_external_index(&state);

        let evaluate =
            |media_type: &str, value: &Value| engine.evaluate_logic(media_type, value);
        logic.reload_manifest(&config_path, evaluated.manifest.clone(), &evaluate);

        let watch_roots = state.init_dirs();
        *self
            .shared
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state);

        Ok(ServerConfig {
            watch_roots,
            mpd,
            cache_root,
            state_root,
            resolved_dependencies: dependencies,
            interfaces: evaluated.interfaces,
            actions,
            media_types,
            config_dir,
            config_path,
            compiler_jobs,
            host: evaluated.config.server.host,
            allowed_origins: evaluated.config.server.allowed_origins,
            port,
        })
    }

    fn resolve_playback_uris(
        &self,
        media_type: &str,
        id: &str,
        slice: PlaybackSlice,
        logic: &LogicEngine,
    ) -> Vec<String> {
        let Some(record) = logic.entity_record(media_type, id) else {
            return Vec::new();
        };
        slice.apply(&record.playable_uris).to_vec()
    }

    fn execute_action(&self, name: &str, ctx: &ActionCtx) -> Result<()> {
        let state = self.current_state().context("Media state is not loaded")?;
        let engine = open_engine(&state)?;
        engine.execute_action(name, ctx)
    }

    fn handle_fs_events<'a>(
        &'a self,
        paths: &'a [PathBuf],
        state: &'a Arc<ServerState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(crate::watcher::handle_fs_events(
            Arc::clone(&self.shared),
            paths,
            state,
        ))
    }

    fn extra_watch_dirs(&self, _state: &ServerState) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let inits = self
            .current_state()
            .map_or_else(Vec::new, |s| s.init_dirs());

        let index = self
            .shared
            .external_index
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut non_recursive = Vec::new();
        for dep_file in index.keys() {
            if !inits.iter().any(|init| dep_file.starts_with(init))
                && let Some(parent) = dep_file.parent()
            {
                non_recursive.push(parent.to_path_buf());
            }
        }
        drop(index);

        (Vec::new(), non_recursive)
    }

    fn trigger_update(
        &self,
        state: Arc<ServerState>,
        request: UpdateRequest,
        event_txs: Vec<mpsc::Sender<String>>,
    ) -> Result<()> {
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            if let Err(e) =
                crate::update::run_server_update(state, shared, request, event_txs).await
            {
                log::error!("Server update failed: {e}");
            }
        });
        Ok(())
    }

    fn force_update_entity(
        &self,
        state: Arc<ServerState>,
        media_type: &str,
        id: &str,
    ) -> Result<bool> {
        let key = {
            let Ok(logic) = state.logic.try_read() else {
                return Ok(false);
            };
            logic.entity_record(media_type, id).map(|record| EntityKey {
                media_type: record.media_type.clone(),
                dir: record.path.clone(),
            })
        };

        let Some(key) = key else {
            return Ok(false);
        };

        self.trigger_update(
            state,
            UpdateRequest {
                entities: vec![key],
                force: true,
                ..UpdateRequest::default()
            },
            Vec::new(),
        )?;
        Ok(true)
    }
}

/// Collects the set of known root directories for one media type.
#[must_use]
pub fn known_roots(logic: &LogicEngine, media_type: &str) -> HashSet<PathBuf> {
    logic
        .all_entities()
        .filter(|record| record.media_type == media_type)
        .map(|record| record.path.clone())
        .collect()
}
