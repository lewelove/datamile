use crate::lua::LuaEngine;
use mlua::Table;
use serde_json::{Value, json};

fn compile_leaf(engine: &LuaEngine, type_name: &str, seed: &Value) -> Value {
    let prepared = engine.media_prepare(type_name, seed).expect("prepare");
    assert!(prepared.children.is_empty(), "leaf types have no children");
    engine
        .media_finalize(type_name, prepared.ctx_id, &json!({}))
        .expect("finalize")
}

/// Verify that a fresh engine has no baked-in media types.
#[test]
fn test_engine_has_no_builtin_media_roots() {
    let engine = LuaEngine::new().expect("engine");
    let roots: Table = engine
        .lua
        .load("return __DATAMILE_MEDIA_ROOTS()")
        .eval()
        .expect("roots");
    assert_eq!(roots.raw_len(), 0);
}

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

/// Verify the four output stages populate tags, objects, images and location.
#[test]
fn test_media_output_boxes() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("thing")
        d.compile.thing.tag("genre", function(ctx) return "Ambient" end)
        d.compile.thing.tag("styles", function(ctx) return { "A", "B" } end)
        d.compile.thing.object("info", function(ctx) return { total = 2 } end)
        d.compile.thing.image("main", function(ctx) return { sources = "cover.jpg" } end)
        d.compile.thing.location(function(ctx)
            return { kind = "directory", path = ctx.path }
        end)
        d.compile.thing.id(function(ctx) return "thing-" .. ctx.tags.genre end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let value = compile_leaf(&engine, "thing", &json!({ "path": "/tmp/thing" }));
    assert_eq!(value["id"], "thing-Ambient");
    assert_eq!(value["tags"]["genre"], "Ambient");
    let mut styles: Vec<&str> = value["tags"]["styles"]
        .as_array()
        .expect("styles array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    styles.sort_unstable();
    assert_eq!(styles, vec!["A", "B"]);
    assert_eq!(value["objects"]["info"]["total"], 2);
    assert_eq!(value["images"]["main"]["sources"], "cover.jpg");
    assert_eq!(value["location"]["kind"], "directory");
}

/// Verify every input, tag, image, object and id hook runs exactly once.
#[test]
fn test_media_hooks_run_once() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        counts = {}
        order = {}
        local function bump(name)
            counts[name] = (counts[name] or 0) + 1
            table.insert(order, name)
        end

        d.media.type("thing")
        d.compile.thing.input("first", function(ctx) bump("input:first") return 1 end)
        d.compile.thing.input("second", function(ctx) bump("input:second") return ctx.first + 1 end)
        d.compile.thing.tag("alpha", function(ctx) bump("tag:alpha") return "a" end)
        d.compile.thing.tag("beta", function(ctx) bump("tag:beta") return "b" end)
        d.compile.thing.image("main", function(ctx) bump("image") return { sources = "x.jpg" } end)
        d.compile.thing.object("info", function(ctx) bump("object") return {} end)
        d.compile.thing.id(function(ctx) bump("id") return "id" end)
        d.compile.thing.location(function(ctx) bump("location") return nil end)
    "#;
    engine.lua.load(code).exec().expect("register");

    compile_leaf(&engine, "thing", &json!({ "path": "/tmp/thing" }));

    let order: Table = engine.lua.globals().get("order").expect("order");
    let order: Vec<String> = order
        .sequence_values::<String>()
        .filter_map(Result::ok)
        .collect();

    let inputs: Vec<&str> = order
        .iter()
        .filter(|name| name.starts_with("input:"))
        .map(String::as_str)
        .collect();
    assert_eq!(inputs, vec!["input:first", "input:second"]);

    let mut sorted = order.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        order.len(),
        "a hook ran more than once: {order:?}"
    );
}

/// Verify children compile before the parent finalizes and inject their output.
#[test]
fn test_children_compile_bottom_up() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("leaf")
        d.compile.leaf.tag("name", function(ctx) return ctx.path .. "-leaf" end)
        d.compile.leaf.id(function(ctx) return ctx.tags.name end)

        d.media.type("root")
        d.compile.root.child("items", "leaf", function(ctx)
            return {
                { path = ctx.path .. "/a" },
                { path = ctx.path .. "/b" },
            }
        end)
        d.compile.root.object("info", function(ctx)
            local first = ctx.items[1]
            return { first_id = first.id, first_tag = first.tags.name, count = #ctx.items }
        end)
        d.compile.root.id(function(ctx) return "root-" .. ctx.items[1].id end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let prepared = engine
        .media_prepare("root", &json!({ "path": "/tmp/root" }))
        .expect("prepare");
    assert_eq!(prepared.children.len(), 1);
    assert_eq!(prepared.children[0].field, "items");
    assert_eq!(prepared.children[0].type_name, "leaf");
    assert_eq!(prepared.children[0].targets.len(), 2);

    let leaves: Vec<Value> = prepared.children[0]
        .targets
        .iter()
        .map(|target| compile_leaf(&engine, "leaf", target))
        .collect();

    let value = engine
        .media_finalize("root", prepared.ctx_id, &json!({ "items": leaves }))
        .expect("finalize");

    assert_eq!(value["id"], "root-/tmp/root/a-leaf");
    assert_eq!(value["objects"]["info"]["count"], 2);
    assert_eq!(value["objects"]["info"]["first_tag"], "/tmp/root/a-leaf");
}

/// Verify that child selectors receive the parent context and may emit seeds.
#[test]
fn test_child_selector_sees_ctx() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("leaf")
        d.compile.leaf.tag("title", function(ctx) return ctx.manifest.title end)

        d.media.type("root")
        d.compile.root.input("manifest", function(ctx) return { title = "Virtual" } end)
        d.compile.root.child("items", "leaf", function(ctx)
            return { { manifest = ctx.manifest } }
        end)
        d.compile.root.id(function(ctx) return "root" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let prepared = engine
        .media_prepare("root", &json!({ "path": "/parent" }))
        .expect("prepare");
    assert_eq!(
        prepared.children[0].targets[0]["manifest"]["title"],
        "Virtual"
    );
}

/// Verify track ids use discnumber.tracknumber only when discnumber > 1.
#[test]
fn test_track_id_uses_discnumber() {
    let engine = LuaEngine::new().expect("engine");
    engine
        .lua
        .load(
            r#"
            d.media.type("track")
            d.compile.track.input("audio", function(ctx) return ctx.audio end)
            d.compile.track.id(function(ctx)
                local tags = ctx.tags
                local disc = tonumber(tags.discnumber or 1)
                local number = tostring(tags.tracknumber or "")
                if disc > 1 then
                    number = tostring(tags.discnumber) .. "." .. number
                end
                return table.concat({
                    tags.albumartist or tags.artist or "",
                    tags.album or "",
                    number,
                    tags.title or "",
                }, " - ")
            end)
            d.compile.track.location(function(ctx)
                return { kind = "file", path = ctx.path }
            end)
            d.compile.track.tag("albumartist", function(ctx)
                return d.get(ctx, "audio.tags.albumartist")
            end)
            d.compile.track.tag("album", function(ctx)
                return d.get(ctx, "audio.tags.album")
            end)
            d.compile.track.tag("title", function(ctx)
                return d.get(ctx, "audio.tags.title")
            end)
            d.compile.track.tag("tracknumber", function(ctx)
                return d.get(ctx, "audio.tags.tracknumber")
            end)
            d.compile.track.tag("discnumber", function(ctx)
                return d.get(ctx, "audio.tags.discnumber")
            end)
        "#,
        )
        .exec()
        .expect("register track");

    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.flac");
    std::fs::write(&file, b"fake").expect("write");

    let base = json!({
        "path": file.to_string_lossy(),
        "audio": {
            "tags": {
                "albumartist": "Artist",
                "album": "Album",
                "title": "Title",
                "tracknumber": "3",
                "discnumber": 1,
            }
        }
    });
    let single = compile_leaf(&engine, "track", &base);
    assert_eq!(single["id"], "Artist - Album - 3 - Title");
    assert_eq!(single["location"]["kind"], "file");

    let mut multi = base;
    multi["audio"]["tags"]["discnumber"] = json!(2);
    let multi = compile_leaf(&engine, "track", &multi);
    assert_eq!(multi["id"], "Artist - Album - 2.3 - Title");
}

/// Verify that tag validation rejects map tables, which serialize as JSON
/// objects instead of tag lists.
#[test]
fn test_media_tag_rejects_map() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("bad")
        d.compile.bad.tag("nested", function(ctx) return { key = "value" } end)
    "#;
    engine.lua.load(code).exec().expect("register");
    let prepared = engine
        .media_prepare("bad", &json!({ "path": "/tmp/bad" }))
        .expect("prepare");
    let result = engine.media_finalize("bad", prepared.ctx_id, &json!({}));
    assert!(result.is_err());
}

/// Verify that tag validation rejects non-scalar tag values.
#[test]
fn test_media_tag_rejects_object() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("bad")
        d.compile.bad.tag("nested", function(ctx) return { { key = "value" } } end)
    "#;
    engine.lua.load(code).exec().expect("register");
    let prepared = engine
        .media_prepare("bad", &json!({ "path": "/tmp/bad" }))
        .expect("prepare");
    let result = engine.media_finalize("bad", prepared.ctx_id, &json!({}));
    assert!(result.is_err());
}

/// Verify a failed prepare leaves no context behind, so a retry cannot leak
/// Lua tables.
#[test]
fn test_media_prepare_failure_drops_ctx() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("bad")
        d.compile.bad.input("boom", function(ctx) error("nope") end)
    "#;
    engine.lua.load(code).exec().expect("register");

    assert!(
        engine
            .media_prepare("bad", &json!({ "path": "/tmp/bad" }))
            .is_err()
    );

    let active: Table = engine
        .lua
        .load("return __DATAMILE_ACTIVE_CTX")
        .eval()
        .expect("ctx table");
    assert_eq!(active.len().expect("len"), 0);
}
