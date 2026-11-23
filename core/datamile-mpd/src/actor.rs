use mpd_client::Client;
use mpd_client::client::{ConnectionEvent, Subsystem};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc};

use crate::commands::{MpdCommand, handle_command};
use crate::status::{RawMpdStatus, broadcast_status};

#[derive(Clone)]
pub struct MpdEngine {
    tx: mpsc::Sender<MpdCommand>,
}

impl MpdEngine {
    /// Wraps a command channel, for callers that own the actor loop themselves.
    #[must_use]
    pub const fn new(tx: mpsc::Sender<MpdCommand>) -> Self {
        Self { tx }
    }

    pub async fn send(&self, command: MpdCommand) {
        let _ = self.tx.send(command).await;
    }
}

/// Starts the reconnecting MPD actor.
///
/// Returns the command handle plus the raw status feed. The feed is a
/// broadcast channel so any number of consumers can enrich the same snapshot.
#[must_use]
pub fn start_actor(addr: String) -> (MpdEngine, broadcast::Sender<RawMpdStatus>) {
    let (tx, mut rx) = mpsc::channel::<MpdCommand>(32);
    let (status_tx, _) = broadcast::channel::<RawMpdStatus>(64);
    let status_feed = status_tx.clone();

    tokio::spawn(async move {
        loop {
            match TcpStream::connect(&addr).await {
                Ok(stream) => match Client::connect(stream).await {
                    Ok((client, mut events)) => {
                        log::info!("MPD Connected: {addr}");

                        let _ = client
                            .command(mpd_client::commands::SetBinaryLimit(131_072))
                            .await;
                        let _ = broadcast_status(&client, &status_feed).await;

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
                                                let _ = broadcast_status(&client, &status_feed).await;
                                            }
                                        }
                                        ConnectionEvent::ConnectionClosed(e) => {
                                            log::warn!("MPD Connection closed: {e:?}");
                                            break;
                                        }
                                    }
                                }
                                Some(cmd) = rx.recv() => {
                                    if matches!(cmd, MpdCommand::Refresh) {
                                        let _ = broadcast_status(&client, &status_feed).await;
                                    } else {
                                        if let Err(e) = handle_command(&client, cmd).await {
                                            log::error!("MPD Execution Error: {e}");
                                        }
                                        let _ = broadcast_status(&client, &status_feed).await;
                                    }
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

    (MpdEngine { tx }, status_tx)
}
