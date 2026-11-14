use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use libdatamile::utils::expand_path;
use mpd_client::Client;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;

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
    #[serde(rename = "type")]
    pub root_type: String,
    pub id: String,
    pub path: PathBuf,
    pub lock_path: PathBuf,
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

fn load_engine(config_path: &Path) -> Result<libdatamile::lua::LuaEngine> {
    let engine = libdatamile::lua::LuaEngine::new()?;
    engine.evaluate_config(config_path)?;
    Ok(engine)
}

fn discover(
    engine: &libdatamile::lua::LuaEngine,
    path: &Path,
) -> Vec<ResolvedAlbumTarget> {
    let Ok(roots) = crate::compile::media::discover_targets(engine, path, None) else {
        return Vec::new();
    };

    resolve_discovered(roots)
}

fn discover_all(engine: &libdatamile::lua::LuaEngine) -> Vec<ResolvedAlbumTarget> {
    let Ok(roots) = crate::compile::media::discover_targets_all(engine) else {
        return Vec::new();
    };

    resolve_discovered(roots)
}

fn resolve_discovered(
    roots: Vec<crate::compile::media::RootTarget>,
) -> Vec<ResolvedAlbumTarget> {
    roots
        .into_iter()
        .filter_map(|root| {
            let content = std::fs::read_to_string(&root.lock_path).ok()?;
            let lock_json: serde_json::Value = serde_json::from_str(&content).ok()?;
            let id = lock_json
                .get(&root.root_type)
                .and_then(|entity| entity.get("id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();

            Some(ResolvedAlbumTarget {
                root_type: root.root_type,
                id,
                path: root.target,
                lock_path: root.lock_path,
                lock: lock_json,
            })
        })
        .collect()
}

async fn resolve_by_id(
    id: &str,
    _media_roots: &[PathBuf],
    config_path: &Path,
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

    let engine = load_engine(config_path)?;
    Ok(discover_all(&engine)
        .into_iter()
        .find(|target| target.id == id)
        .into_iter()
        .collect())
}

async fn resolve_by_playing(
    media_roots: &[PathBuf],
    config_path: &Path,
) -> Result<Vec<ResolvedAlbumTarget>> {
    for root in media_roots {
        let Ok(playing_album_dir) = playing_album(&root.to_string_lossy()).await else {
            continue;
        };

        let engine = load_engine(config_path)?;
        let found: Vec<_> = discover(&engine, &playing_album_dir)
            .into_iter()
            .take(1)
            .collect();
        if !found.is_empty() {
            return Ok(found);
        }
    }

    Ok(Vec::new())
}

pub async fn resolve_target_albums(
    media_roots: &[PathBuf],
    target: &TargetFlags,
    config_path: &Path,
) -> Result<Vec<ResolvedAlbumTarget>> {
    if let Some(query_str) = &target.query {
        resolve_by_query(query_str).await
    } else if let Some(id) = &target.id {
        resolve_by_id(id, media_roots, config_path).await
    } else if target.playing {
        resolve_by_playing(media_roots, config_path).await
    } else if target.library {
        let engine = load_engine(config_path)?;
        Ok(discover_all(&engine))
    } else if let Some(dir) = &target.recursive {
        let engine = load_engine(config_path)?;
        Ok(discover(&engine, &expand_path(dir)))
    } else {
        let dir_str = target.directory.as_deref().unwrap_or(".");
        let engine = load_engine(config_path)?;
        Ok(discover(&engine, &expand_path(dir_str)))
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

    let media_roots = config.root_dirs();

    let resolved_targets =
        resolve_target_albums(&media_roots, &target, &config.path).await?;

    let entities_payload: Vec<serde_json::Value> = resolved_targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": t.root_type,
                "id": t.id,
                "path": t.path.to_string_lossy(),
                "lock_path": t.lock_path.to_string_lossy(),
                "lock": t.lock
            })
        })
        .collect();

    let is_atty = std::io::stdin().is_terminal();

    let combined_json = serde_json::json!({
        "entities": entities_payload,
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

pub async fn playing_track_url() -> Result<String> {
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

/// Walk upward from the playing track until a shallow media selection yields a
/// root whose lock exists.
pub async fn playing_album(lib_root: &str) -> Result<PathBuf> {
    let rel_path = playing_track_url().await?;
    let clean_rel = rel_path.trim_start_matches('/');
    let root = expand_path(lib_root)
        .canonicalize()
        .unwrap_or_else(|_| expand_path(lib_root));
    let full_path = root.join(clean_rel);

    let mut curr = full_path
        .parent()
        .map(Path::to_path_buf)
        .context("Invalid track path")?;

    let config_path = libdatamile::lua::resolve_config_path().unwrap_or_default();
    let engine = load_engine(&config_path).ok();

    if let Some(engine) = engine.as_ref() {
        while curr.starts_with(&root) && curr != root {
            if let Ok(roots) = crate::compile::media::discover_targets(
                engine,
                &curr,
                Some(std::slice::from_ref(&curr)),
            ) && let Some(found) = roots.iter().find(|r| r.lock_path.exists())
            {
                return Ok(found.target.clone());
            }

            let Some(parent) = curr.parent() else {
                break;
            };
            curr = parent.to_path_buf();
        }
    }

    full_path
        .parent()
        .map(Path::to_path_buf)
        .context("Invalid track path")
}
