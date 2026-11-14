use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::server::state::AppState;

const STATE_DIR: &str = "state";

fn namespace_file(state_root: &Path, namespace: &str) -> Option<PathBuf> {
    let valid = !namespace.is_empty()
        && namespace
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));

    valid.then(|| state_root.join(STATE_DIR).join(format!("{namespace}.json")))
}

fn read_namespace(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}))
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

pub async fn get_state(
    AxumPath(namespace): AxumPath<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let state_root = state.config.read().await.state_root.clone();

    let Some(path) = namespace_file(&state_root, &namespace) else {
        return bad_namespace();
    };

    Json(read_namespace(&path)).into_response()
}

pub async fn save_state(
    AxumPath(namespace): AxumPath<String>,
    State(state): State<Arc<AppState>>,
    Json(patch): Json<Value>,
) -> Response {
    if !patch.is_object() {
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

    let mut current = read_namespace(&path);
    merge(&mut current, patch);

    let saved = match serde_json::to_string_pretty(&current) {
        Ok(saved) => saved,
        Err(_) => {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Some(parent) = path.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    if libdatamile::utils::write_atomic_cache_file(&path, &saved).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let _ = state
        .tx
        .send(json!({ "event": "state.updated", "namespace": namespace }).to_string());

    Json(Map::<String, Value>::new()).into_response()
}

fn bad_namespace() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "Invalid state namespace" })),
    )
        .into_response()
}
