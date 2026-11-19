use mlua::Table;

use crate::lua::LuaEngine;

/// Verify that `d.json.decode` and `d.json.encode` convert data between Lua and JSON.
#[test]
fn test_d_json_encode_decode() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let json_input = r#"{"title":"Album","year":2024}"#;

    let code = format!("return d.json.decode([==[{json_input}]==])");
    let decoded: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let title: String = decoded.get("title").expect("Missing title");
    let year: u32 = decoded.get("year").expect("Missing year");
    assert_eq!(title, "Album");
    assert_eq!(year, 2024);

    let encoded: String = engine
        .lua
        .load("return d.json.encode({ title = 'Album', year = 2024 })")
        .eval()
        .expect("Execution failed");
    assert!(encoded.contains("\"title\":\"Album\""));
    assert!(encoded.contains("\"year\":2024"));
}

/// Verify that `d.json.decode` returns an error when the JSON format is invalid.
#[test]
fn test_d_json_decode_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let result = engine
        .lua
        .load("return d.json.decode('{bad_json:')")
        .eval::<Table>();
    assert!(result.is_err());
}

/// Verify that `d.json.encode` returns an error for values that cannot be serialized.
#[test]
fn test_d_json_encode_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let result = engine
        .lua
        .load("return d.json.encode({ func = function() end })")
        .eval::<String>();
    assert!(result.is_err());
}
