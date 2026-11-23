use anyhow::{Context, Result};
use datamile_fs::expand_path;

use crate::x::{TargetFlags, resolve_target_albums};

pub struct QueryFlags {
    pub playing: bool,
    pub lock: bool,
    pub id: bool,
    pub json: bool,
}

pub async fn run(query_str: Option<String>, flags: QueryFlags) -> Result<()> {
    let config = crate::config::load().context("Failed to load config")?;
    let music_dir = expand_path(&config.app.storage.music_directory)
        .canonicalize()
        .unwrap_or_else(|_| expand_path(&config.app.storage.music_directory));

    if query_str.is_none() && !flags.playing {
        anyhow::bail!("No query provided. Use --playing or provide an SQL query.");
    }

    let target_flags = TargetFlags {
        playing: flags.playing,
        id: None,
        query: query_str,
        directory: None,
        recursive: None,
        library: false,
    };

    let resolved_albums = resolve_target_albums(&music_dir, &target_flags).await?;

    if flags.json {
        let albums_json: Vec<serde_json::Value> =
            resolved_albums.into_iter().map(|a| a.lock).collect();
        println!("{}", serde_json::to_string_pretty(&albums_json)?);
    } else {
        for album in resolved_albums {
            if flags.id {
                println!("{}", album.id);
            } else if flags.lock {
                let lock_file = album.path.join("album.lock.json");
                println!("{}", lock_file.display());
            } else {
                println!("{}", album.path.display());
            }
        }
    }

    Ok(())
}
