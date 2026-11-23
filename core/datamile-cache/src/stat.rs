use std::collections::HashMap;
use std::fs;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CacheError;

#[cfg(test)]
mod tests;

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

/// Reads a stat cache, treating a missing or unreadable file as empty.
#[must_use]
pub fn load_stat_cache(path: &Path) -> HashMap<String, FileStat> {
    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cache) = serde_json::from_str::<HashMap<String, FileStat>>(&content)
    {
        return cache;
    }

    HashMap::new()
}

pub fn save_stat_cache<S: BuildHasher>(
    cache: &HashMap<String, FileStat, S>,
    path: &Path,
) -> Result<(), CacheError> {
    let content = serde_json::to_string_pretty(cache)?;
    datamile_fs::write_atomic(path, &content)?;

    Ok(())
}
