use std::collections::HashMap;
use std::sync::Arc;

use ax_ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade, ws as ax_ws};
use axum::response::Response;
use datamile_logic::SortOrder;
use datamile_mpd::MpdCommand;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::state::ServerState;

const DEFAULT_MEDIA_TYPE: &str = "album";

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

pub(crate) async fn init_payload(state: &ServerState) -> String {
    let (media, manifest, shelves) = {
        let logic = state.logic.read().await;

        let mut media: HashMap<String, HashMap<String, Box<RawValue>>> = HashMap::new();
        for record in logic.all_entities() {
            media
                .entry(record.media_type.clone())
                .or_default()
                .insert(record.id.clone(), record.dict_raw.clone());
        }

        let mut shelves = HashMap::new();
        for (key, def) in &logic.manifest.shelves {
            shelves.insert(
                key.clone(),
                logic.request_shelf_view(&def.media, key, None, SortOrder::Forward),
            );
        }

        (media, logic.manifest.clone(), shelves)
    };

    let actions = state.config.read().await.actions.clone();
    let ui_data = state.ui_state.read().await.clone();

    json!({
        "type": "INIT",
        "media": media,
        "manifest": manifest,
        "actions": actions,
        "shelves": shelves,
        "ui_state": ui_data
    })
    .to_string()
}

async fn send_json(socket: &mut WebSocket, payload: Value) {
    let _ = socket
        .send(ax_ws::Message::Text(payload.to_string().into()))
        .await;
}

fn requested_media_type(req: &Value) -> Option<String> {
    req.get("media_type")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Falls back to the declaring unit's `media`, then to `album` for clients
/// that predate the field.
async fn resolve_library_media(req: &Value, state: &ServerState) -> String {
    if let Some(media_type) = requested_media_type(req) {
        return media_type;
    }

    let library = req
        .get("library")
        .and_then(Value::as_str)
        .unwrap_or("library");
    let logic = state.logic.read().await;
    logic
        .manifest
        .libraries
        .get(library)
        .map(|def| def.media.clone())
        .filter(|media| !media.is_empty())
        .unwrap_or_else(|| DEFAULT_MEDIA_TYPE.to_string())
}

pub(crate) async fn view_response(state: &ServerState, req: &Value) -> Value {
    let media_type = resolve_library_media(req, state).await;
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
        &media_type,
        library,
        library_filter,
        sort,
        filter_key,
        filter_val,
        order,
    );

    json!({ "type": "VIEW_DATA", "media_type": media_type, "ids": ids })
}

pub(crate) async fn shelf_response(state: &ServerState, req: &Value) -> Value {
    let shelf = req.get("shelf").and_then(Value::as_str).unwrap_or("");
    let order = req.get("order").and_then(Value::as_str);
    let reverse = req.get("reverse").and_then(Value::as_bool).unwrap_or(false);

    let sort_order = if reverse {
        SortOrder::Reverse
    } else {
        SortOrder::Forward
    };

    let ids = {
        let logic = state.logic.read().await;
        let media_type = requested_media_type(req).unwrap_or_else(|| {
            logic
                .manifest
                .shelves
                .get(shelf)
                .map(|def| def.media.clone())
                .filter(|media| !media.is_empty())
                .unwrap_or_else(|| DEFAULT_MEDIA_TYPE.to_string())
        });

        let ids = logic.request_shelf_view(&media_type, shelf, order, sort_order);
        drop(logic);
        (media_type, ids)
    };

    let (media_type, ids) = ids;

    json!({ "type": "SHELF_DATA", "media_type": media_type, "ids": ids })
}

pub(crate) async fn group_response(state: &ServerState, req: &Value) -> Value {
    let media_type = resolve_library_media(req, state).await;
    let library = req
        .get("library")
        .and_then(Value::as_str)
        .unwrap_or("library");
    let library_filter = req.get("library_filter").and_then(Value::as_str);
    let key = req.get("key").and_then(Value::as_str).unwrap_or("");

    let result =
        state
            .logic
            .read()
            .await
            .request_group(&media_type, library, library_filter, key);

    json!({
        "type": "GROUP_RESULT",
        "media_type": media_type,
        "key": key,
        "result": result
    })
}

async fn process_client_msg(text: &str, socket: &mut WebSocket, state: &ServerState) {
    let Ok(req) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(req_type) = req.get("type").and_then(Value::as_str) else {
        return;
    };
    let response = match req_type {
        "VIEW_REQUEST" => Some(view_response(state, &req).await),
        "SHELF_REQUEST" => Some(shelf_response(state, &req).await),
        "GROUP_REQUEST" => Some(group_response(state, &req).await),
        _ => None,
    };
    if let Some(response) = response {
        send_json(socket, response).await;
    }
}

async fn handle_inbound_msg(
    msg: Result<ax_ws::Message, axum::Error>,
    socket: &mut WebSocket,
    state: &ServerState,
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

async fn handle_socket(mut socket: WebSocket, state: Arc<ServerState>) {
    log::info!("Client connected");

    let init_payload = init_payload(&state).await;
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
