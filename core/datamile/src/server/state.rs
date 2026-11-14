use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use libdatamile::config::{ActionDef, CoversRegistry, InterfaceDef};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};

use crate::server::logic::LogicEngine;
use crate::server::mpd::MpdEngine;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub struct CoverTaskRegistry {
    tasks: Mutex<HashSet<PathBuf>>,
}

pub struct CoverTaskGuard {
    path: PathBuf,
    registry: Arc<CoverTaskRegistry>,
}

impl CoverTaskRegistry {
    #[must_use]
    pub fn try_claim(self: &Arc<Self>, path: PathBuf) -> Option<CoverTaskGuard> {
        let mut set = self
            .tasks
            .lock()
            .expect("cover task registry lock poisoned");
        if set.insert(path.clone()) {
            Some(CoverTaskGuard {
                path,
                registry: Arc::clone(self),
            })
        } else {
            None
        }
    }
}

impl Drop for CoverTaskGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.registry.tasks.lock() {
            set.remove(&self.path);
        }
    }
}

/// Everything one compiled root depends on. Keyed by lock path so two roots
/// may share a directory or an id without colliding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootMeta {
    pub root_type: String,
    pub target: PathBuf,
    pub files: HashSet<PathBuf>,
    pub dirs: HashSet<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub file_to_locks: HashMap<PathBuf, HashSet<PathBuf>>,
    pub dir_to_locks: HashMap<PathBuf, HashSet<PathBuf>>,
    pub lock_to_meta: HashMap<PathBuf, RootMeta>,
}

impl DependencyGraph {
    pub fn update_lock(&mut self, lock_path: PathBuf, meta: RootMeta) {
        self.remove_lock(&lock_path);
        self.index_lock(&lock_path, &meta);
        self.lock_to_meta.insert(lock_path, meta);
    }

    pub fn remove_lock(&mut self, lock_path: &Path) {
        let Some(old) = self.lock_to_meta.remove(lock_path) else {
            return;
        };

        for file in &old.files {
            remove_edge(&mut self.file_to_locks, file, lock_path);
        }
        for dir in &old.dirs {
            remove_edge(&mut self.dir_to_locks, dir, lock_path);
        }
    }

    fn index_lock(&mut self, lock_path: &Path, meta: &RootMeta) {
        for file in &meta.files {
            self.file_to_locks
                .entry(file.clone())
                .or_default()
                .insert(lock_path.to_path_buf());
        }
        for dir in &meta.dirs {
            self.dir_to_locks
                .entry(dir.clone())
                .or_default()
                .insert(lock_path.to_path_buf());
        }
    }

    pub fn locks_for_file(&self, path: &Path) -> Option<&HashSet<PathBuf>> {
        self.file_to_locks.get(path)
    }

    pub fn locks_for_dir(&self, path: &Path) -> Option<&HashSet<PathBuf>> {
        self.dir_to_locks.get(path)
    }

    /// Locks whose target or lock lives under `path`.
    pub fn locks_under(&self, path: &Path) -> Vec<PathBuf> {
        self.lock_to_meta
            .iter()
            .filter(|(lock, meta)| {
                lock.starts_with(path) || meta.target.starts_with(path)
            })
            .map(|(lock, _)| lock.clone())
            .collect()
    }

    pub fn prune(&mut self) -> bool {
        let before = self.lock_to_meta.len();
        self.lock_to_meta
            .retain(|lock, meta| lock.exists() && meta.target.exists());

        let removed = self.lock_to_meta.len() != before;
        if removed {
            self.reindex();
        }

        removed
    }

    fn reindex(&mut self) {
        self.file_to_locks.clear();
        self.dir_to_locks.clear();

        for (lock, meta) in &self.lock_to_meta {
            for file in &meta.files {
                self.file_to_locks
                    .entry(file.clone())
                    .or_default()
                    .insert(lock.clone());
            }
            for dir in &meta.dirs {
                self.dir_to_locks
                    .entry(dir.clone())
                    .or_default()
                    .insert(lock.clone());
            }
        }
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
    pub tx: broadcast::Sender<String>,
    /// Latest MPD snapshot, replayed to fresh WebSocket clients.
    pub playback: Arc<RwLock<Option<serde_json::Value>>>,
    pub config: RwLock<AppConfig>,
    pub mpd_engine: MpdEngine,
    /// Lock paths of roots the inotify classifier saw change.
    pub tracked_roots: Arc<Mutex<HashSet<String>>>,
    pub full_rescan_needed: Arc<AtomicBool>,
    pub active_writes: Arc<Mutex<HashSet<PathBuf>>>,
    pub cover_tasks: Arc<CoverTaskRegistry>,
    pub update_lock: Arc<Mutex<UpdateState>>,
    pub deps_graph: Arc<RwLock<DependencyGraph>>,
    /// Serializes read-merge-write cycles for `/api/state/{namespace}`.
    pub state_lock: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct AppConfig {
    pub media_roots: Vec<PathBuf>,
    pub cache_root: PathBuf,
    pub state_root: PathBuf,
    pub resolved_dependencies: Vec<PathBuf>,
    pub covers: CoversRegistry,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub app: libdatamile::config::AppConfig,
}

fn remove_edge(map: &mut HashMap<PathBuf, HashSet<PathBuf>>, key: &Path, lock: &Path) {
    let Some(locks) = map.get_mut(key) else {
        return;
    };
    locks.remove(lock);
    if locks.is_empty() {
        map.remove(key);
    }
}
