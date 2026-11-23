use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use datamile_cache::calculate_path_hash;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct CurrentState {
    pub hash: String,
}

pub fn library_cache_dir(cache_root: &Path, music_directory: &Path) -> PathBuf {
    let lib_hash = calculate_path_hash(music_directory);
    cache_root.join("libraries").join(lib_hash)
}

pub fn deps_graph_path(cache_root: &Path, music_directory: &Path) -> PathBuf {
    library_cache_dir(cache_root, music_directory).join("dependencies.json")
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
