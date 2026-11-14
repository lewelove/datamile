use std::fs;
use std::path::PathBuf;

use crate::lua::LuaEngine;

/// Verify that `evaluate_config` resolves a relative `media_directory`
/// against the config directory so Lua sees the same absolute path as Rust.
#[test]
fn test_config_resolves_media() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    let music = config_dir.join("music");
    fs::create_dir_all(&music).expect("music dir");

    let init = config_dir.join("init.lua");
    fs::write(
        &init,
        r#"datamile.config({ storage = { media_directory = "./music" } })"#,
    )
    .expect("init.lua");

    let engine = LuaEngine::new().expect("engine");
    let evaluated = engine.evaluate_config(&init).expect("evaluate");

    assert_eq!(
        PathBuf::from(&evaluated.app.storage.media_directory),
        music.canonicalize().expect("canonicalize")
    );
}

/// Verify that an empty `media_directory` stays empty instead of resolving to
/// the config directory.
#[test]
fn test_evaluate_config_keeps_empty_media_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");

    let init = config_dir.join("init.lua");
    fs::write(&init, "datamile.config({})").expect("init.lua");

    let engine = LuaEngine::new().expect("engine");
    let evaluated = engine.evaluate_config(&init).expect("evaluate");

    assert!(evaluated.app.storage.media_directory.is_empty());
}

/// Verify both starter configs load without relying on binary-baked defaults.
#[test]
fn test_starter_examples_load() {
    for name in ["music-basic", "music-manifestless"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
            .join("init.lua");

        let engine = LuaEngine::new().expect("engine");
        engine.evaluate_config(&path).expect("evaluate starter");

        let roots = engine.media_roots().expect("roots");
        assert_eq!(roots, vec!["album".to_string()], "example {name}");
    }
}
