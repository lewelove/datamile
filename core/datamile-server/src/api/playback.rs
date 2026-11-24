use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use datamile_mpd::MpdCommand;
use serde_json::json;

use crate::backend::PlaybackSlice;
use crate::state::ServerState;

fn parse_usize(params: &HashMap<String, String>, key: &str) -> Option<usize> {
    params.get(key).and_then(|value| value.parse().ok())
}

async fn send_play(state: &Arc<ServerState>, command: MpdCommand) -> Response {
    state.mpd_engine.send(command).await;
    Json(json!({"status": "ok"})).into_response()
}

#[allow(
    clippy::implicit_hasher,
    reason = "axum's Query extractor hands back the default hasher"
)]
pub async fn play_media(
    Path((media_type, id)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let slice = PlaybackSlice {
        start: parse_usize(&params, "start"),
        count: parse_usize(&params, "count"),
    };
    let offset = parse_usize(&params, "offset").unwrap_or(0);

    let tracks = {
        let logic = state.logic.read().await;
        state
            .backend
            .resolve_playback_uris(&media_type, &id, slice, &logic)
    };

    send_play(&state, MpdCommand::Play { tracks, offset }).await
}

#[allow(
    clippy::implicit_hasher,
    reason = "axum's Query extractor hands back the default hasher"
)]
pub async fn queue_media(
    Path((media_type, id)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    let slice = PlaybackSlice {
        start: parse_usize(&params, "start"),
        count: parse_usize(&params, "count"),
    };

    let tracks = {
        let logic = state.logic.read().await;
        state
            .backend
            .resolve_playback_uris(&media_type, &id, slice, &logic)
    };

    send_play(&state, MpdCommand::Queue { tracks }).await
}

/// The file MPD is playing right now, so CLI callers need no MPD client.
pub async fn current_playback(State(state): State<Arc<ServerState>>) -> Response {
    let file = state.now_playing.read().await.clone();
    Json(json!({ "file": file })).into_response()
}

pub async fn jump_to_index(
    Path(index): Path<usize>,
    State(state): State<Arc<ServerState>>,
) -> Response {
    send_play(&state, MpdCommand::Jump { index }).await
}

pub async fn next_track(State(state): State<Arc<ServerState>>) -> Response {
    send_play(&state, MpdCommand::Next).await
}

pub async fn prev_track(State(state): State<Arc<ServerState>>) -> Response {
    send_play(&state, MpdCommand::Prev).await
}

pub async fn stop_playback(State(state): State<Arc<ServerState>>) -> Response {
    send_play(&state, MpdCommand::Stop).await
}

pub async fn clear_queue(State(state): State<Arc<ServerState>>) -> Response {
    send_play(&state, MpdCommand::Clear).await
}

pub async fn toggle_pause(State(state): State<Arc<ServerState>>) -> Response {
    send_play(&state, MpdCommand::TogglePause).await
}
