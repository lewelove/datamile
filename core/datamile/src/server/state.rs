use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use datamile_logic::LogicEngine;
use datamile_lua_domain::{ActionDef, InterfaceDef};
use datamile_mpd::MpdEngine;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, watch};

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub file_to_albums: HashMap<PathBuf, HashSet<PathBuf>>,
    pub album_to_files: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl DependencyGraph {
    pub fn update_album_deps(&mut self, album_path: PathBuf, new_deps: HashSet<PathBuf>) {
        self.remove_album(&album_path);

        for dep in &new_deps {
            self.file_to_albums
                .entry(dep.clone())
                .or_default()
                .insert(album_path.clone());
        }
        self.album_to_files.insert(album_path, new_deps);
    }

    pub fn remove_album(&mut self, album_path: &Path) {
        if let Some(deps) = self.album_to_files.remove(album_path) {
            for dep in deps {
                if let Some(albums) = self.file_to_albums.get_mut(&dep) {
                    albums.remove(album_path);
                    if albums.is_empty() {
                        self.file_to_albums.remove(&dep);
                    }
                }
            }
        }
    }

    pub fn prune(&mut self) -> bool {
        let initial_albums_len = self.album_to_files.len();
        self.album_to_files.retain(|album, _| album.exists());
        let albums_removed = self.album_to_files.len() != initial_albums_len;

        if !albums_removed {
            return false;
        }

        let mut new_file_to_albums: HashMap<PathBuf, HashSet<PathBuf>> =
            HashMap::with_capacity(self.file_to_albums.len());
        for (album, deps) in &self.album_to_files {
            for dep in deps {
                new_file_to_albums
                    .entry(dep.clone())
                    .or_default()
                    .insert(album.clone());
            }
        }

        self.file_to_albums = new_file_to_albums;
        true
    }

    pub fn load_from_file(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(mut graph) = serde_json::from_str::<Self>(&content)
        {
            graph.prune();
            return graph;
        }
        Self::default()
    }

    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        datamile_fs::write_atomic(path, &content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateScope {
    All,
    Paths(HashSet<PathBuf>),
}

impl UpdateScope {
    pub fn merge(&mut self, other: Self) {
        match (self, other) {
            (Self::All, _) => {}
            (this, Self::All) => *this = Self::All,
            (Self::Paths(set_a), Self::Paths(set_b)) => {
                set_a.extend(set_b);
            }
        }
    }
}

#[derive(Default)]
pub struct UpdateState {
    pub is_running: bool,
    pub pending: Option<PendingUpdateParams>,
}

pub struct PendingUpdateParams {
    pub scope: UpdateScope,
    pub force: bool,
    pub jobs: Option<usize>,
    pub silent: bool,
    pub log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
}

pub struct AppState {
    pub logic: Arc<RwLock<LogicEngine>>,
    pub ui_state: RwLock<serde_json::Value>,
    pub tx: broadcast::Sender<String>,
    pub config: RwLock<AppConfig>,
    pub mpd_engine: MpdEngine,
    pub tracked_albums: Arc<Mutex<HashSet<String>>>,
    pub full_rescan_needed: Arc<AtomicBool>,
    pub active_writes: Arc<Mutex<HashSet<PathBuf>>>,
    pub image_tasks: Arc<ImageTaskRegistry>,
    pub update_lock: Arc<Mutex<UpdateState>>,
    pub deps_graph: Arc<RwLock<DependencyGraph>>,
}

#[derive(Clone)]
pub struct AppConfig {
    pub music_directory: PathBuf,
    pub cache_root: PathBuf,
    pub state_root: PathBuf,
    pub resolved_dependencies: Vec<PathBuf>,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub app: libdatamile::config::AppConfig,
}
