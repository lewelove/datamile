use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use datamile_cache::EntityKey;
use datamile_logic::LogicEngine;
use datamile_lua_domain::ActionCtx;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::state::{ServerConfig, ServerState};

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub path: Option<String>,
    #[serde(default)]
    pub media_type: Option<String>,
    /// Exact entity keys to recompile. Non-empty means `path` and `media_type`
    /// are ignored, so forcing one entity never recompiles its sibling media
    /// types in the same directory.
    #[serde(default)]
    pub entities: Vec<EntityKey>,
    pub force: bool,
    pub jobs: Option<usize>,
}

impl UpdateRequest {
    #[must_use]
    pub fn preserve() -> Self {
        Self::default()
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
    fn execute_action(&self, name: &str, ctx: &ActionCtx) -> anyhow::Result<()>;
    fn handle_fs_events<'a>(
        &'a self,
        paths: &'a [PathBuf],
        state: &'a Arc<ServerState>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
    /// Extra watch targets beyond the config dir and music directory.
    fn extra_watch_dirs(&self, state: &ServerState) -> (Vec<PathBuf>, Vec<PathBuf>);
    /// One serialized event stream per attached client.
    fn trigger_update(
        &self,
        state: Arc<ServerState>,
        request: UpdateRequest,
        event_txs: Vec<mpsc::Sender<String>>,
    ) -> anyhow::Result<()>;
    fn force_update_entity(
        &self,
        state: Arc<ServerState>,
        media_type: &str,
        id: &str,
    ) -> anyhow::Result<bool>;
}
