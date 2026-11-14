use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
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

pub async fn update_state(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    let content = {
        let mut ui = state.ui_state.write().await;
        if let Some(obj) = payload.as_object()
            && let Some(ui_obj) = ui.as_object_mut()
        {
            for (k, v) in obj {
                ui_obj.insert(k.clone(), v.clone());
            }
        }
        serde_json::to_string_pretty(&*ui).ok()
    };

    if let Some(data) = content {
        let state_file = {
            let guard = state.config.read().await;
            guard.state_root.join("state.json")
        };
        let _ = tokio::fs::write(state_file, data).await;
    }

    Json(json!({"status": "saved"})).into_response()
}

pub async fn notify_force_update() -> Response {
    log::info!("Force updating library...");
    Json(json!({"status": "ok"})).into_response()
}

pub async fn trigger_full_reset(State(state): State<Arc<AppState>>) -> Response {
    log::info!("Rebuilding library database...");
    let start_time = std::time::Instant::now();

    let media_directory = state.config.read().await.media_directory.clone();
    let logic_arc = Arc::clone(&state.logic);

    let (album_count, manifest) = tokio::task::spawn_blocking(move || {
        let scanner = crate::server::library::scanner::Library::new(media_directory);
        let mut logic = logic_arc.blocking_write();
        scanner.scan(&mut logic);
        (logic.albums.len(), logic.manifest.clone())
    })
    .await
    .unwrap_or_else(|_| (0, libdatamile::lua::LogicManifest::default()));

    let elapsed = start_time.elapsed().as_millis();
    log::info!("Updated {album_count} albums.");
    log::info!("Rebuilt Logic Engine in {elapsed}ms.");

    let _ = state.tx.send(
        json!({
            "type": "LOGIC_UPDATE",
            "manifest": manifest
        })
        .to_string(),
    );
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

pub async fn force_update_album(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let path_opt = {
        let logic = state.logic.read().await;
        logic.album_path(&id).map(StdPath::to_path_buf)
    };

    if let Some(path) = path_opt {
        tokio::spawn(async move {
            if let Err(e) = crate::update::run_server_update(
                state,
                Some(path),
                ForceMode::Force,
                None,
                LogVerbosity::Silent,
                None,
            )
            .await
            {
                log::error!("Failed to force update album: {e}");
            }
        });
        return Json(json!({"status": "ok"})).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
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
    let ids = logic.find_ids(q_str);
    let mut results = Vec::new();
    for id in ids {
        if let Some(record) = logic.album_record(&id)
            && let Ok(lock_json) =
                serde_json::from_str::<serde_json::Value>(&record.raw_json)
        {
            results.push(json!({
                "id": id,
                "path": record.path,
                "lock_path": record.lock_path,
                "lock": lock_json,
            }));
        }
    }
    Json(results).into_response()
}
