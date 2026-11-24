use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use datamile_logic::LogicEngine;
use datamile_lua_domain::{ActionDef, InterfaceDef};
use datamile_mpd::{MpdConfig, MpdEngine};
use tokio::sync::{RwLock, broadcast, watch};

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
    pub actions: HashMap<String, ActionDef>,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub compiler_jobs: Option<usize>,
    /// `server.port`; the daemon binds it and exits if a reload changes it.
    pub port: u16,
}

pub struct ServerState {
    pub logic: Arc<RwLock<LogicEngine>>,
    pub ui_state: RwLock<serde_json::Value>,
    pub tx: broadcast::Sender<String>,
    pub config: RwLock<ServerConfig>,
    pub mpd_engine: MpdEngine,
    pub image_tasks: Arc<ImageTaskRegistry>,
    /// Last file reported by MPD, used to resolve `playing` action targets.
    pub now_playing: RwLock<String>,
    /// Why the last config reload failed, if it did. `/api/identity` reports it
    /// so a CLI refuses to delegate to a daemon with a stale config.
    pub reload_error: RwLock<Option<String>>,
    pub backend: Arc<dyn MediaBackend>,
}
