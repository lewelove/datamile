use crate::server::mpd::MpdCommand;
use crate::server::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

pub async fn play_album(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tracks = get_tracks_internal(&id, &state, None).await;
    state
        .mpd_engine
        .send(MpdCommand::Play { tracks, offset })
        .await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn play_disc(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let disc = params.get("disc").cloned();
    let offset = params
        .get("offset")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let tracks = get_tracks_internal(&id, &state, disc).await;
    state
        .mpd_engine
        .send(MpdCommand::Play { tracks, offset })
        .await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn queue_album(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let tracks = get_tracks_internal(&id, &state, None).await;
    state.mpd_engine.send(MpdCommand::Queue { tracks }).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn jump_to_index(
    Path(index): Path<usize>,
    State(state): State<Arc<AppState>>,
) -> Response {
    state.mpd_engine.send(MpdCommand::Jump { index }).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn next_track(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Next).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn prev_track(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Prev).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn stop_playback(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Stop).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn clear_queue(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Clear).await;
    Json(json!({"status": "ok"})).into_response()
}

pub async fn toggle_pause(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::TogglePause).await;
    Json(json!({"status": "ok"})).into_response()
}

async fn get_tracks_internal(
    id: &str,
    state: &Arc<AppState>,
    disc_filter: Option<String>,
) -> Vec<String> {
    let target_disc = disc_filter.and_then(|s| s.parse::<u32>().ok());

    let logic = state.logic.read().await;
    logic.track_uris_for(id, target_disc)
}
