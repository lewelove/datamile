use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

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

/// Shared album-domain state handed to the update and inotify pipelines.
#[derive(Clone, Default)]
pub struct LegacyShared {
    pub deps_graph: Arc<std::sync::RwLock<DependencyGraph>>,
    pub active_writes: Arc<Mutex<HashSet<PathBuf>>>,
    pub tracked_albums: Arc<Mutex<HashSet<String>>>,
    pub full_rescan_needed: Arc<AtomicBool>,
    pub update_lock: Arc<Mutex<UpdateState>>,
}
