use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::lua::{EngineContext, LuaEngine};

/// Register the media types these tests exercise. Nothing is built into the
/// engine, so every fixture declares its own selectors.
fn register_fixture(engine: &LuaEngine) {
    let code = r#"
        local function audio_extensions()
            local configured = d.get(REGISTRY.config, "compiler.audio_extensions")
                or d.get(REGISTRY.config, "manifest.audio_extensions")
            if type(configured) == "table" and #configured > 0 then
                return configured
            end
            return d.fs.audio_extensions()
        end

        local function harvest(path)
            local seeds = {}
            for _, file in ipairs(d.fs.files(path, { ext = audio_extensions() })) do
                table.insert(seeds, { path = file, audio = d.fs.read_audio(file) })
            end
            return d.list.sort_by(seeds, function(item)
                return {
                    d.str.leading_int(d.get(item.audio, "tags.discnumber")) or 1,
                    d.str.leading_int(d.get(item.audio, "tags.tracknumber")) or math.huge,
                    item.path,
                }
            end)
        end

        d.media("track", {
            kind = "file",
            select = function(parent)
                return harvest(parent.path)
            end,
        })

        d.media("album", {
            kind = "root",
            dir = "/",
            select = function(dir)
                return d.fs.roots(dir, { marker = "album.toml", prune = false })
            end,
        })
    "#;
    engine.lua.load(code).exec().expect("register fixture");
}

/// Register a fake audio file and seed the harvest cache so `d.fs.read_audio`
/// returns the supplied tags without parsing real audio.
fn write_track(dir: &Path, cache: &Path, name: &str, tags: &Value) {
    let path = dir.join(name);
    fs::write(&path, b"fake audio").expect("write track");

    let meta = fs::metadata(&path).expect("metadata");
    let canon = path.canonicalize().expect("canonicalize");
    let key = crate::harvest::cache_key(&canon, &meta);

    let track = json!({
        "path": path,
        "tags": tags,
        "physics": {
            "file_size": meta.len(),
            "mtime": 0,
            "duration_ms": 1000,
            "sample_rate": 44100,
            "bit_depth": 16,
            "channels": 2,
            "audio_bitrate": 900,
            "overall_bitrate": 900,
            "format": "flac",
        }
    });

    let harvest = cache.join("harvest");
    fs::create_dir_all(&harvest).expect("harvest dir");
    fs::write(
        harvest.join(format!("{key}.json")),
        serde_json::to_string(&track).expect("serialize"),
    )
    .expect("write harvest");
}

fn engine_with_cache(cache: &Path) -> LuaEngine {
    let engine = LuaEngine::new().expect("engine");
    engine
        .lua
        .set_app_data(EngineContext::new(cache.to_path_buf()));
    register_fixture(&engine);
    engine
}

fn source_path(value: &Value) -> &str {
    value
        .as_str()
        .or_else(|| value["path"].as_str())
        .expect("source path")
}

fn source_names(sources: &[Value]) -> Vec<String> {
    sources
        .iter()
        .map(|s| {
            PathBuf::from(source_path(s))
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let album = tmp.path().join("album");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&album).expect("album dir");
    (tmp, album, cache)
}

/// Verify that physical track sources are ordered by disc, then track number.
#[test]
fn test_track_sources_sorted_by_disc_and_number() {
    let (_tmp, album, cache) = fixture();

    write_track(
        &album,
        &cache,
        "a.flac",
        &json!({ "discnumber": 1, "tracknumber": 2 }),
    );
    write_track(
        &album,
        &cache,
        "b.flac",
        &json!({ "discnumber": 1, "tracknumber": 1 }),
    );
    write_track(
        &album,
        &cache,
        "c.flac",
        &json!({ "discnumber": 2, "tracknumber": 1 }),
    );

    let engine = engine_with_cache(&cache);
    let sources = engine
        .media_select("track", &json!({ "path": album.to_string_lossy() }))
        .expect("select");

    assert_eq!(source_names(&sources), vec!["b.flac", "a.flac", "c.flac"]);
}

/// Verify that tracks without numbers keep a deterministic path order.
#[test]
fn test_track_sources_fall_back_to_path() {
    let (_tmp, album, cache) = fixture();

    write_track(&album, &cache, "z.flac", &json!({}));
    write_track(&album, &cache, "y.flac", &json!({}));
    write_track(&album, &cache, "x.flac", &json!({}));

    let engine = engine_with_cache(&cache);
    let sources = engine
        .media_select("track", &json!({ "path": album.to_string_lossy() }))
        .expect("select");

    assert_eq!(source_names(&sources), vec!["x.flac", "y.flac", "z.flac"]);
}

/// Verify that `compiler.audio_extensions` narrows the scanned extensions.
#[test]
fn test_track_sources_respects_audio_extensions() {
    let (_tmp, album, cache) = fixture();

    write_track(&album, &cache, "a.flac", &json!({ "tracknumber": 1 }));
    write_track(&album, &cache, "b.mp3", &json!({ "tracknumber": 2 }));

    let engine = engine_with_cache(&cache);
    engine
        .lua
        .load(r#"REGISTRY.config.compiler = { audio_extensions = { "mp3" } }"#)
        .exec()
        .expect("config");

    let sources = engine
        .media_select("track", &json!({ "path": album.to_string_lossy() }))
        .expect("select");

    assert_eq!(source_names(&sources), vec!["b.mp3"]);
}

/// Track numbers may carry a total ("03/12"); the leading integer orders them.
#[test]
fn test_track_fractional_numbers() {
    let (_tmp, album, cache) = fixture();

    write_track(&album, &cache, "a.flac", &json!({ "tracknumber": "10/12" }));
    write_track(&album, &cache, "b.flac", &json!({ "tracknumber": "02/12" }));
    write_track(&album, &cache, "c.flac", &json!({ "tracknumber": "1/12" }));

    let engine = engine_with_cache(&cache);
    let sources = engine
        .media_select("track", &json!({ "path": album.to_string_lossy() }))
        .expect("select");

    assert_eq!(source_names(&sources), vec!["c.flac", "b.flac", "a.flac"]);
}

/// A single recursive walk finds nested albums without re-scanning.
#[test]
fn test_album_select_finds_nested_albums() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();

    for dir in ["A", "A/disc", "A/nested", "B"] {
        fs::create_dir_all(root.join(dir)).expect("dir");
    }
    for marker in ["A/album.toml", "A/nested/album.toml", "B/album.toml"] {
        fs::write(root.join(marker), "").expect("marker");
    }

    let engine = LuaEngine::new().expect("engine");
    register_fixture(&engine);
    let mut sources = engine
        .media_select("album", &json!(root.to_string_lossy()))
        .expect("select");
    sources.sort_by_key(|s| source_path(s).to_string());

    let names: Vec<String> = sources
        .iter()
        .map(|s| {
            let path = PathBuf::from(source_path(s));
            path.strip_prefix(root)
                .expect("relative")
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(names, vec!["A", "A/nested", "B"]);
}

/// Selecting a specific directory evaluates that directory as a root.
#[test]
fn test_album_select_specific_directory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let album = root.join("A");
    fs::create_dir_all(&album).expect("dir");
    fs::write(album.join("album.toml"), "").expect("marker");
    fs::write(root.join("album.toml"), "").expect("marker");

    let engine = LuaEngine::new().expect("engine");
    register_fixture(&engine);
    let same = engine
        .media_select("album", &json!(album.to_string_lossy()))
        .expect("select");
    assert_eq!(same.len(), 1);

    let plain = root.join("plain");
    fs::create_dir_all(&plain).expect("dir");
    let none = engine
        .media_select("album", &json!(plain.to_string_lossy()))
        .expect("select");
    assert!(none.is_empty());
}
