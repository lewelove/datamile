use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tempfile::NamedTempFile;

pub fn write_output(dest_path: Option<&Path>, value: &Value) -> Result<()> {
    dest_path.map_or_else(
        || write_stdout(value),
        |dest| write_json_atomic(dest, value),
    )
}

fn write_stdout(value: &Value) -> Result<()> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    serde_json::to_writer_pretty(&mut handle, value)
        .context("Failed to serialize JSON to stdout")?;
    handle
        .write_all(b"\n")
        .context("Failed to write newline to stdout")?;
    Ok(())
}

fn write_json_atomic(dest_path: &Path, value: &Value) -> Result<()> {
    let parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .context(format!("Failed to create directory {}", parent.display()))?;

    let json_str =
        serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;

    let mut temp =
        NamedTempFile::new_in(parent).context("Failed to create temporary file")?;
    temp.write_all(json_str.as_bytes())
        .context("Failed to write to temporary file")?;
    temp.write_all(b"\n")
        .context("Failed to write newline to temporary file")?;
    temp.persist(dest_path)
        .context(format!("Failed to persist file to {}", dest_path.display()))?;

    Ok(())
}
