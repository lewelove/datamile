use anyhow::{Context, Result};
use libdatamile::utils::expand_path;
use mpd_client::Client;
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use tokio::net::TcpStream;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugMode {
    Enabled,
    Disabled,
}

pub struct TargetFlags {
    pub playing: bool,
    pub id: Option<String>,
    pub query: Option<String>,
    pub directory: Option<String>,
    pub recursive: Option<String>,
    pub library: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ResolvedAlbumTarget {
    pub id: String,
    pub path: PathBuf,
    pub lock: serde_json::Value,
}

async fn resolve_by_query(query_str: &str) -> Result<Vec<ResolvedAlbumTarget>> {
    let client = reqwest::Client::new();
    let res = client
        .post("http://127.0.0.1:8000/api/internal/query")
        .json(&serde_json::json!({ "query": query_str }))
        .send()
        .await
        .context("Failed to connect to the Datamile server. Is it running?")?;

    if !res.status().is_success() {
        let err_text = res.text().await.unwrap_or_default();
        anyhow::bail!("Server rejected query: {err_text}");
    }

    let items: Vec<ResolvedAlbumTarget> =
        res.json().await.context("Invalid response from server")?;
    Ok(items)
}

async fn resolve_by_id(
    id: &str,
    music_directory: &Path,
) -> Result<Vec<ResolvedAlbumTarget>> {
    let client = reqwest::Client::new();
    let q_resp = client
        .post("http://127.0.0.1:8000/api/internal/query")
        .json(&serde_json::json!({ "query": id }))
        .send()
        .await;

    if let Ok(resp) = q_resp
        && resp.status().is_success()
        && let Ok(results) = resp.json::<Vec<ResolvedAlbumTarget>>().await
        && let Some(matched) = results.into_iter().find(|r| r.id == id)
    {
        return Ok(vec![matched]);
    }

    let mut results = Vec::new();
    for entry in WalkDir::new(music_directory)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_name() == "album.lock.json"
            && let Ok(content) = std::fs::read_to_string(entry.path())
            && let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&content)
            && lock_json.pointer("/album/id").and_then(|v| v.as_str()) == Some(id)
            && let Some(parent) = entry.path().parent()
        {
            results.push(ResolvedAlbumTarget {
                id: id.to_string(),
                path: parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf()),
                lock: lock_json,
            });
            break;
        }
    }
    Ok(results)
}

async fn resolve_by_playing(music_directory: &Path) -> Result<Vec<ResolvedAlbumTarget>> {
    let playing_album_dir = get_playing_album(&music_directory.to_string_lossy()).await?;
    let lock_path = playing_album_dir.join("album.lock.json");
    if lock_path.exists()
        && let Ok(content) = std::fs::read_to_string(&lock_path)
        && let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let id = lock_json
            .pointer("/album/id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(vec![ResolvedAlbumTarget {
            id,
            path: playing_album_dir
                .canonicalize()
                .unwrap_or(playing_album_dir),
            lock: lock_json,
        }])
    } else {
        Ok(Vec::new())
    }
}

fn resolve_by_scan(root: &Path) -> Vec<ResolvedAlbumTarget> {
    let mut results = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_name() == "album.lock.json"
            && let Ok(content) = std::fs::read_to_string(entry.path())
            && let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(parent) = entry.path().parent()
        {
            let id = lock_json
                .pointer("/album/id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            results.push(ResolvedAlbumTarget {
                id,
                path: parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf()),
                lock: lock_json,
            });
        }
    }
    results
}

fn resolve_by_dir(dir: &Path) -> Vec<ResolvedAlbumTarget> {
    let p = expand_path(dir.to_str().unwrap_or("."));
    let lock_path = p.join("album.lock.json");
    if lock_path.exists()
        && let Ok(content) = std::fs::read_to_string(&lock_path)
        && let Ok(lock_json) = serde_json::from_str::<serde_json::Value>(&content)
    {
        let id = lock_json
            .pointer("/album/id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        vec![ResolvedAlbumTarget {
            id,
            path: p.canonicalize().unwrap_or(p),
            lock: lock_json,
        }]
    } else {
        Vec::new()
    }
}

pub async fn resolve_target_albums(
    music_directory: &Path,
    target: &TargetFlags,
) -> Result<Vec<ResolvedAlbumTarget>> {
    if let Some(query_str) = &target.query {
        resolve_by_query(query_str).await
    } else if let Some(id) = &target.id {
        resolve_by_id(id, music_directory).await
    } else if target.playing {
        resolve_by_playing(music_directory).await
    } else if target.library {
        Ok(resolve_by_scan(music_directory))
    } else if let Some(dir) = &target.recursive {
        Ok(resolve_by_scan(&expand_path(dir)))
    } else {
        let dir_str = target.directory.as_deref().unwrap_or(".");
        Ok(resolve_by_dir(Path::new(dir_str)))
    }
}

pub async fn execute(
    name: String,
    target: TargetFlags,
    _debug: DebugMode,
    trailing_args: Vec<String>,
) -> Result<()> {
    let name_key = name.replace('-', "_");
    let config =
        libdatamile::lua::ResolvedConfig::load().context("Failed to load config")?;

    if !config.actions.contains_key(&name_key) && name != "intermediary" {
        anyhow::bail!("Action '{name}' is not declared in configuration.");
    }

    let music_directory = expand_path(&config.app.storage.music_directory)
        .canonicalize()
        .unwrap_or_else(|_| expand_path(&config.app.storage.music_directory));

    let resolved_targets = resolve_target_albums(&music_directory, &target).await?;

    let albums_payload: Vec<serde_json::Value> = resolved_targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "path": t.path.to_string_lossy(),
                "lock": t.lock
            })
        })
        .collect();

    let is_atty = std::io::stdin().is_terminal();

    let combined_json = serde_json::json!({
        "albums": albums_payload,
        "options": trailing_args.join(" "),
        "isatty": is_atty
    });

    if name == "intermediary" {
        let pretty_json = serde_json::to_string_pretty(&combined_json)?;
        println!("{pretty_json}");
        return Ok(());
    }

    let engine = libdatamile::lua::LuaEngine::new()?;
    engine.evaluate_config(&config.path)?;
    engine.execute_action(&name_key, &combined_json)?;

    Ok(())
}

pub async fn get_playing_track_url() -> Result<String> {
    let host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string());
    let addr = format!("{host}:{port}");

    let stream = TcpStream::connect(&addr)
        .await
        .context("Failed to connect to MPD")?;
    let (client, _) = Client::connect(stream)
        .await
        .context("Failed to initialize MPD client")?;

    let current_song = client.command(mpd_client::commands::CurrentSong).await?;
    let song = current_song.context("No song is currently playing")?;
    Ok(song.song.url)
}

pub async fn get_playing_album(lib_root: &str) -> Result<PathBuf> {
    let rel_path = get_playing_track_url().await?;
    let clean_rel = rel_path.trim_start_matches('/');
    let root = expand_path(lib_root)
        .canonicalize()
        .unwrap_or_else(|_| expand_path(lib_root));
    let full_path = root.join(clean_rel);

    let mut curr = full_path
        .parent()
        .map(Path::to_path_buf)
        .context("Invalid track path")?;

    while curr.starts_with(&root) && curr != root {
        if curr.join("album.toml").exists()
            || curr.join("metadata.toml").exists()
            || curr.join("album.lock.json").exists()
        {
            return Ok(curr);
        }
        let Some(parent) = curr.parent() else {
            break;
        };
        curr = parent.to_path_buf();
    }

    full_path
        .parent()
        .map(Path::to_path_buf)
        .context("Invalid track path")
}
