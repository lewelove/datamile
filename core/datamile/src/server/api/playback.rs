use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::server::mpd::MpdCommand;
use crate::server::state::AppState;

#[derive(Deserialize)]
pub struct PlayBody {
    #[serde(rename = "type")]
    root_type: String,
    id: String,
    track_ids: Option<Vec<String>>,
    #[serde(default)]
    offset: usize,
}

#[derive(Deserialize)]
pub struct JumpBody {
    index: usize,
}

pub async fn get(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = state.playback.read().await.clone().unwrap_or_else(|| {
        json!({
            "state": "stop",
            "file": "",
            "album_id": null,
            "track_index": null,
            "elapsed": 0.0,
            "duration": 0.0,
            "title": null,
            "artist": null,
            "queue": []
        })
    });

    Json(snapshot).into_response()
}

async fn resolve_tracks(body: &PlayBody, state: &Arc<AppState>) -> Vec<String> {
    let logic = state.logic.read().await;
    logic.track_uris_for(&body.root_type, &body.id, body.track_ids.as_deref())
}

pub async fn play(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlayBody>,
) -> Response {
    let tracks = resolve_tracks(&body, &state).await;
    if tracks.is_empty() {
        return Json(json!({ "status": "empty" })).into_response();
    }

    state
        .mpd_engine
        .send(MpdCommand::Play {
            tracks,
            offset: body.offset,
        })
        .await;

    Json(json!({ "status": "ok" })).into_response()
}

pub async fn queue(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PlayBody>,
) -> Response {
    let tracks = resolve_tracks(&body, &state).await;
    state.mpd_engine.send(MpdCommand::Queue { tracks }).await;

    Json(json!({ "status": "ok" })).into_response()
}

pub async fn jump(
    State(state): State<Arc<AppState>>,
    Json(body): Json<JumpBody>,
) -> Response {
    state
        .mpd_engine
        .send(MpdCommand::Jump { index: body.index })
        .await;
    Json(json!({ "status": "ok" })).into_response()
}

pub async fn next(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Next).await;
    Json(json!({ "status": "ok" })).into_response()
}

pub async fn prev(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Prev).await;
    Json(json!({ "status": "ok" })).into_response()
}

pub async fn stop(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Stop).await;
    Json(json!({ "status": "ok" })).into_response()
}

pub async fn clear(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::Clear).await;
    Json(json!({ "status": "ok" })).into_response()
}

pub async fn toggle_pause(State(state): State<Arc<AppState>>) -> Response {
    state.mpd_engine.send(MpdCommand::TogglePause).await;
    Json(json!({ "status": "ok" })).into_response()
}
