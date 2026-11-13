use super::{DependencyGraph, RootMeta};
use std::collections::HashSet;
use std::path::PathBuf;

fn root_meta(target: &str, files: &[&str], dirs: &[&str]) -> RootMeta {
    RootMeta {
        root_type: "album".to_string(),
        target: PathBuf::from(target),
        files: files.iter().map(PathBuf::from).collect(),
        dirs: dirs.iter().map(PathBuf::from).collect(),
    }
}

/// Re-registering a lock drops stale reverse edges so removed dependencies no
/// longer mark the root dirty.
#[test]
fn update_lock_drops_stale_edges() {
    let mut graph = DependencyGraph::default();
    let lock = PathBuf::from("/music/a/album.lock.json");

    graph.update_lock(
        lock.clone(),
        root_meta("/music/a", &["/music/a/1.flac", "/music/a/2.flac"], &[]),
    );
    graph.update_lock(
        lock.clone(),
        root_meta("/music/a", &["/music/a/1.flac"], &[]),
    );

    assert!(
        graph
            .locks_for_file(&PathBuf::from("/music/a/2.flac"))
            .is_none()
    );
    assert_eq!(
        graph.locks_for_file(&PathBuf::from("/music/a/1.flac")),
        Some(&HashSet::from([lock]))
    );
}

/// Two file-backed roots may live in one directory; each keeps its own lock.
#[test]
fn sibling_roots_share_a_directory() {
    let mut graph = DependencyGraph::default();
    let dir = PathBuf::from("/films");
    let dune_lock = dir.join("Dune.lock.json");
    let arrival_lock = dir.join("Arrival.lock.json");

    graph.update_lock(
        dune_lock.clone(),
        root_meta("/films/Dune.mkv", &["/films/Dune.mkv"], &["/films"]),
    );
    graph.update_lock(
        arrival_lock.clone(),
        root_meta("/films/Arrival.mkv", &["/films/Arrival.mkv"], &["/films"]),
    );

    assert_eq!(
        graph.locks_for_dir(&dir),
        Some(&HashSet::from([dune_lock.clone(), arrival_lock.clone()]))
    );

    graph.remove_lock(&dune_lock);
    assert_eq!(
        graph.locks_for_dir(&dir),
        Some(&HashSet::from([arrival_lock]))
    );
}

/// Removing a root clears both directions of the graph.
#[test]
fn remove_lock_clears_reverse_edges() {
    let mut graph = DependencyGraph::default();
    let lock = PathBuf::from("/music/a/album.lock.json");
    graph.update_lock(
        lock.clone(),
        root_meta("/music/a", &["/music/a/1.flac"], &["/music/a"]),
    );

    graph.remove_lock(&lock);

    assert!(
        graph
            .locks_for_file(&PathBuf::from("/music/a/1.flac"))
            .is_none()
    );
    assert!(graph.locks_for_dir(&PathBuf::from("/music/a")).is_none());
    assert!(graph.lock_to_meta.is_empty());
}

/// `locks_under` matches both directory targets and file targets.
#[test]
fn locks_under_matches_targets() {
    let mut graph = DependencyGraph::default();
    let album_lock = PathBuf::from("/music/a/album.lock.json");
    let film_lock = PathBuf::from("/films/Dune.lock.json");
    let unrelated = PathBuf::from("/music/b/album.lock.json");

    graph.update_lock(album_lock.clone(), root_meta("/music/a", &[], &[]));
    graph.update_lock(film_lock.clone(), root_meta("/films/Dune.mkv", &[], &[]));
    graph.update_lock(unrelated.clone(), root_meta("/music/b", &[], &[]));

    let mut found = graph.locks_under(&PathBuf::from("/music"));
    found.sort();
    assert_eq!(found, vec![album_lock, unrelated]);
    assert_eq!(
        graph.locks_under(&PathBuf::from("/films/Dune.mkv")),
        vec![film_lock]
    );
}
