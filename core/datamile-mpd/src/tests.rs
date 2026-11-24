use serde_json::json;

use crate::config::MpdConfig;
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

/// Every field has a sane default so an empty `mpd` table still connects.
#[test]
fn mpd_config_defaults_point_at_localhost() {
    let config = MpdConfig::default();

    assert!(config.enabled);
    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 6600);
    assert_eq!(config.music_directory, None);
    assert_eq!(config.addr(), "127.0.0.1:6600");
}

/// Partial declarations keep the defaults for the omitted fields.
#[test]
fn mpd_config_partial_keeps_defaults() {
    let config: MpdConfig = serde_json::from_str(r#"{ "port": 7000 }"#).expect("parse");

    assert_eq!(config.host, "127.0.0.1");
    assert_eq!(config.port, 7000);
    assert!(config.enabled);
}

/// The block round-trips, including an explicit music directory.
#[test]
fn mpd_config_round_trips() {
    let config = MpdConfig {
        enabled: false,
        host: "mpd.local".to_string(),
        port: 6601,
        music_directory: Some("~/library".to_string()),
    };

    let json = serde_json::to_value(&config).expect("serialize");
    let back: MpdConfig = serde_json::from_value(json).expect("deserialize");

    assert_eq!(config, back);
}
