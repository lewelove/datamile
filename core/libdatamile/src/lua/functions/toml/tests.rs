use mlua::Table;

use crate::lua::LuaEngine;

/// Verify that `d.toml.decode` and `d.toml.encode` convert data between Lua and TOML.
#[test]
fn test_d_toml_encode_decode() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let toml_input = r#"[album]
title = "Demo""#;

    let code = format!("return d.toml.decode([==[{toml_input}]==])");
    let decoded: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let album: Table = decoded.get("album").expect("Missing album section");
    let title: String = album.get("title").expect("Missing title");
    assert_eq!(title, "Demo");

    let encoded: String = engine
        .lua
        .load("return d.toml.encode({ album = { title = 'Demo' } })")
        .eval()
        .expect("Execution failed");
    assert!(encoded.contains("[album]"));
    assert!(encoded.contains("title = \"Demo\""));
}

/// Verify that `d.toml.decode` returns an error when the TOML format is invalid.
#[test]
fn test_d_toml_decode_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let result = engine
        .lua
        .load("return d.toml.decode('[album')")
        .eval::<Table>();
    assert!(result.is_err());
}

/// Verify that `d.toml.encode` returns an error for values that cannot be serialized.
#[test]
fn test_d_toml_encode_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let result = engine
        .lua
        .load("return d.toml.encode({ func = function() end })")
        .eval::<String>();
    assert!(result.is_err());
}
