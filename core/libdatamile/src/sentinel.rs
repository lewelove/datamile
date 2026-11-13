#[cfg(test)]
mod tests;

use crate::error::DatamileError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum TrustState {
    Valid,
    Missing,
    BrokenIntent,
    BrokenPhysics,
    BrokenAssets,
}

pub fn verify_trust(
    album_root: &Path,
    expected_id: Option<&str>,
) -> Result<TrustState, DatamileError> {
    let lock_path = album_root.join("album.lock.json");
    if !lock_path.exists() {
        return Ok(TrustState::Missing);
    }

    let lock_content =
        fs::read_to_string(&lock_path).map_err(DatamileError::ManifestIoError)?;

    let lock_json: serde_json::Value =
        serde_json::from_str(&lock_content).map_err(DatamileError::JsonError)?;

    let Some(album_data) = lock_json.get("album") else {
        return Ok(TrustState::Missing);
    };

    if let Some(expected) = expected_id {
        let lock_id = album_data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if lock_id != expected {
            return Ok(TrustState::BrokenIntent);
        }
    }

    if check_manifest_mtimes(album_root, album_data) == TrustState::BrokenIntent {
        return Ok(TrustState::BrokenIntent);
    }

    if check_image_integrity(album_root, album_data) == TrustState::BrokenAssets {
        return Ok(TrustState::BrokenAssets);
    }

    if check_tracks_integrity(album_root, &lock_json) == TrustState::BrokenPhysics {
        return Ok(TrustState::BrokenPhysics);
    }

    Ok(TrustState::Valid)
}

fn check_manifest_mtimes(
    album_root: &Path,
    album_data: &serde_json::Value,
) -> TrustState {
    if let Some(manifests) = album_data
        .pointer("/objects/manifests")
        .and_then(serde_json::Value::as_object)
    {
        for m in manifests.values() {
            if let Some(file) = m.get("file") {
                let rel_path = file
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let lock_mtime = file
                    .get("mtime")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let abs_path = if Path::new(rel_path).is_absolute() {
                    PathBuf::from(rel_path)
                } else {
                    album_root.join(rel_path)
                };

                let current_mtime = fs::metadata(&abs_path)
                    .and_then(|meta| meta.modified())
                    .map_or(0, |t| {
                        t.duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    });

                if current_mtime != lock_mtime && lock_mtime != 0 {
                    return TrustState::BrokenIntent;
                }
            }
        }
    }
    TrustState::Valid
}

fn check_image_integrity(
    album_root: &Path,
    album_data: &serde_json::Value,
) -> TrustState {
    if let Some(images) = album_data
        .get("images")
        .and_then(serde_json::Value::as_object)
    {
        for entries in images.values().filter_map(serde_json::Value::as_array) {
            for entry in entries {
                let Some(file) = entry.pointer("/source/file") else {
                    continue;
                };
                let path_str = file
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if path_str.is_empty() {
                    continue;
                }
                let abs_path = if Path::new(path_str).is_absolute() {
                    PathBuf::from(path_str)
                } else {
                    album_root.join(path_str)
                };
                if !abs_path.exists() {
                    return TrustState::BrokenAssets;
                }
                let lock_size = file
                    .get("byte_size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let current_size = fs::metadata(&abs_path).map_or(0, |m| m.len());
                if lock_size != current_size {
                    return TrustState::BrokenAssets;
                }
            }
        }
    }
    TrustState::Valid
}

fn check_tracks_integrity(
    album_root: &Path,
    lock_json: &serde_json::Value,
) -> TrustState {
    let is_virtual = lock_json
        .pointer("/album/objects/info/virtual")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    if is_virtual {
        return TrustState::Valid;
    }

    if let Some(tracks) = lock_json
        .pointer("/album/tracks")
        .and_then(serde_json::Value::as_array)
    {
        for track in tracks {
            if let Some(file) = track.get("location") {
                let rel_path = file
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if rel_path.is_empty() {
                    return TrustState::BrokenPhysics;
                }

                let abs_path = if Path::new(rel_path).is_absolute() {
                    PathBuf::from(rel_path)
                } else {
                    album_root.join(rel_path)
                };
                let Ok(meta) = fs::metadata(&abs_path) else {
                    return TrustState::BrokenPhysics;
                };

                let lock_track_mtime = file
                    .get("mtime")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let lock_track_size = file
                    .get("byte_size")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);

                let current_track_mtime = meta.modified().map_or(0, |t| {
                    t.duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                });
                let current_track_size = meta.len();

                if lock_track_mtime != current_track_mtime
                    || lock_track_size != current_track_size
                {
                    return TrustState::BrokenPhysics;
                }
            } else {
                return TrustState::BrokenPhysics;
            }
        }
    }
    TrustState::Valid
}
