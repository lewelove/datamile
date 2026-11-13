use crate::models::{AlbumData, FormattingConfig};
use anyhow::Result;
use libactions::fs::{escape_toml_string, sanitize_filename};
use std::fs;
use std::path::Path;

// Creates album directory and populates canonical TOML manifests.
pub fn create_album_directory(
    data: &AlbumData,
    formatting: &FormattingConfig,
    root: &Path,
) -> Result<()> {
    let formatted = formatting
        .album
        .replace("{albumartist}", &data.albumartist)
        .replace("{album}", &data.album);

    let dir_name = sanitize_filename(&formatted);
    let album_path = root.join(dir_name);

    fs::create_dir_all(&album_path)?;

    let meta_path = album_path.join("album.toml");
    write_album_toml(data, &meta_path)?;

    let history_path = album_path.join("history.toml");
    write_history_toml(data, &history_path)?;

    let virtual_path = album_path.join("virtual.toml");
    write_virtual_toml(&virtual_path)?;

    Ok(())
}

fn write_album_toml(data: &AlbumData, path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    let mut lines = vec![
        "[album]".to_string(),
        String::new(),
        format!(
            "albumartist = \"{}\"",
            escape_toml_string(&data.albumartist)
        ),
        format!("album = \"{}\"", escape_toml_string(&data.album)),
        format!("date = \"{}\"", escape_toml_string(&data.date)),
        "genre = \"\"".to_string(),
        String::new(),
    ];

    let total_discs = data.tracks.iter().map(|t| t.discnumber).max().unwrap_or(1);

    for t in &data.tracks {
        lines.push("[[tracks]]".to_string());
        if total_discs > 1 {
            lines.push(format!("discnumber = {}", t.discnumber));
        }
        lines.push(format!("tracknumber = {}", t.tracknumber));
        lines.push(format!("title = \"{}\"", escape_toml_string(&t.title)));

        if let Some(ref art) = t.artist
            && art != &data.albumartist
        {
            lines.push(format!("artist = \"{}\"", escape_toml_string(art)));
        }
        lines.push(String::new());
    }

    fs::write(path, lines.join("\n"))?;
    Ok(())
}

fn write_history_toml(data: &AlbumData, path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let artist_escaped = escape_toml_string(&data.albumartist);
    let album_escaped = escape_toml_string(&data.album);
    let date_escaped = escape_toml_string(&data.date);
    let content = format!(
        "[album]\n\nalbumartist = \"{artist_escaped}\"\nalbum = \"{album_escaped}\"\ndate = \"{date_escaped}\"\n\ndate_added_datamile = {now}\n"
    );
    fs::write(path, content)?;
    Ok(())
}

fn write_virtual_toml(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    let content = "[album]\n\nvirtual = true\n".to_string();
    fs::write(path, content)?;
    Ok(())
}
