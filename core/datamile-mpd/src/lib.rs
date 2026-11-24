pub mod actor;
pub mod commands;
pub mod config;
pub mod status;

#[cfg(test)]
mod tests;

pub use actor::{MpdEngine, start_actor};
use anyhow::{Context, Result};
pub use commands::MpdCommand;
pub use config::MpdConfig;
use mpd_client::Client;
pub use status::{MpdPlayState, RawMpdStatus, RawQueueSong};
use tokio::net::TcpStream;

/// One-shot query for the file MPD is playing, without starting the actor.
///
/// Used by the CLI when the server is offline, so `x --playing` needs no extra
/// MPD dependency.
pub async fn current_playing_file(addr: &str) -> Result<Option<String>> {
    let stream = TcpStream::connect(addr)
        .await
        .context("Failed to connect to MPD")?;
    let (client, _events) = Client::connect(stream)
        .await
        .context("Failed to initialize MPD client")?;

    let song = client
        .command(mpd_client::commands::CurrentSong)
        .await
        .context("MPD current song query failed")?;

    Ok(song.map(|song| song.song.url))
}
