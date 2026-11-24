use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use datamile_cache::FileStat;
use serde::{Deserialize, Serialize};

use crate::state::MediaState;

/// External file dependencies of each compiled root.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub file_to_roots: HashMap<PathBuf, HashSet<PathBuf>>,
    pub root_to_files: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl DependencyGraph {
    pub fn update_root_deps(&mut self, root: PathBuf, new_deps: HashSet<PathBuf>) {
        self.remove_root(&root);

        for dep in &new_deps {
            self.file_to_roots
                .entry(dep.clone())
                .or_default()
                .insert(root.clone());
        }
        self.root_to_files.insert(root, new_deps);
    }

    pub fn remove_root(&mut self, root: &Path) {
        if let Some(deps) = self.root_to_files.remove(root) {
            for dep in deps {
                if let Some(roots) = self.file_to_roots.get_mut(&dep) {
                    roots.remove(root);
                    if roots.is_empty() {
                        self.file_to_roots.remove(&dep);
                    }
                }
            }
        }
    }

    pub fn prune(&mut self) -> bool {
        let before = self.root_to_files.len();
        self.root_to_files.retain(|root, _| root.exists());
        if self.root_to_files.len() == before {
            return false;
        }

        let mut rebuilt: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        for (root, deps) in &self.root_to_files {
            for dep in deps {
                rebuilt.entry(dep.clone()).or_default().insert(root.clone());
            }
        }
        self.file_to_roots = rebuilt;
        true
    }

    #[must_use]
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
            (Self::Paths(a), Self::Paths(b)) => {
                a.extend(b);
            }
        }
    }
}

#[derive(Default)]
pub struct UpdateState {
    pub is_running: bool,
    pub pending: Option<PendingUpdate>,
}

pub struct PendingUpdate {
    pub scope: UpdateScope,
    pub media_type: Option<String>,
    pub force: bool,
    pub jobs: Option<usize>,
    pub silent: bool,
    pub log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
}

/// Shared engine state handed to the update and watcher pipelines.
pub struct MediaShared {
    pub state: RwLock<Option<MediaState>>,
    pub deps_graph: RwLock<DependencyGraph>,
    pub active_writes: Mutex<HashSet<PathBuf>>,
    /// Last successful stat per known root, used to skip clean recompiles.
    pub root_stats: RwLock<HashMap<PathBuf, FileStat>>,
    pub tracked: Mutex<HashSet<(String, PathBuf)>>,
    pub full_rescan_needed: AtomicBool,
    pub update_lock: Mutex<UpdateState>,
}

impl Default for MediaShared {
    fn default() -> Self {
        Self {
            state: RwLock::new(None),
            deps_graph: RwLock::new(DependencyGraph::default()),
            active_writes: Mutex::new(HashSet::new()),
            root_stats: RwLock::new(HashMap::new()),
            tracked: Mutex::new(HashSet::new()),
            full_rescan_needed: AtomicBool::new(false),
            update_lock: Mutex::new(UpdateState::default()),
        }
    }
}

impl Clone for MediaShared {
    fn clone(&self) -> Self {
        Self {
            state: RwLock::new(
                self.state
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            deps_graph: RwLock::new(
                self.deps_graph
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            active_writes: Mutex::new(
                self.active_writes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            root_stats: RwLock::new(
                self.root_stats
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            tracked: Mutex::new(
                self.tracked
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone(),
            ),
            full_rescan_needed: AtomicBool::new(
                self.full_rescan_needed
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            update_lock: Mutex::new(UpdateState::default()),
        }
    }
}

pub type SharedRef = Arc<MediaShared>;
