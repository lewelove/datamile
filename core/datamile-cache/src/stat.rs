use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CacheError;

#[cfg(test)]
mod tests;

/// Suffix of a compiled lock file: one lock per media type per entity dir.
pub const LOCK_SUFFIX: &str = ".lock.json";

/// Whether `path` names a compiled lock file.
///
/// A lock is always an output: [`walk_files`] skips these and a compile may not
/// read one, so a lock can never feed the next lock.
#[must_use]
pub fn is_lock_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().ends_with(LOCK_SUFFIX))
}

/// The unique identity of one compiled entity: its media type and entity dir.
///
/// One directory may hold `<media_type>.lock.json` for several media types, so
/// the directory alone never identifies an entity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityKey {
    pub media_type: String,
    /// Always canonicalized before storage or comparison.
    pub dir: PathBuf,
}

/// Cheap change detector for a file: its nanosecond mtime and its size.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct FileStat {
    pub mtime: u128,
    pub size: u64,
}

impl From<&std::fs::Metadata> for FileStat {
    fn from(meta: &std::fs::Metadata) -> Self {
        let mtime = meta
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        Self {
            mtime,
            size: meta.len(),
        }
    }
}

#[must_use]
pub fn calculate_path_hash(path: &Path) -> String {
    blake3::hash(path.as_os_str().as_encoded_bytes())
        .to_hex()
        .to_string()
}

/// Digest over the contents of `paths`, insensitive to the order they arrive
/// in but sensitive to any file changing.
#[must_use]
pub fn hash_files(paths: &[PathBuf]) -> String {
    let mut sorted = paths.to_vec();
    sorted.sort();

    let mut hasher = blake3::Hasher::new();
    for path in &sorted {
        if let Ok(content) = fs::read(path) {
            hasher.update(&content);
        }
    }

    hasher.finalize().to_hex().to_string()
}

/// Fingerprints for one root entity: the walked files plus the external files.
///
/// A path outside the root entity's directory is an external dependency; root
/// entities cannot nest, so every path is internal to at most one of them.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
pub struct RootState {
    pub files: BTreeMap<String, FileStat>,
    /// Stat of `<dir>/<media_type>.lock.json` at the end of the last pass.
    ///
    /// `None` means the entry predates lock tracking, so the root recompiles
    /// once after an upgrade.
    #[serde(default)]
    pub lock_stat: Option<FileStat>,
}

/// One media root's state file, keyed by root entity directory.
pub type RootStates = BTreeMap<String, RootState>;

/// Every source file under `dir`, keyed by absolute path.
///
/// Compiled locks are skipped: they are outputs, so they never fingerprint an
/// entity and adding one must not make the entity dirty.
#[must_use]
pub fn walk_files(dir: &Path) -> BTreeMap<String, FileStat> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && !is_lock_file(entry.path()))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((
                entry.path().to_string_lossy().to_string(),
                FileStat::from(&meta),
            ))
        })
        .collect()
}

/// The full fingerprint map for a root entity.
///
/// Its walked files plus the stat of every external file it read. An external
/// file that no longer exists is dropped, which makes the next pass retry the
/// compile.
#[must_use]
pub fn root_files(dir: &Path, external: &[PathBuf]) -> BTreeMap<String, FileStat> {
    let mut files = walk_files(dir);
    for dep in external {
        if let Ok(meta) = fs::metadata(dep) {
            files.insert(dep.to_string_lossy().to_string(), FileStat::from(&meta));
        }
    }
    files
}

/// The dirty rule: no state entry, no recorded lock stat, a changed walked set,
/// a changed or missing external file, or a missing or changed lock on disk.
#[must_use]
pub fn root_is_dirty(
    dir: &Path,
    media_type: &str,
    walked: &BTreeMap<String, FileStat>,
    entry: Option<&RootState>,
) -> bool {
    let Some(entry) = entry else {
        return true;
    };
    let Some(lock_stat) = entry.lock_stat else {
        return true;
    };

    walked_changed(dir, walked, entry)
        || external_changed(dir, entry)
        || lock_changed(dir, media_type, lock_stat)
}

/// The walked set must match the internal part of the recorded map exactly.
fn walked_changed(
    dir: &Path,
    walked: &BTreeMap<String, FileStat>,
    entry: &RootState,
) -> bool {
    let internal = entry
        .files
        .keys()
        .filter(|path| Path::new(path).starts_with(dir))
        .count();

    if internal != walked.len() {
        return true;
    }

    walked
        .iter()
        .any(|(path, stat)| entry.files.get(path) != Some(stat))
}

/// An external file that changed or vanished invalidates the root.
fn external_changed(dir: &Path, entry: &RootState) -> bool {
    entry
        .files
        .iter()
        .filter(|(path, _)| !Path::new(path).starts_with(dir))
        .any(|(path, stat)| {
            fs::metadata(path).map_or(true, |meta| FileStat::from(&meta) != *stat)
        })
}

/// A deleted or hand-edited lock means the artifact is not the one this entry
/// recorded, whatever the source files say.
fn lock_changed(dir: &Path, media_type: &str, recorded: FileStat) -> bool {
    let path = dir.join(format!("{media_type}{LOCK_SUFFIX}"));

    fs::metadata(path).map_or(true, |meta| FileStat::from(&meta) != recorded)
}

/// Reads a state file. A missing file is empty; an unparsable one is an error,
/// because treating it as empty would report every root entity as removed.
pub fn load_state(path: &Path) -> Result<RootStates, CacheError> {
    match fs::read_to_string(path) {
        Ok(content) => {
            serde_json::from_str(&content).map_err(|source| CacheError::JsonParse {
                path: path.to_path_buf(),
                source,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RootStates::new()),
        Err(e) => Err(e.into()),
    }
}

/// Writes a state file atomically, sorted, and skipped when the bytes match.
pub fn save_state(states: &RootStates, path: &Path) -> Result<(), CacheError> {
    let content = serde_json::to_string_pretty(states)?;
    if fs::read_to_string(path).is_ok_and(|old| old == content) {
        return Ok(());
    }
    datamile_fs::write_atomic(path, &content)?;

    Ok(())
}
