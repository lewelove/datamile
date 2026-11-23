use serde_json::json;

use crate::status::{MpdPlayState, RawMpdStatus, RawQueueSong};

/// The wire state is lowercase so the WebSocket payload can be forwarded as is.
#[test]
fn play_state_serializes_lowercase() {
    assert_eq!(
        serde_json::to_value(MpdPlayState::Play).unwrap(),
        json!("play")
    );
    assert_eq!(
        serde_json::to_value(MpdPlayState::Pause).unwrap(),
        json!("pause")
    );
    assert_eq!(
        serde_json::to_value(MpdPlayState::Stop).unwrap(),
        json!("stop")
    );
}

/// A raw snapshot carries no Datamile identifiers; callers add them.
#[test]
fn raw_status_serializes_queue_without_media_fields() {
    let status = RawMpdStatus {
        state: MpdPlayState::Pause,
        file: "Artist/Album/01.flac".to_string(),
        elapsed: 12.5,
        duration: 240.0,
        title: Some("Track".to_string()),
        artist: Some("Artist".to_string()),
        queue: vec![RawQueueSong {
            id: 0,
            file: "Artist/Album/01.flac".to_string(),
            title: Some("Track".to_string()),
            artist: Some("Artist".to_string()),
        }],
    };

    let value = serde_json::to_value(&status).unwrap();

    assert_eq!(value["state"], json!("pause"));
    assert_eq!(value["queue"][0]["id"], json!(0));
    assert_eq!(value["queue"][0]["file"], json!("Artist/Album/01.flac"));
    assert!(value["queue"][0].get("media_id").is_none());
    assert!(value["queue"][0].get("node_index").is_none());
}
