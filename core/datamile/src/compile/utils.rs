use std::path::Path;

use libdatamile::error::DatamileError;
use serde_json::{Value, json};

pub fn strip_empty_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                strip_empty_values(v);
            }
            map.retain(|_, v| match v {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_empty_values(v);
            }
            arr.retain(|v| match v {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
        }
        _ => {}
    }
}

pub fn sort_json_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                std::mem::take(map).into_iter().collect();
            entries.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
            for (k, mut v) in entries {
                sort_json_keys(&mut v);
                map.insert(k, v);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                sort_json_keys(v);
            }
        }
        _ => {}
    }
}

pub fn is_valid_hex_color(s: &str) -> bool {
    if !s.starts_with('#') {
        return false;
    }
    let hex = &s[1..];
    let len = hex.len();
    if len != 3 && len != 4 && len != 6 && len != 8 {
        return false;
    }
    hex.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn validate_and_format_colors(
    colors: &Value,
    album_root: &Path,
) -> Result<Value, DatamileError> {
    let mut formatted_colors = serde_json::Map::new();

    if let Some(fg) = colors.get("foreground")
        && let Some(s) = fg.as_str()
    {
        if !is_valid_hex_color(s) {
            return Err(DatamileError::InvalidColorFormat {
                path: album_root.to_path_buf(),
                key: "foreground".to_string(),
                found: s.to_string(),
            });
        }
        formatted_colors.insert("foreground".to_string(), json!(s));
    }

    if let Some(bg) = colors.get("background") {
        if let Some(arr) = bg.as_array() {
            let mut bg_arr = Vec::new();
            for (i, v) in arr.iter().enumerate() {
                if let Some(s) = v.as_str() {
                    if !is_valid_hex_color(s) {
                        return Err(DatamileError::InvalidColorFormat {
                            path: album_root.to_path_buf(),
                            key: format!("background[{i}]"),
                            found: s.to_string(),
                        });
                    }
                    bg_arr.push(s.to_string());
                }
            }
            formatted_colors.insert("background".to_string(), json!(bg_arr));
        } else if let Some(s) = bg.as_str() {
            if !is_valid_hex_color(s) {
                return Err(DatamileError::InvalidColorFormat {
                    path: album_root.to_path_buf(),
                    key: "background".to_string(),
                    found: s.to_string(),
                });
            }
            formatted_colors.insert("background".to_string(), json!(vec![s]));
        }
    }

    Ok(Value::Object(formatted_colors))
}
