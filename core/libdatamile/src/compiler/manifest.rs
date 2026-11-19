use crate::error::DatamileError;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;

pub const BUILTIN_MANIFESTS: &[&str] = &["metadata", "theme", "virtual"];

pub fn validate_and_filter_manifest_names(
    names: &[String],
) -> Result<Vec<String>, DatamileError> {
    let mut seen = HashSet::with_capacity(names.len());
    let mut result = Vec::with_capacity(names.len());

    for name in names {
        if name.is_empty() {
            return Err(DatamileError::InvalidManifestName {
                name: name.clone(),
                reason: "Manifest name cannot be empty".to_string(),
            });
        }

        if !name
            .chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        {
            return Err(DatamileError::InvalidManifestName {
                name: name.clone(),
                reason: "Manifest name must contain only lowercase alphanumeric characters and underscores".to_string(),
            });
        }

        if !seen.insert(name.as_str()) {
            return Err(DatamileError::DuplicateManifestName { name: name.clone() });
        }

        if !BUILTIN_MANIFESTS.contains(&name.as_str()) {
            result.push(name.clone());
        }
    }

    Ok(result)
}

pub fn load_manifests(
    album_root: &Path,
    manifest_names: Option<&[String]>,
    cache_root: &Path,
) -> Result<serde_json::Map<String, Value>, DatamileError> {
    let metadata_path = album_root.join("album.toml");
    if !metadata_path.exists() {
        return Err(DatamileError::MissingPrimaryManifest {
            path: album_root.to_path_buf(),
        });
    }

    let mut result_manifests = serde_json::Map::new();

    let primary_json =
        parse_single_manifest(&metadata_path, album_root, "metadata", None, cache_root)?;
    let expected_tracks = primary_json
        .get("tracks")
        .and_then(|t| t.as_array())
        .map_or(0, std::vec::Vec::len);
    result_manifests.insert("metadata".to_string(), primary_json);

    let aux_names = ["theme", "virtual"]
        .into_iter()
        .chain(manifest_names.into_iter().flatten().map(String::as_str));

    for m_name in aux_names {
        let m_path = album_root.join(format!("{m_name}.toml"));
        if m_path.exists() {
            let aux_json = parse_single_manifest(
                &m_path,
                album_root,
                m_name,
                Some(expected_tracks),
                cache_root,
            )?;
            result_manifests.insert(m_name.to_string(), aux_json);
        }
    }

    Ok(result_manifests)
}

fn parse_single_manifest(
    path: &Path,
    album_root: &Path,
    name: &str,
    expected_tracks: Option<usize>,
    cache_root: &Path,
) -> Result<Value, DatamileError> {
    let mut json_val = crate::cache::read_object_cached(path, cache_root)?;

    let album_obj = json_val.get("album").cloned().unwrap_or_else(|| json!({}));

    let tracks_obj = if let Some(tracks_arr) =
        json_val.get_mut("tracks").and_then(Value::as_array_mut)
    {
        if tracks_arr.is_empty() && expected_tracks.is_some() {
            Value::Array(vec![])
        } else {
            if let Some(expected) = expected_tracks
                && tracks_arr.len() != expected
            {
                return Err(DatamileError::TrackCountMismatch {
                    manifest: name.to_string(),
                    path: album_root.to_path_buf(),
                    primary_count: expected,
                    aux_count: tracks_arr.len(),
                });
            }

            let mut tuples = Vec::new();
            let mut seen_ids = HashSet::new();

            for (idx, t) in tracks_arr.iter_mut().enumerate() {
                let track_no =
                    extract_strict_u32(t.get("tracknumber"), "tracknumber", None)
                        .map_err(|_| DatamileError::MissingTrackIdentity {
                            manifest: name.to_string(),
                            path: album_root.to_path_buf(),
                            index: idx + 1,
                        })?;
                let disc_no =
                    extract_strict_u32(t.get("discnumber"), "discnumber", Some(1))?;

                if !seen_ids.insert((disc_no, track_no)) {
                    return Err(DatamileError::DuplicateTrackIdentity {
                        manifest: name.to_string(),
                        path: album_root.to_path_buf(),
                        disc: disc_no,
                        track: track_no,
                    });
                }
                tuples.push((disc_no, track_no, t.clone()));
            }

            tuples.sort_by_key(|(d, t, _)| (*d, *t));

            let sorted_tracks: Vec<Value> =
                tuples.into_iter().map(|(_, _, val)| val).collect();
            Value::Array(sorted_tracks)
        }
    } else {
        Value::Array(vec![])
    };

    Ok(json!({
        "album": album_obj,
        "tracks": tracks_obj
    }))
}

pub fn extract_strict_u32(
    val: Option<&Value>,
    name: &str,
    default: Option<u32>,
) -> Result<u32, DatamileError> {
    let Some(v) = val else {
        return default.map_or_else(
            || {
                Err(DatamileError::InvalidIdentityFormat {
                    field: name.to_string(),
                    message: "Missing expected integer".to_string(),
                })
            },
            Ok,
        );
    };
    match v {
        Value::Number(n) => {
            n.as_u64()
                .and_then(|i| u32::try_from(i).ok())
                .ok_or_else(|| DatamileError::InvalidIdentityFormat {
                    field: name.to_string(),
                    message: "Value exceeds 32-bit integer limits".to_string(),
                })
        }
        Value::String(s) => {
            let base = s.split('/').next().unwrap_or("").trim();
            base.parse::<u32>()
                .map_err(|_| DatamileError::InvalidIdentityFormat {
                    field: name.to_string(),
                    message: format!("Cannot interpret string '{s}' as integer"),
                })
        }
        Value::Null => default.map_or_else(
            || {
                Err(DatamileError::InvalidIdentityFormat {
                    field: name.to_string(),
                    message: "Field cannot be null".to_string(),
                })
            },
            Ok,
        ),
        _ => Err(DatamileError::InvalidIdentityFormat {
            field: name.to_string(),
            message: "Unsupported data type found".to_string(),
        }),
    }
}
