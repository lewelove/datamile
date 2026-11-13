use crate::lua::LuaEngine;
use std::fs;
use std::path::PathBuf;

/// Verify that `evaluate_config` resolves a relative `music_directory`
/// against the config directory so Lua sees the same absolute path as Rust.
#[test]
fn test_evaluate_config_resolves_music_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    let music = config_dir.join("music");
    fs::create_dir_all(&music).expect("music dir");

    let init = config_dir.join("init.lua");
    fs::write(
        &init,
        r#"datamile.config({ storage = { music_directory = "./music" } })"#,
    )
    .expect("init.lua");

    let engine = LuaEngine::new().expect("engine");
    let evaluated = engine.evaluate_config(&init).expect("evaluate");

    assert_eq!(
        PathBuf::from(&evaluated.app.storage.music_directory),
        music.canonicalize().expect("canonicalize")
    );
}

/// Verify that an empty `music_directory` stays empty instead of resolving to
/// the config directory.
#[test]
fn test_evaluate_config_keeps_empty_music_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    fs::create_dir_all(&config_dir).expect("config dir");

    let init = config_dir.join("init.lua");
    fs::write(&init, "datamile.config({})").expect("init.lua");

    let engine = LuaEngine::new().expect("engine");
    let evaluated = engine.evaluate_config(&init).expect("evaluate");

    assert!(evaluated.app.storage.music_directory.is_empty());
}
