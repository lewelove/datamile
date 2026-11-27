use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, UNIX_EPOCH};

use crate::{
    FileStat, RootState, RootStates, calculate_path_hash, hash_files, load_state,
    root_files, root_is_dirty, save_state, walk_files,
};

/// Writes an empty `<media_type>.lock.json` and returns its fresh stat.
fn write_lock(dir: &Path, media_type: &str) -> FileStat {
    let path = dir.join(format!("{media_type}.lock.json"));
    fs::write(&path, "{}").unwrap();
    FileStat::from(&fs::metadata(&path).unwrap())
}

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

/// A state file round-trips through disk, and a second save of unchanged
/// content leaves the same bytes on disk.
#[test]
fn state_round_trips_and_is_byte_stable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");

    let mut states = RootStates::new();
    states.insert(
        "/music/A".to_string(),
        RootState {
            files: [("album.toml".to_string(), FileStat { mtime: 7, size: 42 })]
                .into_iter()
                .collect(),
            lock_stat: Some(FileStat { mtime: 8, size: 2 }),
        },
    );

    save_state(&states, &path).unwrap();
    assert_eq!(load_state(&path).unwrap(), states);

    let before = fs::read(&path).unwrap();
    save_state(&states, &path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
}

/// A missing state file is empty; a corrupt one is an error, never empty.
#[test]
fn missing_state_is_empty_and_corrupt_state_is_an_error() {
    let dir = tempfile::tempdir().unwrap();

    assert!(
        load_state(&dir.path().join("missing.json"))
            .unwrap()
            .is_empty()
    );

    let path = dir.path().join("state.json");
    fs::write(&path, "not json").unwrap();
    assert!(load_state(&path).is_err());
}

/// A compiled lock is an output: the walk never reports one, so it cannot feed
/// the fingerprint map of the next pass.
#[test]
fn walk_files_skips_lock_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("album.toml"), "a").unwrap();
    fs::write(dir.path().join("album.lock.json"), "{}").unwrap();
    fs::write(dir.path().join("vinyl.lock.json"), "{}").unwrap();

    let walked = walk_files(dir.path());

    assert_eq!(walked.len(), 1, "only the entity manifest is walked");
    assert!(walked.keys().all(|path| path.ends_with("album.toml")));
}

/// A state entry written before lock tracking has no `lock_stat` and loads as
/// `None`, which makes the root recompile once after an upgrade.
#[test]
fn root_state_without_lock_stat_loads_as_none() {
    let entry: RootState =
        serde_json::from_str(r#"{"files":{"/music/A/album.toml":{"mtime":1,"size":2}}}"#)
            .unwrap();

    assert!(entry.lock_stat.is_none());
    assert_eq!(entry.files.len(), 1);
}

/// A root with no state entry is dirty; a matching one is clean.
#[test]
fn dirty_rule_covers_the_walked_set() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("A");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("album.toml"), "a").unwrap();

    let mut walked = walk_files(&root);
    let clean = RootState {
        files: walked.clone(),
        lock_stat: Some(write_lock(&root, "album")),
    };

    assert!(
        root_is_dirty(&root, "album", &walked, None),
        "no entry is dirty"
    );
    assert!(!root_is_dirty(&root, "album", &walked, Some(&clean)));

    fs::write(root.join("01.flac"), "audio").unwrap();
    walked = walk_files(&root);
    assert!(
        root_is_dirty(&root, "album", &walked, Some(&clean)),
        "a new file is dirty"
    );
}

/// The lock is part of freshness: a missing lock, a rewritten one, an entry
/// without a recorded stat, or another media type's lock are all dirty.
#[test]
fn dirty_rule_covers_the_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("A");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("album.toml"), "a").unwrap();

    let walked = walk_files(&root);
    let untracked = RootState {
        files: walked.clone(),
        lock_stat: None,
    };
    assert!(
        root_is_dirty(&root, "album", &walked, Some(&untracked)),
        "an entry without a lock stat is dirty"
    );

    let clean = RootState {
        files: walked.clone(),
        lock_stat: Some(write_lock(&root, "album")),
    };
    assert!(!root_is_dirty(&root, "album", &walked, Some(&clean)));

    fs::write(root.join("album.lock.json"), "{ }").unwrap();
    assert!(
        root_is_dirty(&root, "album", &walked, Some(&clean)),
        "a rewritten lock is dirty"
    );

    fs::remove_file(root.join("album.lock.json")).unwrap();
    assert!(
        root_is_dirty(&root, "album", &walked, Some(&clean)),
        "a missing lock is dirty"
    );

    // The recorded stat belongs to the album lock only; the vinyl lock of the
    // same dir never satisfies it.
    let album = RootState {
        files: walked.clone(),
        lock_stat: Some(write_lock(&root, "album")),
    };
    fs::write(root.join("vinyl.lock.json"), "{\"vinyl\":{}}").unwrap();
    assert!(!root_is_dirty(&root, "album", &walked, Some(&album)));
    assert!(
        root_is_dirty(&root, "vinyl", &walked, Some(&album)),
        "the album stat says nothing about the vinyl lock"
    );
}

/// An external entry inside the root entity's map makes the root dirty when it
/// changes, even though the walk never sees it.
#[test]
fn dirty_rule_covers_external_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("A");
    let external = dir.path().join("genre.toml");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("album.toml"), "a").unwrap();
    fs::write(&external, "genre = \"Rock\"").unwrap();

    let files = root_files(&root, std::slice::from_ref(&external));
    let entry = RootState {
        files,
        lock_stat: Some(write_lock(&root, "album")),
    };
    let walked = walk_files(&root);

    assert!(!root_is_dirty(&root, "album", &walked, Some(&entry)));

    sleep(Duration::from_millis(5));
    fs::write(&external, "genre = \"Jazz\"").unwrap();
    assert!(root_is_dirty(&root, "album", &walked, Some(&entry)));

    fs::remove_file(&external).unwrap();
    assert!(root_is_dirty(&root, "album", &walked, Some(&entry)));
}
