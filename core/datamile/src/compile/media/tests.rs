use super::{RootTarget, compile_root};
use libdatamile::lua::LuaEngine;
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn root_target(root: &Path) -> RootTarget {
    RootTarget {
        root_type: "root".to_string(),
        target: root.to_path_buf(),
        lock_path: root.join("root.lock.json"),
        source: json!({ "path": root.to_string_lossy() }),
    }
}

/// Leaves finalize before their parents so parents aggregate compiled children
/// (3-level chain: root -> middle -> leaf).
#[test]
fn compile_root_runs_bottom_up() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("m1/l1")).expect("dirs");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("leaf")
        d.compile.leaf.tag("name", function(ctx) return ctx.path:match("[^/]+$") end)
        d.compile.leaf.id(function(ctx) return "leaf:" .. ctx.tags.name end)

        d.media.type("middle")
        d.compile.middle.child("leaves", "leaf", function(ctx)
            return { { path = ctx.path .. "/l1" } }
        end)
        d.compile.middle.id(function(ctx) return "middle:" .. #ctx.leaves end)
        d.compile.middle.object("info", function(ctx)
            return { leaf_id = ctx.leaves[1].id }
        end)

        d.media.type("root")
        d.compile.root.lock(function(t) return t.path .. "/root.lock.json" end)
        d.compile.root.child("middles", "middle", function(ctx)
            return { { path = ctx.path .. "/m1" } }
        end)
        d.compile.root.id(function(ctx) return "root:" .. ctx.middles[1].id end)
        d.compile.root.object("info", function(ctx)
            return {
                middle_id = ctx.middles[1].id,
                leaf_id = ctx.middles[1].objects.info.leaf_id,
            }
        end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root), tmp.path()).expect("compile");

    let entity = &build.lock_json["root"];
    assert_eq!(entity["id"], "root:middle:1");
    assert_eq!(entity["objects"]["info"]["leaf_id"], "leaf:l1");
    assert_eq!(entity["objects"]["info"]["middle_id"], "middle:1");
    assert_eq!(entity["middles"][0]["leaves"][0]["tags"]["name"], "l1");
    assert_eq!(build.lock_path, root.join("root.lock.json"));
}

/// A child without an explicit selector falls back to the child type's select.
#[test]
fn compile_root_uses_child_type_select() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("items")).expect("dirs");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media.type("leaf")
        d.compile.leaf.select(function(scope)
            return { { path = scope.path .. "/items" } }
        end)
        d.compile.leaf.tag("kind", function(ctx) return "item" end)

        d.media.type("root")
        d.compile.root.lock(function(t) return t.path .. "/root.lock.json" end)
        d.compile.root.child("leaves", "leaf")
        d.compile.root.id(function(ctx) return "root" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root), tmp.path()).expect("compile");

    assert_eq!(build.lock_json["root"]["leaves"][0]["tags"]["kind"], "item");
}
