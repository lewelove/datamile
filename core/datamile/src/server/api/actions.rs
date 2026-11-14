use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::server::logic::LogicEngine;
use crate::server::state::AppState;

#[derive(Deserialize, Default)]
pub struct ActionBody {
    #[serde(rename = "type")]
    root_type: Option<String>,
    id: Option<String>,
    ids: Option<Vec<String>>,
    playing: Option<bool>,
    query: Option<String>,
    options: Option<String>,
}

impl ActionBody {
    fn requested_ids(&self) -> Vec<String> {
        let mut ids = self.ids.clone().unwrap_or_default();
        if let Some(id) = self.id.clone()
            && !ids.contains(&id)
        {
            ids.push(id);
        }

        ids
    }
}

async fn playing_target(state: &Arc<AppState>) -> Option<(String, String)> {
    let snapshot = state.playback.read().await.clone()?;

    let logic = state.logic.read().await;
    if let Some(target) = snapshot
        .get("file")
        .and_then(serde_json::Value::as_str)
        .and_then(|file| logic.track_target(file))
    {
        return Some(target);
    }

    let root_type = snapshot
        .get("root_type")
        .and_then(serde_json::Value::as_str)?;
    let album_id = snapshot
        .get("album_id")
        .and_then(serde_json::Value::as_str)?;

    logic
        .entity(root_type, album_id)
        .map(|_| (root_type.to_string(), album_id.to_string()))
}

fn targets_for_ids(
    logic: &LogicEngine,
    root_type: Option<&str>,
    ids: &[String],
) -> Vec<(String, String)> {
    let mut targets = Vec::new();

    for id in ids {
        if let Some(root_type) = root_type {
            if logic.entity(root_type, id).is_some() {
                targets.push((root_type.to_string(), id.clone()));
            }
            continue;
        }

        targets.extend(logic.targets_for_id(id));
    }

    targets
}

fn scoped_targets(
    mut targets: Vec<(String, String)>,
    root_type: Option<&str>,
) -> Vec<(String, String)> {
    if let Some(root_type) = root_type {
        targets.retain(|(target_type, _)| target_type == root_type);
    }

    targets
}

async fn resolve_targets(
    body: &ActionBody,
    state: &Arc<AppState>,
) -> Vec<(String, String)> {
    if body.playing == Some(true) {
        return playing_target(state).await.into_iter().collect();
    }

    if let Some(query) = body.query.as_deref().filter(|q| !q.trim().is_empty()) {
        let targets = state.logic.read().await.find_targets(query);
        return scoped_targets(targets, body.root_type.as_deref());
    }

    {
        let logic = state.logic.read().await;
        let targets =
            targets_for_ids(&logic, body.root_type.as_deref(), &body.requested_ids());
        drop(logic);
        targets
    }
}

async fn entity_payload(
    state: &Arc<AppState>,
    targets: Vec<(String, String)>,
) -> Vec<serde_json::Value> {
    let logic = state.logic.read().await;

    targets
        .into_iter()
        .filter_map(|(root_type, id)| {
            let record = logic.entity(&root_type, &id)?;
            let lock = logic.lock(&root_type, &id)?.clone();

            Some(json!({
                "type": root_type,
                "id": id,
                "path": record.path.to_string_lossy(),
                "lock_path": record.lock_path.to_string_lossy(),
                "lock": lock
            }))
        })
        .collect()
}

pub async fn execute_action(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<ActionBody>,
) -> Response {
    let name_key = name.replace('-', "_");

    let config_path = {
        let config_guard = state.config.read().await;
        if !config_guard.actions.contains_key(&name_key) {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Action '{name}' is not registered.")})),
            )
                .into_response();
        }
        config_guard.config_path.clone()
    };

    let targets = resolve_targets(&body, &state).await;
    let entities = entity_payload(&state, targets).await;

    let payload = json!({
        "entities": entities,
        "options": body.options.unwrap_or_default(),
        "isatty": false
    });

    let res = tokio::task::spawn_blocking(move || {
        let engine = libdatamile::lua::LuaEngine::new()?;
        engine.evaluate_config(&config_path)?;
        engine.execute_action(&name_key, &payload)
    })
    .await;

    match res {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
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

#[cfg(test)]
mod tests;
