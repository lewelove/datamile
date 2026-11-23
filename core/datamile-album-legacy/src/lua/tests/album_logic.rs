//! `evaluate_album_logic` contract: the album lock is evaluated as its
//! `album` table with `nodes.track` also exposed as `tracks`, so album filters
//! can still read `album.tracks`.

use serde_json::json;

use crate::lua::LuaEngine;

#[test]
fn album_logic_unwraps_album_and_grafts_tracks() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine
        .runtime()
        .lua
        .load(
            "datamile.filter('has_tracks', {
                match = function(e) return e.tracks[1].title == 'One' end
            })",
        )
        .set_name("setup.lua")
        .exec()
        .expect("Failed to run setup chunk");

    let res = engine
        .evaluate_album_logic(&json!({
            "album": {
                "id": "x",
                "nodes": { "track": [{ "title": "One" }] }
            }
        }))
        .expect("album logic must evaluate");

    assert_eq!(res["filters"]["has_tracks"], json!(true));
}

#[test]
fn album_logic_accepts_a_plain_entity() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine
        .runtime()
        .lua
        .load("datamile.filter('always', { match = function(e) return e.id == 'x' end })")
        .set_name("setup.lua")
        .exec()
        .expect("Failed to run setup chunk");

    let res = engine
        .evaluate_album_logic(&json!({ "id": "x" }))
        .expect("plain entity must evaluate");

    assert_eq!(res["filters"]["always"], json!(true));
}
