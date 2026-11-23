use anyhow::{Context, Result};
use mpd_client::responses::PlayState;
use mpd_client::{Client, commands};
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MpdPlayState {
    Play,
    Pause,
    Stop,
}

impl MpdPlayState {
    const fn from_mpd(state: PlayState) -> Self {
        match state {
            PlayState::Playing => Self::Play,
            PlayState::Paused => Self::Pause,
            PlayState::Stopped => Self::Stop,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RawQueueSong {
    pub id: usize,
    pub file: String,
    pub title: Option<String>,
    pub artist: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RawMpdStatus {
    pub state: MpdPlayState,
    pub file: String,
    pub elapsed: f64,
    pub duration: f64,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub queue: Vec<RawQueueSong>,
}

/// Reads the current player, song and queue, then publishes the raw snapshot.
///
/// The payload is deliberately free of any Datamile domain knowledge: callers
/// enrich it with their own media lookups before broadcasting.
pub async fn broadcast_status(
    client: &Client,
    tx: &broadcast::Sender<RawMpdStatus>,
) -> Result<()> {
    let (status, current_song, queue) = client
        .command_list((commands::Status, commands::CurrentSong, commands::Queue))
        .await
        .context("Batched status update failed")?;

    let (file, title, artist) = current_song.as_ref().map_or_else(
        || (String::new(), None, None),
        |s| {
            (
                s.song.url.clone(),
                s.song.title().map(ToString::to_string),
                s.song.artists().first().map(ToString::to_string),
            )
        },
    );

    let queue = queue
        .iter()
        .enumerate()
        .map(|(id, song)| RawQueueSong {
            id,
            file: song.song.url.clone(),
            title: song.song.title().map(ToString::to_string),
            artist: song.song.artists().first().map(ToString::to_string),
        })
        .collect();

    let _ = tx.send(RawMpdStatus {
        state: MpdPlayState::from_mpd(status.state),
        file,
        elapsed: status.elapsed.map_or(0.0, |t| t.as_secs_f64()),
        duration: status.duration.map_or(0.0, |t| t.as_secs_f64()),
        title,
        artist,
        queue,
    });

    Ok(())
}
