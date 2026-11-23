use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::backend::ActionTargetQuery;
use crate::state::ServerState;

#[allow(
    clippy::implicit_hasher,
    reason = "axum's Query extractor hands back the default hasher"
)]
pub async fn execute_action(
    axum::extract::Path(name): axum::extract::Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let name_key = name.replace('-', "_");

    if !state.config.read().await.actions.contains_key(&name_key) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Action '{name}' is not registered.")})),
        )
            .into_response();
    }

    let query = ActionTargetQuery::from_params(&params);
    let playing = state.now_playing.read().await.clone();
    let playing_file = if playing.is_empty() {
        None
    } else {
        Some(playing.as_str())
    };

    let albums = {
        let logic = state.logic.read().await;
        state
            .backend
            .resolve_action_targets(&query, &logic, playing_file)
    };

    let payload = json!({
        "albums": albums,
        "options": query.options_string(),
        "isatty": false
    });

    let backend = Arc::clone(&state.backend);
    let res =
        tokio::task::spawn_blocking(move || backend.execute_action(&name_key, &payload))
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
