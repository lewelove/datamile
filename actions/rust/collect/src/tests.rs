use crate::fs::create_album_directory;
use crate::models::{AlbumData, FormattingConfig, Track};
use tempfile::tempdir;
use walkdir::WalkDir;

/// Verify that create_album_directory produces only TOML manifests and no JSON files.
#[test]
fn test_create_album_directory_no_json_files() {
    let temp = tempdir().expect("Failed to create temporary test directory");
    let album_data = AlbumData {
        albumartist: "Artist".to_string(),
        album: "Album".to_string(),
        date: "2024".to_string(),
        tracks: vec![Track {
            discnumber: 1,
            tracknumber: 1,
            title: "Track 1".to_string(),
            artist: None,
        }],
    };
    let formatting = FormattingConfig {
        album: "{albumartist} - {album}".to_string(),
    };

    create_album_directory(&album_data, &formatting, temp.path())
        .expect("Failed to create album directory");

    let album_dir = temp.path().join("Artist - Album");
    assert!(album_dir.exists(), "Album directory should exist");
    assert!(
        album_dir.join("album.toml").exists(),
        "album.toml should exist"
    );
    assert!(
        !album_dir.join("metadata.toml").exists(),
        "metadata.toml must not be written"
    );
    assert!(
        album_dir.join("history.toml").exists(),
        "history.toml should exist"
    );
    assert!(
        album_dir.join("virtual.toml").exists(),
        "virtual.toml should exist"
    );

    for entry in WalkDir::new(&album_dir).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if let Some(ext) = path.extension() {
            assert_ne!(
                ext,
                "json",
                "No JSON files should be created, found: {}",
                path.display()
            );
        }
    }
}
