use crate::lua::LuaEngine;
use mlua::{LuaSerdeExt, Table};
use serde_json::json;

/// Verify that registered media roots are only types with a lock function.
#[test]
fn test_media_roots_require_lock() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("root")
        d.compile.root.lock(function(source) return source.path .. "/root.lock.json" end)
        d.media.type("child")
        return __DATAMILE_MEDIA_ROOTS()
    "#;
    let roots: Table = engine.lua.load(code).eval().expect("roots");
    let names: Vec<String> = roots
        .sequence_values::<String>()
        .filter_map(Result::ok)
        .collect();
    assert!(names.contains(&"root".to_string()));
    assert!(!names.contains(&"child".to_string()));
}

/// Verify that media compilation calls source-scoped tag, object, location, image, and child functions.
#[test]
fn test_media_compile_shape() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("child")
        d.compile.child.select(function(source)
            return {
                { path = source.path .. "/a" },
                { path = source.path .. "/b" },
            }
        end)
        d.compile.child.tag("kind", function(source) return source.path end)

        d.media.type("root")
        d.compile.root.select(function(source) return { { path = source.path } } end)
        d.compile.root.id(function(_, tags) return "root-" .. tags.genre end)
        d.compile.root.tag("genre", function(source) return "Ambient" end)
        d.compile.root.tag("styles", function(source) return { "A", "B" } end)
        d.compile.root.object("info", function(source) return { total = 2 } end)
        d.compile.root.location(function(source)
            return { kind = "directory", path = source.path }
        end)
        d.compile.root.media("children", "child")
    "#;
    engine.lua.load(code).exec().expect("register");
    let value = engine
        .media_compile("root", &json!({ "path": "/tmp/root" }))
        .expect("compile");
    assert_eq!(value["id"], "root-Ambient");
    assert_eq!(value["tags"]["genre"], "Ambient");
    let mut styles: Vec<&str> = value["tags"]["styles"]
        .as_array()
        .expect("styles array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    styles.sort_unstable();
    assert_eq!(styles, vec!["A", "B"]);
    assert_eq!(value["objects"]["info"]["total"], 2);
    assert_eq!(value["location"]["kind"], "directory");
    assert_eq!(value["media"]["children"]["type"], "child");
    assert_eq!(
        value["media"]["children"]["sources"][0]["path"],
        "/tmp/root/a"
    );
}

/// Verify that child selectors receive the parent source and may emit data-backed sources.
#[test]
fn test_media_source_data() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("child")
        d.compile.child.select(function(source)
            return { { data = { title = "Virtual" } } }
        end)
        d.compile.child.tag("title", function(source) return source.data.title end)
        d.compile.child.id(function(_, tags) return tags.title end)

        d.media.type("root")
        d.compile.root.select(function(source) return { { path = source.path } } end)
        d.compile.root.media("items", "child")

        return __DATAMILE_MEDIA_COMPILE("root", { path = "/parent" })
    "#;
    let lua_value: mlua::Value = engine.lua.load(code).eval().expect("nested");
    let value: serde_json::Value = engine.lua.from_value(lua_value).expect("json");
    assert_eq!(
        value["media"]["items"]["sources"][0]["data"]["title"],
        "Virtual"
    );
}

/// Verify that track ids use discnumber.tracknumber only when discnumber > 1.
#[test]
fn test_track_id_uses_discnumber() {
    let engine = LuaEngine::new().expect("engine");
    let base = json!({
        "data": {
            "albumartist": "Artist",
            "album": "Album",
            "title": "Title",
            "tracknumber": "3",
            "discnumber": 1,
        }
    });
    let single = engine.media_compile("track", &base).expect("single");
    assert_eq!(single["id"], "Artist - Album - 3 - Title");
    assert!(single.get("location").is_none());

    let mut multi = base;
    multi["data"]["discnumber"] = json!(2);
    let multi = engine.media_compile("track", &multi).expect("multi");
    assert_eq!(multi["id"], "Artist - Album - 2.3 - Title");
}

/// Verify that tag validation rejects non-scalar tag values.
#[test]
fn test_media_tag_rejects_object() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("bad")
        d.compile.bad.tag("nested", function(source) return { { key = "value" } } end)
    "#;
    engine.lua.load(code).exec().expect("register");
    let result = engine.media_compile("bad", &json!({ "path": "/tmp/bad" }));
    assert!(result.is_err());
}
