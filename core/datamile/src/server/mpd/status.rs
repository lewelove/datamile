use std::sync::Arc;

use anyhow::{Context, Result};
use mpd_client::responses::{PlayState, SongInQueue};
use mpd_client::{Client, commands};
use tokio::sync::{RwLock, broadcast};

use crate::server::logic::{LogicEngine, TrackLookupItem};

fn format_queue_item(
    idx: usize,
    s: &SongInQueue,
    meta: Option<TrackLookupItem>,
) -> serde_json::Value {
    meta.map_or_else(
        || {
            serde_json::json!({
                "id": idx,
                "file": s.song.url,
                "title": s.song.title(),
                "artist": s.song.artists().first(),
                "album_id": serde_json::Value::Null,
                "track_index": serde_json::Value::Null,
                "track_no": serde_json::Value::Null,
                "disc_no": 1,
                "duration": "",
                "duration_ms": 0,
            })
        },
        |m| {
            let title = if m.title.is_empty() {
                s.song.title().map(ToString::to_string)
            } else {
                Some(m.title)
            };
            let artist = if m.artist.is_empty() {
                s.song.artists().first().map(ToString::to_string)
            } else {
                Some(m.artist)
            };
            serde_json::json!({
                "id": idx,
                "file": s.song.url,
                "title": title,
                "artist": artist,
                "album_id": m.album_id,
                "track_index": m.track_index,
                "track_no": m.track_no,
                "disc_no": m.disc_no,
                "duration": m.duration,
                "duration_ms": m.duration_ms,
            })
        },
    )
}

pub async fn broadcast_status(
    client: &Client,
    tx: &broadcast::Sender<String>,
    logic: &Arc<RwLock<LogicEngine>>,
) -> Result<()> {
    let (status, current_song, queue) = client
        .command_list((commands::Status, commands::CurrentSong, commands::Queue))
        .await
        .context("Batched status update failed")?;

    let (file_path, title, artist) = current_song.as_ref().map_or_else(
        || (String::new(), None, None),
        |s| {
            (
                s.song.url.clone(),
                s.song.title().map(ToString::to_string),
                s.song.artists().first().map(ToString::to_string),
            )
        },
    );

    let (queue_json, album_id, track_index) = {
        let l = logic.read().await;
        let clean_file_path = file_path.trim_start_matches('/');
        let current_track = l
            .track_lookup
            .get(clean_file_path)
            .or_else(|| l.track_lookup.get(&file_path));
        let album_id = current_track.map(|t| t.album_id.clone());
        let track_index = current_track.map(|t| t.track_index as u64);
        let track_metas: Vec<Option<TrackLookupItem>> = queue
            .iter()
            .map(|s| {
                let clean_url = s.song.url.trim_start_matches('/');
                l.track_lookup
                    .get(clean_url)
                    .or_else(|| l.track_lookup.get(&s.song.url))
                    .cloned()
            })
            .collect();
        drop(l);

        let q_json: serde_json::Value = queue
            .iter()
            .enumerate()
            .zip(track_metas)
            .map(|((idx, s), meta)| format_queue_item(idx, s, meta))
            .collect();

        (q_json, album_id, track_index)
    };

    let state_str = match status.state {
        PlayState::Playing => "play",
        PlayState::Paused => "pause",
        PlayState::Stopped => "stop",
    };

    let payload = serde_json::json!({
        "type": "MPD_STATUS",
        "state": state_str,
        "file": file_path,
        "album_id": album_id,
        "track_index": track_index,
        "elapsed": status.elapsed.map_or(0.0, |t| t.as_secs_f64()),
        "duration": status.duration.map_or(0.0, |t| t.as_secs_f64()),
        "title": title,
        "artist": artist,
        "queue": queue_json
    });

    let _ = tx.send(payload.to_string());
    Ok(())
}
