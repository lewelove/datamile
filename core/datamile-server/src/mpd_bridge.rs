use std::sync::Arc;

use datamile_logic::PlayableLookupItem;
use datamile_mpd::{RawMpdStatus, RawQueueSong};
use serde_json::Value;
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

fn meta_str(meta: Option<&Value>, keys: &[&str]) -> Option<String> {
    let meta = meta?;
    for key in keys {
        match meta.get(*key) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn meta_u64(meta: Option<&Value>, keys: &[&str], fallback: u64) -> u64 {
    let Some(meta) = meta else {
        return fallback;
    };
    for key in keys {
        if let Some(value) = meta.get(*key) {
            if let Some(n) = value.as_u64() {
                return n;
            }
            if let Some(s) = value.as_str()
                && let Ok(n) = s.parse()
            {
                return n;
            }
        }
    }
    fallback
}

fn queue_item(
    song: &RawQueueSong,
    item: Option<&PlayableLookupItem>,
) -> serde_json::Value {
    let Some(item) = item else {
        return serde_json::json!({
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
        });
    };

    let meta = Some(&item.meta);
    serde_json::json!({
        "id": song.id,
        "file": song.file,
        "title": song.title.clone().or_else(|| meta_str(meta, &["title"])),
        "artist": song.artist.clone().or_else(|| meta_str(meta, &["artist", "albumartist"])),
        "media_type": item.media_type,
        "media_id": item.media_id,
        "node_index": item.node_index,
        "track_no": meta_u64(meta, &["track_no", "tracknumber"], 0),
        "disc_no": meta_u64(meta, &["disc_no", "discnumber"], 1),
        "duration": meta_str(meta, &["duration"]).unwrap_or_default(),
        "duration_ms": meta_u64(meta, &["duration_ms", "duration_milliseconds"], 0),
    })
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
