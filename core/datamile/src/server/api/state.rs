use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::server::state::AppState;

const STATE_DIR: &str = "state";

fn namespace_file(state_root: &Path, namespace: &str) -> Option<PathBuf> {
    let valid = !namespace.is_empty()
        && namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));

    valid.then(|| state_root.join(STATE_DIR).join(format!("{namespace}.json")))
}

/// Missing file is an empty namespace; corrupt or unreadable state must surface
/// as an error so a later patch cannot overwrite it with `{}`.
async fn read_namespace(path: &Path) -> io::Result<Value> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => {
            serde_json::from_str(&raw).map_err(|e| io::Error::other(e.to_string()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e),
    }
}

/// Merge object keys from `patch` into `current`; non-objects replace.
fn merge(current: &mut Value, patch: Value) {
    match patch {
        Value::Object(patch_map) => match current.as_object_mut() {
            Some(current_map) => {
                for (key, value) in patch_map {
                    current_map.insert(key, value);
                }
            }
            None => {
                *current = Value::Object(patch_map);
            }
        },
        other => {
            *current = other;
        }
    }
}

/// Read-merge-write under one lock so overlapping patches cannot drop keys.
async fn persist_patch(lock: &Mutex<()>, path: &Path, payload: Value) -> io::Result<()> {
    let _guard = lock.lock().await;

    let mut current = read_namespace(path).await?;
    merge(&mut current, payload);

    let saved = serde_json::to_string_pretty(&current)
        .map_err(|e| io::Error::other(e.to_string()))?;
    let owned = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        libdatamile::utils::write_atomic_cache_file(&owned, &saved)
    })
    .await
    .map_err(|e| io::Error::other(e.to_string()))?
}

pub async fn read_state(
    AxumPath(namespace): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let state_root = state.config.read().await.state_root.clone();

    let Some(path) = namespace_file(&state_root, &namespace) else {
        return bad_namespace();
    };

    match read_namespace(&path).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => {
            log::error!("Failed to read state namespace '{namespace}': {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn save_state(
    AxumPath(namespace): AxumPath<String>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> Response {
    if !payload.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Body must be a JSON object" })),
        )
            .into_response();
    }

    let state_root = state.config.read().await.state_root.clone();

    let Some(path) = namespace_file(&state_root, &namespace) else {
        return bad_namespace();
    };

    if let Err(e) = persist_patch(&state.state_lock, &path, payload).await {
        log::error!("Failed to save state namespace '{namespace}': {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let _ = state
        .tx
        .send(json!({ "event": "state.updated", "namespace": namespace }).to_string());

    StatusCode::NO_CONTENT.into_response()
}

fn bad_namespace() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "Invalid state namespace" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
