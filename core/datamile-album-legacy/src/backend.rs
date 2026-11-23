use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use datamile_logic::LogicEngine;
use datamile_lua_domain::{FormattedGrouperResult, GrouperFormatContext};
use datamile_server::{
    ActionTargetQuery, MediaBackend, PlaybackSlice, ServerConfig, ServerState,
    UpdateRequest,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::deps::{DependencyGraph, LegacyShared};

/// The album domain: everything the media-agnostic server must not know.
pub struct AlbumLegacyBackend {
    shared: LegacyShared,
    music_directory: RwLock<PathBuf>,
}

impl Default for AlbumLegacyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AlbumLegacyBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: LegacyShared::default(),
            music_directory: RwLock::new(PathBuf::new()),
        }
    }

    fn music_directory(&self) -> PathBuf {
        self.music_directory
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn playing_target(
        &self,
        logic: &LogicEngine,
        playing_file: Option<&str>,
    ) -> Option<String> {
        let file = playing_file?;
        let clean = file.trim_start_matches('/');

        if let Some(item) = logic
            .playable_lookup
            .get(clean)
            .or_else(|| logic.playable_lookup.get(file))
        {
            return Some(item.media_id.clone());
        }

        let music_directory = self.music_directory();
        let full = music_directory.join(clean);
        let mut current = full.parent()?.to_path_buf();

        while current.starts_with(&music_directory) && current != music_directory {
            if let Some(id) = logic.entity_id_by_path(&current) {
                return Some(id.to_string());
            }
            current = current.parent()?.to_path_buf();
        }

        None
    }

    fn param_targets(
        &self,
        query: &ActionTargetQuery,
        logic: &LogicEngine,
    ) -> Vec<String> {
        if let Some(query_str) = &query.query {
            return logic.find_ids(query_str);
        }

        if let Some(id) = &query.id {
            return vec![id.clone()];
        }

        if query.library || query.recursive.is_some() {
            let root = query.recursive.as_ref().map_or_else(
                || self.music_directory(),
                |dir| datamile_fs::expand_path(dir),
            );
            let root = root.canonicalize().unwrap_or(root);

            return logic
                .entities
                .values()
                .filter(|record| record.path.starts_with(&root))
                .map(|record| record.id.clone())
                .collect();
        }

        if let Some(dir) = &query.directory {
            let path = datamile_fs::expand_path(dir);
            let path = path.canonicalize().unwrap_or(path);
            if let Some(id) = logic.entity_id_by_path(&path) {
                return vec![id.to_string()];
            }
        }

        Vec::new()
    }
}

impl MediaBackend for AlbumLegacyBackend {
    fn scan_library(&self, logic: &mut LogicEngine) {
        let Ok(config) = crate::config::load() else {
            log::error!("Failed to load config for library scan");
            return;
        };

        let music_directory =
            datamile_fs::expand_path(&config.app.storage.music_directory)
                .canonicalize()
                .unwrap_or_else(|_| {
                    datamile_fs::expand_path(&config.app.storage.music_directory)
                });

        crate::library::Library::new(music_directory).scan(logic);
    }

    fn reload_config_and_logic(&self, logic: &mut LogicEngine) -> Result<ServerConfig> {
        let config = crate::config::load().context("Failed to load configuration")?;
        let config_path = config.path.clone();
        let config_dir = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let music_directory =
            datamile_fs::expand_path(&config.app.storage.music_directory)
                .canonicalize()
                .context("Invalid music_directory path")?;
        let cache_root = datamile_fs::expand_path(&config.app.storage.cache);
        let state_root = datamile_fs::expand_path(&config.app.storage.state);

        let engine = crate::lua::LuaEngine::new()?;
        engine.evaluate_config(&config_path)?;
        let formatter = |grouper: &str, ctx: &GrouperFormatContext<'_>| {
            engine.evaluate_grouper_format(grouper, ctx)
        };
        let evaluate = |value: &Value| engine.evaluate_album_logic(value);
        logic.reload_manifest(
            &config_path,
            config.manifest.clone(),
            &evaluate,
            &formatter,
        );

        self.music_directory
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&music_directory);

        let deps_json_path =
            crate::update::cache::deps_graph_path(&cache_root, &music_directory);
        let loaded = DependencyGraph::load_from_file(&deps_json_path);
        *self
            .shared
            .deps_graph
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = loaded;

        Ok(ServerConfig {
            music_directory,
            cache_root,
            state_root,
            resolved_dependencies: config.dependencies.clone(),
            interfaces: config.interfaces.clone(),
            actions: config.actions.clone(),
            config_dir,
            config_path,
            compiler_jobs: config.app.compiler.jobs,
        })
    }

    fn resolve_playback_uris(
        &self,
        media_type: &str,
        id: &str,
        slice: PlaybackSlice,
        logic: &LogicEngine,
    ) -> Vec<String> {
        let Some(record) = logic.entity_record(id) else {
            return Vec::new();
        };

        if record.media_type != media_type {
            return Vec::new();
        }

        slice.apply(&record.playable_uris).to_vec()
    }

    fn resolve_action_targets(
        &self,
        query: &ActionTargetQuery,
        logic: &LogicEngine,
        playing_file: Option<&str>,
    ) -> Vec<Value> {
        let ids = if query.playing {
            self.playing_target(logic, playing_file)
                .into_iter()
                .collect()
        } else {
            self.param_targets(query, logic)
        };

        ids.into_iter()
            .filter_map(|id| {
                let record = logic.entity_record(&id)?;
                let lock = serde_json::from_str::<Value>(&record.raw_json).ok()?;
                Some(serde_json::json!({
                    "path": record.path.to_string_lossy(),
                    "lock": lock
                }))
            })
            .collect()
    }

    fn execute_action(&self, name: &str, payload: &Value) -> Result<()> {
        let config = crate::config::load()?;
        let engine = crate::lua::LuaEngine::new()?;
        engine.evaluate_config(&config.path)?;
        engine.execute_action(name, payload)
    }

    fn format_grouper(
        &self,
        grouper: &str,
        ctx: &GrouperFormatContext<'_>,
    ) -> FormattedGrouperResult {
        let Ok(config) = crate::config::load() else {
            return datamile_logic::plain_grouper_formatter(grouper, ctx);
        };
        let Ok(engine) = crate::lua::LuaEngine::new() else {
            return datamile_logic::plain_grouper_formatter(grouper, ctx);
        };
        let _ = engine.evaluate_config(&config.path);
        engine.evaluate_grouper_format(grouper, ctx)
    }

    fn handle_fs_events<'a>(
        &'a self,
        paths: &'a [PathBuf],
        state: &'a Arc<ServerState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let flags = crate::inotify::classifier::classify_events(
                paths,
                state,
                &self.shared,
                &self.music_directory(),
            )
            .await;
            crate::inotify::handler::process_events(flags, state, &self.shared).await;
        })
    }

    fn extra_watch_dirs(&self, _state: &ServerState) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let music_directory = self.music_directory();
        let graph = self
            .shared
            .deps_graph
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut non_recursive = Vec::new();
        for dep_file in graph.file_to_albums.keys() {
            if !dep_file.starts_with(&music_directory)
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
        let shared = self.shared.clone();

        tokio::spawn(async move {
            if let Err(e) =
                crate::update::server::run_server_update(state, shared, request, log_tx)
                    .await
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
            match logic.entity_record(id) {
                Some(record) if record.media_type == media_type => {
                    Some(record.path.clone())
                }
                _ => None,
            }
        };

        let Some(path) = path else {
            return Ok(false);
        };

        let request = UpdateRequest::force(Some(path.to_string_lossy().to_string()));
        self.trigger_update(state, request, None)?;

        Ok(true)
    }

    fn tracked_changes(&self) -> (bool, Vec<String>) {
        let full_rescan = self
            .shared
            .full_rescan_needed
            .swap(false, Ordering::Relaxed);

        let tracked = self
            .shared
            .tracked_albums
            .lock()
            .map_or_else(|_| Vec::new(), |mut guard| guard.drain().collect());

        (full_rescan, tracked)
    }
}
