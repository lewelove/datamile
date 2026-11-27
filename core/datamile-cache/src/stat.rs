use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CacheError;

#[cfg(test)]
mod tests;

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
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
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
}

/// One media root's state file, keyed by root entity directory.
pub type RootStates = BTreeMap<String, RootState>;

/// Every file under `dir`, keyed by absolute path.
#[must_use]
pub fn walk_files(dir: &Path) -> BTreeMap<String, FileStat> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
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

/// The dirty rule: no state entry, a changed walked set, or a changed or missing
/// external file.
#[must_use]
pub fn root_is_dirty(
    dir: &Path,
    walked: &BTreeMap<String, FileStat>,
    entry: Option<&RootState>,
) -> bool {
    let Some(entry) = entry else {
        return true;
    };

    let internal = entry
        .files
        .keys()
        .filter(|path| Path::new(path).starts_with(dir))
        .count();

    if internal != walked.len() {
        return true;
    }

    let walked_changed = walked
        .iter()
        .any(|(path, stat)| entry.files.get(path) != Some(stat));
    if walked_changed {
        return true;
    }

    entry
        .files
        .iter()
        .filter(|(path, _)| !Path::new(path).starts_with(dir))
        .any(|(path, stat)| {
            fs::metadata(path).map_or(true, |meta| FileStat::from(&meta) != *stat)
        })
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
