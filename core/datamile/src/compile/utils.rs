use serde_json::{Value, json};

/// Lua's empty table can reach us as an empty array; only a real object is data.
pub fn lua_object(value: Option<&Value>) -> Value {
    match value {
        Some(v @ Value::Object(_)) => v.clone(),
        _ => json!({}),
    }
}

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
