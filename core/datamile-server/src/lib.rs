pub mod api;
pub mod backend;
pub mod instance;
pub mod mpd_bridge;
pub mod state;
pub mod watcher;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
pub use backend::{ActionTargetQuery, MediaBackend, PlaybackSlice, UpdateRequest};
use datamile_logic::LogicEngine;
use datamile_lua_domain::LogicManifest;
pub use instance::{Identity, IdentityMismatch, InstanceLock, VERSION};
pub use state::{
    ImageClaim, ImageTaskGuard, ImageTaskRegistry, ServerConfig, ServerState,
};

/// Port used when the config leaves `server.port` at 0.
pub const DEFAULT_PORT: u16 = 8000;
use tokio::sync::{RwLock, broadcast};
use tower_http::cors::{Any, CorsLayer};

fn create_directories(cache_root: &Path, state_root: &Path) {
    std::fs::create_dir_all(cache_root).ok();
    std::fs::create_dir_all(cache_root.join("objects")).ok();
    std::fs::create_dir_all(state_root).ok();
}

fn load_state(state_root: &Path) -> serde_json::Value {
    let state_file = state_root.join("state.json");
    if state_file.exists() {
        let data = std::fs::read_to_string(&state_file).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_else(|_| default_state())
    } else {
        default_state()
    }
}

fn default_state() -> serde_json::Value {
    serde_json::json!({
        "activeTab": "home",
        "homeSubView": "libraries",
        "activeLibrary": "library",
        "activeCabinet": "default",
        "sortKey": "default",
        "sortOrder": "default",
        "groupKey": "genre",
        "filter": {
            "key": null,
            "val": null
        },
        "activeShelf": null,
        "activeShelfOrder": "original",
        "activeShelfOrderReverse": false,
        "sidebarWidth": 280
    })
}

/// Boots the HTTP/WS server around a media backend.
///
/// The backend owns every domain decision: what a media type is, how a lock
/// maps to playable URIs, and how an update runs.
pub async fn run(backend: Arc<dyn MediaBackend>) -> Result<()> {
    let mut logic_engine = LogicEngine::new(PathBuf::new(), LogicManifest::default());
    let config = backend
        .reload_config_and_logic(&mut logic_engine)
        .context("Failed to load application configuration")?;

    create_directories(&config.cache_root, &config.state_root);
    backend.scan_library(&mut logic_engine);

    let ui_state = load_state(&config.state_root);
    let logic = Arc::new(RwLock::new(logic_engine));
    let (tx, _) = broadcast::channel(1024);

    let mpd_enabled = config.mpd.enabled;
    let (mpd_engine, mpd_status_tx) = if mpd_enabled {
        datamile_mpd::start_actor(config.mpd.addr())
    } else {
        // A disabled MPD still needs a command sink; the receiver is dropped, so
        // every command is a no-op.
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let (status_tx, _) = broadcast::channel(1);
        (datamile_mpd::MpdEngine::new(tx), status_tx)
    };

    let port = if config.port == 0 {
        DEFAULT_PORT
    } else {
        config.port
    };

    let state = Arc::new(ServerState {
        logic,
        ui_state: RwLock::new(ui_state),
        tx: tx.clone(),
        config: RwLock::new(config),
        mpd_engine,
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: RwLock::new(String::new()),
        reload_error: RwLock::new(None),
        backend: Arc::clone(&backend),
    });

    if mpd_enabled {
        mpd_bridge::start_status_bridge(&mpd_status_tx, Arc::clone(&state));
    }
    watcher::start(Arc::clone(&state));

    if let Err(e) =
        backend.trigger_update(Arc::clone(&state), UpdateRequest::preserve(), Vec::new())
    {
        log::error!("Startup auto-update error: {e}");
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = api::router(Arc::clone(&state)).layer(cors);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log::info!("Datamile Server listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}

/// Directory holding the watcher's extra non-recursive watch targets.
#[must_use]
pub fn dependency_dirs(dependencies: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = HashSet::new();
    for dep in dependencies {
        if let Some(parent) = dep.parent() {
            dirs.insert(parent.to_path_buf());
        }
    }
    dirs.into_iter().collect()
}
