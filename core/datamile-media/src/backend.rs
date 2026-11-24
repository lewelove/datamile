use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use datamile_cache::FileStat;
use datamile_logic::LogicEngine;
use datamile_server::{
    ActionTargetQuery, MediaBackend, PlaybackSlice, ServerConfig, ServerState,
    UpdateRequest,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::deps::{DependencyGraph, MediaShared};
use crate::schema::{discover_roots, read_media_manifest, validate_media_schema};
use crate::state::{MediaState, deps_graph_path, open_engine};

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

    fn playing_target(
        logic: &LogicEngine,
        playing_file: Option<&str>,
    ) -> Option<(String, String)> {
        let file = playing_file?;
        let clean = file.trim_start_matches('/');
        let item = logic
            .playable_lookup
            .get(clean)
            .or_else(|| logic.playable_lookup.get(file))?;
        Some((item.media_type.clone(), item.media_id.clone()))
    }

    fn item_for(logic: &LogicEngine, media_type: &str, id: &str) -> Option<Value> {
        let record = logic.entity_record(media_type, id)?;
        let lock = serde_json::from_str::<Value>(&record.raw_json).ok()?;
        Some(serde_json::json!({
            "type": record.media_type,
            "id": record.id,
            "path": record.path.to_string_lossy(),
            "lock": lock
        }))
    }

    fn param_targets(
        query: &ActionTargetQuery,
        logic: &LogicEngine,
        state: &MediaState,
    ) -> Vec<Value> {
        let media_types: Vec<String> = query.media_type.as_ref().map_or_else(
            || state.media.roots_order.clone(),
            |media_type| vec![media_type.clone()],
        );

        let ids_for = |media_type: &str| -> Vec<String> {
            if let Some(query_str) = &query.query {
                return logic.find_ids(media_type, query_str);
            }
            if let Some(id) = &query.id {
                return if logic.entity_record(media_type, id).is_some() {
                    vec![id.clone()]
                } else {
                    Vec::new()
                };
            }
            if query.library || query.recursive.is_some() {
                let root = query.recursive.as_ref().map_or_else(
                    || state.init_dirs().first().cloned().unwrap_or_default(),
                    |dir| datamile_fs::expand_path(dir),
                );
                let root = root.canonicalize().unwrap_or(root);
                return logic
                    .all_entities()
                    .filter(|record| {
                        record.media_type == media_type && record.path.starts_with(&root)
                    })
                    .map(|record| record.id.clone())
                    .collect();
            }
            if let Some(dir) = &query.directory {
                let path = datamile_fs::expand_path(dir);
                let path = path.canonicalize().unwrap_or(path);
                return logic
                    .entity_id_by_path(media_type, &path)
                    .map(ToString::to_string)
                    .into_iter()
                    .collect();
            }
            Vec::new()
        };

        media_types
            .iter()
            .flat_map(|media_type| {
                ids_for(media_type)
                    .into_iter()
                    .filter_map(|id| Self::item_for(logic, media_type, &id))
                    .collect::<Vec<_>>()
            })
            .collect()
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
        let mut root_stats = std::collections::HashMap::new();

        for root in &state.roots {
            let lock_path = root.dir.join(format!("{}.lock.json", root.media_type));
            let Ok(lock_json) = std::fs::read_to_string(&lock_path) else {
                continue;
            };
            let Some(schema) = state.root_schema(&root.media_type) else {
                continue;
            };
            let music_directory = state.mpd_base(&root.media_type, &root.dir);

            let Some(parsed) = crate::ingest::parse_lock(
                &root.media_type,
                &root.dir,
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

            if let Ok(meta) = std::fs::metadata(&root.dir) {
                root_stats.insert(root.dir.clone(), FileStat::from(&meta));
            }
        }

        logic.build_cache();
        *self
            .shared
            .root_stats
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root_stats;
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
        validate_media_schema(&media, &evaluated.manifest)?;
        let roots = discover_roots(&engine, &media, &config_dir)?;

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

        let mut mpd = evaluated.config.mpd.clone();
        if let Some(music_directory) = &mpd.music_directory {
            mpd.music_directory = Some(
                datamile_fs::resolve_path(music_directory, &config_dir)
                    .to_string_lossy()
                    .to_string(),
            );
        }

        let compiler_jobs = evaluated
            .config
            .extra
            .get("compiler")
            .and_then(|compiler| compiler.get("jobs"))
            .and_then(Value::as_u64)
            .and_then(|jobs| usize::try_from(jobs).ok());

        let mut dependencies = evaluated.dependencies.clone();
        dependencies.push(
            config_path
                .canonicalize()
                .unwrap_or_else(|_| config_path.clone()),
        );

        let state = MediaState {
            config_path: config_path.clone(),
            config_dir: config_dir.clone(),
            cache_root: cache_root.clone(),
            state_root: state_root.clone(),
            media,
            logic_manifest: evaluated.manifest.clone(),
            roots,
            dependencies: dependencies.clone(),
            actions: evaluated.actions.clone(),
            interfaces: evaluated.interfaces.clone(),
            mpd: mpd.clone(),
            compiler_jobs,
        };

        let inits = state.init_dirs();
        let deps_path = deps_graph_path(&cache_root, &inits);
        *self
            .shared
            .deps_graph
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            DependencyGraph::load_from_file(&deps_path);

        let root_stats = state
            .roots
            .iter()
            .filter_map(|root| {
                std::fs::metadata(&root.dir)
                    .ok()
                    .map(|meta| (root.dir.clone(), FileStat::from(&meta)))
            })
            .collect();
        *self
            .shared
            .root_stats
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = root_stats;

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
            actions: evaluated.actions,
            config_dir,
            config_path,
            compiler_jobs,
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

    fn resolve_action_targets(
        &self,
        query: &ActionTargetQuery,
        logic: &LogicEngine,
        playing_file: Option<&str>,
    ) -> Vec<Value> {
        let Some(state) = self.current_state() else {
            return Vec::new();
        };

        if query.playing {
            return Self::playing_target(logic, playing_file)
                .and_then(|(media_type, id)| Self::item_for(logic, &media_type, &id))
                .into_iter()
                .collect();
        }

        Self::param_targets(query, logic, &state)
    }

    fn execute_action(&self, name: &str, payload: &Value) -> Result<()> {
        let state = self.current_state().context("Media state is not loaded")?;
        let engine = open_engine(&state)?;
        engine.execute_action(name, payload)
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

        let graph = self
            .shared
            .deps_graph
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut non_recursive = Vec::new();
        for dep_file in graph.file_to_roots.keys() {
            if !inits.iter().any(|init| dep_file.starts_with(init))
                && let Some(parent) = dep_file.parent()
            {
                non_recursive.push(parent.to_path_buf());
            }
        }
        drop(graph);

        (Vec::new(), non_recursive)
    }

    fn trigger_update(
        &self,
        state: Arc<ServerState>,
        request: UpdateRequest,
        log_tx: Option<mpsc::Sender<String>>,
    ) -> Result<()> {
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            if let Err(e) =
                crate::update::run_server_update(state, shared, request, log_tx).await
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
        let path = {
            let Ok(logic) = state.logic.try_read() else {
                return Ok(false);
            };
            logic
                .entity_record(media_type, id)
                .map(|record| record.path.clone())
        };

        let Some(path) = path else {
            return Ok(false);
        };

        self.trigger_update(
            state,
            UpdateRequest::force(Some(path.to_string_lossy().to_string())),
            None,
        )?;
        Ok(true)
    }

    fn tracked_changes(&self) -> (bool, Vec<String>) {
        let full_rescan = self
            .shared
            .full_rescan_needed
            .swap(false, Ordering::Relaxed);

        let tracked = self.shared.tracked.lock().map_or_else(
            |_| Vec::new(),
            |mut guard| {
                guard
                    .drain()
                    .map(|(media_type, path)| {
                        format!("{media_type}\u{1f}{}", path.to_string_lossy())
                    })
                    .collect()
            },
        );

        (full_rescan, tracked)
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
