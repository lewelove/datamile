use serde_json::json;

use crate::lua::LuaEngine;

/// Entities are extracted from their own root type, and typed definitions only
/// evaluate for matching roots. Two roots with the same id never cross-talk.
#[test]
fn entity_logic_filters_by_root_type() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.filter("any", { label = "Any", match = function(e) return e.id ~= nil end })
        d.filter("films", { label = "Films", type = "film", match = function(e) return true end })
        d.library("films_lib", { label = "Films", type = "film", match = function(e) return true end })
    "#;
    engine.lua.load(code).exec().expect("register");

    let lock = json!({
        "album": { "id": "Dune" },
        "film": { "id": "Dune" }
    });

    let album = engine.evaluate_entity_logic("album", &lock).expect("album");
    assert_eq!(album["filters"]["any"], json!(true));
    assert!(album["filters"].get("films").is_none());
    assert!(album["libraries"].get("films_lib").is_none());

    let film = engine.evaluate_entity_logic("film", &lock).expect("film");
    assert_eq!(film["filters"]["films"], json!(true));
    assert_eq!(film["libraries"]["films_lib"], json!(true));
}

/// A lock without the requested root type evaluates to empty flags instead of
/// indexing nil.
#[test]
fn entity_logic_missing_root_type_is_empty() {
    let engine = LuaEngine::new().expect("engine");
    engine
        .lua
        .load(
            r#"d.filter("any", { label = "Any", match = function(e) return true end })"#,
        )
        .exec()
        .expect("register");

    let res = engine
        .evaluate_entity_logic("album", &json!({ "film": { "id": "Dune" } }))
        .expect("eval");

    assert!(res["filters"].as_object().expect("filters").is_empty());
}
