use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use datamile_cache::{
    FileStat, RootState, RootStates, load_state, root_files, save_state,
};

use crate::state::MediaState;

/// Reverse index `external file -> root entities that read it`, rebuilt at boot
/// from the state files and kept in memory for the watcher. Nothing on disk
/// duplicates it.
pub type ExternalIndex = BTreeMap<PathBuf, BTreeSet<PathBuf>>;

/// Rebuilds the reverse index by reading every media root's state file.
#[must_use]
pub fn rebuild_external_index(state: &MediaState) -> ExternalIndex {
    let mut index = ExternalIndex::new();

    for (media_type, init) in state.media_roots() {
        let path = crate::state::state_path(
            &state.cache_root,
            &state.config_path,
            &media_type,
            &init,
        );
        let Ok(states) = load_state(&path) else {
            continue;
        };
        for (dir, entry) in &states {
            let dir = PathBuf::from(dir);
            for file in external_files(&dir, entry) {
                index.entry(file).or_default().insert(dir.clone());
            }
        }
    }

    index
}

/// Entries outside the root entity's own directory are external dependencies.
fn external_files(dir: &Path, entry: &RootState) -> Vec<PathBuf> {
    entry
        .files
        .keys()
        .filter(|path| !Path::new(path).starts_with(dir))
        .map(PathBuf::from)
        .collect()
}

pub fn index_root(index: &mut ExternalIndex, dir: &Path, external: &[PathBuf]) {
    unindex_root(index, dir);
    for file in external {
        index
            .entry(file.clone())
            .or_default()
            .insert(dir.to_path_buf());
    }
}

pub fn unindex_root(index: &mut ExternalIndex, dir: &Path) {
    index.retain(|_, roots| {
        roots.remove(dir);
        !roots.is_empty()
    });
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
    /// One serialized event stream per attached client.
    pub event_txs: Vec<tokio::sync::mpsc::Sender<String>>,
}

/// Shared engine state handed to the update and watcher pipelines.
pub struct MediaShared {
    pub state: RwLock<Option<MediaState>>,
    pub external_index: RwLock<ExternalIndex>,
    pub active_writes: Mutex<HashSet<PathBuf>>,
    /// One writer per state file: the pass and the watcher both submit through
    /// here instead of loading, mutating and saving on their own.
    state_writer: Mutex<()>,
    pub update_lock: Mutex<UpdateState>,
}

impl Default for MediaShared {
    fn default() -> Self {
        Self {
            state: RwLock::new(None),
            external_index: RwLock::new(ExternalIndex::new()),
            active_writes: Mutex::new(HashSet::new()),
            state_writer: Mutex::new(()),
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
            external_index: RwLock::new(
                self.external_index
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
            state_writer: Mutex::new(()),
            update_lock: Mutex::new(UpdateState::default()),
        }
    }
}

impl MediaShared {
    /// Records a compiled root entity: the whole `files` map is replaced, so
    /// nothing is carried over from the previous pass.
    pub fn record_root(
        &self,
        state: &MediaState,
        media_type: &str,
        dir: &Path,
        files: BTreeMap<String, FileStat>,
    ) -> Result<()> {
        self.write_state(state, media_type, |states| {
            states.insert(dir.to_string_lossy().to_string(), RootState { files });
        })
    }

    /// Drops a root entity's state entry. Its lock file is left alone.
    pub fn remove_root(
        &self,
        state: &MediaState,
        media_type: &str,
        dir: &Path,
    ) -> Result<()> {
        self.write_state(state, media_type, |states| {
            states.remove(&dir.to_string_lossy().to_string());
        })
    }

    /// Convenience wrapper for the common "compile finished" case.
    pub fn record_compiled(
        &self,
        state: &MediaState,
        media_type: &str,
        dir: &Path,
        external: &[PathBuf],
    ) -> Result<()> {
        self.record_root(state, media_type, dir, root_files(dir, external))
    }

    fn write_state(
        &self,
        state: &MediaState,
        media_type: &str,
        mutate: impl FnOnce(&mut RootStates),
    ) -> Result<()> {
        let Some(init) = state.init_of(media_type) else {
            anyhow::bail!("media type '{media_type}' has no init");
        };
        let path = crate::state::state_path(
            &state.cache_root,
            &state.config_path,
            media_type,
            &init,
        );

        let _guard = self
            .state_writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut states = load_state(&path)?;
        mutate(&mut states);
        save_state(&states, &path)?;

        Ok(())
    }
}

pub type SharedRef = Arc<MediaShared>;
