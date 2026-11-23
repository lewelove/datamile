use std::fs;
use std::path::Path;

use serde_json::json;

use crate::{
    AudioError, is_supported_audio_file, is_supported_extension, probe_file,
    probe_file_cached, sanitize_key, validate_extensions,
};

/// Small tagged MPEG file shared with the embed action's tests.
const FIXTURE_MP3: &[u8] =
    include_bytes!("../../../actions/rust/embed/tests/fixtures/full_test.mp3");

/// Extension helpers are case-insensitive and reject unknown formats.
#[test]
fn extension_matching_ignores_case() {
    assert!(is_supported_extension("flac"));
    assert!(is_supported_extension("FLAC"));
    assert!(!is_supported_extension("txt"));

    assert!(is_supported_audio_file(Path::new("track.MP3")));
    assert!(!is_supported_audio_file(Path::new("cover.png")));
    assert!(!is_supported_audio_file(Path::new("no_extension")));
}

/// Configured extensions must match the registry exactly; validation keeps the
/// historical case-sensitive behavior.
#[test]
fn validate_extensions_accepts_known_and_rejects_unknown() {
    let valid = validate_extensions(&["flac".to_string(), "mp3".to_string()]).unwrap();
    assert_eq!(valid, ["flac", "mp3"]);

    let error = validate_extensions(&["FLAC".to_string()]).unwrap_err();
    assert!(matches!(
        error,
        AudioError::UnsupportedExtension { ref extension } if extension == "FLAC"
    ));
}

/// Tag keys collapse punctuation, whitespace and uppercase into snake_case.
#[test]
fn sanitize_key_normalizes_tag_names() {
    assert_eq!(sanitize_key("Album Artist"), "album_artist");
    assert_eq!(sanitize_key("TPE1"), "tpe1");
    assert_eq!(
        sanitize_key("  --MusicBrainz Release Group ID--  "),
        "musicbrainz_release_group_id"
    );
}

/// Probing the fixture extracts its artist tag and MPEG physics.
#[test]
fn probe_file_reads_tags_and_physics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.mp3");
    fs::write(&path, FIXTURE_MP3).unwrap();

    let metadata = probe_file(&path).unwrap();

    assert_eq!(metadata.path, path);
    assert_eq!(metadata.physics.sample_rate, 48_000);
    assert_eq!(metadata.physics.channels, 2);
    assert_eq!(metadata.physics.format, "Mpeg");
    assert!(metadata.physics.duration_ms > 0);
    assert_eq!(metadata.tags.get("artist"), Some(&json!("Foo artist")));
}

/// Cached probes live under `<cache_root>/objects` and are reused, so repeated
/// calls never create the retired `harvest` namespace.
#[test]
fn probe_file_cached_uses_the_objects_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.mp3");
    fs::write(&path, FIXTURE_MP3).unwrap();
    let cache_root = dir.path().join("cache");

    let first = probe_file_cached(&path, &cache_root).unwrap();
    let second = probe_file_cached(&path, &cache_root).unwrap();

    assert_eq!(first.physics.sample_rate, second.physics.sample_rate);
    assert_eq!(first.tags, second.tags);
    assert!(!cache_root.join("harvest").exists());

    let cached: Vec<_> = fs::read_dir(cache_root.join("objects"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(cached.len(), 1, "expected a single object blob");
}
