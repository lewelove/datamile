//! Contract of the media-agnostic Datamile Lua API: what `init.lua` may
//! declare, how it round-trips into Rust, and how logic is evaluated.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::json;
use tempfile::TempDir;

use crate::config::{ActionCtx, ActionMap};
use crate::engine::DomainLuaEngine;

/// Writes `init.lua` into a fresh temp dir and returns a ready engine.
fn setup(init: &str) -> (TempDir, DomainLuaEngine, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("init.lua");
    fs::write(&path, init).expect("write init.lua");

    let engine = DomainLuaEngine::new().expect("engine");
    (dir, engine, path)
}

/// Every declared section and key survives the Lua round-trip.
#[test]
fn config_reads_every_declared_section() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.config({
            storage = { cache = "/tmp/cache", state = "/tmp/state" },
            server = { host = "0.0.0.0", port = 9000 },
            compiler = { jobs = 3 }
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(eval.config.storage.cache, "/tmp/cache");
    assert_eq!(eval.config.storage.state, "/tmp/state");
    assert_eq!(eval.config.server.host, "0.0.0.0");
    assert_eq!(eval.config.server.port, 9000);
    assert_eq!(eval.config.compiler.jobs, Some(3));
}

/// `d.config` is closed: an option it does not declare is a typo, and the whole
/// config is refused instead of quietly carrying a key nothing reads.
#[test]
fn config_rejects_unknown_options() {
    for (declaration, name) in [
        ("custom = true", "custom"),
        (
            "storage = { cache = \"/tmp/cache\", retention = 3 }",
            "retention",
        ),
        ("server = { host = \"0.0.0.0\", tls = true }", "tls"),
        ("mpd = { port = 6600, password = \"x\" }", "password"),
        ("compiler = { jobs = 3, threads = 3 }", "threads"),
        ("manifest = { audio_extensions = { \"flac\" } }", "manifest"),
    ] {
        let (_dir, engine, path) =
            setup(&format!("datamile.config({{ {declaration} }})"));

        let err = engine
            .evaluate_config(&path)
            .err()
            .unwrap_or_else(|| panic!("{declaration} must be rejected"));

        let message = format!("{err}");
        assert!(
            message.contains(name),
            "error for '{declaration}' must name '{name}': {message}"
        );
    }
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

/// Evaluates `init.lua` and resolves the declared actions into the map clients
/// and endpoints consume.
fn resolved_actions(engine: &DomainLuaEngine, path: &Path) -> Result<ActionMap> {
    let eval = engine.evaluate_config(path)?;
    engine.resolve_actions(&eval.actions)
}

/// `run` receives the whole action context: the parsed lock, the resolved
/// media type, the trailing options and the terminal flag.
#[test]
fn action_runs_with_the_action_context() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            run = function(ctx)
                _G.PROBE_LOCK = ctx.lock.album.title
                _G.PROBE_MEDIA = ctx.media
                _G.PROBE_OPTIONS = table.concat(ctx.options, ",")
                _G.PROBE_ISATTY = ctx.isatty
            end
        })
        "#,
    );

    engine.evaluate_config(&path).expect("evaluate config");

    let ctx = ActionCtx {
        lock: Some(json!({ "album": { "title": "Abbey Road" } })),
        media: Some("album".to_string()),
        options: vec!["--force".to_string(), "x".to_string()],
        isatty: true,
    };
    engine
        .execute_action("probe", &ctx)
        .expect("action must run");

    let globals = engine.runtime.lua.globals();
    let lock: String = globals.get("PROBE_LOCK").expect("lock reached run");
    let media: String = globals.get("PROBE_MEDIA").expect("media reached run");
    let options: String = globals.get("PROBE_OPTIONS").expect("options reached run");
    let isatty: bool = globals.get("PROBE_ISATTY").expect("isatty reached run");

    assert_eq!(lock, "Abbey Road");
    assert_eq!(media, "album");
    assert_eq!(options, "--force,x");
    assert!(isatty);
}

/// The canonical name is the identity of an action, so a second declaration
/// that canonicalizes onto an existing one is a config error whichever
/// spelling comes first.
#[test]
fn action_duplicate_canonical_names_are_refused() {
    for (first, second) in [
        ("open-terminal", "open_terminal"),
        ("open_terminal", "open-terminal"),
    ] {
        let (_dir, engine, path) = setup(&format!(
            r#"
            datamile.action("{first}", {{ media = "album", run = function(_ctx) end }})
            datamile.action("{second}", {{ media = "movie", run = function(_ctx) end }})
            "#
        ));

        let err = engine
            .evaluate_config(&path)
            .err()
            .expect("duplicate canonical names must fail");

        // Declaration errors are raised while `init.lua` runs, so the Lua
        // message sits in the error chain rather than on the outer context.
        let message = format!("{err:#}");
        assert!(message.contains("declared twice"), "unexpected: {message}");
        assert!(message.contains("open_terminal"), "unexpected: {message}");
        assert!(message.contains("open-terminal"), "unexpected: {message}");
    }
}

/// `media` selects the action's scope; anything that is not a list of names
/// fails the whole config instead of silently registering nothing.
#[test]
fn action_media_shape_is_validated() {
    for declaration in [
        "media = \"\"",
        "media = {}",
        "media = { \"\" }",
        "media = { \"album\", \"\" }",
        "media = 7",
    ] {
        let (_dir, engine, path) = setup(&format!(
            "datamile.action(\"probe\", {{ {declaration}, run = function(_ctx) end }})"
        ));

        let err = engine
            .evaluate_config(&path)
            .err()
            .unwrap_or_else(|| panic!("{declaration} must be rejected"));

        let message = format!("{err}");
        assert!(
            message.contains("'media' must be a non-empty string"),
            "error for '{declaration}': {message}"
        );
    }
}

/// A repeated media type is one scope, not two entries of the same name.
#[test]
fn action_media_is_deduplicated() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            media = { "album", "album", "movie" },
            run = function(_ctx) end
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(eval.actions["probe"], vec!["album", "movie"]);
}

/// Every declaration needs a callable body; the old merge that borrowed `run`
/// from an earlier declaration is gone.
#[test]
fn action_without_a_run_function_is_refused() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", { media = "album", label = "Probe" })
        "#,
    );

    let err = engine
        .evaluate_config(&path)
        .err()
        .expect("a missing run function must fail");

    assert!(format!("{err:#}").contains("run"), "unexpected: {err:#}");
}

/// Clients render actions by `label` and `description`; a generic action keeps
/// the flat pair and ignores the unread `config` field.
#[test]
fn generic_action_keeps_its_label_and_description() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", {
            label = "Probe",
            description = "Probe the payload",
            config = { unused = true },
            run = function(_ctx) end
        })
        "#,
    );

    let actions = resolved_actions(&engine, &path).expect("resolve actions");
    let entry = &actions["probe"];

    assert_eq!(entry.label, "Probe");
    assert_eq!(entry.description.as_deref(), Some("Probe the payload"));
    assert!(entry.media.is_none(), "a generic action has no media scope");
}

/// A scoped action carries one presentation per declared media type, built by
/// calling `label`/`description` with that media type.
#[test]
fn scoped_action_resolves_presentation_per_media() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("open-terminal", {
            media = { "album", "movie" },
            label = function(media)
                return media == "album" and "Open Album Terminal" or "Open Movie Terminal"
            end,
            description = function(media)
                return "Open a terminal in the " .. media .. " directory"
            end,
            run = function(_ctx) end
        })
        "#,
    );

    let actions = resolved_actions(&engine, &path).expect("resolve actions");
    let entry = &actions["open_terminal"];
    let media = entry.media.as_ref().expect("scoped action has media");

    assert_eq!(media["album"].label, "Open Album Terminal");
    assert_eq!(
        media["album"].description.as_deref(),
        Some("Open a terminal in the album directory")
    );
    assert_eq!(media["movie"].label, "Open Movie Terminal");
    assert_eq!(entry.label, "open_terminal");
    assert!(entry.description.is_none());
}

/// A scoped action without a label still shows its canonical name.
#[test]
fn scoped_action_label_falls_back_to_its_name() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.action("probe", { media = "album", run = function(_ctx) end })
        "#,
    );

    let actions = resolved_actions(&engine, &path).expect("resolve actions");
    let media = actions["probe"].media.as_ref().expect("scoped action");

    assert_eq!(media["album"].label, "probe");
    assert!(media["album"].description.is_none());
}

/// A presentation callback that cannot produce text for a media type is a
/// config error naming both the action and the media type.
#[test]
fn action_callback_failures_name_the_action_and_media() {
    for callback in [
        "function(_media) return nil end",
        "function(_media) return \"\" end",
        "function(_media) error(\"boom\") end",
    ] {
        let (_dir, engine, path) = setup(&format!(
            r#"
            datamile.action("probe", {{
                media = "album",
                label = {callback},
                run = function(_ctx) end
            }})
            "#
        ));

        let err = resolved_actions(&engine, &path)
            .expect_err("a failed label callback must fail config load");

        let message = format!("{err}");
        assert!(
            message.contains("action 'probe'")
                && message.contains("'label' callback failed")
                && message.contains("media 'album'"),
            "error for {callback}: {message}"
        );
    }
}

/// `server.allowed_origins` extends the LAN allowlist, so a malformed entry
/// fails the config instead of being carried along unused.
#[test]
fn malformed_allowed_origins_fail_config_load() {
    for (entry, name) in [
        ("10.0.0.0/33", "10.0.0.0/33"),
        ("10.0.0.0/nope", "10.0.0.0/nope"),
        ("bad host!", "bad host!"),
        ("box.lan:8080", "box.lan:8080"),
        ("10.0.0.5:8000", "10.0.0.5:8000"),
        ("[::1]:8000", "[::1]:8000"),
    ] {
        let (_dir, engine, path) = setup(&format!(
            "datamile.config({{ server = {{ allowed_origins = {{ \"{entry}\" }} }} }})"
        ));

        let err = engine
            .evaluate_config(&path)
            .err()
            .unwrap_or_else(|| panic!("'{entry}' must be rejected"));

        let message = format!("{err}");
        assert!(
            message.contains("allowed_origins"),
            "error for '{entry}': {message}"
        );
        assert!(message.contains(name), "error for '{entry}': {message}");
    }
}

/// Hostnames, addresses and CIDR blocks are all valid allowlist entries.
#[test]
fn allowed_origins_accept_hosts_and_cidrs() {
    let (_dir, engine, path) = setup(
        r#"
        datamile.config({
            server = { allowed_origins = {
                "datamile.lan", "192.168.0.0/16", "[::1]", "10.0.0.5", "fd00::1"
            } }
        })
        "#,
    );

    let eval = engine.evaluate_config(&path).expect("evaluate config");

    assert_eq!(
        eval.config.server.allowed_origins,
        vec![
            "datamile.lan",
            "192.168.0.0/16",
            "[::1]",
            "10.0.0.5",
            "fd00::1"
        ]
    );
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
