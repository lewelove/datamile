use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use datamile_logic::LogicEngine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::state::{ServerConfig, ServerState};

/// Keys that select targets instead of becoming action options.
const RESERVED_PARAMS: &[&str] = &[
    "playing",
    "id",
    "query",
    "directory",
    "recursive",
    "library",
    "type",
];

#[derive(Debug, Clone, Copy, Default)]
pub struct PlaybackSlice {
    pub start: Option<usize>,
    pub count: Option<usize>,
}

impl PlaybackSlice {
    #[must_use]
    pub fn apply<'a>(&self, uris: &'a [String]) -> &'a [String] {
        let start = self.start.unwrap_or(0).min(uris.len());
        let end = self.count.map_or(uris.len(), |count| {
            start.saturating_add(count).min(uris.len())
        });
        &uris[start..end]
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActionTargetQuery {
    /// Media type the action targets, from the  query parameter.
    pub media_type: Option<String>,
    pub playing: bool,
    pub id: Option<String>,
    pub query: Option<String>,
    pub directory: Option<String>,
    pub recursive: Option<String>,
    pub library: bool,
    pub options: Vec<(String, String)>,
}

impl ActionTargetQuery {
    #[must_use]
    #[allow(
        clippy::implicit_hasher,
        reason = "axum's Query extractor hands back the default hasher"
    )]
    pub fn from_params(params: &HashMap<String, String>) -> Self {
        let mut options = Vec::new();
        for (key, value) in params {
            if RESERVED_PARAMS.contains(&key.as_str()) {
                continue;
            }
            options.push((key.clone(), value.clone()));
        }

        Self {
            media_type: params.get("type").filter(|s| !s.is_empty()).cloned(),
            playing: params.get("playing").is_some_and(|v| v == "true"),
            id: params.get("id").filter(|s| !s.is_empty()).cloned(),
            query: params.get("query").cloned(),
            directory: params.get("directory").cloned(),
            recursive: params.get("recursive").cloned(),
            library: params.get("library").is_some_and(|v| v == "true"),
            options,
        }
    }

    #[must_use]
    pub fn options_string(&self) -> String {
        self.options
            .iter()
            .map(|(key, value)| {
                if value.is_empty() {
                    key.clone()
                } else {
                    format!("{key}={value}")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub media_type: Option<String>,
    pub force: bool,
    pub jobs: Option<usize>,
    pub silent: bool,
}

impl UpdateRequest {
    #[must_use]
    pub fn preserve() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn force(path: Option<String>) -> Self {
        Self {
            path,
            force: true,
            ..Self::default()
        }
    }
}

/// Everything the server needs from the media domain.
///
/// The server never parses a lock, runs Lua, or knows what an album is; it asks
/// the backend for targets, URIs and updates and broadcasts the results.
pub trait MediaBackend: Send + Sync + 'static {
    fn scan_library(&self, logic: &mut LogicEngine);
    fn reload_config_and_logic(
        &self,
        logic: &mut LogicEngine,
    ) -> anyhow::Result<ServerConfig>;
    fn resolve_playback_uris(
        &self,
        media_type: &str,
        id: &str,
        slice: PlaybackSlice,
        logic: &LogicEngine,
    ) -> Vec<String>;
    fn resolve_action_targets(
        &self,
        query: &ActionTargetQuery,
        logic: &LogicEngine,
        playing_file: Option<&str>,
    ) -> Vec<Value>;
    fn execute_action(&self, name: &str, payload: &Value) -> anyhow::Result<()>;
    fn handle_fs_events<'a>(
        &'a self,
        paths: &'a [PathBuf],
        state: &'a Arc<ServerState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
    /// Extra watch targets beyond the config dir and music directory.
    fn extra_watch_dirs(&self, state: &ServerState) -> (Vec<PathBuf>, Vec<PathBuf>);
    fn trigger_update(
        &self,
        state: Arc<ServerState>,
        request: UpdateRequest,
        log_tx: Option<mpsc::Sender<String>>,
    ) -> anyhow::Result<()>;
    fn force_update_entity(
        &self,
        state: Arc<ServerState>,
        media_type: &str,
        id: &str,
    ) -> anyhow::Result<bool>;
    /// `(full_rescan, tracked_items)` and resets the tracking state.
    fn tracked_changes(&self) -> (bool, Vec<String>);
}
