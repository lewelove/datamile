//! Album config contract: the media-agnostic domain config round-trips into the
//! album `AppConfig`, and its JSON shape stays what `compile/build.rs` feeds to
//! `ctx.config`.

use std::fs;

use serde_json::json;
use tempfile::TempDir;

use crate::lua::LuaEngine;

fn evaluate(init: &str) -> crate::lua::EvaluatedLuaData {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("init.lua");
    fs::write(&path, init).expect("write init.lua");

    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    engine.evaluate_config(&path).expect("evaluate config")
}

#[test]
fn album_config_reads_domain_and_extra_fields() {
    let eval = evaluate(
        r#"
        datamile.config({
            storage = {
                music_directory = "/music",
                cache = "/cache",
                state = "/state"
            },
            manifest = { audio_extensions = { "flac" } },
            compiler = { manifests = { "metadata" }, jobs = 2 },
            server = { host = "0.0.0.0", port = 9000 }
        })
        "#,
    );

    assert_eq!(eval.app.storage.music_directory, "/music");
    assert_eq!(eval.app.storage.cache, "/cache");
    assert_eq!(eval.app.storage.state, "/state");
    assert_eq!(eval.app.compiler.jobs, Some(2));
    assert_eq!(eval.app.server.port, 9000);
}

#[test]
fn album_config_serializes_to_the_dispatcher_shape() {
    let eval = evaluate(
        r#"
        datamile.config({
            storage = {
                music_directory = "/music",
                cache = "/cache",
                state = "/state"
            },
            manifest = { audio_extensions = { "flac" } },
            compiler = { manifests = { "metadata" }, jobs = 2 },
            server = { host = "0.0.0.0", port = 9000 }
        })
        "#,
    );

    let value = serde_json::to_value(&eval.app).expect("serialize app config");

    assert_eq!(
        value,
        json!({
            "storage": {
                "music_directory": "/music",
                "cache": "/cache",
                "state": "/state"
            },
            "compiler": {
                "manifests": ["metadata"],
                "audio_extensions": null,
                "jobs": 2
            },
            "server": { "host": "0.0.0.0", "port": 9000 }
        })
    );
}
