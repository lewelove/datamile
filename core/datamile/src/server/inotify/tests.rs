use std::collections::HashMap;

use serde_json::json;

use super::handler::filter_reingested_ids;

/// Verify that an album ID present in both removal and update sets is purged from removals.
///
/// Moving or renaming a directory generates an inotify removal for the old path and an
/// addition for the new path in the same batch. Since metadata IDs are identical, the ID
/// must not be broadcast as removed.
#[test]
fn test_filter_purges_reingested_id() {
    let album_id = "Artist - 2024 - Album".to_string();

    // Stale removal and fresh re-ingestion for the same album ID.
    let mut removed_ids = vec![album_id.clone()];
    let updated_entries = HashMap::from([(album_id.clone(), json!({ "id": album_id }))]);

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    // The re-ingested ID must be filtered out of the removal list.
    assert!(removed_ids.is_empty());
}

/// Verify that genuinely removed album IDs absent from the update set are retained.
#[test]
fn test_filter_retains_unmatched_id() {
    let removed_id = "Artist - 2020 - Deleted".to_string();
    let reingested_id = "Artist - 2024 - Renamed".to_string();

    let mut removed_ids = vec![removed_id.clone(), reingested_id.clone()];
    let updated_entries =
        HashMap::from([(reingested_id.clone(), json!({ "id": reingested_id }))]);

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    // Only the genuinely removed ID must remain.
    assert_eq!(removed_ids, vec![removed_id]);
}

/// Verify that empty input sets return early without modification.
#[test]
fn test_filter_noop_on_empty_inputs() {
    let mut removed_ids = Vec::new();
    let updated_entries: HashMap<String, serde_json::Value> = HashMap::new();

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    assert!(removed_ids.is_empty());
}

use std::fs;

use libdatamile::lua::LuaEngine;

/// A marker dropped into a pre-existing untracked directory is found by a
/// shallow discovery pass and compiles into a lock.
#[test]
fn shallow_discovery_compiles_new_marker() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let music = tmp.path().join("music");
    let album = music.join("New Album");
    fs::create_dir_all(&album).expect("album dir");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("album")
        d.compile.album.lock(function(source)
            return d.fs.joinpath(source.path, "album.lock.json")
        end)
        d.compile.album.select(function(scope)
            return d.fs.roots(scope, "album.toml")
        end)
        d.compile.album.id(function(ctx) return "new-album" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let before = crate::compile::media::discover_targets_shallow(&engine, &album)
        .expect("shallow before");
    assert!(before.is_empty());

    fs::write(album.join("album.toml"), b"[album]").expect("marker");

    let roots = crate::compile::media::discover_targets_shallow(&engine, &album)
        .expect("shallow after");
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].target, album);

    let build = crate::compile::media::compile_root(&engine, &roots[0], tmp.path())
        .expect("compile");
    assert_eq!(build.id, "new-album");
    assert!(build.lock_path.ends_with("album.lock.json"));
}
