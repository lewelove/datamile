use serde_json::json;

use super::{JumpBody, PlayBody};

/// `track_ids` is optional; omitting it means "the whole entity".
#[test]
fn play_body_defaults_to_all_tracks() {
    let body: PlayBody = serde_json::from_value(json!({
        "type": "album",
        "id": "Artist - Album"
    }))
    .expect("body");

    assert_eq!(body.root_type, "album");
    assert_eq!(body.id, "Artist - Album");
    assert!(body.track_ids.is_none());
    assert_eq!(body.offset, 0);
}

/// Explicit track ids and an offset survive deserialization.
#[test]
fn play_body_keeps_track_selection() {
    let body: PlayBody = serde_json::from_value(json!({
        "type": "album",
        "id": "Artist - Album",
        "track_ids": ["b", "a"],
        "offset": 1
    }))
    .expect("body");

    assert_eq!(body.track_ids, Some(vec!["b".to_string(), "a".to_string()]));
    assert_eq!(body.offset, 1);
}

/// Jump addresses the queue by index only.
#[test]
fn jump_body_reads_index() {
    let body: JumpBody = serde_json::from_value(json!({ "index": 7 })).expect("body");
    assert_eq!(body.index, 7);
}
