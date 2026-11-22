//! Dispatcher contract for `tags` and `objects`: what the lock gets, what it
//! refuses to take, and what it silently drops.

use serde_json::{Value, json};

use crate::lua::LuaEngine;

fn dispatch(setup: &str) -> Result<Value, String> {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine
        .lua
        .load(setup)
        .set_name("setup.lua")
        .exec()
        .expect("Failed to run setup chunk");

    let ctx = json!({ "paths": { "album_root": "/tmp" }, "total_tracks": 1 });
    let manifests = json!({ "metadata": { "album": {}, "tracks": [{}] } });

    engine
        .execute_dispatcher(&ctx, &manifests)
        .map_err(|e| e.to_string())
}

/// `value` is a Lua expression, spliced in as the hook's return value.
fn album_hook(kind: &str, value: &str) -> Result<Value, String> {
    dispatch(&format!(
        "datamile.compile.album.{kind}('probe', function() return {value} end)"
    ))
}

#[test]
fn scalar_tag_lands_in_tags() {
    let res = album_hook("tag", "'Ambient'").expect("scalar tag must be accepted");

    assert_eq!(res.pointer("/tags/probe"), Some(&json!("Ambient")));
}

#[test]
fn scalar_list_tag_keeps_its_order() {
    let res = album_hook("tag", "{ 'second', 1, true }").expect("scalar list is legal");

    assert_eq!(
        res.pointer("/tags/probe"),
        Some(&json!(["second", 1, true]))
    );
}

#[test]
fn false_is_a_legal_tag_value() {
    let res = album_hook("tag", "false").expect("boolean tag is legal");

    assert_eq!(res.pointer("/tags/probe"), Some(&json!(false)));
}

#[test]
fn nested_tag_errors_with_the_tag_name() {
    let err = album_hook("tag", "{{ 1 }, { 2 }}").expect_err("nested table is illegal");

    assert!(
        err.contains("album tag 'probe'") && err.contains("scalar or a list of scalars"),
        "unexpected error: {err}"
    );
}

#[test]
fn table_with_string_keys_errors() {
    let err = album_hook("tag", "{ a = 1 }").expect_err("a map is not a tag list");

    assert!(err.contains("album tag 'probe'"), "unexpected error: {err}");
}

#[test]
fn empty_tags_are_omitted() {
    for value in ["nil", "''", "{}"] {
        let res = album_hook("tag", value).expect("empty tag must not fail");
        assert!(
            res.pointer("/tags/probe").is_none(),
            "value {value} leaked into tags"
        );
    }
}

#[test]
fn object_lands_verbatim() {
    let res = album_hook("object", "{ nested = { list = { 1, 2 } } }")
        .expect("object must be accepted");

    assert_eq!(
        res.pointer("/objects/probe"),
        Some(&json!({ "nested": { "list": [1, 2] } }))
    );
}

#[test]
fn empty_objects_are_omitted() {
    let res = album_hook("object", "nil").expect("empty object must not fail");

    assert!(res.pointer("/objects/probe").is_none());
}
