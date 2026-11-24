use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::backend::UpdateRequest;
use crate::state::ServerState;

/// `POST /api/update`: one serialized event per line, consumed only by the CLI.
pub async fn trigger_update(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<UpdateRequest>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(100);

    if let Err(e) =
        state
            .backend
            .trigger_update(Arc::clone(&state), payload, vec![tx.clone()])
    {
        // The stream contract is one JSON event per line, ending in `Finished`,
        // so the CLI can report the failure instead of a truncated stream.
        for line in start_failure(&e.to_string()) {
            let _ = tx.send(line).await;
        }
    }
    // The backend owns the only other sender; dropping ours ends the stream when
    // the pass is over.
    drop(tx);

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        let msg = rx.recv().await?;
        Some((Ok::<_, std::convert::Infallible>(format!("{msg}\n")), rx))
    });

    Response::builder()
        .header(
            axum::http::header::CONTENT_TYPE,
            "application/x-ndjson; charset=utf-8",
        )
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The two lines that keep the stream contract when a pass cannot even start: a
/// `Failed` event, then `Finished` with one failure.
fn start_failure(error: &str) -> [String; 2] {
    let failed = json!({
        "event": "failed",
        "media_type": "-",
        "dir": null,
        "error": error,
    });
    let finished = json!({
        "event": "finished",
        "discovered": 0,
        "added": 0,
        "updated": 0,
        "unchanged": 0,
        "removed": 0,
        "failed": 1,
    });

    [failed.to_string(), finished.to_string()]
}

/// `GET /api/identity`: what a CLI compares before it delegates anything.
pub async fn identity(State(state): State<Arc<ServerState>>) -> Response {
    let config_path = state.config.read().await.config_path.clone();
    // Canonical on both sides, so the CLI's match rule cannot fail on a symlink.
    let config_path = config_path.canonicalize().unwrap_or(config_path);
    let reload_error = state.reload_error.read().await.clone();

    Json(json!({
        "config_path": config_path.to_string_lossy(),
        "version": crate::VERSION,
        "reload_error": reload_error,
    }))
    .into_response()
}

pub async fn update_state(
    State(state): State<Arc<ServerState>>,
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

pub async fn trigger_reset(State(state): State<Arc<ServerState>>) -> Response {
    log::info!("Rebuilding library database...");
    let start_time = std::time::Instant::now();

    let logic_arc = Arc::clone(&state.logic);
    let backend = Arc::clone(&state.backend);

    let (count, manifest) = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        backend.scan_library(&mut logic);
        (logic.entity_count(), logic.manifest.clone())
    })
    .await
    .unwrap_or_else(|_| (0, datamile_lua_domain::LogicManifest::default()));

    let elapsed = start_time.elapsed().as_millis();
    log::info!("Updated {count} media items.");
    log::info!("Rebuilt Logic Engine in {elapsed}ms.");

    let actions = state.config.read().await.actions.clone();
    let _ = state.tx.send(
        json!({
            "type": "LOGIC_UPDATE",
            "manifest": manifest,
            "actions": actions
        })
        .to_string(),
    );

    Json(json!({"status": "ok"})).into_response()
}
