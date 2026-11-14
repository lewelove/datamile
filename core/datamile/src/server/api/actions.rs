use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::server::state::AppState;

struct TargetAlbumEntry {
    path: PathBuf,
    lock_path: PathBuf,
    lock: serde_json::Value,
}

async fn resolve_playing_target(state: &AppState) -> Option<String> {
    let playing_path = crate::x::playing_track_url().await.ok()?;
    let clean_playing = playing_path.trim_start_matches('/');
    let logic = state.logic.read().await;

    if let Some(item) = logic
        .track_lookup
        .get(clean_playing)
        .or_else(|| logic.track_lookup.get(&playing_path))
    {
        return Some(item.album_id.clone());
    }

    let media_root = state.config.read().await.media_roots.first().cloned()?;
    let dir = crate::x::playing_album(&media_root.to_string_lossy())
        .await
        .ok()?;
    let canon = dir.canonicalize().unwrap_or(dir);
    logic.album_id_by_target(&canon).map(ToString::to_string)
}

async fn resolve_param_target(
    params: &HashMap<String, String>,
    state: &AppState,
) -> Vec<String> {
    let logic = state.logic.read().await;
    if let Some(q) = params.get("query") {
        return logic.find_ids(q);
    }
    if let Some(id) = params.get("id").filter(|s| !s.is_empty()) {
        return vec![id.clone()];
    }
    if params.get("library").is_some_and(|v| v == "true")
        || params.contains_key("recursive")
    {
        let roots = if let Some(dir) = params.get("recursive") {
            vec![libdatamile::utils::expand_path(dir)]
        } else {
            state.config.read().await.media_roots.clone()
        };
        let roots: Vec<_> = roots
            .into_iter()
            .map(|root| root.canonicalize().unwrap_or(root))
            .collect();

        return logic
            .albums
            .values()
            .filter(|a| roots.iter().any(|root| a.path.starts_with(root)))
            .map(|a| a.id.clone())
            .collect();
    }
    if let Some(dir) = params.get("directory") {
        let p = libdatamile::utils::expand_path(dir);
        let p_canon = p.canonicalize().unwrap_or(p);
        if let Some(id) = logic.album_id_by_target(&p_canon) {
            return vec![id.to_string()];
        }
    }
    Vec::new()
}

async fn resolve_target_entries(
    params: &HashMap<String, String>,
    state: &Arc<AppState>,
) -> Vec<TargetAlbumEntry> {
    let is_playing = params.get("playing").is_some_and(|v| v == "true");
    let target_ids = if is_playing {
        resolve_playing_target(state).await.into_iter().collect()
    } else {
        resolve_param_target(params, state).await
    };

    let logic = state.logic.read().await;
    let mut list = Vec::new();
    for target_id in target_ids {
        if let Some(record) = logic.album_record(&target_id)
            && let Ok(lock_json) =
                serde_json::from_str::<serde_json::Value>(&record.raw_json)
        {
            list.push(TargetAlbumEntry {
                path: record.path.clone(),
                lock_path: record.lock_path.clone(),
                lock: lock_json,
            });
        }
    }
    list
}

pub async fn execute_action(
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let name_key = name.replace('-', "_");

    let config_guard = state.config.read().await;
    if !config_guard.actions.contains_key(&name_key) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Action '{name}' is not registered.")})),
        )
            .into_response();
    }
    let config_path = config_guard.config_path.clone();
    drop(config_guard);

    let target_entries = resolve_target_entries(&params, &state).await;

    let album_entries: Vec<serde_json::Value> = target_entries
        .into_iter()
        .map(|e| {
            json!({
                "path": e.path.to_string_lossy(),
                "lock_path": e.lock_path.to_string_lossy(),
                "lock": e.lock
            })
        })
        .collect();

    let mut options_vec = Vec::new();
    for (k, v) in &params {
        if k != "playing"
            && k != "id"
            && k != "query"
            && k != "directory"
            && k != "recursive"
            && k != "library"
        {
            if v.is_empty() {
                options_vec.push(k.clone());
            } else {
                options_vec.push(format!("{k}={v}"));
            }
        }
    }
    let options_str = options_vec.join(" ");

    let combined_json = json!({
        "albums": album_entries,
        "options": options_str,
        "isatty": false
    });

    let res = tokio::task::spawn_blocking(move || {
        let engine = libdatamile::lua::LuaEngine::new()?;
        engine.evaluate_config(&config_path)?;
        engine.execute_action(&name_key, &combined_json)
    })
    .await;

    match res {
        Ok(Ok(())) => Json(json!({"status": "ok"})).into_response(),
        Ok(Err(e)) => {
            log::error!("Action '{name}' execution failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
        Err(e) => {
            log::error!("Action task join error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}
