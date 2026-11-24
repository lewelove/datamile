//! Contract of the media-agnostic Datamile Lua API: what `init.lua` may
//! declare, how it round-trips into Rust, and how logic is evaluated.

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use tempfile::TempDir;

use crate::engine::DomainLuaEngine;

/// Writes `init.lua` into a fresh temp dir and returns a ready engine.
fn setup(init: &str) -> (TempDir, DomainLuaEngine, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("init.lua");
    fs::write(&path, init).expect("write init.lua");

    let engine = DomainLuaEngine::new().expect("engine");
    (dir, engine, path)
}

#[test]
fn config_keeps_domain_specific_fields_in_extra() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.config({
            storage = {
                cache = "/tmp/cache",
                state = "/tmp/state",
                music_directory = "/music"
            },
            server = { host = "0.0.0.0", port = 9000 },
            manifest = { audio_extensions = { "flac" } },
            compiler = { jobs = 3 },
            custom = true
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(eval.config.storage.cache, "/tmp/cache");
    assert_eq!(
        eval.config.storage.extra["music_directory"],
        json!("/music")
    );
    assert_eq!(eval.config.extra["manifest"]["audio_extensions"][0], "flac");
    assert_eq!(eval.config.extra["compiler"]["jobs"], 3);
    assert_eq!(eval.config.extra["custom"], json!(true));
    assert_eq!(eval.config.server.host, "0.0.0.0");
    assert_eq!(eval.config.server.port, 9000);
}

#[test]
fn config_defaults_apply_when_nothing_is_declared() {
    let (_dir, engine, path) = setup("");

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(eval.config.server.host, "127.0.0.1");
    assert_eq!(eval.config.server.port, 8000);
    assert_eq!(eval.config.storage.cache, "~/.cache/datamile");
    assert_eq!(eval.config.storage.state, "~/.local/share/datamile");
}

#[test]
fn interface_runs_with_the_given_context() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.interface("probe", {
            label = "Probe",
            run = function(ctx) _G.PROBE_CTX = ctx.endpoint end
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");
    assert_eq!(eval.interfaces["probe"].label, "Probe");

    engine
        .execute_interface("probe", &json!({ "endpoint": "ws://local" }))
        .expect("interface must run");

    let seen: String = engine
        .runtime
        .lua
        .globals()
        .get("PROBE_CTX")
        .expect("interface set the global");
    assert_eq!(seen, "ws://local");
}

#[test]
fn undeclared_interface_is_an_error() {
    let (_dir, engine, path) = setup("");

    engine.evaluate_config(&path).expect("evaluate config");

    let err = engine
        .execute_interface("missing", &json!({}))
        .expect_err("unknown interface must fail");

    assert!(
        err.to_string().contains("not declared"),
        "unexpected: {err}"
    );
}

#[test]
fn action_runs_with_the_given_intermediary() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            run = function(ctx) _G.PROBE_ACTION = ctx.albums[1].path end
        })
        "#,
    );

    engine.evaluate_config(&path).expect("evaluate config");

    engine
        .execute_action("probe", &json!({ "albums": [{ "path": "/a" }] }))
        .expect("action must run");

    let seen: String = engine
        .runtime
        .lua
        .globals()
        .get("PROBE_ACTION")
        .expect("action set the global");
    assert_eq!(seen, "/a");
}

/// Clients render actions by `label` and `description`, so `d.action` must
/// forward both instead of registering only the run function.
#[test]
fn action_keeps_its_label_and_description() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            label = "Probe",
            description = "Probe the payload",
            run = function(_ctx) end
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");
    let action = eval.actions.get("probe").expect("action is registered");

    assert_eq!(action.label, "Probe");
    assert_eq!(action.description.as_deref(), Some("Probe the payload"));
}

/// A bare `d.action` still needs a label for the interface to show.
#[test]
fn action_label_falls_back_to_its_name() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            run = function(_ctx) end
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(eval.actions["probe"].label, "probe");
    assert!(eval.actions["probe"].description.is_none());
}

#[test]
fn logic_manifest_normalizes_ordering_and_targets() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.filter("b", { label = "B" })
        datamile.filter("a", { label = "A" })
        datamile.grouper("g", { label = "G" })
        datamile.order("z", { label = "Z" })
        datamile.order("y", { label = "Y" })
        datamile.library("lib", {
            label = "Lib",
            filters = { "a", "missing" },
            groupers = { "g" },
            orders = { "y" }
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");
    let manifest = eval.manifest;

    assert_eq!(manifest.filters_order, vec!["b", "a"]);
    assert_eq!(manifest.orders_order, vec!["z", "y"]);

    let grouper = &manifest.groupers["g"];
    assert_eq!(grouper.index, Some(false));
    assert_eq!(grouper.count, Some(true));

    let library = &manifest.libraries["lib"];
    assert_eq!(library.allowed_filters, vec!["a"]);
    assert_eq!(library.allowed_groupers, vec!["g"]);
    assert_eq!(library.allowed_orders, vec!["y"]);
}

#[test]
fn evaluate_logic_accepts_any_entity_table() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.filter("odd", { match = function(e) return e.n % 2 == 1 end })
        datamile.grouper("parity", {
            select = function(e) return e.n % 2 == 0 and "even" or "odd" end
        })
        datamile.order("n", { sort = function(e) return e.n end })
        datamile.library("small", { match = function(e) return e.n < 10 end })
        datamile.shelf("three", {
            match = function(e) return e.n == 3 end,
            sort = function(e) return e.n end
        })
        "#,
    );

    engine.evaluate_config(&path).expect("evaluate config");

    let res = engine
        .evaluate_logic("album", &json!({ "n": 3 }))
        .expect("evaluate logic");

    assert_eq!(res["filters"]["odd"], json!(true));
    assert_eq!(res["groupers"]["parity"], json!(["odd"]));
    assert_eq!(res["orders"]["n"], json!(3));
    assert_eq!(res["libraries"]["small"], json!(true));
    assert_eq!(res["shelves"]["three"], json!(true));
    assert_eq!(res["shelf_sorts"]["three"], json!(3));
}

#[test]
fn load_extension_runs_before_init_lua() {
    let (_dir, engine, path) = setup(r#"datamile.filter("from_init", { label = "I" })"#);

    engine
        .load_extension(
            "extension.lua",
            r#"d.filter("from_extension", { label = "E" })"#,
        )
        .expect("load extension");

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert!(eval.manifest.filters.contains_key("from_extension"));
    assert!(eval.manifest.filters.contains_key("from_init"));
}

#[test]
fn d_and_datamile_are_the_same_table() {
    let (_dir, engine, _path) = setup("");

    let same: bool = engine
        .runtime
        .lua
        .load("return d == datamile")
        .eval()
        .expect("evaluate comparison");

    assert!(same, "d and datamile must alias one table");
}

/// Logic units only evaluate for their declared `media` type; a missing one
/// keeps the legacy album behaviour during the transition.
#[test]
fn logic_is_scoped_by_media_type() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.library("albums", { media = "album", match = function() return true end })
        datamile.library("books", { media = "book", match = function() return true end })
        datamile.filter("legacy", { match = function() return true end })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");
    assert_eq!(eval.manifest.libraries["books"].media, "book");
    assert_eq!(eval.manifest.filters["legacy"].media, "album");

    let album = engine
        .evaluate_logic("album", &json!({}))
        .expect("album logic");
    assert_eq!(album["libraries"]["albums"], json!(true));
    assert_eq!(album["libraries"]["books"], json!(null));
    assert_eq!(album["filters"]["legacy"], json!(true));

    let book = engine
        .evaluate_logic("book", &json!({}))
        .expect("book logic");
    assert_eq!(book["libraries"]["books"], json!(true));
    assert_eq!(book["libraries"]["albums"], json!(null));
    assert_eq!(book["filters"]["legacy"], json!(null));
}

/// The `mpd` block is part of the domain config, not an opaque extra.
#[test]
fn config_reads_the_mpd_block() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.config({
            mpd = { enabled = false, host = "mpd.local", port = 7000, music_directory = "/music" }
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert!(!eval.config.mpd.enabled);
    assert_eq!(eval.config.mpd.host, "mpd.local");
    assert_eq!(eval.config.mpd.port, 7000);
    assert_eq!(eval.config.mpd.music_directory.as_deref(), Some("/music"));
}
