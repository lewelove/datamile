use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::backend::UpdateRequest;
use crate::state::ServerState;

pub async fn trigger_update(
    State(state): State<Arc<ServerState>>,
    Json(payload): Json<UpdateRequest>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<String>(100);

    if let Err(e) =
        state
            .backend
            .trigger_update(Arc::clone(&state), payload, Some(tx.clone()))
    {
        let _ = tx.send(format!("Update failed: {e}")).await;
    }

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

pub async fn tracked_changes(State(state): State<Arc<ServerState>>) -> Response {
    let (full_rescan, tracked_items) = state.backend.tracked_changes();

    Json(json!({
        "full_rescan": full_rescan,
        "tracked_items": tracked_items
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
