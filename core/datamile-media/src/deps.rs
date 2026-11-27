use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use anyhow::Result;
use datamile_cache::{
    EntityKey, FileStat, RootState, RootStates, load_state, root_files, save_state,
};

use crate::compiler::CompiledRoot;
use crate::state::MediaState;

/// Reverse index `external file -> root entities that read it`, rebuilt at boot
/// from the state files and kept in memory for the watcher. Nothing on disk
/// duplicates it.
pub type ExternalIndex = BTreeMap<PathBuf, BTreeSet<EntityKey>>;

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
            let key = EntityKey {
                media_type: media_type.clone(),
                dir: PathBuf::from(dir),
            };
            for file in external_files(&key.dir, entry) {
                index.entry(file).or_default().insert(key.clone());
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

pub fn index_entity(index: &mut ExternalIndex, key: &EntityKey, external: &[PathBuf]) {
    unindex_entity(index, key);
    for file in external {
        index.entry(file.clone()).or_default().insert(key.clone());
    }
}

pub fn unindex_entity(index: &mut ExternalIndex, key: &EntityKey) {
    index.retain(|_, keys| {
        keys.remove(key);
        !keys.is_empty()
    });
}

/// What one update pass must cover. `media_type` narrows a pass that already
/// covers whole media roots; explicit entity keys widen nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateScope {
    All {
        media_type: Option<String>,
    },
    Paths {
        paths: HashSet<PathBuf>,
        media_type: Option<String>,
    },
    Entities(BTreeSet<EntityKey>),
}

impl UpdateScope {
    /// Coalesces a queued request into this one, keeping the narrowest scope
    /// that covers both.
    pub fn merge(&mut self, other: Self) {
        // The placeholder is never read: it only hands `merged` both sides by
        // value.
        *self = merged(
            std::mem::replace(self, Self::All { media_type: None }),
            other,
        );
    }
}

/// The merge matrix: two scopes of one shape stay narrow while they agree on a
/// media type, and any disagreement widens to a full pass, which is always a
/// safe superset.
fn merged(existing: UpdateScope, incoming: UpdateScope) -> UpdateScope {
    match (existing, incoming) {
        (UpdateScope::All { media_type: None }, _)
        | (_, UpdateScope::All { media_type: None }) => {
            UpdateScope::All { media_type: None }
        }

        (
            UpdateScope::All {
                media_type: Some(a),
            },
            UpdateScope::All {
                media_type: Some(b),
            },
        ) => UpdateScope::All {
            media_type: (a == b).then_some(a),
        },

        (
            UpdateScope::All {
                media_type: Some(a),
            },
            UpdateScope::Paths { media_type: b, .. },
        )
        | (
            UpdateScope::Paths { media_type: b, .. },
            UpdateScope::All {
                media_type: Some(a),
            },
        ) => UpdateScope::All {
            media_type: (b.as_deref() == Some(a.as_str())).then_some(a),
        },

        (
            UpdateScope::All {
                media_type: Some(a),
            },
            UpdateScope::Entities(keys),
        )
        | (
            UpdateScope::Entities(keys),
            UpdateScope::All {
                media_type: Some(a),
            },
        ) => collapsed_to_type(&a, &keys),

        (
            UpdateScope::Paths {
                paths: mut pa,
                media_type: ma,
            },
            UpdateScope::Paths {
                paths: pb,
                media_type: mb,
            },
        ) => {
            if ma == mb {
                pa.extend(pb);
                UpdateScope::Paths {
                    paths: pa,
                    media_type: ma,
                }
            } else {
                UpdateScope::All { media_type: None }
            }
        }

        (UpdateScope::Paths { media_type, .. }, UpdateScope::Entities(keys))
        | (UpdateScope::Entities(keys), UpdateScope::Paths { media_type, .. }) => {
            media_type.map_or(UpdateScope::All { media_type: None }, |media_type| {
                collapsed_to_type(&media_type, &keys)
            })
        }

        (UpdateScope::Entities(mut ea), UpdateScope::Entities(eb)) => {
            ea.extend(eb);
            UpdateScope::Entities(ea)
        }
    }
}

/// Entity keys collapse into one media type only when all of them share it.
fn collapsed_to_type(media_type: &str, keys: &BTreeSet<EntityKey>) -> UpdateScope {
    let all_match = keys.iter().all(|key| key.media_type == media_type);

    UpdateScope::All {
        media_type: all_match.then(|| media_type.to_string()),
    }
}

#[derive(Default)]
pub struct UpdateState {
    pub is_running: bool,
    pub pending: Option<PendingUpdate>,
}

pub struct PendingUpdate {
    pub scope: UpdateScope,
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

/// The fingerprints of one root entity: its walked files plus the external ones.
pub type RootFiles = BTreeMap<String, FileStat>;

/// One pending state entry: the root entity's directory, its file map, and the
/// stat of the lock the pass wrote or verified.
pub type StateEntry = (PathBuf, RootFiles, FileStat);

/// Pending state entries, grouped by media type.
pub type StateEntries = BTreeMap<String, Vec<StateEntry>>;

/// The state entries for a batch of compiled roots, grouped by media type, so
/// one write covers every root of a media type instead of one write per root.
///
/// The file maps are built here, outside the writer lock, so the walks do not
/// hold it.
#[must_use]
pub fn state_entries(compiled: &[CompiledRoot]) -> StateEntries {
    let mut grouped = StateEntries::new();

    for root in compiled {
        grouped.entry(root.media_type.clone()).or_default().push((
            root.dir.clone(),
            root_files(&root.dir, &root.dependencies),
            root.lock_stat,
        ));
    }

    grouped
}

impl MediaShared {
    /// Records a batch of compiled root entities of one media type with a single
    /// load and save of that media type's state file.
    ///
    /// Each entry replaces the root entity's whole `files` map, so nothing is
    /// carried over from the previous pass.
    pub fn record_roots(
        &self,
        state: &MediaState,
        media_type: &str,
        entries: Vec<StateEntry>,
    ) -> Result<()> {
        self.write_state(state, media_type, |states| {
            for (dir, files, lock_stat) in entries {
                let entry = RootState {
                    files,
                    lock_stat: Some(lock_stat),
                };
                states.insert(dir.to_string_lossy().to_string(), entry);
            }
        })
    }

    /// Drops a batch of root entity state entries. Their lock files are left
    /// alone.
    pub fn remove_roots(
        &self,
        state: &MediaState,
        media_type: &str,
        dirs: &[&Path],
    ) -> Result<()> {
        self.write_state(state, media_type, |states| {
            for dir in dirs {
                states.remove(&dir.to_string_lossy().to_string());
            }
        })
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
