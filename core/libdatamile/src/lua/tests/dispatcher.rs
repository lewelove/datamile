//! Dispatcher contract for `tags` and `objects`: what the lock gets, what it
//! refuses to take, and what it silently drops.

use serde_json::{Value, json};

use crate::lua::LuaEngine;

/// The engine ships no id, so every case that expects a lock to be built has to
/// register one first.
const TEST_ID: &str = "datamile.compile.album.id(function() return 'test-id' end)";

fn dispatch_raw(
    setup: &str,
    manifests: Value,
    total_tracks: u32,
) -> Result<Value, String> {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine
        .lua
        .load(setup)
        .set_name("setup.lua")
        .exec()
        .expect("Failed to run setup chunk");

    let ctx = json!({
        "paths": { "album_root": "/tmp" },
        "total_tracks": total_tracks,
    });

    engine
        .execute_dispatcher(&ctx, &manifests)
        .map_err(|e| e.to_string())
}

fn dispatch_with(
    setup: &str,
    manifests: Value,
    total_tracks: u32,
) -> Result<Value, String> {
    dispatch_raw(&format!("{TEST_ID}\n{setup}"), manifests, total_tracks)
}

fn dispatch(setup: &str) -> Result<Value, String> {
    dispatch_with(
        setup,
        json!({ "metadata": { "album": {}, "tracks": [{}] } }),
        1,
    )
}

/// `value` is a Lua expression, spliced in as the hook's return value.
fn album_hook(kind: &str, value: &str) -> Result<Value, String> {
    dispatch(&format!(
        "datamile.compile.album.{kind}('probe', function() return {value} end)"
    ))
}

/// Same, but for the track hooks, which get the track index appended.
fn track_hook(
    owner: &str,
    kind: &str,
    value: &str,
    total_tracks: u32,
) -> Result<Value, String> {
    let manifests = json!({
        "metadata": {
            "album": {},
            "tracks": (0..total_tracks).map(|_| json!({})).collect::<Vec<_>>()
        }
    });
    let setup = format!(
        "datamile.compile.{owner}.{kind}('probe', function(_, _, i) return {value} end)"
    );

    dispatch_with(&setup, manifests, total_tracks)
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
    for value in ["nil", "''", "{}"] {
        let res = album_hook("object", value).expect("empty object must not fail");
        assert!(
            res.pointer("/objects/probe").is_none(),
            "value {value} leaked into objects"
        );
    }
}

#[test]
fn object_with_a_function_errors_with_the_object_name() {
    let err = album_hook("object", "{ cb = function() end }")
        .expect_err("a function is not JSON");

    assert!(
        err.contains("album object 'probe'") && err.contains("probe.cb is a function"),
        "unexpected error: {err}"
    );
}

#[test]
fn sparse_tag_lists_error() {
    // Holes and float/0-based keys serialize as JSON objects, not arrays.
    for value in [
        "{ [1] = 'a', [3] = 'b' }",
        "{ [0] = 'a' }",
        "{ [2] = 'a' }",
        "{ [1.5] = 'a' }",
    ] {
        let err = album_hook("tag", value).expect_err("a sparse table is not a list");

        assert!(
            err.contains("album tag 'probe'") && err.contains("dense 1-based list"),
            "value {value} gave unexpected error: {err}"
        );
    }
}

#[test]
fn non_scalar_non_table_tag_errors() {
    let err = album_hook("tag", "function() end").expect_err("a function is not a tag");

    assert!(
        err.contains("album tag 'probe'") && err.contains("got function"),
        "unexpected error: {err}"
    );
}

#[test]
fn track_tags_land_per_track() {
    let res = track_hook("track", "tag", "'track ' .. i", 2).expect("track tag is legal");

    assert_eq!(res.pointer("/tracks/0/tags/probe"), Some(&json!("track 1")));
    assert_eq!(res.pointer("/tracks/1/tags/probe"), Some(&json!("track 2")));
}

#[test]
fn track_objects_land_per_track() {
    let res =
        track_hook("track", "object", "{ idx = i }", 2).expect("track object is legal");

    assert_eq!(
        res.pointer("/tracks/1/objects/probe"),
        Some(&json!({ "idx": 2 }))
    );
}

#[test]
fn track_hooks_answer_to_their_aliases() {
    for owner in ["tracks", "track", "t"] {
        let res =
            track_hook(owner, "tag", "'x'", 1).expect("alias must register the hook");
        assert_eq!(
            res.pointer("/tracks/0/tags/probe"),
            Some(&json!("x")),
            "alias {owner} did not register"
        );
    }
}

#[test]
fn track_tag_error_names_the_index() {
    let err =
        track_hook("track", "tag", "{{ 1 }}", 2).expect_err("nested table is illegal");

    assert!(
        err.contains("track tag 'probe' at index 1"),
        "unexpected error: {err}"
    );
}

#[test]
fn true_shorthand_reads_the_manifest() {
    let manifests = json!({
        "metadata": {
            "album": { "genre": ["Ambient"] },
            "tracks": [{ "title": "One" }]
        }
    });
    let res = dispatch_with(
        "datamile.compile.album.tag('genre', true)\n\
         datamile.compile.track.tag('title', true)",
        manifests,
        1,
    )
    .expect("shorthand is legal");

    assert_eq!(res.pointer("/tags/genre"), Some(&json!(["Ambient"])));
    assert_eq!(res.pointer("/tracks/0/tags/title"), Some(&json!("One")));
}

#[test]
fn the_engine_ships_no_tags_images_or_objects() {
    let res = dispatch("").expect("a config with no hooks still dispatches");

    assert_eq!(res.pointer("/tags"), Some(&json!({})));
    assert_eq!(res.pointer("/tracks/0/tags"), Some(&json!({})));
    assert_eq!(res.pointer("/images"), Some(&json!({})));
}

#[test]
fn missing_id_is_an_error() {
    let err = dispatch_raw(
        "",
        json!({ "metadata": { "album": {}, "tracks": [{}] } }),
        1,
    )
    .expect_err("no id registered");

    assert!(
        err.contains("Album id is not registered"),
        "unexpected error: {err}"
    );
}
