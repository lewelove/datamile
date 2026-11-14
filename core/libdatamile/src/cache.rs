use std::fs;
use std::path::Path;
use std::time::SystemTime;

use serde_json::Value;

use crate::error::DatamileError;
use crate::types::toml_to_json;

pub fn read_object_cached(
    path: &Path,
    cache_root: &Path,
) -> Result<Value, DatamileError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let parse_as_json = if ext.eq_ignore_ascii_case("json") {
        true
    } else if ext.eq_ignore_ascii_case("toml") {
        false
    } else {
        return Err(DatamileError::InvalidFileExtension {
            path: path.to_path_buf(),
            expected: "json or toml".to_string(),
        });
    };

    let meta = fs::metadata(path).map_err(DatamileError::ManifestIoError)?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key_str = format!("{}:{size}:{mtime}", canon_path.display());
    let key = blake3::hash(key_str.as_bytes()).to_hex().to_string();

    let cache_dir = cache_root.join("objects");
    let cache_file = cache_dir.join(format!("{key}.json"));

    if let Ok(content) = fs::read_to_string(&cache_file) {
        if let Ok(json_val) = serde_json::from_str::<Value>(&content) {
            return Ok(json_val);
        }
        let _ = fs::remove_file(&cache_file);
    }

    let raw = fs::read_to_string(path).map_err(DatamileError::ManifestIoError)?;

    let json_val = if raw.trim().is_empty() {
        Value::Null
    } else if parse_as_json {
        serde_json::from_str::<Value>(&raw).map_err(|source| {
            DatamileError::JsonParseError {
                path: path.to_path_buf(),
                source,
            }
        })?
    } else {
        let toml_val = toml::from_str::<toml::Value>(&raw).map_err(|source| {
            DatamileError::ManifestParseError {
                path: path.to_path_buf(),
                source,
            }
        })?;
        toml_to_json(toml_val)
    };

    if let Ok(json_str) = serde_json::to_string(&json_val) {
        let _ = crate::utils::write_atomic_cache_file(&cache_file, &json_str);
    }

    Ok(json_val)
}
