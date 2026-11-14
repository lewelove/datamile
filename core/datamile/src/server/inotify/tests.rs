use super::handler::filter_reingested_ids;

fn target(root_type: &str, id: &str) -> (String, String) {
    (root_type.to_string(), id.to_string())
}

/// Verify that an id present in both removal and update sets is purged from
/// removals. Renaming generates a stale removal and a fresh re-ingestion.
#[test]
fn test_filter_purges_reingested_id() {
    let album = target("album", "Artist - 2024 - Album");
    let mut removed = vec![album.clone()];

    filter_reingested_ids(&mut removed, std::slice::from_ref(&album));

    assert!(removed.is_empty());
}

/// Verify that genuinely removed targets absent from the update set remain.
#[test]
fn test_filter_retains_unmatched_id() {
    let removed_target = target("album", "Artist - 2020 - Deleted");
    let reingested = target("album", "Artist - 2024 - Renamed");
    let mut removed = vec![removed_target.clone(), reingested.clone()];

    filter_reingested_ids(&mut removed, std::slice::from_ref(&reingested));

    assert_eq!(removed, vec![removed_target]);
}

/// Ids colliding across root types must not cancel each other's removal.
#[test]
fn test_filter_scopes_targets_by_root_type() {
    let album = target("album", "Dune");
    let film = target("film", "Dune");
    let mut removed = vec![album.clone(), film];

    filter_reingested_ids(&mut removed, std::slice::from_ref(&album));

    assert_eq!(removed, vec![target("film", "Dune")]);
}

/// Verify that empty input sets return early without modification.
#[test]
fn test_filter_noop_on_empty_inputs() {
    let mut removed = Vec::new();

    filter_reingested_ids(&mut removed, &[]);

    assert!(removed.is_empty());
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
    let code = format!(
        r#"
        d.media("album", {{
            kind = "root",
            dir = "{music}",
            select = function(dir)
                return d.fs.roots(dir, "album.toml")
            end,
        }})
        d.compile.album.id(function() return "new-album" end)
    "#,
        music = music.to_string_lossy()
    );
    engine.lua.load(&code).exec().expect("register");

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
