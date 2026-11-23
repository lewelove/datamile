use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, UNIX_EPOCH};

use crate::{
    FileStat, calculate_path_hash, hash_files, load_stat_cache, save_stat_cache,
};

/// A stat carries the byte size and the epoch nanoseconds of the file mtime.
#[test]
fn file_stat_mirrors_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, b"hello").unwrap();

    let meta = fs::metadata(&path).unwrap();
    let stat = FileStat::from(&meta);
    let expected_mtime = meta
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    assert_eq!(stat.size, 5);
    assert_eq!(stat.mtime, expected_mtime);
}

/// The library hash identifies a directory, not its contents.
#[test]
fn path_hash_depends_only_on_the_path() {
    let hash = calculate_path_hash(Path::new("/music/library"));

    assert_eq!(hash, calculate_path_hash(Path::new("/music/library")));
    assert_ne!(hash, calculate_path_hash(Path::new("/music/other")));
}

/// `hash_files` ignores the order it is given but notices edited contents.
#[test]
fn hash_files_is_order_independent_and_content_sensitive() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.lua");
    let second = dir.path().join("second.lua");
    fs::write(&first, "a = 1").unwrap();
    fs::write(&second, "b = 2").unwrap();

    let original = hash_files(&[first.clone(), second.clone()]);
    assert_eq!(hash_files(&[second.clone(), first.clone()]), original);

    sleep(Duration::from_millis(5));
    fs::write(&second, "b = 3").unwrap();
    assert_ne!(hash_files(&[first, second]), original);
}

/// A saved stat cache round-trips through disk, while a missing or corrupt
/// file reads as an empty cache.
#[test]
fn stat_cache_round_trips_and_tolerates_bad_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");

    let mut cache = HashMap::new();
    cache.insert("album.toml".to_string(), FileStat { mtime: 7, size: 42 });

    save_stat_cache(&cache, &path).unwrap();
    assert_eq!(load_stat_cache(&path), cache);

    fs::write(&path, "not json").unwrap();
    assert!(load_stat_cache(&path).is_empty());

    assert!(load_stat_cache(&dir.path().join("missing.json")).is_empty());
}
