#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::DatamileError;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum TrustState {
    Valid,
    Missing,
    BrokenIntent,
    BrokenPhysics,
    BrokenAssets,
}

/// Verify a compiled lock for any root type. Relative provenance resolves
/// against the lock's own directory.
pub fn verify_trust(
    lock_path: &Path,
    root_type: &str,
    expected_id: Option<&str>,
) -> Result<TrustState, DatamileError> {
    if !lock_path.exists() {
        return Ok(TrustState::Missing);
    }

    let lock_content =
        fs::read_to_string(lock_path).map_err(DatamileError::ManifestIoError)?;

    let lock_json: serde_json::Value =
        serde_json::from_str(&lock_content).map_err(DatamileError::JsonError)?;

    let Some(entity) = lock_json.get(root_type) else {
        return Ok(TrustState::Missing);
    };

    let entity_root = lock_path.parent().unwrap_or(lock_path);

    if let Some(expected) = expected_id {
        let lock_id = entity
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if lock_id != expected {
            return Ok(TrustState::BrokenIntent);
        }
    }

    if check_manifest_mtimes(entity_root, entity) == TrustState::BrokenIntent {
        return Ok(TrustState::BrokenIntent);
    }

    if check_files(entity_root, entity) == TrustState::BrokenIntent {
        return Ok(TrustState::BrokenIntent);
    }

    if check_image_integrity(entity_root, entity) == TrustState::BrokenAssets {
        return Ok(TrustState::BrokenAssets);
    }

    if is_virtual(entity) {
        return Ok(TrustState::Valid);
    }

    if check_physics(entity, entity_root) == TrustState::BrokenPhysics {
        return Ok(TrustState::BrokenPhysics);
    }

    Ok(TrustState::Valid)
}

fn is_virtual(entity: &serde_json::Value) -> bool {
    entity
        .pointer("/objects/info/virtual")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn check_manifest_mtimes(entity_root: &Path, entity: &serde_json::Value) -> TrustState {
    if let Some(manifests) = entity
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
                let abs_path = resolve(entity_root, rel_path);

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

/// Attached files live under `files.<name>` as one entry or a list.
fn check_files(entity_root: &Path, entity: &serde_json::Value) -> TrustState {
    if let Some(files) = entity.get("files").and_then(serde_json::Value::as_object) {
        for value in files.values() {
            if check_file_entries(entity_root, value) == TrustState::BrokenIntent {
                return TrustState::BrokenIntent;
            }
        }
    }

    TrustState::Valid
}

fn check_file_entries(entity_root: &Path, value: &serde_json::Value) -> TrustState {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                if check_file_entry(entity_root, item) == TrustState::BrokenIntent {
                    return TrustState::BrokenIntent;
                }
            }
            TrustState::Valid
        }
        other => check_file_entry(entity_root, other),
    }
}

fn check_file_entry(entity_root: &Path, entry: &serde_json::Value) -> TrustState {
    let rel_path = entry
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if rel_path.is_empty() {
        return TrustState::BrokenIntent;
    }

    let abs_path = resolve(entity_root, rel_path);
    let Ok(meta) = fs::metadata(&abs_path) else {
        return TrustState::BrokenIntent;
    };

    let lock_mtime = entry
        .get("mtime")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let current_mtime = meta.modified().map_or(0, |t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });

    if lock_mtime != 0 && lock_mtime != current_mtime {
        return TrustState::BrokenIntent;
    }

    let lock_size = entry
        .get("byte_size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    if lock_size != 0 && lock_size != meta.len() {
        return TrustState::BrokenIntent;
    }

    TrustState::Valid
}

fn check_image_integrity(entity_root: &Path, entity: &serde_json::Value) -> TrustState {
    if let Some(images) = entity.get("images").and_then(serde_json::Value::as_object) {
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
                let abs_path = resolve(entity_root, path_str);
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

/// Walk every compiled entity: a parent location plus any child arrays whose
/// elements carry their own provenance.
fn check_physics(entity: &serde_json::Value, entity_root: &Path) -> TrustState {
    match entity {
        serde_json::Value::Object(map) => {
            if let Some(location) = map.get("location")
                && !location_matches(location, entity_root)
            {
                return TrustState::BrokenPhysics;
            }

            for (key, value) in map {
                if matches!(key.as_str(), "location" | "images" | "tags" | "objects") {
                    continue;
                }
                if check_physics(value, entity_root) == TrustState::BrokenPhysics {
                    return TrustState::BrokenPhysics;
                }
            }
            TrustState::Valid
        }
        serde_json::Value::Array(items) => {
            for item in items {
                // Leaf entities are physical; containers delegate to their children.
                let missing_location = item.get("id").is_some()
                    && item.get("location").is_none()
                    && !has_child_entities(item);
                if missing_location
                    || check_physics(item, entity_root) == TrustState::BrokenPhysics
                {
                    return TrustState::BrokenPhysics;
                }
            }
            TrustState::Valid
        }
        _ => TrustState::Valid,
    }
}

fn has_child_entities(item: &serde_json::Value) -> bool {
    item.as_object().is_some_and(|map| {
        map.iter().any(|(key, value)| {
            !matches!(key.as_str(), "location" | "images" | "tags" | "objects")
                && value
                    .as_array()
                    .is_some_and(|items| items.iter().any(serde_json::Value::is_object))
        })
    })
}

fn location_matches(location: &serde_json::Value, entity_root: &Path) -> bool {
    let rel_path = location
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if rel_path.is_empty() {
        return false;
    }

    let abs_path = resolve(entity_root, rel_path);
    let Ok(meta) = fs::metadata(&abs_path) else {
        return false;
    };

    let kind = location
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let kind_matches = match kind {
        "dir" => meta.is_dir(),
        "file" => meta.is_file(),
        _ => false,
    };
    if !kind_matches {
        return false;
    }

    let lock_mtime = location
        .get("mtime")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let current_mtime = meta.modified().map_or(0, |t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    });

    if lock_mtime != 0 && lock_mtime != current_mtime {
        return false;
    }

    let lock_size = location
        .get("byte_size")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    lock_size == 0 || lock_size == meta.len()
}

fn resolve(entity_root: &Path, path_str: &str) -> PathBuf {
    if Path::new(path_str).is_absolute() {
        PathBuf::from(path_str)
    } else {
        entity_root.join(path_str)
    }
}
