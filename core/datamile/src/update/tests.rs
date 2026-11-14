use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use libdatamile::lua::LuaEngine;
use serde_json::json;

use crate::compile::LogVerbosity;
use crate::compile::media::RootTarget;
use crate::server::state::{DependencyGraph, RootMeta, UpdateScope};
use crate::update::cache::LibraryCache;
use crate::update::client::ForceMode;
use crate::update::queue::{WorkQueueContext, rebuild_cache, resolve_work_queue};
use crate::update::verify::{CacheIndex, verify_targets};

/// Directory-backed roots, each in its own directory and with its own lock.
struct Fixture {
    album_dir: PathBuf,
    dune_dir: PathBuf,
    arrival_dir: PathBuf,
    graph: DependencyGraph,
    targets: Vec<RootTarget>,
    baseline: LibraryCache,
}

fn target(root_type: &str, target: &Path, lock: &Path) -> RootTarget {
    RootTarget {
        root_type: root_type.to_string(),
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
    let dune_dir = root.join("Dune");
    let arrival_dir = root.join("Arrival");
    let lyrics_dir = album_dir.join("Lyrics");
    for dir in [&album_dir, &dune_dir, &arrival_dir, &lyrics_dir] {
        fs::create_dir_all(dir).expect("dir");
    }

    let album_toml = album_dir.join("album.toml");
    let track = album_dir.join("01.flac");
    let lyrics = lyrics_dir.join("01.lrc");
    let dune = dune_dir.join("Dune.mkv");
    let arrival = arrival_dir.join("Arrival.mkv");
    for file in [&album_toml, &track, &lyrics, &dune, &arrival] {
        fs::write(file, b"data").expect("file");
    }

    let dune_lock = dune_dir.join("Dune.lock.json");
    let arrival_lock = arrival_dir.join("Arrival.lock.json");
    let album_lock = album_dir.join("album.lock.json");
    for lock in [&dune_lock, &arrival_lock, &album_lock] {
        fs::write(lock, b"{}").expect("lock");
    }

    let mut graph = DependencyGraph::default();
    graph.update_lock(
        dune_lock.clone(),
        meta("film", &dune_dir, &[&dune], &[&dune_dir]),
    );
    graph.update_lock(
        arrival_lock.clone(),
        meta("film", &arrival_dir, &[&arrival], &[&arrival_dir]),
    );
    graph.update_lock(
        album_lock.clone(),
        meta("album", &album_dir, &[&album_toml, &track], &[&album_dir]),
    );

    let targets = vec![
        target("film", &dune_dir, &dune_lock),
        target("film", &arrival_dir, &arrival_lock),
        target("album", &album_dir, &album_lock),
    ];

    let mut baseline = LibraryCache::new();
    rebuild_cache(&mut baseline, &graph);

    (
        tmp,
        Fixture {
            album_dir,
            dune_dir,
            arrival_dir,
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

/// Path lookups map a changed file to its own root only.
#[test]
fn sibling_roots_map_files_independently() {
    let (_tmp, fixture) = fixture();
    let dune = fixture.dune_dir.join("Dune.mkv");
    let arrival = fixture.arrival_dir.join("Arrival.mkv");

    assert_eq!(
        fixture.graph.locks_for_file(&dune),
        Some(&HashSet::from([fixture.dune_dir.join("Dune.lock.json")]))
    );
    assert_eq!(
        fixture.graph.locks_for_file(&arrival),
        Some(&HashSet::from([fixture
            .arrival_dir
            .join("Arrival.lock.json")]))
    );
}

/// A file change dirties only the root that owns its directory.
#[test]
fn changed_file_dirties_own_root() {
    let (_tmp, fixture) = fixture();
    fs::write(fixture.arrival_dir.join("Arrival.mkv"), b"extended").expect("write");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.arrival_dir.join("Arrival.lock.json")])
    );
}

/// A file added to a recorded directory dirties its root even though the file
/// was never inspected before.
#[test]
fn added_file_dirties_directory_roots() {
    let (_tmp, fixture) = fixture();
    fs::write(fixture.dune_dir.join("cover.jpg"), b"jpeg").expect("write");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.dune_dir.join("Dune.lock.json")])
    );
}

/// A subdirectory added inside an existing root dirties that root.
#[test]
fn added_subdirectory_dirties_root() {
    let (_tmp, fixture) = fixture();
    let info = fixture.album_dir.join("Info");
    fs::create_dir_all(&info).expect("dir");
    fs::write(info.join("notes.toml"), b"[info]").expect("file");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.album_dir.join("album.lock.json")])
    );
}

/// A subdirectory removed from an existing root dirties that root.
#[test]
fn removed_subdirectory_dirties_root() {
    let (_tmp, fixture) = fixture();
    fs::remove_dir_all(fixture.album_dir.join("Lyrics")).expect("remove");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.album_dir.join("album.lock.json")])
    );
}

/// Removing an inspected file dirties its root.
#[test]
fn removed_file_dirties_root() {
    let (_tmp, fixture) = fixture();
    fs::remove_file(fixture.album_dir.join("01.flac")).expect("remove");

    let dirty = dirty(&fixture);
    assert_eq!(
        dirty,
        HashSet::from([fixture.album_dir.join("album.lock.json")])
    );
}

/// A root whose lock vanished is always dirty so it recompiles.
#[test]
fn missing_lock_is_dirty() {
    let (_tmp, fixture) = fixture();
    fs::remove_file(fixture.dune_dir.join("Dune.lock.json")).expect("remove");

    let dirty = dirty(&fixture);
    assert!(dirty.contains(&fixture.dune_dir.join("Dune.lock.json")));
}

/// Overlapping scope paths (a directory and an album inside it) must not
/// queue the same root twice.
#[test]
fn overlapping_scope_paths_dedupe_roots() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let album = root.join("album");
    fs::create_dir_all(&album).expect("album dir");
    fs::write(album.join("album.toml"), b"[album]").expect("marker");
    fs::write(album.join("album.lock.json"), b"{}").expect("lock");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("album")
        d.compile.album.lock(function(t) return t.path .. "/album.lock.json" end)
        d.compile.album.select(function(scope)
            local marker = d.fs.joinpath(scope.path, "album.toml")
            if d.fs.exists(marker) then
                return { { path = scope.path } }
            end
            if not scope.recursive then
                return {}
            end
            return d.fs.roots(scope, "album.toml")
        end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let scope = UpdateScope::Paths(HashSet::from([root.clone(), album.clone()]));
    let cache = LibraryCache::new();
    let graph = DependencyGraph::default();
    let ctx = WorkQueueContext {
        engine: &engine,
        scope: &scope,
        media_directory: &root,
        cache: &cache,
        deps_graph: &graph,
        force: ForceMode::Force,
        effective_jobs: Some(1),
        tracked: None,
        verbosity: LogVerbosity::Silent,
        log_txs: &[],
    };

    let queue = resolve_work_queue(&ctx).expect("queue");
    assert_eq!(queue.roots.len(), 1);
    assert_eq!(queue.roots[0].target, album);
}

/// Progress messages reach every log subscriber, not just the server log.
#[test]
fn verifying_message_reaches_log_txs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canonicalize");
    let album = root.join("album");
    fs::create_dir_all(&album).expect("album dir");
    fs::write(album.join("album.toml"), b"[album]").expect("marker");
    fs::write(album.join("album.lock.json"), b"{}").expect("lock");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("album")
        d.compile.album.lock(function(t) return t.path .. "/album.lock.json" end)
        d.compile.album.select(function(scope)
            local marker = d.fs.joinpath(scope.path, "album.toml")
            if d.fs.exists(marker) then
                return { { path = scope.path } }
            end
            return {}
        end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let scope = UpdateScope::Paths(HashSet::from([album]));
    let cache = LibraryCache::new();
    let graph = DependencyGraph::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let ctx = WorkQueueContext {
        engine: &engine,
        scope: &scope,
        media_directory: &root,
        cache: &cache,
        deps_graph: &graph,
        force: ForceMode::Force,
        effective_jobs: Some(1),
        tracked: None,
        verbosity: LogVerbosity::Verbose,
        log_txs: &[tx],
    };

    resolve_work_queue(&ctx).expect("queue");

    assert_eq!(rx.try_recv().expect("message"), "Verifying 1 entities...");
}
