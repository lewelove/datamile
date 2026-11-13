use crate::compile::media::RootTarget;
use crate::server::state::{DependencyGraph, RootMeta};
use crate::update::cache::LibraryCache;
use crate::update::client::ForceMode;
use crate::update::queue::rebuild_cache;
use crate::update::verify::{CacheIndex, verify_targets};
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Two file-backed roots sharing a directory plus a directory-backed album:
/// each keeps its own lock and dirty state.
struct Fixture {
    root: PathBuf,
    graph: DependencyGraph,
    targets: Vec<RootTarget>,
    baseline: LibraryCache,
}

fn target(target: &Path, lock: &Path) -> RootTarget {
    RootTarget {
        root_type: "film".to_string(),
        target: target.to_path_buf(),
        lock_path: lock.to_path_buf(),
        source: json!({ "path": target.to_string_lossy() }),
    }
}

fn meta(root_type: &str, target: &Path, files: &[&Path], dirs: &[&Path]) -> RootMeta {
    RootMeta {
        root_type: root_type.to_string(),
        target: target.to_path_buf(),
        files: files.iter().map(|p| (*p).to_path_buf()).collect(),
        dirs: dirs.iter().map(|p| (*p).to_path_buf()).collect(),
    }
}

fn fixture() -> (tempfile::TempDir, Fixture) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let album_dir = root.join("album");
    fs::create_dir_all(&album_dir).expect("album dir");

    let dune = root.join("Dune.mkv");
    let arrival = root.join("Arrival.mkv");
    let album_toml = album_dir.join("album.toml");
    let track = album_dir.join("01.flac");
    fs::write(&dune, b"dune").expect("dune");
    fs::write(&arrival, b"arrival").expect("arrival");
    fs::write(&album_toml, b"[album]").expect("marker");
    fs::write(&track, b"track").expect("track");

    let dune_lock = root.join("Dune.lock.json");
    let arrival_lock = root.join("Arrival.lock.json");
    let album_lock = album_dir.join("album.lock.json");
    for lock in [&dune_lock, &arrival_lock, &album_lock] {
        fs::write(lock, b"{}").expect("lock");
    }

    let mut graph = DependencyGraph::default();
    graph.update_lock(dune_lock.clone(), meta("film", &dune, &[&dune], &[&root]));
    graph.update_lock(
        arrival_lock.clone(),
        meta("film", &arrival, &[&arrival], &[&root]),
    );
    graph.update_lock(
        album_lock.clone(),
        meta("album", &album_dir, &[&album_toml, &track], &[&album_dir]),
    );

    let targets = vec![
        target(&dune, &dune_lock),
        target(&arrival, &arrival_lock),
        RootTarget {
            root_type: "album".to_string(),
            target: album_dir,
            lock_path: album_lock,
            source: json!({}),
        },
    ];

    let mut baseline = LibraryCache::new();
    rebuild_cache(&mut baseline, &graph);

    (
        tmp,
        Fixture {
            root,
            graph,
            targets,
            baseline,
        },
    )
}

fn dirty(fixture: &Fixture) -> HashSet<PathBuf> {
    let index = CacheIndex::new(&fixture.baseline);

    verify_targets(
        fixture.targets.clone(),
        &index,
        &fixture.graph,
        ForceMode::Preserve,
        Some(1),
    )
    .expect("verify")
    .into_iter()
    .map(|root| root.lock_path)
    .collect()
}

/// A freshly rebuilt cache marks every root clean.
#[test]
fn clean_after_rebuild() {
    let (_tmp, fixture) = fixture();
    assert!(dirty(&fixture).is_empty());
}

/// Path lookups map a changed file to its own root only, even when sibling
/// roots share a directory.
#[test]
fn sibling_roots_map_files_independently() {
    let (_tmp, fixture) = fixture();
    let arrival = fixture.root.join("Arrival.mkv");

    assert_eq!(
        fixture.graph.locks_for_file(&arrival),
        Some(&HashSet::from([fixture.root.join("Arrival.lock.json")]))
    );
}

/// Verification is directory-granular: a change inside a watched directory is
/// conservatively dirty for every root watching it.
#[test]
fn shared_directory_change_dirties_all_watchers() {
    let (_tmp, fixture) = fixture();
    fs::write(fixture.root.join("Arrival.mkv"), b"arrival-extended").expect("write");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([
            fixture.root.join("Dune.lock.json"),
            fixture.root.join("Arrival.lock.json"),
        ])
    );
}

/// A file added to a recorded directory dirties its roots even though the file
/// was never inspected before.
#[test]
fn added_file_dirties_directory_roots() {
    let (_tmp, fixture) = fixture();
    fs::write(fixture.root.join("cover.jpg"), b"jpeg").expect("write");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([
            fixture.root.join("Dune.lock.json"),
            fixture.root.join("Arrival.lock.json"),
        ])
    );
}

/// Removing an inspected file dirties its root.
#[test]
fn removed_file_dirties_root() {
    let (_tmp, fixture) = fixture();
    fs::remove_file(fixture.root.join("album/01.flac")).expect("remove");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.root.join("album/album.lock.json")])
    );
}

/// A root whose lock vanished is always dirty so it recompiles.
#[test]
fn missing_lock_is_dirty() {
    let (_tmp, fixture) = fixture();
    fs::remove_file(fixture.root.join("Dune.lock.json")).expect("remove");

    let dirty = dirty(&fixture);
    assert!(dirty.contains(&fixture.root.join("Dune.lock.json")));
}
