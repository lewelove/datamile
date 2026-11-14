use std::path::PathBuf;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::compile::LogVerbosity;
use crate::server::inotify::handler::{IngestOrigin, RecompiledAlbumItem};
use crate::server::state::AppState;
use crate::update::client::ForceMode;

#[derive(Serialize, Deserialize, Debug)]
pub struct EntityIngestPayload {
    pub root_type: String,
    pub target: PathBuf,
    pub lock_path: PathBuf,
    pub id: String,
    pub lock_json: String,
    pub eval_res: Option<serde_json::Value>,
    #[serde(default)]
    pub files: Vec<PathBuf>,
    #[serde(default)]
    pub dirs: Vec<PathBuf>,
    #[serde(default)]
    pub modified: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UpdateTriggerPayload {
    pub path: Option<String>,
    #[serde(default)]
    pub force: bool,
    pub jobs: Option<usize>,
    #[serde(default)]
    pub silent: bool,
}

pub async fn trigger_update(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateTriggerPayload>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(100);

    let target_path = payload.path.map(PathBuf::from);
    let force_mode = if payload.force {
        ForceMode::Force
    } else {
        ForceMode::Preserve
    };
    let jobs = payload.jobs;
    let verbosity = if payload.silent {
        LogVerbosity::Silent
    } else {
        LogVerbosity::Verbose
    };

    tokio::spawn(async move {
        if let Err(e) = crate::update::run_server_update(
            state,
            target_path,
            force_mode,
            jobs,
            verbosity,
            Some(tx.clone()),
        )
        .await
        {
            let _ = tx.send(format!("Update failed: {e}")).await;
        }
    });

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        let msg = rx.recv().await?;
        Some((Ok::<_, std::convert::Infallible>(format!("{msg}\n")), rx))
    });

    Response::builder()
        .header(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub async fn tracked_albums(State(state): State<Arc<AppState>>) -> Response {
    let full_rescan = state
        .full_rescan_needed
        .swap(false, std::sync::atomic::Ordering::Relaxed);
    let tracked: Vec<String> = state
        .tracked_roots
        .lock()
        .map_or_else(|_| Vec::new(), |mut guard| guard.drain().collect());
    Json(json!({
        "full_rescan": full_rescan,
        "tracked_roots": tracked
    }))
    .into_response()
}

pub async fn notify_force_update() -> Response {
    log::info!("Force updating library...");
    Json(json!({"status": "ok"})).into_response()
}

pub async fn trigger_full_reset(State(state): State<Arc<AppState>>) -> Response {
    log::info!("Rebuilding library database...");
    let start_time = std::time::Instant::now();

    let media_roots = state.config.read().await.media_roots.clone();
    let logic_arc = Arc::clone(&state.logic);

    let album_count = tokio::task::spawn_blocking(move || {
        let scanner = crate::server::library::scanner::Library::new(media_roots);
        let mut logic = logic_arc.blocking_write();
        scanner.scan(&mut logic);
        logic.albums.len()
    })
    .await
    .unwrap_or(0);

    let elapsed = start_time.elapsed().as_millis();
    log::info!("Updated {album_count} albums.");
    log::info!("Rebuilt Logic Engine in {elapsed}ms.");

    let _ = state
        .tx
        .send(json!({ "event": "logic.updated" }).to_string());
    Json(json!({"status": "ok"})).into_response()
}

pub async fn trigger_reload(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(path) = params.get("path") {
        let target_path = PathBuf::from(path);
        tokio::spawn(async move {
            if let Err(e) = crate::update::run_server_update(
                state,
                Some(target_path),
                ForceMode::Force,
                None,
                LogVerbosity::Silent,
                None,
            )
            .await
            {
                log::error!("Failed to reload album: {e}");
            }
        });
        return Json(json!({"status": "ok"})).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

pub async fn ingest_reload_payloads(
    State(state): State<Arc<AppState>>,
    Json(payloads): Json<Vec<EntityIngestPayload>>,
) -> Response {
    let recompiled_items: Vec<RecompiledAlbumItem> = payloads
        .into_iter()
        .map(crate::server::inotify::handler::RecompiledAlbumItem::from)
        .collect();

    crate::server::inotify::handler::ingest_and_broadcast_albums(
        std::collections::HashSet::new(),
        recompiled_items,
        IngestOrigin::External,
        &state,
    )
    .await;
    StatusCode::OK.into_response()
}

pub async fn run_query(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let q_str = payload
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let logic = state.logic.read().await;
    let results: Vec<serde_json::Value> = logic
        .find_targets(q_str)
        .into_iter()
        .filter_map(|(root_type, id)| {
            let record = logic.entity(&root_type, &id)?;
            let lock_json =
                serde_json::from_str::<serde_json::Value>(&record.raw_json).ok()?;

            Some(json!({
                "type": root_type,
                "id": id,
                "path": record.path,
                "lock_path": record.lock_path,
                "lock": lock_json,
            }))
        })
        .collect();

    Json(results).into_response()
}
