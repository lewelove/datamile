#[cfg(test)]
mod tests;

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
