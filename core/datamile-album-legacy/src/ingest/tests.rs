//! Lock ingestion contract: playable nodes come from `album.nodes.track` and
//! their path from `track.location.path`.

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::ingest::parse_album_lock;

fn lock(tracks: Value) -> String {
    json!({
        "album": {
            "id": "Artist - Album",
            "location": { "kind": "dir", "path": ".", "mtime": 1 },
            "tags": { "album": "Album", "albumartist": "Artist" },
            "objects": {
                "info": { "total_tracks": 2, "total_discs": 1, "duration_milliseconds": 1000 }
            },
            "images": {
                "main": [{
                    "address": "addr-1",
                    "width": 200,
                    "height": 200,
                    "fit": "crop",
                    "filter": "lanczos",
                    "source": { "file": { "path": "cover.jpg" } }
                }]
            },
            "nodes": { "track": tracks }
        }
    })
    .to_string()
}

fn physical_track(path: &str, track_no: u32) -> Value {
    json!({
        "location": { "kind": "file", "path": path, "mtime": 1, "byte_size": 2 },
        "tags": { "title": format!("Track {track_no}"), "tracknumber": track_no, "discnumber": 1 },
        "objects": { "info": { "duration_formatted": "1:00", "duration_milliseconds": 60000 } }
    })
}

#[test]
fn playable_nodes_come_from_the_track_locations() {
    let dir = TempDir::new().expect("temp dir");
    let music = dir.path();
    let album = music.join("album");
    std::fs::create_dir_all(&album).expect("album dir");

    let parsed = parse_album_lock(
        &album,
        "Artist - Album",
        &lock(json!([physical_track("01.flac", 1)])),
        music,
    )
    .expect("parse succeeds")
    .expect("album parses");

    assert_eq!(parsed.playables.len(), 1);
    assert_eq!(parsed.playables[0].path, "album/01.flac");
    assert_eq!(parsed.playables[0].media_id, "Artist - Album");
    assert_eq!(parsed.playables[0].track_no, 1);
    assert_eq!(parsed.playables[0].title, "Track 1");
    assert_eq!(parsed.entity.title, "Album");
    assert_eq!(parsed.entity.subtitle, "Artist");
    assert_eq!(parsed.entity.playable_uris, vec!["album/01.flac"]);
    assert_eq!(parsed.entity.image_namespaces.len(), 1);
    assert_eq!(parsed.images[0].0, "addr-1");
}

/// A virtual node has no location and is therefore not playable.
#[test]
fn virtual_nodes_are_not_playable() {
    let dir = TempDir::new().expect("temp dir");
    let album = dir.path().join("album");
    std::fs::create_dir_all(&album).expect("album dir");

    let virtual_track = json!({
        "tags": { "title": "Interlude" },
        "objects": { "info": { "duration_formatted": "0:10", "duration_milliseconds": 10000 } }
    });

    let parsed = parse_album_lock(
        &album,
        "Artist - Album",
        &lock(json!([virtual_track])),
        dir.path(),
    )
    .expect("parse succeeds")
    .expect("album parses");

    assert!(parsed.playables.is_empty());
    assert!(parsed.entity.playable_uris.is_empty());
}

/// A lock without the album object cannot produce a record.
#[test]
fn missing_album_is_skipped() {
    let dir = TempDir::new().expect("temp dir");

    let parsed = parse_album_lock(&dir.path().join("x"), "x", "{}", dir.path())
        .expect("parse succeeds");

    assert!(parsed.is_none());
}
