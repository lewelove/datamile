use std::fs;
use std::path::Path;

use anyhow::Result;

#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    name.replace(&['/', '<', '>', ':', '"', '\\', '|', '?', '*'][..], "_")
}

#[must_use]
pub fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[must_use]
pub fn toml_array(arr: &[String]) -> String {
    let escaped: Vec<String> = arr
        .iter()
        .map(|s| format!("\"{}\"", escape_toml_string(s)))
        .collect();
    format!("[{}]", escaped.join(", "))
}

pub fn write_file_content(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}
