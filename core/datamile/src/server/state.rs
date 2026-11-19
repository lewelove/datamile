use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use libdatamile::config::{ActionDef, InterfaceDef};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};

use crate::server::logic::LogicEngine;
use crate::server::mpd::MpdEngine;

#[derive(Default)]
pub struct ImageTaskRegistry {
    tasks: Mutex<HashSet<PathBuf>>,
}

pub struct ImageTaskGuard {
    path: PathBuf,
    registry: Arc<ImageTaskRegistry>,
}

impl ImageTaskRegistry {
    #[must_use]
    pub fn try_claim(self: &Arc<Self>, path: PathBuf) -> Option<ImageTaskGuard> {
        let Ok(mut set) = self.tasks.lock() else {
            return None;
        };
        if set.insert(path.clone()) {
            Some(ImageTaskGuard {
                path,
                registry: Arc::clone(self),
            })
        } else {
            None
        }
    }
}

impl Drop for ImageTaskGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.registry.tasks.lock() {
            set.remove(&self.path);
        }
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
        libdatamile::utils::write_atomic_cache_file(path, &content)?;
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
