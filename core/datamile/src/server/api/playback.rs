use crate::server::mpd::MpdCommand;
use crate::server::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::path::Path as StdPath;
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
    let mut paths = Vec::new();
    let target_disc = disc_filter.and_then(|s| s.parse::<u32>().ok());

    let (json_str, album_dir) = {
        let logic = state.logic.read().await;
        let json_opt = logic.get_album_json(id);
        let path_opt = logic.get_album_path(id).map(StdPath::to_path_buf);
        drop(logic);
        (json_opt, path_opt)
    };

    let music_directory = { state.config.read().await.music_directory.clone() };

    if let (Some(raw), Some(dir)) = (json_str, album_dir)
        && let Ok(parsed) = serde_json::from_str::<Value>(&raw)
        && let Some(tracks) = parsed.pointer("/album/tracks").and_then(|t| t.as_array())
    {
        for track in tracks {
            if let Some(td) = target_disc {
                let current_disc = track
                    .pointer("/tags/discnumber")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(1, |v| u32::try_from(v).unwrap_or(1));
                if current_disc != td {
                    continue;
                }
            }

            if let Some(tp) = track.pointer("/location/path").and_then(|v| v.as_str()) {
                let abs = if StdPath::new(tp).is_absolute() {
                    std::path::PathBuf::from(tp)
                } else {
                    dir.join(tp)
                };
                let uri = abs.strip_prefix(&music_directory).map_or_else(
                    |_| abs.to_string_lossy().to_string(),
                    |p| p.to_string_lossy().to_string(),
                );
                paths.push(uri);
            }
        }
    }
    paths
}
