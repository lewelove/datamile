use std::path::Path;

use serde_json::Value;

use crate::error::DatamileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlbumKind {
    Physical,
    Virtual,
}

pub(crate) fn is_virtual_album(
    parsed_manifests: &serde_json::Map<String, Value>,
) -> AlbumKind {
    let is_virt = parsed_manifests
        .get("virtual")
        .and_then(|v| v.get("album"))
        .and_then(|a| a.get("virtual"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if is_virt {
        AlbumKind::Virtual
    } else {
        AlbumKind::Physical
    }
}

pub(crate) fn validate_mandatory_fields(
    primary_album: &Value,
    album_root: &Path,
) -> Result<(), DatamileError> {
    let check_str = |k: &str| -> Result<(), DatamileError> {
        let v = primary_album.get(k);
        if let Some(s) = v.and_then(Value::as_str) {
            if !s.is_empty() {
                return Ok(());
            }
        } else if v.and_then(Value::as_number).is_some() {
            return Ok(());
        }
        Err(DatamileError::TypeMismatch {
            path: album_root.to_path_buf(),
            key: k.to_string(),
            expected_type: "string".to_string(),
            found_val: "missing or empty".to_string(),
        })
    };

    check_str("albumartist").or_else(|_| check_str("artist"))?;
    check_str("album")?;
    check_str("date")
}
