use std::sync::Arc;

use mpd_client::Client;
use mpd_client::client::{ConnectionEvent, Subsystem};
use tokio::net::TcpStream;
use tokio::sync::{RwLock, broadcast, mpsc};

use crate::server::logic::LogicEngine;
use crate::server::mpd::commands::{MpdCommand, handle_command};
use crate::server::mpd::status::broadcast_status;
use crate::server::state::AppConfig;

#[derive(Clone)]
pub struct MpdEngine {
    tx: mpsc::Sender<MpdCommand>,
}

impl MpdEngine {
    pub async fn send(&self, command: MpdCommand) {
        let _ = self.tx.send(command).await;
    }
}

pub fn start_actor(
    broadcast_tx: broadcast::Sender<String>,
    logic: Arc<RwLock<LogicEngine>>,
    playback: Arc<RwLock<Option<serde_json::Value>>>,
    _app_config: Arc<AppConfig>,
) -> MpdEngine {
    let (tx, mut rx) = mpsc::channel::<MpdCommand>(32);
    let engine_handle = MpdEngine { tx };

    let mpd_host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mpd_port = std::env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string());
    let addr = format!("{mpd_host}:{mpd_port}");

    tokio::spawn(async move {
        loop {
            match TcpStream::connect(&addr).await {
                Ok(stream) => match Client::connect(stream).await {
                    Ok((client, mut events)) => {
                        log::info!("MPD Connected: {addr}");

                        let _ = client
                            .command(mpd_client::commands::SetBinaryLimit(131_072))
                            .await;
                        let _ =
                            broadcast_status(&client, &broadcast_tx, &logic, &playback)
                                .await;

                        loop {
                            tokio::select! {
                                Some(event) = events.next() => {
                                    match event {
                                        ConnectionEvent::SubsystemChange(sub) => {
                                            if matches!(
                                                sub,
                                                Subsystem::Player |
                                                Subsystem::Queue |
                                                Subsystem::Options
                                            ) {
                                                let _ = broadcast_status(
                                                    &client,
                                                    &broadcast_tx,
                                                    &logic,
                                                    &playback,
                                                )
                                                .await;
                                            }
                                        }
                                        ConnectionEvent::ConnectionClosed(e) => {
                                            log::warn!("MPD Connection closed: {e:?}");
                                            break;
                                        }
                                    }
                                }
                                Some(cmd) = rx.recv() => {
                                    if !matches!(cmd, MpdCommand::Refresh)
                                        && let Err(e) = handle_command(&client, cmd).await
                                    {
                                        log::error!("MPD Execution Error: {e}");
                                    }
                                    let _ = broadcast_status(
                                        &client,
                                        &broadcast_tx,
                                        &logic,
                                        &playback,
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("MPD Handshake Failed: {e}");
                    }
                },
                Err(e) => {
                    log::error!("MPD Connection Failed: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });

    engine_handle
}
