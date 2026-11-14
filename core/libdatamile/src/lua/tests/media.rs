use mlua::Table;
use serde_json::{Value, json};

use crate::lua::LuaEngine;

fn compile_leaf(engine: &LuaEngine, type_name: &str, seed: &Value) -> Value {
    let prepared = engine
        .media_prepare(type_name, seed, None)
        .expect("prepare");
    assert!(prepared.children.is_empty(), "leaf types have no children");
    engine
        .media_finalize(type_name, prepared.ctx_id, &json!({}))
        .expect("finalize")
}

fn validate(engine: &LuaEngine) -> Result<(), String> {
    engine.media_validate().map_err(|err| err.to_string())
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

/// Verify that only `kind = "root"` types are treated as roots.
#[test]
fn test_media_roots_are_root_kind() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("root", {
            kind = "root",
            dir = "/music",
            select = function() return {} end,
        })
        d.media("child", {
            kind = "virtual",
            select = function() return {} end,
        })
        return __DATAMILE_MEDIA_ROOTS()
    "#;
    let roots: Table = engine.lua.load(code).eval().expect("roots");
    let names: Vec<String> = roots
        .sequence_values::<String>()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(names, vec!["root".to_string()]);
}

/// Verify root declarations require a directory and a selector.
#[test]
fn test_root_requires_dir_and_select() {
    let engine = LuaEngine::new().expect("engine");
    let err = engine
        .lua
        .load(r#"d.media("album", { kind = "root", select = function() return {} end })"#)
        .exec()
        .expect_err("missing dir");
    assert!(err.to_string().contains("non-empty string"));

    let err = engine
        .lua
        .load(r#"d.media("album", { kind = "root", dir = "/music" })"#)
        .exec()
        .expect_err("missing select");
    assert!(err.to_string().contains("select"));
}

/// Verify `root` cannot be combined with child kinds and cannot be a child.
#[test]
fn test_root_kind_is_exclusive() {
    let engine = LuaEngine::new().expect("engine");
    let err = engine
        .lua
        .load(r#"d.media("album", { kind = { "root", "dir" } })"#)
        .exec()
        .expect_err("combined root");
    assert!(err.to_string().contains("cannot be combined"));

    let code = r#"
        d.media("album", {
            kind = "root",
            dir = "/music",
            select = function() return {} end,
            children = { tracks = "track" },
        })
        d.media("track", {
            kind = "file",
            select = function() return {} end,
        })
        d.media("bad", {
            kind = "dir",
            select = function() return {} end,
            children = { albums = "album" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");
    let err = validate(&engine).expect_err("root child");
    assert!(err.contains("cannot reference root type"));
}

/// Verify child kind arrays cannot include `root` or unknown kinds.
#[test]
fn test_child_kind_validation() {
    let engine = LuaEngine::new().expect("engine");
    let err = engine
        .lua
        .load(r#"d.media("bad", { kind = { "file", "root" }, select = function() return {} end })"#)
        .exec()
        .expect_err("root kind");
    assert!(err.to_string().contains("cannot be combined"));

    let err = engine
        .lua
        .load(r#"d.media("bad", { kind = "tape", select = function() return {} end })"#)
        .exec()
        .expect_err("unknown kind");
    assert!(err.to_string().contains("unknown kind"));
}

/// Verify file and virtual parents may only declare virtual children.
#[test]
fn test_physical_parent_child_rules() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("chapter", {
            kind = "virtual",
            select = function() return {} end,
        })
        d.media("episode", {
            kind = "file",
            select = function() return {} end,
        })
        d.media("good", {
            kind = "file",
            select = function() return {} end,
            children = { chapters = "chapter" },
        })
        d.media("bad", {
            kind = "virtual",
            select = function() return {} end,
            children = { episodes = "episode" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");
    let err = validate(&engine).expect_err("physical child");
    assert!(err.contains("only have virtual children"));
}

/// Verify the output boxes populate tags, objects, images, files and id.
#[test]
fn test_media_output_boxes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join("thing.toml");
    std::fs::write(&file, b"[thing]").expect("write");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("thing", {
            kind = "file",
            select = function() return {} end,
        })
        d.compile.thing.file("manifest", function(ctx) return ctx.path end)
        d.compile.thing.tag("genre", function(ctx) return "Ambient" end)
        d.compile.thing.tag("styles", function(ctx) return { "A", "B" } end)
        d.compile.thing.object("info", function(ctx) return { total = 2 } end)
        d.compile.thing.image("main", function(ctx) return { sources = "cover.jpg" } end)
        d.compile.thing.id(function(ctx) return "thing-" .. ctx.tags.genre end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let value =
        compile_leaf(&engine, "thing", &json!({ "path": file.to_string_lossy() }));
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
    assert_eq!(
        value["files"]["manifest"]["path"],
        file.to_str().expect("utf8")
    );
    assert!(value["files"]["manifest"]["path"].is_string());
    assert!(value["files"]["manifest"]["mtime"].is_number());
    assert!(value["files"]["manifest"]["byte_size"].is_number());
    assert!(value.get("location").is_none());
}

/// Verify `.file()` exposes stats to later tags and ids.
#[test]
fn test_file_stats_available_to_tags() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join("album.toml");
    std::fs::write(&file, b"[album]").expect("write");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("album", {
            kind = "dir",
            select = function() return {} end,
        })
        d.compile.album.file("manifest", function(ctx) return "album.toml" end)
        d.compile.album.tag("manifest_size", function(ctx)
            return ctx.files.manifest.byte_size
        end)
        d.compile.album.id(function(ctx)
            return "album-" .. tostring(ctx.tags.manifest_size)
        end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let value = compile_leaf(
        &engine,
        "album",
        &json!({ "path": tmp.path().to_string_lossy() }),
    );
    assert_eq!(
        value["files"]["manifest"]["path"],
        file.to_str().expect("utf8")
    );
    assert_eq!(value["tags"]["manifest_size"], json!(b"[album]".len()));
    assert_eq!(value["id"], "album-7");
}

/// `.file()` accepts lists and omits missing paths.
#[test]
fn test_file_lists_and_missing_paths() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("a.lrc"), b"a").expect("write");
    std::fs::write(tmp.path().join("b.lrc"), b"bb").expect("write");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("track", {
            kind = "dir",
            select = function() return {} end,
        })
        d.compile.track.file("lyrics", function()
            return { "a.lrc", "missing.lrc", "b.lrc" }
        end)
        d.compile.track.file("none", function() return nil end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let value = compile_leaf(
        &engine,
        "track",
        &json!({ "path": tmp.path().to_string_lossy() }),
    );
    let lyrics = value["files"]["lyrics"].as_array().expect("lyrics list");
    assert_eq!(lyrics.len(), 2);
    assert!(lyrics[0]["path"].as_str().expect("path").ends_with("a.lrc"));
    assert_eq!(lyrics[1]["byte_size"], 2);
    assert!(value["files"].get("none").is_none());
}

/// Verify every input, file, image, tag, object and id hook runs exactly once.
#[test]
fn test_media_hooks_run_once() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("x.toml"), b"x").expect("write");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        counts = {}
        order = {}
        local function bump(name)
            counts[name] = (counts[name] or 0) + 1
            table.insert(order, name)
        end

        d.media("thing", {
            kind = "dir",
            select = function() return {} end,
        })
        d.compile.thing.input("first", function(ctx) bump("input:first") return 1 end)
        d.compile.thing.input("second", function(ctx) bump("input:second") return ctx.first + 1 end)
        d.compile.thing.file("manifest", function(ctx) bump("file") return "x.toml" end)
        d.compile.thing.tag("alpha", function(ctx) bump("tag:alpha") return "a" end)
        d.compile.thing.tag("beta", function(ctx) bump("tag:beta") return "b" end)
        d.compile.thing.image("main", function(ctx) bump("image") return { sources = "x.jpg" } end)
        d.compile.thing.object("info", function(ctx) bump("object") return {} end)
        d.compile.thing.id(function(ctx) bump("id") return "id" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    compile_leaf(
        &engine,
        "thing",
        &json!({ "path": tmp.path().to_string_lossy() }),
    );

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

/// Verify file, image, tag, id and object hooks run in the documented order.
#[test]
fn test_output_hook_order() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("x.toml"), b"x").expect("write");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        order = {}
        d.media("thing", {
            kind = "dir",
            select = function() return {} end,
        })
        d.compile.thing.file("f", function() table.insert(order, "file") return "x.toml" end)
        d.compile.thing.image("i", function() table.insert(order, "image") return { sources = "x.jpg" } end)
        d.compile.thing.tag("t", function() table.insert(order, "tag") return "t" end)
        d.compile.thing.id(function() table.insert(order, "id") return "id" end)
        d.compile.thing.object("o", function() table.insert(order, "object") return {} end)
    "#;
    engine.lua.load(code).exec().expect("register");

    compile_leaf(
        &engine,
        "thing",
        &json!({ "path": tmp.path().to_string_lossy() }),
    );

    let order: Table = engine.lua.globals().get("order").expect("order");
    let order: Vec<String> = order
        .sequence_values::<String>()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(order, vec!["file", "image", "tag", "id", "object"]);
}

/// Verify children compile before the parent finalizes and inject their output.
#[test]
fn test_children_compile_bottom_up() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "dir",
            select = function(parent)
                return { parent.path .. "/a", parent.path .. "/b" }
            end,
        })
        d.compile.leaf.tag("name", function(ctx) return ctx.path .. "-leaf" end)
        d.compile.leaf.id(function(ctx) return ctx.tags.name end)

        d.media("root", {
            kind = "dir",
            select = function() return {} end,
            children = { items = "leaf" },
        })
        d.compile.root.object("info", function(ctx)
            local first = ctx.items[1]
            return { first_id = first.id, first_tag = first.tags.name, count = #ctx.items }
        end)
        d.compile.root.id(function(ctx) return "root-" .. ctx.items[1].id end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let prepared = engine
        .media_prepare("root", &json!({ "path": "/tmp/root" }), None)
        .expect("prepare");
    assert_eq!(prepared.children.len(), 1);
    assert_eq!(prepared.children[0].field, "items");
    assert_eq!(prepared.children[0].type_name, "leaf");
    assert_eq!(
        prepared.children[0].targets,
        vec![json!("/tmp/root/a"), json!("/tmp/root/b")]
    );

    let leaves: Vec<Value> = prepared.children[0]
        .targets
        .iter()
        .map(|target| {
            let seed = json!({ "path": target.as_str().expect("target") });
            compile_leaf(&engine, "leaf", &seed)
        })
        .collect();

    let value = engine
        .media_finalize("root", prepared.ctx_id, &json!({ "items": leaves }))
        .expect("finalize");

    assert_eq!(value["id"], "root-/tmp/root/a-leaf");
    assert_eq!(value["objects"]["info"]["count"], 2);
    assert_eq!(value["objects"]["info"]["first_tag"], "/tmp/root/a-leaf");
}

/// Verify child selectors inherit parent inputs and child ctx.parent.
#[test]
fn test_child_ctx_parent() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "virtual",
            select = function(parent)
                return { { title = parent.manifest.title } }
            end,
        })
        d.media("root", {
            kind = "dir",
            select = function() return {} end,
            children = { items = "leaf" },
        })
        d.compile.root.input("manifest", function() return { title = "Parent" } end)
        d.compile.leaf.tag("title", function(ctx)
            return ctx.parent.manifest.title .. "/" .. ctx.parent.manifest.title
        end)
        d.compile.leaf.id(function(ctx) return ctx.tags.title end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let prepared = engine
        .media_prepare("root", &json!({ "path": "/parent" }), None)
        .expect("prepare");
    assert_eq!(prepared.children[0].targets[0]["title"], "Parent");

    let child_prepared = engine
        .media_prepare(
            "leaf",
            &prepared.children[0].targets[0],
            Some(prepared.ctx_id),
        )
        .expect("child prepare");
    let value = engine
        .media_finalize("leaf", child_prepared.ctx_id, &json!({}))
        .expect("child finalize");
    assert_eq!(value["tags"]["title"], "Parent/Parent");
}

/// Verify that tag validation rejects map tables, which serialize as JSON
/// objects instead of tag lists.
#[test]
fn test_media_tag_rejects_map() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("bad", {
            kind = "virtual",
            select = function() return {} end,
        })
        d.compile.bad.tag("nested", function() return { key = "value" } end)
    "#;
    engine.lua.load(code).exec().expect("register");
    let prepared = engine
        .media_prepare("bad", &json!({}), None)
        .expect("prepare");
    let result = engine.media_finalize("bad", prepared.ctx_id, &json!({}));
    assert!(result.is_err());
}

/// Verify that tag validation rejects non-scalar tag values.
#[test]
fn test_media_tag_rejects_object() {
    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("bad", {
            kind = "virtual",
            select = function() return {} end,
        })
        d.compile.bad.tag("nested", function() return { { key = "value" } } end)
    "#;
    engine.lua.load(code).exec().expect("register");
    let prepared = engine
        .media_prepare("bad", &json!({}), None)
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
        d.media("bad", {
            kind = "virtual",
            select = function() return {} end,
        })
        d.compile.bad.input("boom", function() error("nope") end)
    "#;
    engine.lua.load(code).exec().expect("register");

    assert!(engine.media_prepare("bad", &json!({}), None).is_err());

    let active: Table = engine
        .lua
        .load("return __DATAMILE_ACTIVE_CTX")
        .eval()
        .expect("ctx table");
    assert_eq!(active.len().expect("len"), 0);
}
