use std::collections::HashSet;

use crate::error::DatamileError;

pub const BUILTIN_MANIFESTS: &[&str] =
    &["album", "history", "id", "local", "theme", "virtual"];

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
