use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashMode {
    Compute,
    Skip,
}

/// File metadata embedded in locks: whole seconds since the Unix epoch and the
/// byte size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    pub mtime: u64,
    pub byte_size: u64,
}

pub fn file_meta(meta: &std::fs::Metadata) -> std::io::Result<FileMeta> {
    let mtime = meta
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_secs();

    Ok(FileMeta {
        mtime,
        byte_size: meta.len(),
    })
}

/// Tagged, human-readable digest: `blake3-<standard base64>`.
#[must_use]
pub fn format_blake3_string(raw_hash: &[u8; 32]) -> String {
    format!("blake3-{}", STANDARD.encode(raw_hash))
}

/// Tagged digest of `bytes`; hashes the content when the caller does not
/// already hold it.
#[must_use]
pub fn blake3_string(bytes: &[u8]) -> String {
    format_blake3_string(blake3::hash(bytes).as_bytes())
}

pub fn read_file_info(
    path: &std::path::Path,
    rel_path: &str,
    hash_mode: HashMode,
) -> Result<serde_json::Value, anyhow::Error> {
    let m = std::fs::metadata(path)?;
    let meta = file_meta(&m)?;

    let hash_val = match hash_mode {
        HashMode::Compute => {
            let content = std::fs::read(path)?;
            let hash = blake3::hash(&content);
            let raw = hash.as_bytes();
            let address: String = URL_SAFE_NO_PAD.encode(raw).chars().take(16).collect();
            serde_json::json!({
                "string": format_blake3_string(raw),
                "address": address
            })
        }
        HashMode::Skip => serde_json::Value::Null,
    };

    Ok(serde_json::json!({
        "path": rel_path,
        "hash": hash_val,
        "mtime": meta.mtime,
        "byte_size": meta.byte_size
    }))
}

#[must_use]
pub fn calculate_blake3_address(content: &[u8]) -> String {
    let hash = blake3::hash(content);
    let raw = hash.as_bytes();
    URL_SAFE_NO_PAD.encode(raw).chars().take(16).collect()
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
