use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::backend::ActionTargetQuery;
use crate::state::ServerState;

/// `POST /api/select`: the same target resolution an action uses, without
/// running the action. The CLI runs the action itself; only the selection comes
/// from the daemon's already-built index.
pub async fn select_targets(
    State(state): State<Arc<ServerState>>,
    Json(query): Json<ActionTargetQuery>,
) -> Response {
    let playing = state.now_playing.read().await.clone();
    let playing = Some(playing.as_str()).filter(|file| !file.is_empty());

    let items = {
        let logic = state.logic.read().await;
        state
            .backend
            .resolve_action_targets(&query, &logic, playing)
    };

    Json(items).into_response()
}
