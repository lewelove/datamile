use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::resolve_album_image;

fn write_lock(album_dir: &Path, image_path: &Path) {
    let lock = serde_json::json!({
        "album": { "images": { "main": [{ "source": { "file": { "path": image_path } } }] } }
    });
    fs::write(album_dir.join("album.lock.json"), lock.to_string()).expect("write lock");
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, b"x").expect("write file");
}

/// The lock image wins over candidate files sitting in the album directory.
#[test]
fn lock_image_wins_over_candidates() {
    let album = TempDir::new().expect("album dir");
    write_lock(album.path(), Path::new("art/main.png"));
    touch(&album.path().join("art/main.png"));
    touch(&album.path().join("cover.jpg"));

    assert_eq!(
        resolve_album_image(album.path()),
        Some(album.path().join("art/main.png"))
    );
}

/// An absolute lock path is used as-is instead of being joined to the album.
#[test]
fn absolute_lock_path_is_returned_unchanged() {
    let album = TempDir::new().expect("album dir");
    let outside = TempDir::new().expect("outside dir");
    let image = outside.path().join("art.png");
    touch(&image);
    write_lock(album.path(), &image);

    assert_eq!(resolve_album_image(album.path()), Some(image));
}

/// A lock path whose file disappeared falls back to the probe list.
#[test]
fn missing_lock_target_falls_back_to_candidates() {
    let album = TempDir::new().expect("album dir");
    write_lock(album.path(), Path::new("art/gone.png"));
    touch(&album.path().join("front.jpg"));

    assert_eq!(
        resolve_album_image(album.path()),
        Some(album.path().join("front.jpg"))
    );
}

/// Candidates are probed in the documented order, first match wins.
#[test]
fn candidates_are_probed_in_order() {
    let album = TempDir::new().expect("album dir");
    touch(&album.path().join("front.png"));
    touch(&album.path().join("folder.jpg"));
    touch(&album.path().join("cover.png"));

    assert_eq!(
        resolve_album_image(album.path()),
        Some(album.path().join("cover.png"))
    );
}

/// No lock and no candidate means no artwork, not a panic.
#[test]
fn missing_lock_and_candidates_returns_none() {
    let album = TempDir::new().expect("album dir");

    assert_eq!(resolve_album_image(album.path()), None);
}
