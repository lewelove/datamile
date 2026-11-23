use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::api::tree::apply_depth;
use crate::state::ServerState;

#[derive(Deserialize)]
pub struct DepthQuery {
    pub depth: Option<usize>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
}

pub async fn list_types(State(state): State<Arc<ServerState>>) -> Response {
    let types = {
        let logic = state.logic.read().await;

        let mut types: HashMap<String, usize> = HashMap::new();
        for record in logic.entities.values() {
            *types.entry(record.media_type.clone()).or_default() += 1;
        }

        drop(logic);
        types
    };

    Json(json!({ "types": types })).into_response()
}

pub async fn list_media(
    Path(media_type): Path<String>,
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let logic = state.logic.read().await;

    let ids = query.q.as_deref().map_or_else(
        || {
            logic
                .entities
                .values()
                .filter(|record| record.media_type == media_type)
                .map(|record| record.id.clone())
                .collect()
        },
        |needle| logic.find_ids(needle),
    );

    let items: Vec<serde_json::Value> = ids
        .into_iter()
        .filter_map(|id| logic.entity_record(&id))
        .filter(|record| record.media_type == media_type)
        .map(|record| {
            json!({
                "type": record.media_type,
                "id": record.id,
                "path": record.path,
            })
        })
        .collect();

    Json(items).into_response()
}

pub async fn get_media(
    Path((media_type, id)): Path<(String, String)>,
    Query(query): Query<DepthQuery>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let raw = {
        let logic = state.logic.read().await;
        logic.entity_json(&id)
    };

    let Some(raw) = raw else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    apply_depth(&mut value, &media_type, query.depth);

    (
        [(header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

pub async fn update_media(
    Path((media_type, id)): Path<(String, String)>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    match state
        .backend
        .force_update_entity(Arc::clone(&state), &media_type, &id)
    {
        Ok(true) => Json(json!({"status": "ok"})).into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            log::error!("Failed to force update '{id}': {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}
