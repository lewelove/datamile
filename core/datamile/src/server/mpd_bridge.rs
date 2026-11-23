use std::sync::Arc;

use datamile_logic::{LogicEngine, PlayableLookupItem};
use datamile_mpd::{RawMpdStatus, RawQueueSong};
use tokio::sync::{RwLock, broadcast};

fn track_meta<'a>(logic: &'a LogicEngine, file: &str) -> Option<&'a PlayableLookupItem> {
    let clean = file.trim_start_matches('/');
    logic
        .playable_lookup
        .get(clean)
        .or_else(|| logic.playable_lookup.get(file))
}

fn queue_item(
    song: &RawQueueSong,
    meta: Option<&PlayableLookupItem>,
) -> serde_json::Value {
    meta.map_or_else(
        || {
            serde_json::json!({
                "id": song.id,
                "file": song.file,
                "title": song.title,
                "artist": song.artist,
                "album_id": serde_json::Value::Null,
                "track_index": serde_json::Value::Null,
                "track_no": serde_json::Value::Null,
                "disc_no": 1,
                "duration": "",
                "duration_ms": 0,
            })
        },
        |m| {
            serde_json::json!({
                "id": song.id,
                "file": song.file,
                "title": song.title.clone().or_else(|| Some(m.title.clone())),
                "artist": song.artist.clone().or_else(|| Some(m.artist.clone())),
                "album_id": m.media_id,
                "track_index": m.node_index,
                "track_no": m.track_no,
                "disc_no": m.disc_no,
                "duration": m.duration,
                "duration_ms": m.duration_ms,
            })
        },
    )
}

/// Adds Datamile identifiers to a raw MPD snapshot and publishes it on `/ws`.
fn enrich(raw: &RawMpdStatus, logic: &LogicEngine) -> String {
    let current = track_meta(logic, &raw.file);

    let queue: Vec<serde_json::Value> = raw
        .queue
        .iter()
        .map(|song| queue_item(song, track_meta(logic, &song.file)))
        .collect();

    serde_json::json!({
        "type": "MPD_STATUS",
        "state": raw.state,
        "file": raw.file,
        "album_id": current.map(|m| m.media_id.clone()),
        "track_index": current.map(|m| m.node_index as u64),
        "elapsed": raw.elapsed,
        "duration": raw.duration,
        "title": raw.title,
        "artist": raw.artist,
        "queue": queue
    })
    .to_string()
}

pub fn start_status_bridge(
    status_tx: &broadcast::Sender<RawMpdStatus>,
    logic: Arc<RwLock<LogicEngine>>,
    tx: broadcast::Sender<String>,
) {
    let mut rx = status_tx.subscribe();
    tokio::spawn(async move {
        while let Ok(raw) = rx.recv().await {
            let payload = {
                let guard = logic.read().await;
                enrich(&raw, &guard)
            };
            let _ = tx.send(payload);
        }
    });
}
