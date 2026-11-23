use std::sync::Arc;

use datamile_logic::PlayableLookupItem;
use datamile_mpd::{RawMpdStatus, RawQueueSong};
use tokio::sync::broadcast;

use crate::state::ServerState;

fn track_meta<'a>(
    logic: &'a datamile_logic::LogicEngine,
    file: &str,
) -> Option<&'a PlayableLookupItem> {
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
                "media_type": serde_json::Value::Null,
                "media_id": serde_json::Value::Null,
                "node_index": serde_json::Value::Null,
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
                "media_type": m.media_type,
                "media_id": m.media_id,
                "node_index": m.node_index,
                "track_no": m.track_no,
                "disc_no": m.disc_no,
                "duration": m.duration,
                "duration_ms": m.duration_ms,
            })
        },
    )
}

/// Adds Datamile identifiers to a raw MPD snapshot and publishes it on `/ws`.
async fn publish(raw: &RawMpdStatus, state: &ServerState) {
    let (media_type, media_id, node_index, queue) = {
        let logic = state.logic.read().await;
        let current = track_meta(&logic, &raw.file);

        let queue: Vec<serde_json::Value> = raw
            .queue
            .iter()
            .map(|song| queue_item(song, track_meta(&logic, &song.file)))
            .collect();

        let media_type = current.map(|m| m.media_type.clone());
        let media_id = current.map(|m| m.media_id.clone());
        let node_index = current.map(|m| m.node_index);
        drop(logic);

        (media_type, media_id, node_index, queue)
    };

    let payload = serde_json::json!({
        "type": "MPD_STATUS",
        "state": raw.state,
        "media_type": media_type,
        "media_id": media_id,
        "node_index": node_index,
        "file": raw.file,
        "elapsed": raw.elapsed,
        "duration": raw.duration,
        "title": raw.title,
        "artist": raw.artist,
        "queue": queue
    })
    .to_string();

    *state.now_playing.write().await = raw.file.clone();
    let _ = state.tx.send(payload);
}

pub fn start_status_bridge(
    status_tx: &broadcast::Sender<RawMpdStatus>,
    state: Arc<ServerState>,
) {
    let mut rx = status_tx.subscribe();

    tokio::spawn(async move {
        while let Ok(raw) = rx.recv().await {
            publish(&raw, &state).await;
        }
    });
}
