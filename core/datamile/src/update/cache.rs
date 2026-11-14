use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Last observed stat for every file a compiled root depends on.
pub type LibraryCache = HashMap<PathBuf, FileStat>;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileStat {
    pub mtime: u128,
    pub size: u64,
}

impl From<&std::fs::Metadata> for FileStat {
    fn from(m: &std::fs::Metadata) -> Self {
        let mtime = m
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            mtime,
            size: m.len(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CurrentState {
    pub hash: String,
}

pub fn calculate_path_hash(path: &Path) -> String {
    blake3::hash(path.as_os_str().as_encoded_bytes())
        .to_hex()
        .to_string()
}

pub fn library_cache_dir(cache_root: &Path, media_directory: &Path) -> PathBuf {
    let lib_hash = calculate_path_hash(media_directory);
    cache_root.join("libraries").join(lib_hash)
}

pub fn deps_graph_path(cache_root: &Path, media_directory: &Path) -> PathBuf {
    library_cache_dir(cache_root, media_directory).join("dependencies.json")
}

pub fn lua_config_hash(dependencies: &[PathBuf]) -> String {
    let mut sorted_deps = dependencies.to_vec();
    sorted_deps.sort();
    let mut hasher = blake3::Hasher::new();
    for dep in &sorted_deps {
        if let Ok(content) = fs::read(dep) {
            hasher.update(&content);
        }
    }
    hasher.finalize().to_hex().to_string()
}

pub fn load_cache(path: &Path) -> LibraryCache {
    if let Ok(content) = fs::read_to_string(path)
        && let Ok(cache) = serde_json::from_str::<LibraryCache>(&content)
    {
        return cache;
    }
    LibraryCache::new()
}

pub fn save_cache(cache: &LibraryCache, path: &Path) -> Result<()> {
    let content = serde_json::to_string_pretty(cache)?;
    fs::write(path, content)?;
    Ok(())
}

pub async fn validate_library_root(cache_dir: &Path, current_hash: &str) -> Result<()> {
    let current_json_path = cache_dir.join("current.json");
    if current_json_path.exists() {
        let content = fs::read_to_string(&current_json_path).unwrap_or_default();
        let saved_state: Result<CurrentState, _> = serde_json::from_str(&content);
        if let Ok(state) = saved_state
            && state.hash == current_hash
        {
            return Ok(());
        }
    }

    let state = CurrentState {
        hash: current_hash.to_string(),
    };
    let content = serde_json::to_string(&state)?;
    let _ = fs::write(&current_json_path, content);
    let _ = trigger_server_reset().await;
    Ok(())
}

async fn trigger_server_reset() -> Result<()> {
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:8000/api/internal/reset")
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await;
    Ok(())
}
