//! Lock shape contract: physical tracks carry a `location`, virtual ones do
//! not, and the root always describes its own directory.

use serde_json::json;
use tempfile::TempDir;

use crate::compile::build::album_root_location;
use crate::compile::tracks::track_location;

#[test]
fn physical_track_location_carries_file_metadata() {
    let ctx_track = json!({
        "file": { "path": "01.flac", "mtime": 1700000000, "byte_size": 31000000 }
    });

    assert_eq!(
        track_location(&ctx_track),
        Some(json!({
            "kind": "file",
            "path": "01.flac",
            "mtime": 1700000000,
            "byte_size": 31000000
        }))
    );
}

/// A virtual track has an empty path, so it must not emit `location`.
#[test]
fn virtual_track_has_no_location() {
    let ctx_track = json!({ "file": { "path": "", "mtime": 0, "byte_size": 0 } });

    assert_eq!(track_location(&ctx_track), None);
}

#[test]
fn root_location_is_the_directory_itself() {
    let dir = TempDir::new().expect("temp dir");

    let location = album_root_location(dir.path());

    assert_eq!(location["kind"], json!("dir"));
    assert_eq!(location["path"], json!("."));
    assert!(location["mtime"].as_u64().is_some());
}
