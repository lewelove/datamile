use std::path::{Path, PathBuf};

/// Filenames the engine itself probes when no lock information is available.
const COVER_CANDIDATES: &[&str] = &[
    "cover.jpg",
    "cover.png",
    "folder.jpg",
    "front.jpg",
    "cover.jpeg",
    "front.png",
];

/// Resolves `album.images.main[0].source.file.path` from `album.lock.json`.
///
/// This is the single place that knows where the lock keeps album artwork, so a
/// lock schema change only has to touch this function.
#[must_use]
pub fn resolve_album_image_from_lock(album_dir: &Path) -> Option<PathBuf> {
    let lock_path = album_dir.join("album.lock.json");
    let content = std::fs::read_to_string(lock_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let path_str = val
        .pointer("/album/images/main/0/source/file/path")
        .and_then(serde_json::Value::as_str)?;

    let target = if Path::new(path_str).is_absolute() {
        PathBuf::from(path_str)
    } else {
        album_dir.join(path_str)
    };

    target.is_file().then_some(target)
}

/// Falls back to the well-known artwork filenames in the album directory.
#[must_use]
pub fn resolve_cover_candidate(album_dir: &Path) -> Option<PathBuf> {
    COVER_CANDIDATES
        .iter()
        .map(|name| album_dir.join(name))
        .find(|path| path.is_file())
}
