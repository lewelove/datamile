use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use tempfile::NamedTempFile;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashMode {
    Compute,
    Skip,
}

#[must_use]
pub fn expand_path(path_str: &str) -> PathBuf {
    if path_str.starts_with('~')
        && let Some(home) = dirs::home_dir()
    {
        if path_str == "~" {
            return home;
        }
        if let Some(stripped) = path_str.strip_prefix("~/") {
            return home.join(stripped);
        }
    }
    PathBuf::from(path_str)
}

#[must_use]
pub fn resolve_path(path_str: &str, config_dir: &Path) -> PathBuf {
    let expanded = expand_path(path_str);
    if expanded.is_absolute() {
        return expanded;
    }

    let direct = config_dir.join(&expanded);
    if direct.exists() {
        return direct;
    }

    if let Ok(entries) = std::fs::read_dir(config_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                let sub_candidate = entry.path().join(&expanded);
                if sub_candidate.exists() {
                    return sub_candidate;
                }
            }
        }
    }

    direct
}

pub fn read_file_info(
    path: &std::path::Path,
    rel_path: &str,
    hash_mode: HashMode,
) -> Result<serde_json::Value, anyhow::Error> {
    let m = std::fs::metadata(path)?;
    let mtime = m
        .modified()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs();
    let byte_size = m.len();

    let hash_val = match hash_mode {
        HashMode::Compute => {
            let content = std::fs::read(path)?;
            let hash = blake3::hash(&content);
            let raw = hash.as_bytes();
            let b64 = STANDARD.encode(raw);
            let string = format!("blake3-{b64}");
            let b64_url = URL_SAFE_NO_PAD.encode(raw);
            let address: String = b64_url.chars().take(16).collect();
            serde_json::json!({
                "string": string,
                "address": address
            })
        }
        HashMode::Skip => serde_json::Value::Null,
    };

    Ok(serde_json::json!({
        "path": rel_path,
        "hash": hash_val,
        "mtime": mtime,
        "byte_size": byte_size
    }))
}

#[must_use]
pub fn calculate_blake3_address(content: &[u8]) -> String {
    let hash = blake3::hash(content);
    let raw = hash.as_bytes();
    URL_SAFE_NO_PAD.encode(raw).chars().take(16).collect()
}

/// Renders a lock tag as display text.
///
/// Tags keep their JSON type, so `album = 1984` stays a number and `genre` may be
/// a list. Anything that renders those as strings only would show `Unknown` for a
/// perfectly good value, so numbers and booleans are stringified and scalar lists
/// are joined.
#[must_use]
pub fn tag_text(value: Option<&serde_json::Value>) -> Option<String> {
    fn scalar_text(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    match value? {
        serde_json::Value::Array(items) => {
            let joined = items
                .iter()
                .filter_map(scalar_text)
                .collect::<Vec<_>>()
                .join(", ");

            if joined.is_empty() {
                return None;
            }
            Some(joined)
        }
        other => scalar_text(other),
    }
}

/// Writes bytes through a temp file in the same directory, then renames.
///
/// A reader must never observe a half-written cache file: `path.exists()` only
/// becomes true once the whole content is on disk.
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid file path: no parent directory",
        )
    })?;

    std::fs::create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(bytes.as_ref())?;
    temp.persist(path).map_err(|e| e.error)?;

    Ok(())
}
