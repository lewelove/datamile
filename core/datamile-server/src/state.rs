use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use datamile_logic::LogicEngine;
use datamile_lua_domain::{ActionMap, InterfaceDef};
use datamile_mpd::{MpdConfig, MpdEngine};
use tokio::sync::{RwLock, broadcast, watch};

use crate::AllowList;
use crate::backend::MediaBackend;

pub enum ImageClaim {
    Claimed(ImageTaskGuard),
    /// Fires `true` when the current owner finishes, successfully or not.
    InFlight(watch::Receiver<bool>),
}

/// One in-flight image task per path, plus a signal for whoever waits on it.
#[derive(Default)]
pub struct ImageTaskRegistry {
    tasks: Mutex<HashMap<PathBuf, watch::Sender<bool>>>,
}

pub struct ImageTaskGuard {
    path: PathBuf,
    tx: watch::Sender<bool>,
    registry: Arc<ImageTaskRegistry>,
}

impl ImageTaskRegistry {
    /// Returns `None` only if the task map is poisoned.
    #[must_use]
    pub fn claim(self: &Arc<Self>, path: PathBuf) -> Option<ImageClaim> {
        let Ok(mut tasks) = self.tasks.lock() else {
            return None;
        };

        if let Some(tx) = tasks.get(&path) {
            return Some(ImageClaim::InFlight(tx.subscribe()));
        }

        let (tx, _rx) = watch::channel(false);
        tasks.insert(path.clone(), tx.clone());

        Some(ImageClaim::Claimed(ImageTaskGuard {
            path,
            tx,
            registry: Arc::clone(self),
        }))
    }
}

impl Drop for ImageTaskGuard {
    fn drop(&mut self) {
        if let Ok(mut tasks) = self.registry.tasks.lock() {
            tasks.remove(&self.path);
        }
        // Wake waiters even when the render failed: they re-check the file.
        let _ = self.tx.send(true);
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    /// Every registered `d.media.root` `init` directory, watched recursively.
    pub watch_roots: Vec<PathBuf>,
    pub mpd: MpdConfig,
    pub cache_root: PathBuf,
    pub state_root: PathBuf,
    pub resolved_dependencies: Vec<PathBuf>,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: ActionMap,
    /// Every media type declared by `d.media`, so `POST /api/x` can reject an
    /// unregistered `media` value before it resolves directories.
    pub media_types: Vec<String>,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub compiler_jobs: Option<usize>,
    /// `server.host`; the daemon binds it at startup.
    pub host: String,
    /// `server.allowed_origins`; extra hostnames or CIDR blocks next to the
    /// built-in loopback and LAN allowlist.
    pub allowed_origins: Vec<String>,
    /// `server.port`; the daemon binds it and exits if a reload changes it.
    pub port: u16,
}

pub struct ServerState {
    pub logic: Arc<RwLock<LogicEngine>>,
    pub ui_state: RwLock<serde_json::Value>,
    pub tx: broadcast::Sender<String>,
    pub config: RwLock<ServerConfig>,
    /// The `Host`/`Origin` allowlist the guard and the CORS policy read. A
    /// reload replaces it in place, so a new origin works without a restart.
    pub allow: std::sync::RwLock<AllowList>,
    pub mpd_engine: MpdEngine,
    pub image_tasks: Arc<ImageTaskRegistry>,
    /// Last file reported by MPD, used to resolve `playing` action targets.
    pub now_playing: RwLock<String>,
    /// Why the last config reload failed, if it did. `/api/identity` reports it
    /// so a CLI refuses to delegate to a daemon with a stale config.
    pub reload_error: RwLock<Option<String>>,
    pub backend: Arc<dyn MediaBackend>,
}

impl ServerState {
    /// Rebuilds the allowlist from `server.host` and `server.allowed_origins`.
    /// A config reload calls this after it swaps the config.
    pub fn reload_allow(&self, host: &str, allowed_origins: &[String]) {
        *self
            .allow
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            AllowList::from_config(host, allowed_origins);
    }
}
