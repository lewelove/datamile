use super::{TrustState, verify_trust};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .expect("metadata")
        .modified()
        .expect("mtime")
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("epoch")
        .as_secs()
}

fn byte_size(path: &Path) -> u64 {
    fs::metadata(path).expect("metadata").len()
}

fn write_lock(album_root: &Path, value: &Value) {
    fs::write(
        album_root.join("album.lock.json"),
        serde_json::to_string_pretty(value).expect("serialize"),
    )
    .expect("lock");
}

fn album_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Manifest mtimes live under `/album/objects/manifests` and a divergence in
/// seconds must be reported as `BrokenIntent`.
#[test]
fn test_manifest_mtime_compiled_under_objects() {
    let tmp = album_root();
    let root = tmp.path();
    let manifest = root.join("album.toml");
    fs::write(&manifest, "[album]\nalbum = \"A\"\n").expect("manifest");

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "objects": {
                    "manifests": {
                        "album": {
                            "file": {
                                "path": "album.toml",
                                "mtime": mtime_secs(&manifest) + 10,
                            }
                        }
                    }
                }
            }
        }),
    );

    assert_eq!(
        verify_trust(root, Some("A")).expect("trust"),
        TrustState::BrokenIntent
    );
}

/// Track provenance lives under `location`; a matching record is trusted.
#[test]
fn test_tracks_integrity_reads_location() {
    let tmp = album_root();
    let root = tmp.path();
    let track = root.join("track.flac");
    fs::write(&track, b"audio").expect("track");

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "tracks": [
                    {
                        "id": "A - 1",
                        "location": {
                            "kind": "file",
                            "path": "track.flac",
                            "mtime": mtime_secs(&track),
                            "byte_size": byte_size(&track),
                        }
                    }
                ]
            }
        }),
    );

    assert_eq!(
        verify_trust(root, Some("A")).expect("trust"),
        TrustState::Valid
    );
}

/// A byte size divergence is physical, not intentional.
#[test]
fn test_tracks_integrity_detects_size_change() {
    let tmp = album_root();
    let root = tmp.path();
    let track = root.join("track.flac");
    fs::write(&track, b"audio").expect("track");

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "tracks": [
                    {
                        "id": "A - 1",
                        "location": {
                            "kind": "file",
                            "path": "track.flac",
                            "mtime": mtime_secs(&track),
                            "byte_size": byte_size(&track) + 1,
                        }
                    }
                ]
            }
        }),
    );

    assert_eq!(
        verify_trust(root, Some("A")).expect("trust"),
        TrustState::BrokenPhysics
    );
}

/// Tracks without compiled provenance cannot be trusted.
#[test]
fn test_tracks_integrity_requires_location() {
    let tmp = album_root();
    let root = tmp.path();

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "tracks": [{ "id": "A - 1" }]
            }
        }),
    );

    assert_eq!(
        verify_trust(root, Some("A")).expect("trust"),
        TrustState::BrokenPhysics
    );
}

/// Virtual albums have no physical tracks and stay valid.
#[test]
fn test_tracks_integrity_skips_virtual_albums() {
    let tmp = album_root();
    let root = tmp.path();

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "objects": { "info": { "virtual": true } },
                "tracks": [{ "id": "A - 1" }]
            }
        }),
    );

    assert_eq!(
        verify_trust(root, Some("A")).expect("trust"),
        TrustState::Valid
    );
}
