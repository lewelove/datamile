use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use tempfile::NamedTempFile;

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

pub fn file_info(
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

pub fn write_atomic_cache_file(cache_file: &Path, content: &str) -> std::io::Result<()> {
    let parent = cache_file.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid cache file path: no parent directory",
        )
    })?;

    std::fs::create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(content.as_bytes())?;
    temp.persist(cache_file).map_err(|e| e.error)?;

    Ok(())
}
