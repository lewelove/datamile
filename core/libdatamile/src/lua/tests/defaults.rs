use crate::lua::{EngineContext, LuaEngine};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Register a fake audio file and seed the harvest cache so `d.fs.read_audio`
/// returns the supplied tags without parsing real audio.
fn write_track(dir: &Path, cache: &Path, name: &str, tags: &Value) {
    let path = dir.join(name);
    fs::write(&path, b"fake audio").expect("write track");

    let meta = fs::metadata(&path).expect("metadata");
    let mtime = meta
        .modified()
        .expect("mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_nanos();
    let canon = path.canonicalize().expect("canonicalize");
    let key_str = format!("{}:{}:{mtime}", canon.display(), meta.len());
    let key = blake3::hash(key_str.as_bytes()).to_hex().to_string();

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
    engine
}

fn source_names(sources: &[Value]) -> Vec<String> {
    sources
        .iter()
        .map(|s| {
            PathBuf::from(s["path"].as_str().expect("source path"))
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

/// Verify that `theme.toml` is compiled into `objects.colors` with the
/// background palette normalized to a list of valid hex colors.
#[test]
fn test_album_colors_from_theme() {
    let (_tmp, album, cache) = fixture();
    fs::write(
        album.join("theme.toml"),
        "[album.colors]\nbackground = \"#AABBCC\"\nforeground = \"#FFF\"\n",
    )
    .expect("theme");

    let engine = engine_with_cache(&cache);
    let value = engine
        .media_compile("album", &json!({ "path": album.to_string_lossy() }))
        .expect("compile");

    assert_eq!(value["objects"]["colors"]["background"], json!(["#AABBCC"]));
    assert_eq!(value["objects"]["colors"]["foreground"], json!("#FFF"));
}

/// Verify that malformed theme colors are dropped instead of compiled.
#[test]
fn test_album_colors_rejects_invalid_hex() {
    let (_tmp, album, cache) = fixture();
    fs::write(
        album.join("theme.toml"),
        "[album.colors]\nbackground = [\"#AABBCC\", \"nope\", \"#GGG\"]\nforeground = \"red\"\n",
    )
    .expect("theme");

    let engine = engine_with_cache(&cache);
    let value = engine
        .media_compile("album", &json!({ "path": album.to_string_lossy() }))
        .expect("compile");

    assert_eq!(value["objects"]["colors"]["background"], json!(["#AABBCC"]));
    assert!(value["objects"]["colors"].get("foreground").is_none());
}
