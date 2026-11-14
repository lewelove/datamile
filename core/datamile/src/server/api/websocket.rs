use std::sync::Arc;

use ax_ws::WebSocket;
use axum::extract::{State, WebSocketUpgrade, ws as ax_ws};
use axum::response::Response;

use crate::server::mpd::MpdCommand;
use crate::server::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    log::info!("Client connected");

    let mut rx = state.tx.subscribe();
    state.mpd_engine.send(MpdCommand::Refresh).await;

    loop {
        tokio::select! {
            Some(msg) = socket.recv() => {
                if !handle_inbound(msg) {
                    break;
                }
            }
            res = rx.recv() => {
                if !forward_event(res, &mut socket).await {
                    break;
                }
            }
        }
    }

    log::info!("Client disconnected");
}

fn handle_inbound(msg: Result<ax_ws::Message, axum::Error>) -> bool {
    !matches!(msg, Ok(ax_ws::Message::Close(_)) | Err(_))
}

async fn forward_event(
    res: Result<String, tokio::sync::broadcast::error::RecvError>,
    socket: &mut WebSocket,
) -> bool {
    match res {
        Ok(payload) => socket
            .send(ax_ws::Message::Text(payload.into()))
            .await
            .is_ok(),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            log::warn!("Client lagged by {skipped} messages");
            true
        }
        Err(_) => false,
    }
}
