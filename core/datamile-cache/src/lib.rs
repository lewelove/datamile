use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

pub mod stat;

#[cfg(test)]
mod tests;

pub use stat::{
    EntityKey, FileStat, RootState, RootStates, calculate_path_hash, hash_files,
    load_state, root_files, root_is_dirty, save_state, walk_files,
};

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Invalid file extension: Expected '{expected}' in {path}")]
    InvalidFileExtension { path: PathBuf, expected: String },

    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse Error: Failed to parse JSON in {path}: {source}")]
    JsonParse {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("Parse Error: Failed to parse TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("Serialization Error: {0}")]
    JsonSerialize(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectFormat {
    Json,
    Toml,
}

/// Reads a JSON or TOML document, parsing it at most once per
/// `(canonical path, size, mtime)`.
pub fn read_object_cached(path: &Path, cache_root: &Path) -> Result<Value, CacheError> {
    let format = object_format(path)?;

    read_or_compute_cached(path, cache_root, "objects", |path| {
        parse_object(path, format)
    })
}

fn object_format(path: &Path) -> Result<ObjectFormat, CacheError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    if ext.eq_ignore_ascii_case("json") {
        return Ok(ObjectFormat::Json);
    }

    if ext.eq_ignore_ascii_case("toml") {
        return Ok(ObjectFormat::Toml);
    }

    Err(CacheError::InvalidFileExtension {
        path: path.to_path_buf(),
        expected: "json or toml".to_string(),
    })
}

/// Returns the cached value for `path` under `<cache_root>/<namespace>`, or
/// computes and persists it.
///
/// The key covers the canonical path plus size and nanosecond mtime, so any
/// edit invalidates the entry. A cache blob that no longer deserializes as `T`
/// is dropped and rebuilt from `compute`.
pub fn read_or_compute_cached<T, E, F>(
    path: &Path,
    cache_root: &Path,
    namespace: &str,
    compute: F,
) -> Result<T, E>
where
    T: Serialize + DeserializeOwned,
    E: From<std::io::Error>,
    F: FnOnce(&Path) -> Result<T, E>,
{
    let stat = FileStat::from(&fs::metadata(path)?);
    let cache_file = cache_root
        .join(namespace)
        .join(format!("{}.json", disk_cache_key(path, stat)));

    if let Ok(content) = fs::read_to_string(&cache_file) {
        if let Ok(cached) = serde_json::from_str::<T>(&content) {
            return Ok(cached);
        }
        let _ = fs::remove_file(&cache_file);
    }

    let computed = compute(path)?;

    if let Ok(json_str) = serde_json::to_string(&computed) {
        let _ = datamile_fs::write_atomic(&cache_file, &json_str);
    }

    Ok(computed)
}

fn disk_cache_key(path: &Path, stat: FileStat) -> String {
    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key_str = format!("{}:{}:{}", canon_path.display(), stat.size, stat.mtime);

    blake3::hash(key_str.as_bytes()).to_hex().to_string()
}

fn parse_object(path: &Path, format: ObjectFormat) -> Result<Value, CacheError> {
    let raw = fs::read_to_string(path)?;

    if raw.trim().is_empty() {
        return Ok(Value::Null);
    }

    match format {
        ObjectFormat::Json => {
            serde_json::from_str::<Value>(&raw).map_err(|source| CacheError::JsonParse {
                path: path.to_path_buf(),
                source,
            })
        }
        ObjectFormat::Toml => {
            let toml_val = toml::from_str::<toml::Value>(&raw).map_err(|source| {
                CacheError::TomlParse {
                    path: path.to_path_buf(),
                    source,
                }
            })?;

            Ok(datamile_fs::toml_to_json(toml_val))
        }
    }
}
