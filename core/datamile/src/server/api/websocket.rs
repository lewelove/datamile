use std::sync::Arc;

use ax_ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade, ws as ax_ws};
use axum::response::Response;
use serde_json::{Value, json};

use crate::server::logic::SortOrder;
use crate::server::mpd::MpdCommand;
use crate::server::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn build_init_payload(state: &AppState) -> String {
    let (dict, track_map, manifest, shelves) = {
        let l = state.logic.read().await;
        let mut s = std::collections::HashMap::new();
        for key in l.manifest.shelves.keys() {
            s.insert(
                key.clone(),
                l.request_shelf_view(key, None, SortOrder::Forward),
            );
        }
        (
            l.dict_snapshot(),
            l.track_lookup.clone(),
            l.manifest.clone(),
            s,
        )
    };
    let ui_data = state.ui_state.read().await.clone();
    let covers = state.config.read().await.covers.clone();

    json!({
        "type": "INIT_DICT",
        "dict": dict,
        "trackMap": track_map,
        "manifest": manifest,
        "shelves": shelves,
        "ui_state": ui_data,
        "config": {
            "covers": covers
        }
    })
    .to_string()
}

async fn handle_view_request(req: &Value, socket: &mut WebSocket, state: &AppState) {
    let library = req
        .get("library")
        .and_then(Value::as_str)
        .unwrap_or("library");
    let library_filter = req.get("library_filter").and_then(Value::as_str);
    let sort = req.get("sort").and_then(Value::as_str).unwrap_or("default");
    let reverse = req.get("reverse").and_then(Value::as_bool).unwrap_or(false);
    let filter_key = req
        .get("filter")
        .and_then(|v| v.get("key"))
        .and_then(Value::as_str);
    let filter_val = req
        .get("filter")
        .and_then(|v| v.get("val"))
        .and_then(Value::as_str);

    let order = if reverse {
        SortOrder::Reverse
    } else {
        SortOrder::Forward
    };

    let ids = state.logic.read().await.request_view(
        library,
        library_filter,
        sort,
        filter_key,
        filter_val,
        order,
    );
    let payload = json!({ "type": "VIEW_DATA", "ids": ids }).to_string();
    let _ = socket.send(ax_ws::Message::Text(payload.into())).await;
}

async fn handle_shelf_request(req: &Value, socket: &mut WebSocket, state: &AppState) {
    let shelf = req.get("shelf").and_then(Value::as_str).unwrap_or("");
    let order = req.get("order").and_then(Value::as_str);
    let reverse = req.get("reverse").and_then(Value::as_bool).unwrap_or(false);

    let sort_order = if reverse {
        SortOrder::Reverse
    } else {
        SortOrder::Forward
    };

    let ids = state
        .logic
        .read()
        .await
        .request_shelf_view(shelf, order, sort_order);
    let payload = json!({ "type": "SHELF_DATA", "ids": ids }).to_string();
    let _ = socket.send(ax_ws::Message::Text(payload.into())).await;
}

async fn handle_group_request(req: &Value, socket: &mut WebSocket, state: &AppState) {
    let library = req
        .get("library")
        .and_then(Value::as_str)
        .unwrap_or("library");
    let library_filter = req.get("library_filter").and_then(Value::as_str);
    let key = req.get("key").and_then(Value::as_str).unwrap_or("");

    let result = state
        .logic
        .read()
        .await
        .request_group(library, library_filter, key);
    let payload =
        json!({ "type": "GROUP_RESULT", "key": key, "result": result }).to_string();
    let _ = socket.send(ax_ws::Message::Text(payload.into())).await;
}

async fn process_client_msg(text: &str, socket: &mut WebSocket, state: &AppState) {
    let Ok(req) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(req_type) = req.get("type").and_then(Value::as_str) else {
        return;
    };
    match req_type {
        "VIEW_REQUEST" => handle_view_request(&req, socket, state).await,
        "SHELF_REQUEST" => handle_shelf_request(&req, socket, state).await,
        "GROUP_REQUEST" => handle_group_request(&req, socket, state).await,
        _ => {}
    }
}

async fn handle_inbound_msg(
    msg: Result<ax_ws::Message, axum::Error>,
    socket: &mut WebSocket,
    state: &AppState,
) -> bool {
    match msg {
        Ok(ax_ws::Message::Text(text)) => {
            process_client_msg(&text, socket, state).await;
            true
        }
        Ok(ax_ws::Message::Close(_)) | Err(_) => {
            log::info!("Client disconnected");
            false
        }
        _ => true,
    }
}

async fn handle_broadcast_msg(
    res: Result<String, tokio::sync::broadcast::error::RecvError>,
    socket: &mut WebSocket,
) -> bool {
    match res {
        Ok(msg) => socket.send(ax_ws::Message::Text(msg.into())).await.is_ok(),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            log::warn!("Client lagged by {skipped} messages");
            true
        }
        Err(_) => false,
    }
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    log::info!("Client connected");

    let init_payload = build_init_payload(&state).await;
    if socket
        .send(ax_ws::Message::Text(init_payload.into()))
        .await
        .is_err()
    {
        return;
    }

    state.mpd_engine.send(MpdCommand::Refresh).await;

    let mut rx = state.tx.subscribe();
    loop {
        tokio::select! {
            Some(msg) = socket.recv() => {
                if !handle_inbound_msg(msg, &mut socket, &state).await {
                    break;
                }
            }
            res = rx.recv() => {
                if !handle_broadcast_msg(res, &mut socket).await {
                    break;
                }
            }
        }
    }
}
