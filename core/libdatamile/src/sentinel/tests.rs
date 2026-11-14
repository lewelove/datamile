use std::fs;
use std::path::Path;
use std::time::SystemTime;

use serde_json::{Value, json};

use super::{TrustState, verify_trust};

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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
        TrustState::Valid
    );
}

/// A file-backed root stores its own location in the lock and resolves it
/// against the lock directory.
#[test]
fn test_file_backed_root_location() {
    let tmp = album_root();
    let root = tmp.path();
    let film = root.join("Dune.mkv");
    fs::write(&film, b"video").expect("film");

    fs::write(
        root.join("Dune.lock.json"),
        serde_json::to_string_pretty(&json!({
            "film": {
                "id": "Dune",
                "location": {
                    "kind": "file",
                    "path": film.to_string_lossy(),
                    "mtime": mtime_secs(&film),
                    "byte_size": byte_size(&film),
                }
            }
        }))
        .expect("serialize"),
    )
    .expect("lock");

    let lock = root.join("Dune.lock.json");
    assert_eq!(
        verify_trust(&lock, "film", Some("Dune")).expect("trust"),
        TrustState::Valid
    );

    fs::remove_file(&film).expect("remove");
    assert_eq!(
        verify_trust(&lock, "film", Some("Dune")).expect("trust"),
        TrustState::BrokenPhysics
    );
}

/// Asking for the wrong root type cannot trust a foreign lock.
#[test]
fn test_root_type_must_match() {
    let tmp = album_root();
    let root = tmp.path();
    write_lock(
        root,
        &json!({ "album": { "id": "A", "objects": { "info": { "virtual": true } } } }),
    );

    assert_eq!(
        verify_trust(&root.join("album.lock.json"), "film", None).expect("trust"),
        TrustState::Missing
    );
}

/// Physics checks recurse through nested child arrays (show -> season -> episode).
#[test]
fn test_nested_children_physics() {
    let tmp = album_root();
    let root = tmp.path();
    let episode = root.join("ep1.mkv");
    fs::write(&episode, b"episode").expect("episode");

    write_lock(
        root,
        &json!({
            "show": {
                "id": "Show",
                "seasons": [
                    {
                        "id": "Season 1",
                        "episodes": [
                            {
                                "id": "ep1",
                                "location": {
                                    "kind": "file",
                                    "path": "ep1.mkv",
                                    "mtime": mtime_secs(&episode),
                                    "byte_size": byte_size(&episode),
                                }
                            }
                        ]
                    }
                ]
            }
        }),
    );

    let lock = root.join("album.lock.json");
    assert_eq!(
        verify_trust(&lock, "show", None).expect("trust"),
        TrustState::Valid
    );

    fs::remove_file(&episode).expect("remove");
    assert_eq!(
        verify_trust(&lock, "show", None).expect("trust"),
        TrustState::BrokenPhysics
    );
}

/// Attached `files` entries are part of the intentional provenance surface.
#[test]
fn test_files_integrity() {
    let tmp = album_root();
    let root = tmp.path();
    let manifest = root.join("album.toml");
    fs::write(&manifest, "[album]\n").expect("manifest");

    write_lock(
        root,
        &json!({
            "album": {
                "id": "A",
                "files": {
                    "manifest": {
                        "path": "album.toml",
                        "mtime": mtime_secs(&manifest),
                        "byte_size": byte_size(&manifest),
                    }
                },
                "objects": { "info": { "virtual": true } }
            }
        }),
    );

    let lock = root.join("album.lock.json");
    assert_eq!(
        verify_trust(&lock, "album", Some("A")).expect("trust"),
        TrustState::Valid
    );

    fs::write(&manifest, "[album]\nalbum = \"A\"\n").expect("manifest change");
    assert_eq!(
        verify_trust(&lock, "album", Some("A")).expect("trust"),
        TrustState::BrokenIntent
    );
}

/// A location kind must agree with the physical path.
#[test]
fn test_location_kind_must_match_physics() {
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
                            "kind": "dir",
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
        verify_trust(&root.join("album.lock.json"), "album", Some("A")).expect("trust"),
        TrustState::BrokenPhysics
    );
}
