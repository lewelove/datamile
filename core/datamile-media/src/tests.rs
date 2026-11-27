//! Contract of the schema-driven media engine: DSL validation, discovery,
//! single-pass compilation and every hot-reload guard.

// The fixture guard keeps its `TempDir` alive in an underscore field, and every
// test reads the entity dir through it. Clippy reads that as using an underscore
// binding, which is exactly what the guard is for here.
#![allow(
    clippy::used_underscore_binding,
    reason = "`_fixture` holds a TempDir guard that tests are meant to read through"
)]

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datamile_cache::EntityKey;
use datamile_logic::{LogicEngine, SortOrder};
use datamile_lua_domain::{ActionDeclarations, DomainLuaEngine, LogicManifest};
use datamile_mpd::MpdEngine;
use datamile_server::{
    ImageTaskRegistry, MediaBackend, ServerConfig, ServerState, UpdateRequest,
};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::backend::MediaEngineBackend;
use crate::compiler::compile_root;
use crate::deps::{
    ExternalIndex, MediaShared, UpdateScope, index_entity, unindex_entity,
};
use crate::lua_bridge::register_native_helpers;
use crate::schema::{
    MediaManifest, discover_roots, read_media_manifest, validate_media_schema,
};
use crate::state::{COMPILE_ROOT_LUA, MEDIA_LUA, MediaState, state_path};
use crate::update::{
    ForceMode, LogSink, PassResult, UpdateEvent, UpdateSink, run_pass, scope_for_request,
};
use crate::watcher::resolve_targets;

fn engine_and_data(
    config_path: &Path,
) -> (DomainLuaEngine, datamile_lua_domain::EvaluatedDomainData) {
    let engine = DomainLuaEngine::new().expect("engine");
    engine
        .load_extension("media.lua", MEDIA_LUA)
        .expect("media.lua");
    engine
        .load_extension("compile_root.lua", COMPILE_ROOT_LUA)
        .expect("compile_root.lua");
    register_native_helpers(&engine.runtime).expect("natives");
    let evaluated = engine
        .evaluate_config(config_path)
        .expect("evaluate config");
    (engine, evaluated)
}

fn engine_for(config_path: &Path) -> DomainLuaEngine {
    engine_and_data(config_path).0
}

/// Validates a schema with no logic or action declarations attached.
fn validate_schema_alone(media: &MediaManifest) -> anyhow::Result<()> {
    validate_media_schema(media, &LogicManifest::default(), &ActionDeclarations::new())
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, content).expect("write");
}

/// Album DSL used by most tests. `external` is read from outside the root so
/// the dependency graph has something to trigger on.
fn album_dsl(lib: &Path, cache: &Path, state_dir: &Path, external: &Path) -> String {
    format!(
        r#"
d.config({{
    storage = {{ cache = "{cache}", state = "{state}" }}
}})

local track = d.media.node("track", {{
    select = function(ctx)
        return d.fs.files(ctx.path, {{ ext = "flac", paths_only = true }})
    end,
}})
track.tag("title", function(ctx) return ctx.location.path end)
track.tag("tracknumber", function(ctx)
    return d.str.leading_int(ctx.location.path)
end)
track.object("info", function(_ctx)
    return {{ duration_milliseconds = 60000, duration_formatted = d.fmt.duration(60000) }}
end)

local bonus = d.media.node("bonus", {{
    select = function(_ctx) return {{ {{ title = "Virtual" }} }} end,
}})
bonus.tag("title", function(ctx) return ctx.item.title end)

local album = d.media.root("album", {{
    init = "{lib}",
    select = function(dir) return d.fs.roots(dir, "album.toml") end,
    id = function(ctx) return ctx.tags.albumartist .. " - " .. ctx.tags.album end,
    nodes = {{ "track", "bonus" }},
}})
album.tag("album", function(ctx)
    return d.fs.read_toml(ctx.path .. "/album.toml").album
end)
album.tag("albumartist", function(ctx)
    return d.fs.read_toml(ctx.path .. "/album.toml").albumartist
end)
album.tag("genre", function(ctx)
    local ext = d.fs.read_toml("{external}/genre.toml")
    return ext and ext.genre
end)
album.object("info", function(_ctx) return {{ owned = true }} end)
album.image("main", function(ctx)
    return {{
        fit = {{ "crop", 32, 32 }},
        filter = "lanczos",
        sources = {{ d.fs.first(ctx.path, {{ "cover.png" }}) }}
    }}
end)
"#,
        cache = cache.display(),
        state = state_dir.display(),
        lib = lib.display(),
        external = external.display(),
    )
}

fn make_album(lib: &Path, name: &str, tracks: &[&str]) {
    let dir = lib.join(name);
    write(
        &dir.join("album.toml"),
        &format!("album = \"{name}\"\nalbumartist = \"Artist\"\n"),
    );
    for track in tracks {
        write(&dir.join(track), "audio");
    }
}

fn write_cover(dir: &Path) {
    let img = image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([10, 20, 30]));
    img.save_with_format(dir.join("cover.png"), image::ImageFormat::Png)
        .expect("cover");
}

struct Fixture {
    _root: TempDir,
    config: PathBuf,
    lib: PathBuf,
    cache: PathBuf,
    state_dir: PathBuf,
    external: PathBuf,
}

fn fixture() -> Fixture {
    let root = TempDir::new().expect("temp");
    let config_dir = root.path().join("config");
    let lib = root.path().join("library");
    let cache = root.path().join("cache");
    let state_dir = root.path().join("state");
    let external = root.path().join("external");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::create_dir_all(&lib).expect("lib");
    std::fs::create_dir_all(&external).expect("external");
    write(&external.join("genre.toml"), "genre = \"Rock\"\n");

    let config = config_dir.join("init.lua");
    write(&config, &album_dsl(&lib, &cache, &state_dir, &external));

    Fixture {
        _root: root,
        config,
        lib,
        cache,
        state_dir,
        external,
    }
}

fn media_state(f: &Fixture) -> (MediaState, LogicManifest) {
    let (engine, evaluated) = engine_and_data(&f.config);
    let media = read_media_manifest(&engine).expect("media manifest");
    validate_media_schema(&media, &evaluated.manifest, &evaluated.actions)
        .expect("schema");
    let roots = discover_roots(&engine, &media, f.config.parent().expect("dir"))
        .expect("discovery")
        .roots;

    let state = MediaState {
        config_path: f.config.clone(),
        config_dir: f.config.parent().expect("dir").to_path_buf(),
        cache_root: f.cache.clone(),
        state_root: f.state_dir.clone(),
        media,
        logic_manifest: evaluated.manifest.clone(),
        roots,
        dependencies: vec![f.config.clone()],
        actions: HashMap::new(),
        interfaces: evaluated.interfaces,
        mpd: datamile_mpd::MpdConfig {
            music_directory: Some(f.lib.to_string_lossy().to_string()),
            ..datamile_mpd::MpdConfig::default()
        },
        compiler_jobs: None,
        port: datamile_server::DEFAULT_PORT,
    };

    (state, evaluated.manifest)
}

fn lock_path(lib: &Path, album: &str) -> PathBuf {
    lib.join(album).join("album.lock.json")
}

fn read_lock(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("lock")).expect("json")
}

// ---------------------------------------------------------------- schema ----

#[test]
fn schema_rejects_missing_init_and_functions() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("broken", {})
        "#,
    );
    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_schema_alone(&media).expect_err("missing init must fail");
    assert!(err.to_string().contains("init"), "unexpected: {err}");
}

#[test]
fn schema_rejects_missing_select_and_id() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("broken", { init = "/tmp" })
        "#,
    );
    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_schema_alone(&media).expect_err("missing select/id must fail");
    assert!(
        err.to_string().contains("select") || err.to_string().contains("id"),
        "unexpected: {err}"
    );
}

#[test]
fn schema_rejects_unknown_logic_media() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("album", {
            init = "/tmp",
            select = function() return {} end,
            id = function() return "x" end
        })
        d.library("movies", { media = "movie", match = function() return true end })
        "#,
    );
    let (engine, evaluated) = engine_and_data(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_media_schema(&media, &evaluated.manifest, &evaluated.actions)
        .expect_err("unknown media must fail");
    assert!(err.to_string().contains("movie"), "unexpected: {err}");
}

/// An action scoped to a media type no `d.media.root` declares would never
/// resolve, so the schema refuses it before discovery runs.
#[test]
fn schema_rejects_an_action_scoped_to_unknown_media() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("album", {
            init = "/tmp",
            select = function() return {} end,
            id = function() return "x" end
        })
        d.action("open_book", { media = "book", run = function(_ctx) end })
        "#,
    );
    let (engine, evaluated) = engine_and_data(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_media_schema(&media, &evaluated.manifest, &evaluated.actions)
        .expect_err("unknown action media must fail");

    let message = err.to_string();
    assert!(
        message.contains("declares unknown media type"),
        "unexpected: {message}"
    );
    assert!(message.contains("book"), "unexpected: {message}");
}

#[test]
fn schema_rejects_duplicate_root_names() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("album", {
            init = "/tmp",
            select = function() return {} end,
            id = function() return "x" end
        })
        d.media.root("album", {
            init = "/tmp",
            select = function() return {} end,
            id = function() return "x" end
        })
        "#,
    );
    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_schema_alone(&media).expect_err("duplicate names must fail");
    assert!(err.to_string().contains("duplicate"), "unexpected: {err}");
}

/// One directory may be compiled by several media types at once; each type
/// keeps its own root entity there.
#[test]
fn discovery_allows_two_types_in_one_directory() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(&lib.join("A/album.toml"), "album = \"A\"\n");
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "a" end
            }})
            d.media.root("book", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "b" end
            }})
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");
    let discovery = discover_roots(&engine, &media, root.path()).expect("shared dir ok");

    assert_eq!(discovery.roots.len(), 2, "both media types keep the dir");
    assert_eq!(discovery.roots[0].media_type, "album");
    assert_eq!(discovery.roots[1].media_type, "book");
    assert_eq!(discovery.roots[0].dir, discovery.roots[1].dir);
}

/// Nested dirs are allowed across media types: a film's soundtrack folder can
/// be its own album.
#[test]
fn discovery_allows_cross_type_nesting() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(&lib.join("Film/movie.toml"), "title = \"Film\"\n");
    write(
        &lib.join("Film/soundtrack/album.toml"),
        "album = \"Soundtrack\"\n",
    );
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            d.media.root("movie", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "movie.toml") end,
                id = function() return "m" end
            }})
            d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "a" end
            }})
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");
    let discovery = discover_roots(&engine, &media, root.path()).expect("nesting ok");

    assert_eq!(discovery.roots.len(), 2);
    assert_eq!(discovery.roots[0].media_type, "movie");
    assert_eq!(discovery.roots[1].media_type, "album");
}

// ------------------------------------------------------------ compilation ----

#[allow(
    clippy::cognitive_complexity,
    reason = "one lock asserts every node kind, image field and tag it produced"
)]
#[test]
fn compile_emits_physical_and_virtual_nodes_and_images() {
    let f = fixture();
    make_album(&f.lib, "A", &["02.flac", "01.flac"]);
    write_cover(&f.lib.join("A"));

    let engine = engine_for(&f.config);
    let root = f.lib.join("A");
    let compiled =
        compile_root(&engine, "album", &root, &f.cache, None).expect("compile");
    let lock: Value = serde_json::from_str(&compiled.lock_json).expect("json");
    let album = &lock["album"];

    assert_eq!(album["id"], json!("Artist - A"));
    assert_eq!(album["location"]["kind"], json!("dir"));
    assert_eq!(
        album["location"]["path"],
        json!(root.canonicalize().expect("canon").to_string_lossy())
    );
    assert!(
        album["location"].get("mtime").is_none(),
        "dirs carry no mtime"
    );

    let tracks = album["nodes"]["track"].as_array().expect("tracks");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["location"]["kind"], json!("file"));
    assert_eq!(tracks[0]["location"]["path"], json!("01.flac"));
    assert!(tracks[0]["location"]["byte_size"].as_u64().unwrap_or(0) > 0);
    assert!(tracks[0]["location"]["mtime"].as_u64().unwrap_or(0) > 0);

    let bonus = &album["nodes"]["bonus"][0];
    assert!(
        bonus.get("location").is_none(),
        "virtual node has no location"
    );
    assert_eq!(bonus["tags"]["title"], json!("Virtual"));

    let image = &album["images"]["main"][0];
    assert!(image["source"]["file"].get("hash").is_none());
    assert_eq!(image["source"]["file"]["path"], json!("cover.png"));
    assert!(image["address"].as_str().is_some_and(|a| a.len() == 16));

    assert!(compiled.modified, "first compile writes the lock");
    assert!(lock_path(&f.lib, "A").exists());
}

/// One compiled album and the schema needed to read its lock back.
fn compiled_album() -> (Fixture, Value, crate::schema::MediaManifest) {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);

    let engine = engine_for(&f.config);
    let compiled = compile_root(&engine, "album", &f.lib.join("A"), &f.cache, None)
        .expect("compile");
    let lock = serde_json::from_str(&compiled.lock_json).expect("json");
    let media = read_media_manifest(&engine).expect("manifest");

    (f, lock, media)
}

/// The lock decides the file, the config only the base: the URI is the entity
/// location joined with the node location, minus `music_directory`.
#[test]
fn playable_uri_is_the_entity_location_relative_to_the_music_directory() {
    let (f, lock, media) = compiled_album();
    let schema = media.roots.get("album").expect("album schema");
    let base = f.lib.canonicalize().expect("canon");
    let dir = lock["album"]["location"]["path"]
        .as_str()
        .expect("entity dir");
    let node = lock["album"]["nodes"]["track"][0]["location"]["path"]
        .as_str()
        .expect("track path");

    let parsed =
        crate::ingest::parse_lock("album", &lock.to_string(), schema, Some(&base))
            .expect("parse")
            .expect("entity");

    assert_eq!(parsed.entity.path, PathBuf::from(dir));
    assert_eq!(
        parsed.playables[0].path,
        Path::new(dir)
            .strip_prefix(&base)
            .expect("entity under the base")
            .join(node)
            .to_string_lossy()
    );
}

/// The entity dict carries the entity dir so a client can act on an entity
/// without parsing its lock, matching `GET /api/media/{type}`.
#[test]
fn entity_dict_includes_the_entity_dir() {
    let (f, lock, media) = compiled_album();
    let schema = media.roots.get("album").expect("album schema");

    let parsed = crate::ingest::parse_lock(
        "album",
        &lock.to_string(),
        schema,
        Some(&f.lib.canonicalize().expect("canon")),
    )
    .expect("parse")
    .expect("entity");

    let dict: Value =
        serde_json::from_str(parsed.entity.dict_raw.get()).expect("dict json");
    assert_eq!(dict["path"], json!(parsed.entity.path.to_string_lossy()));
}

/// A file outside `music_directory` keeps its absolute path. Nothing here tries
/// to guess a base: MPD refuses the URI and the refusal lands in the log.
#[test]
fn playable_outside_the_music_directory_stays_absolute() {
    let (f, mut lock, media) = compiled_album();
    let schema = media.roots.get("album").expect("album schema");
    let outside = "/elsewhere/02.flac";
    lock["album"]["nodes"]["track"][0]["location"]["path"] = json!(outside);

    let parsed = crate::ingest::parse_lock(
        "album",
        &lock.to_string(),
        schema,
        Some(&f.lib.canonicalize().expect("canon")),
    )
    .expect("parse")
    .expect("entity");

    assert_eq!(parsed.playables[0].path, outside);
}

/// A lock without a dir location cannot be indexed: its paths would be guesses.
#[test]
fn lock_without_a_dir_location_is_refused() {
    let (_f, _lock, media) = compiled_album();
    let schema = media.roots.get("album").expect("album schema");

    let result =
        crate::ingest::parse_lock("album", r#"{"album":{"id":"x"}}"#, schema, None);

    assert!(result.is_err(), "a lock with no location must not index");
}

/// Lock keys are pinned, not alphabetical: primitives lead in a fixed order and
/// every other key sorts after them.
#[test]
fn lock_orders_root_and_node_primitives_first() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    write_cover(&f.lib.join("A"));

    let engine = engine_for(&f.config);
    let compiled = compile_root(&engine, "album", &f.lib.join("A"), &f.cache, None)
        .expect("compile");
    let lock: Value = serde_json::from_str(&compiled.lock_json).expect("json");

    let keys = |value: &Value| -> Vec<String> {
        value.as_object().expect("object").keys().cloned().collect()
    };

    assert_eq!(
        keys(&lock["album"]),
        ["id", "location", "tags", "objects", "images", "nodes"]
    );
    assert_eq!(
        keys(&lock["album"]["nodes"]["track"][0]),
        ["location", "tags", "objects"]
    );
}

/// `ctx` body hooks run before tags/id and expose parent state to children.
#[test]
fn ctx_hooks_populate_the_compilation_context() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(&lib.join("A/album.toml"), "album = \"A\"\n");
    write(&lib.join("A/01.flac"), "audio");
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            local track = d.media.node("track", {{
                ctx = function(ctx)
                    ctx.marker = ctx.parent.marker .. "-track"
                end,
                select = function(ctx)
                    return d.fs.files(ctx.path, {{ ext = "flac", paths_only = true }})
                end,
            }})
            track.tag("title", function(ctx) return ctx.marker end)

            local album = d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                ctx = function(ctx) ctx.marker = "album" end,
                id = function(ctx) return ctx.marker end,
                nodes = {{ "track" }},
            }})
            album.tag("album", function(ctx) return ctx.marker end)
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let compiled = compile_root(
        &engine,
        "album",
        &lib.join("A"),
        &root.path().join("cache"),
        None,
    )
    .expect("compile");
    let lock: Value = serde_json::from_str(&compiled.lock_json).expect("json");

    assert_eq!(lock["album"]["id"], json!("album"));
    assert_eq!(lock["album"]["tags"]["album"], json!("album"));
    assert_eq!(
        lock["album"]["nodes"]["track"][0]["tags"]["title"],
        json!("album-track")
    );
}

#[test]
fn second_compile_without_changes_does_not_rewrite_the_lock() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);

    let engine = engine_for(&f.config);
    let root = f.lib.join("A");
    let first = compile_root(&engine, "album", &root, &f.cache, None).expect("compile");
    assert!(first.modified);

    let second = compile_root(&engine, "album", &root, &f.cache, None).expect("compile");
    assert!(!second.modified, "identical bytes must not be rewritten");
    assert_eq!(first.lock_json, second.lock_json);
}

#[test]
fn cyclic_objects_fail_the_compile() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(
        &lib.join("A/album.toml"),
        "album = \"A\"\nalbumartist = \"Artist\"\n",
    );
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            local album = d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "a" end,
            }})
            album.object("a", function(ctx) return ctx.objects.b end)
            album.object("b", function(ctx) return ctx.objects.a end)
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let err = compile_root(
        &engine,
        "album",
        &lib.join("A"),
        &root.path().join("cache"),
        None,
    )
    .expect_err("cycle must fail");
    assert!(format!("{err:#}").contains("cyclic"), "unexpected: {err:#}");
}

/// A lock is never an input: a hook that reads `album.lock.json` fails the
/// compile instead of feeding one lock into the next.
#[test]
fn compile_refuses_to_read_a_lock_file() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(
        &lib.join("A/album.toml"),
        "album = \"A\"\nalbumartist = \"Artist\"\n",
    );
    write(&lib.join("A/album.lock.json"), "{}");
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            local album = d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "a" end,
            }})
            album.tag("leak", function(ctx)
                return d.fs.read_json(ctx.path .. "/album.lock.json")
            end)
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let err = compile_root(
        &engine,
        "album",
        &lib.join("A"),
        &root.path().join("cache"),
        None,
    )
    .expect_err("a lock read must fail the compile");
    assert!(
        format!("{err:#}").contains("a lock is never an input"),
        "unexpected: {err:#}"
    );

    // The failed compile released its guard: the same read works outside it.
    let text: String = engine
        .runtime
        .lua
        .load(format!(
            "return d.fs.read('{}')",
            lib.join("A/album.lock.json").display()
        ))
        .eval()
        .expect("read after the failed compile");
    assert_eq!(text, "{}");
}

#[test]
fn invalid_image_fit_fails_the_compile() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(
        &lib.join("A/album.toml"),
        "album = \"A\"\nalbumartist = \"Artist\"\n",
    );
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            local album = d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function() return "a" end,
            }})
            album.image("main", function()
                return {{ fit = {{ "crop", 0, 0 }}, filter = "lanczos" }}
            end)
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let err = compile_root(
        &engine,
        "album",
        &lib.join("A"),
        &root.path().join("cache"),
        None,
    )
    .expect_err("bad fit must fail");
    assert!(
        format!("{err:#}").contains("image 'main'"),
        "unexpected: {err:#}"
    );
}

#[test]
fn multi_level_tree_nests_seasons_and_episodes() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(&lib.join("Show/show.toml"), "show = \"Show\"\n");
    write(&lib.join("Show/Season 1/E01.mkv"), "video");
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            local episode = d.media.node("episode", {{
                select = function(ctx)
                    return d.fs.files(ctx.path, {{ ext = "mkv", paths_only = true }})
                end,
            }})

            local season = d.media.node("season", {{
                select = function(ctx)
                    return d.fs.dirs(ctx.path, {{ paths_only = true }})
                end,
                nodes = {{ "episode" }},
            }})
            season.tag("name", function(ctx) return ctx.location.path end)

            local show = d.media.root("show", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "show.toml") end,
                id = function(ctx) return ctx.path:match("[^/]+$") end,
                nodes = {{ "season" }},
            }})
            show.tag("show", function(ctx) return d.fs.basename(ctx.path) end)
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let compiled = compile_root(
        &engine,
        "show",
        &lib.join("Show"),
        &root.path().join("cache"),
        None,
    )
    .expect("compile");
    let lock: Value = serde_json::from_str(&compiled.lock_json).expect("json");
    let season = &lock["show"]["nodes"]["season"][0];
    assert_eq!(season["location"]["kind"], json!("dir"));
    assert!(
        season["location"].get("mtime").is_none(),
        "dirs carry no mtime"
    );
    assert_eq!(
        season["nodes"]["episode"][0]["location"]["path"],
        json!("Season 1/E01.mkv")
    );
}

#[test]
fn read_toml_is_memoized_within_one_entity() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);

    let engine = engine_for(&f.config);
    let compiled = compile_root(&engine, "album", &f.lib.join("A"), &f.cache, None)
        .expect("compile");
    let lock: Value = serde_json::from_str(&compiled.lock_json).expect("json");
    assert_eq!(lock["album"]["tags"]["album"], json!("A"));

    // The tag read is cached for the entity; editing the manifest after the
    // compile must not leak into the lock produced above.
    write(
        &f.lib.join("A/album.toml"),
        "album = \"Changed\"\nalbumartist = \"Artist\"\n",
    );
    let again = compile_root(&engine, "album", &f.lib.join("A"), &f.cache, None)
        .expect("second compile");
    let lock: Value = serde_json::from_str(&again.lock_json).expect("json");
    assert_eq!(lock["album"]["tags"]["album"], json!("Changed"));
}

// ------------------------------------------------------------ hot reload ----

struct Server {
    state: Arc<ServerState>,
    backend: Arc<MediaEngineBackend>,
    _fixture: Fixture,
}

async fn make_server() -> Server {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    // A file the schema never reads, used as a hot-reload event source.
    write(&f.lib.join("A/notes.txt"), "first");
    let (state, manifest) = media_state(&f);

    let backend = Arc::new(MediaEngineBackend::new());
    *backend.shared().state.write().expect("state") = Some(state.clone());

    let (mpd_tx, _mpd_rx) = tokio::sync::mpsc::channel(1);
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(64);

    let server = Arc::new(ServerState {
        logic: Arc::new(tokio::sync::RwLock::new(LogicEngine::new(
            f.config.clone(),
            manifest,
        ))),
        ui_state: tokio::sync::RwLock::new(json!({})),
        tx: broadcast_tx,
        config: tokio::sync::RwLock::new(ServerConfig {
            watch_roots: state.init_dirs(),
            mpd: state.mpd.clone(),
            cache_root: f.cache.clone(),
            state_root: f.state_dir.clone(),
            resolved_dependencies: state.dependencies.clone(),
            interfaces: HashMap::new(),
            actions: HashMap::new(),
            config_dir: f.config.parent().expect("dir").to_path_buf(),
            config_path: f.config.clone(),
            compiler_jobs: None,
            port: datamile_server::DEFAULT_PORT,
        }),
        mpd_engine: MpdEngine::new(mpd_tx),
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: tokio::sync::RwLock::new(String::new()),
        reload_error: tokio::sync::RwLock::new(None),
        backend: backend.clone(),
    });

    let shared = backend.shared();
    run_pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
        None,
        &LogSink,
    )
    .expect("initial compile");

    let mut logic = server.logic.write().await;
    backend.scan_library(&mut logic);
    drop(logic);

    Server {
        state: server,
        backend,
        _fixture: f,
    }
}

async fn handle(server: &Server, paths: &[PathBuf]) -> Vec<Value> {
    let mut rx = server.state.tx.subscribe();
    server.backend.handle_fs_events(paths, &server.state).await;

    let mut messages = Vec::new();
    while let Ok(message) = rx.try_recv() {
        if let Ok(value) = serde_json::from_str::<Value>(&message) {
            messages.push(value);
        }
    }
    messages
}

fn media_updated(messages: &[Value]) -> Option<&Value> {
    messages
        .iter()
        .find(|message| message["type"] == json!("MEDIA_UPDATED"))
}

#[tokio::test]
async fn nested_edit_recompiles_the_ancestor_root() {
    let server = make_server().await;
    let track = server._fixture.lib.join("A/01.flac");
    write(&track, "changed audio");

    let messages = handle(&server, &[track]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert_eq!(update["media_type"], json!("album"));
    assert!(update["updated"].get("Artist - A").is_some());

    let lock = read_lock(&lock_path(&server._fixture.lib, "A"));
    assert!(
        lock["album"]["nodes"]["track"][0]["location"]["byte_size"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
}

/// A pass that changes nothing rewrites the state file with identical bytes.
#[tokio::test]
async fn state_file_is_byte_stable_across_passes() {
    let server = make_server().await;
    let shared = server.backend.shared();

    let state_file = {
        let guard = shared.state.read().expect("state");
        let state = guard.as_ref().expect("state");
        let path = state_path(
            &state.cache_root,
            &state.config_path,
            "album",
            &state.init_dirs()[0],
        );
        drop(guard);
        path
    };
    let before = std::fs::read(&state_file).expect("state file");

    run_pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
        None,
        &LogSink,
    )
    .expect("second pass");

    assert_eq!(std::fs::read(&state_file).expect("state file"), before);
}

#[tokio::test]
async fn unrelated_edit_does_not_broadcast_an_update() {
    let server = make_server().await;
    let notes = server._fixture.lib.join("A/notes.txt");
    write(&notes, "nothing the schema reads");

    let messages = handle(&server, &[notes]).await;
    assert!(media_updated(&messages).is_none(), "no bogus update");
}

/// A vanished marker drops the entity and its state entry, never its lock.
#[tokio::test]
async fn deleting_the_marker_removes_the_root_and_keeps_its_lock() {
    let server = make_server().await;
    let marker = server._fixture.lib.join("A/album.toml");
    std::fs::remove_file(&marker).expect("remove marker");

    let messages = handle(&server, &[marker]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert_eq!(update["removed"], json!(["Artist - A"]));
    assert!(lock_path(&server._fixture.lib, "A").exists());

    assert!(
        server
            .state
            .logic
            .read()
            .await
            .entity_record("album", "Artist - A")
            .is_none()
    );
}

#[tokio::test]
async fn deleting_the_whole_root_removes_the_entity() {
    let server = make_server().await;
    let dir = server._fixture.lib.join("A");
    std::fs::remove_dir_all(&dir).expect("remove root");

    let messages = handle(&server, &[dir]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert_eq!(update["removed"], json!(["Artist - A"]));
}

#[tokio::test]
async fn pasted_album_is_discovered_and_added() {
    let server = make_server().await;
    make_album(&server._fixture.lib, "B", &["01.flac"]);
    let new_dir = server._fixture.lib.join("B");

    let messages = handle(&server, &[new_dir]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert!(update["updated"].get("Artist - B").is_some());
    assert!(lock_path(&server._fixture.lib, "B").exists());
}

#[tokio::test]
async fn self_written_lock_event_is_ignored() {
    let server = make_server().await;
    let lock = lock_path(&server._fixture.lib, "A");
    server
        .backend
        .shared()
        .active_writes
        .lock()
        .expect("writes")
        .insert(lock.clone());

    let messages = handle(&server, &[lock]).await;
    assert!(messages.is_empty(), "a self-write must not recompile");
}

#[tokio::test]
async fn config_change_reloads_the_logic_manifest() {
    let server = make_server().await;
    let mut init = album_dsl(
        &server._fixture.lib,
        &server._fixture.cache,
        &server._fixture.state_dir,
        &server._fixture.external,
    );
    // The match reads a tag, so a reload that hands it the lock wrapper
    // instead of the entity leaves the library empty.
    init.push_str(
        "\nd.library('all', { media = 'album', match = function(a) return a.tags.album ~= nil end })\n",
    );
    write(&server._fixture.config, &init);

    let messages = handle(&server, std::slice::from_ref(&server._fixture.config)).await;
    let update = messages
        .iter()
        .find(|message| message["type"] == json!("LOGIC_UPDATE"))
        .expect("LOGIC_UPDATE");
    assert!(update["manifest"]["libraries"].get("all").is_some());

    let logic = server.state.logic.read().await;
    assert!(logic.manifest.libraries.contains_key("all"));
    assert_eq!(
        logic
            .request_view(
                "album",
                "all",
                None,
                "default",
                None,
                None,
                SortOrder::Forward
            )
            .len(),
        1,
        "the reloaded index must keep the album in the library"
    );
}

/// A watcher edit inside a known root resolves to exactly that one entity key.
#[tokio::test]
async fn watcher_resolves_a_track_edit_to_one_entity_key() {
    let server = make_server().await;
    let track = server._fixture.lib.join("A/01.flac");
    write(&track, "changed audio");

    let scope = {
        let shared = server.backend.shared();
        let state = shared
            .state
            .read()
            .expect("state")
            .clone()
            .expect("media state");
        let logic = server.state.logic.read().await;
        let engine = crate::state::open_engine(&state).expect("engine");
        resolve_targets(
            &shared,
            &state,
            &engine,
            &logic,
            std::slice::from_ref(&track),
        )
    };

    let dir = server._fixture.lib.join("A");
    let dir = dir.canonicalize().expect("canonical dir");
    let expected = EntityKey {
        media_type: "album".to_string(),
        dir,
    };

    assert_eq!(
        scope,
        Some(UpdateScope::Entities(BTreeSet::from([expected])))
    );
}

#[tokio::test]
async fn external_dependency_change_recompiles_only_its_root() {
    let server = make_server().await;
    let external = server._fixture.external.join("genre.toml");
    write(&external, "genre = \"Jazz\"\n");

    let messages = handle(&server, &[external]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert!(update["updated"].get("Artist - A").is_some());

    let lock = read_lock(&lock_path(&server._fixture.lib, "A"));
    assert_eq!(lock["album"]["tags"]["genre"], json!("Jazz"));
}

// --------------------------------------------------- state and pass events ----

/// Collects every event a pass emits, for membership and count assertions.
#[derive(Default)]
struct Recorder {
    events: std::sync::Mutex<Vec<UpdateEvent>>,
}

impl UpdateSink for Recorder {
    fn event(&self, event: &UpdateEvent) {
        self.events.lock().expect("events").push(event.clone());
    }
}

fn pass(
    shared: &Arc<MediaShared>,
    scope: &UpdateScope,
    force: ForceMode,
) -> (PassResult, Vec<UpdateEvent>) {
    let recorder = Recorder::default();
    let outcome = run_pass(shared, scope, force, None, &recorder).expect("pass");
    (
        outcome.result,
        recorder.events.into_inner().expect("events"),
    )
}

fn shared_for(f: &Fixture) -> Arc<MediaShared> {
    let (state, _) = media_state(f);
    let shared = Arc::new(MediaShared::default());
    *shared.state.write().expect("state") = Some(state);
    shared
}

fn state_file(f: &Fixture, media_type: &str, init: &Path) -> PathBuf {
    state_path(&f.cache, &f.config, media_type, init)
}

fn event_count(events: &[UpdateEvent], matches: impl Fn(&UpdateEvent) -> bool) -> usize {
    events.iter().filter(|event| matches(event)).count()
}

/// One state file per media root, and a pass emits `Started` first and
/// `Finished` last with counts that match the events in between.
#[test]
fn pass_emits_a_started_and_a_finished_event() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    assert!(matches!(events.first(), Some(UpdateEvent::Started { .. })));
    assert!(matches!(events.last(), Some(UpdateEvent::Finished { .. })));
    assert_eq!(result.discovered, 1);
    assert_eq!(result.added, 1);
    assert_eq!(
        event_count(&events, |event| matches!(event, UpdateEvent::Added { .. })),
        1
    );

    let file = state_file(&f, "album", &f.lib);
    assert!(file.exists(), "one state file per media root");
    let states = datamile_cache::load_state(&file).expect("state");
    assert!(states.contains_key(&f.lib.join("A").to_string_lossy().to_string()));
}

/// A root that reads a file outside itself is clean on the second pass: the
/// external entry lives in the same map as the walked files.
#[test]
fn external_file_is_clean_on_the_second_pass() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);

    let (_, first) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );
    assert_eq!(
        event_count(&first, |event| matches!(event, UpdateEvent::Added { .. })),
        1
    );

    let (result, second) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.failed, 0);
    assert!(matches!(
        second.as_slice(),
        [
            UpdateEvent::Started { dirty: 0, .. },
            UpdateEvent::Finished { .. }
        ]
    ));
}

/// Editing the external file recompiles its root exactly once.
#[test]
fn external_file_change_recompiles_exactly_once() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    write(&f.external.join("genre.toml"), "genre = \"Jazz\"\n");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.updated, 1);
    assert_eq!(
        event_count(&events, |event| matches!(
            event,
            UpdateEvent::Updated { .. }
        )),
        1
    );

    let (result, _) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.updated, 0, "a second pass must not recompile");
}

/// A removed root entity loses its state entry, is reported after the diff, and
/// keeps its lock.
#[test]
fn removed_root_loses_its_entry_and_keeps_its_lock() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    std::fs::remove_file(f.lib.join("A/album.toml")).expect("remove marker");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.removed, 1);

    let removed = events
        .iter()
        .position(|event| matches!(event, UpdateEvent::Removed { .. }))
        .expect("Removed");
    let finished = events
        .iter()
        .position(|event| matches!(event, UpdateEvent::Finished { .. }))
        .expect("Finished");
    assert!(removed < finished, "Removed comes before Finished");

    assert!(lock_path(&f.lib, "A").exists(), "locks are never deleted");

    let states =
        datamile_cache::load_state(&state_file(&f, "album", &f.lib)).expect("state");
    assert!(states.is_empty(), "the entry is dropped");
}

/// A root that comes back has a lock but no state entry, so it is `Added` and
/// its lock bytes are not rewritten.
#[test]
fn returning_root_is_added_and_its_lock_is_not_rewritten() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let lock = lock_path(&f.lib, "A");
    let before = std::fs::read(&lock).expect("lock");
    let touched = std::fs::metadata(&lock)
        .expect("lock meta")
        .modified()
        .expect("mtime");

    std::fs::remove_file(state_file(&f, "album", &f.lib)).expect("drop state");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.added, 1);
    assert_eq!(
        event_count(&events, |event| matches!(event, UpdateEvent::Added { .. })),
        1
    );
    assert_eq!(std::fs::read(&lock).expect("lock"), before);
    assert_eq!(
        std::fs::metadata(&lock)
            .expect("lock meta")
            .modified()
            .expect("mtime"),
        touched
    );
}

/// A scoped pass compiles only its path but still reports global removals.
#[test]
fn scoped_pass_reports_global_removals() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    std::fs::remove_dir_all(f.lib.join("B")).expect("remove B");
    write(&f.lib.join("A/01.flac"), "changed audio");

    let mut paths = std::collections::HashSet::new();
    paths.insert(f.lib.join("A"));
    let scope = UpdateScope::Paths {
        paths,
        media_type: None,
    };

    let (result, events) = pass(&shared, &scope, ForceMode::Preserve);

    assert_eq!(result.updated, 1, "A is in scope and changed");
    assert_eq!(result.removed, 1, "B is out of scope but still removed");
    assert_eq!(
        event_count(&events, |event| matches!(
            event,
            UpdateEvent::Removed { .. }
        )),
        1
    );
}

/// A compile failure is reported, counted, and leaves the state entry as it was.
#[test]
fn failed_compile_keeps_the_state_entry() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let before = std::fs::read(state_file(&f, "album", &f.lib)).expect("state file");

    write(&f.lib.join("A/album.toml"), "this is not toml [[[");
    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    assert_eq!(result.failed, 1);
    assert_eq!(
        event_count(&events, |event| matches!(event, UpdateEvent::Failed { .. })),
        1
    );
    assert_eq!(
        std::fs::read(state_file(&f, "album", &f.lib)).expect("state file"),
        before
    );
}

/// The pass records the lock stat of every compiled root, so a deleted or
/// hand-edited lock recompiles the entity even with unchanged sources.
#[test]
fn lock_stat_is_recorded_and_a_missing_lock_recompiles() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let key = f.lib.join("A").to_string_lossy().to_string();
    let entry = datamile_cache::load_state(&state_file(&f, "album", &f.lib))
        .expect("state")
        .remove(&key)
        .expect("entry");
    assert!(entry.lock_stat.is_some(), "the lock stat is recorded");
    assert!(
        !entry.files.keys().any(|path| path.ends_with(".lock.json")),
        "a lock never enters the file map"
    );

    std::fs::remove_file(lock_path(&f.lib, "A")).expect("delete the lock");

    let (result, _) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.updated, 1, "a missing lock makes the entity dirty");
    assert!(lock_path(&f.lib, "A").exists(), "the pass rewrote the lock");

    let (result, _) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.updated, 0, "the rewritten lock is clean");
    assert_eq!(result.unchanged, 1);
}

/// A dirty root that compiles to identical bytes still refreshes `lock_stat`,
/// so the next pass is clean instead of recompiling forever.
#[test]
fn recompile_to_identical_bytes_refreshes_the_lock_stat() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    // A stale stat, as if the lock had been rewritten with the same bytes.
    let path = state_file(&f, "album", &f.lib);
    let mut states = datamile_cache::load_state(&path).expect("state");
    let key = f.lib.join("A").to_string_lossy().to_string();
    states.get_mut(&key).expect("entry").lock_stat =
        Some(datamile_cache::FileStat::default());
    datamile_cache::save_state(&states, &path).expect("save");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.failed, 0);
    assert_eq!(
        event_count(&events, |event| matches!(
            event,
            UpdateEvent::Updated { .. }
        )),
        0,
        "identical bytes are not an update"
    );

    let (result, _) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(
        result.unchanged, 1,
        "the refreshed stat makes the root clean"
    );
    assert_eq!(result.updated, 0);
}

/// A read-only entity dir fails only that entity, names the path, and keeps its
/// previous state entry so the next pass retries it.
#[test]
fn read_only_entity_fails_alone_and_keeps_its_state_entry() {
    use std::os::unix::fs::PermissionsExt;

    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let state_file_path = state_file(&f, "album", &f.lib);
    let before = datamile_cache::load_state(&state_file_path).expect("state");

    for name in ["A", "B"] {
        write(
            &f.lib.join(name).join("album.toml"),
            &format!("album = \"{name}\"\nalbumartist = \"Changed\"\n"),
        );
    }
    std::fs::set_permissions(f.lib.join("A"), std::fs::Permissions::from_mode(0o500))
        .expect("chmod");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    std::fs::set_permissions(f.lib.join("A"), std::fs::Permissions::from_mode(0o700))
        .expect("chmod back");

    assert_eq!(result.failed, 1);
    assert_eq!(result.updated, 1, "the sibling still recompiles");

    let message = events
        .iter()
        .find_map(|event| match event {
            UpdateEvent::Failed { dir, error, .. } => {
                (dir.as_deref() == Some(f.lib.join("A").as_path())).then(|| error.clone())
            }
            _ => None,
        })
        .expect("the read-only entity failed");
    assert!(message.contains("read-only path"), "unexpected: {message}");

    let after = datamile_cache::load_state(&state_file_path).expect("state");
    let key = f.lib.join("A").to_string_lossy().to_string();
    assert_eq!(
        after.get(&key),
        before.get(&key),
        "a failed entity keeps its previous entry"
    );
}

/// A corrupt state file is a failure without a dir; the pass still finishes.
#[test]
fn corrupt_state_file_fails_without_a_dir() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);

    let file = state_file(&f, "album", &f.lib);
    write(&file, "not json");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    assert_eq!(result.failed, 1);
    let failed = events
        .iter()
        .find_map(|event| match event {
            UpdateEvent::Failed { dir, .. } => Some(dir),
            _ => None,
        })
        .expect("Failed");
    assert!(failed.is_none(), "a media root failure carries no dir");
    assert!(matches!(events.last(), Some(UpdateEvent::Finished { .. })));
}

/// A scoped pass must not claim the config digest, or the next full pass would
/// trust it and leave every root the scoped pass skipped compiled by the old
/// config.
#[test]
fn scoped_pass_does_not_claim_the_config_digest() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    // A config edit that changes every lock, then a pass scoped to A only.
    let mut dsl = album_dsl(&f.lib, &f.cache, &f.state_dir, &f.external);
    dsl.push_str("\nalbum.tag(\"extra\", function(_ctx) return \"x\" end)\n");
    write(&f.config, &dsl);

    let mut paths = std::collections::HashSet::new();
    paths.insert(f.lib.join("A"));
    pass(
        &shared,
        &UpdateScope::Paths {
            paths,
            media_type: None,
        },
        ForceMode::Preserve,
    );

    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    let lock = read_lock(&lock_path(&f.lib, "B"));
    assert_eq!(
        lock["album"]["tags"]["extra"],
        json!("x"),
        "a config change must still force B after a scoped pass"
    );
}

/// One call records a whole batch, so a pass writes one state file per media
/// type instead of one per root entity.
#[test]
fn record_roots_writes_a_whole_batch() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    let state = shared
        .state
        .read()
        .expect("state")
        .clone()
        .expect("media state");

    let entries: Vec<_> = [f.lib.join("A"), f.lib.join("B")]
        .into_iter()
        .map(|dir| {
            let files = datamile_cache::walk_files(&dir);
            (dir, files, datamile_cache::FileStat::default())
        })
        .collect();

    shared
        .record_roots(&state, "album", entries)
        .expect("record the batch");

    let states =
        datamile_cache::load_state(&state_file(&f, "album", &f.lib)).expect("state");
    assert_eq!(states.len(), 2, "every entry of the batch is written");
}

/// A clean pass counts every verified root as unchanged, so the Finished counts
/// add up to `discovered` instead of all reading zero.
#[test]
fn clean_pass_counts_unchanged_roots() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    assert_eq!(result.discovered, 2);
    assert_eq!(
        result.unchanged, 2,
        "clean roots are unchanged, not missing"
    );

    let finished = events.last().expect("last event");
    let UpdateEvent::Finished {
        discovered,
        added,
        updated,
        unchanged,
        removed,
        failed,
    } = finished
    else {
        panic!("the last event must be Finished, got {finished:?}");
    };
    assert_eq!(*discovered, 2);
    assert_eq!(*unchanged, 2);
    assert_eq!(
        added + updated + unchanged + removed + failed,
        *discovered,
        "every discovered root is accounted for"
    );
}

/// A removal whose state write fails is reported `Failed`, never `Removed`: the
/// entry is still there, so the next pass must report the removal again.
#[test]
fn failed_state_removal_is_not_reported_as_removed() {
    use std::os::unix::fs::PermissionsExt;

    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    std::fs::remove_dir_all(f.lib.join("B")).expect("remove B");

    // A read-only state directory makes the atomic write fail while the read of
    // the existing state file still works.
    let state_dir = state_file(&f, "album", &f.lib);
    let state_dir = state_dir.parent().expect("parent").to_path_buf();
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o500))
        .expect("chmod");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700))
        .expect("chmod back");

    assert_eq!(result.failed, 1);
    assert_eq!(result.removed, 0);
    assert_eq!(
        event_count(&events, |event| matches!(event, UpdateEvent::Failed { .. })),
        1
    );
    assert_eq!(
        event_count(&events, |event| matches!(
            event,
            UpdateEvent::Removed { .. }
        )),
        0
    );
}

/// A path that matches no root entity is an error, not a clean pass.
#[test]
fn scoped_pass_with_no_matching_root_is_an_error() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let mut paths = std::collections::HashSet::new();
    paths.insert(f.lib.join("missing"));

    let Err(error) = run_pass(
        &shared,
        &UpdateScope::Paths {
            paths,
            media_type: None,
        },
        ForceMode::Preserve,
        None,
        &LogSink,
    ) else {
        panic!("a path that matches nothing must be an error");
    };

    assert!(
        format!("{error}").contains("no root entity"),
        "unexpected error: {error}"
    );
}

// ---------------------------------------------------------- update scope ----

fn key(media_type: &str, dir: &str) -> EntityKey {
    EntityKey {
        media_type: media_type.to_string(),
        dir: PathBuf::from(dir),
    }
}

fn keys(entries: &[(&str, &str)]) -> BTreeSet<EntityKey> {
    entries.iter().map(|(media, dir)| key(media, dir)).collect()
}

fn all(media_type: Option<&str>) -> UpdateScope {
    UpdateScope::All {
        media_type: media_type.map(str::to_string),
    }
}

fn paths(media_type: Option<&str>, entries: &[&str]) -> UpdateScope {
    UpdateScope::Paths {
        paths: entries.iter().map(PathBuf::from).collect(),
        media_type: media_type.map(str::to_string),
    }
}

fn scoped(entries: &[(&str, &str)]) -> UpdateScope {
    UpdateScope::Entities(keys(entries))
}

/// Coalesced requests keep the narrowest scope covering both, and widen to a
/// full pass as soon as the two disagree on a media type or a path.
#[test]
fn update_scope_merge_follows_the_matrix() {
    let album = "album";
    let movie = "movie";
    let cases = [
        // An existing full pass absorbs everything.
        (all(None), all(None), all(None)),
        (all(None), all(Some(album)), all(None)),
        (all(None), paths(Some(album), &["/x"]), all(None)),
        (all(None), scoped(&[(album, "/x")]), all(None)),
        // An incoming full pass absorbs every existing scope.
        (all(Some(album)), all(None), all(None)),
        (paths(Some(album), &["/x"]), all(None), all(None)),
        (scoped(&[(album, "/x")]), all(None), all(None)),
        // Same media type stays narrow, a different one widens.
        (all(Some(album)), all(Some(album)), all(Some(album))),
        (all(Some(album)), all(Some(movie)), all(None)),
        (
            all(Some(album)),
            paths(Some(album), &["/x"]),
            all(Some(album)),
        ),
        (all(Some(album)), paths(Some(movie), &["/x"]), all(None)),
        (all(Some(album)), paths(None, &["/x"]), all(None)),
        (all(Some(album)), scoped(&[(album, "/x")]), all(Some(album))),
        (all(Some(album)), scoped(&[(movie, "/x")]), all(None)),
        (
            paths(Some(album), &["/x"]),
            all(Some(album)),
            all(Some(album)),
        ),
        (paths(Some(album), &["/x"]), all(Some(movie)), all(None)),
        (
            paths(Some(album), &["/x"]),
            paths(Some(album), &["/y"]),
            paths(Some(album), &["/x", "/y"]),
        ),
        (
            paths(Some(album), &["/x"]),
            paths(Some(movie), &["/y"]),
            all(None),
        ),
        (
            paths(Some(album), &["/x"]),
            scoped(&[(album, "/y")]),
            all(Some(album)),
        ),
        (
            paths(Some(album), &["/x"]),
            scoped(&[(movie, "/y")]),
            all(None),
        ),
        (scoped(&[(album, "/x")]), all(Some(album)), all(Some(album))),
        (scoped(&[(album, "/x")]), all(Some(movie)), all(None)),
        (
            scoped(&[(album, "/x")]),
            paths(Some(album), &["/y"]),
            all(Some(album)),
        ),
        (
            scoped(&[(album, "/x")]),
            paths(Some(movie), &["/y"]),
            all(None),
        ),
        (
            scoped(&[(album, "/x")]),
            scoped(&[(album, "/y")]),
            scoped(&[(album, "/x"), (album, "/y")]),
        ),
    ];

    for (existing, incoming, expected) in cases {
        let mut merged = existing.clone();
        merged.merge(incoming.clone());
        assert_eq!(merged, expected, "{existing:?} + {incoming:?}");
    }
}

/// Explicit entity keys win over the path, so forcing one entity never
/// recompiles its sibling media types in the same directory.
#[test]
fn request_entities_take_precedence_over_path_and_media_type() {
    let request = UpdateRequest {
        path: Some("/media/A".to_string()),
        media_type: Some("movie".to_string()),
        entities: vec![key("album", "/media/A")],
        force: true,
        jobs: None,
    };

    assert_eq!(
        scope_for_request(&request),
        scoped(&[("album", "/media/A")])
    );
}

/// A vanished watcher target reconciles as a removal instead of failing the
/// pass on "no root entity matches the given path".
#[test]
fn entities_scope_reports_a_removed_target_as_a_removal() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    let shared = shared_for(&f);
    pass(&shared, &all(None), ForceMode::Force);

    let dir = f.lib.join("A").canonicalize().expect("canonical dir");
    std::fs::remove_dir_all(&dir).expect("remove root");

    let scope = UpdateScope::Entities(keys(&[("album", &dir.to_string_lossy())]));
    let (result, _) = pass(&shared, &scope, ForceMode::Preserve);

    assert_eq!(result.removed, 1);
    assert_eq!(result.failed, 0);
}

/// One external file read by two entities keeps both owners; dropping one owner
/// leaves the other in place.
#[test]
fn external_index_keeps_every_owner_of_a_shared_file() {
    let shared_file = PathBuf::from("/media/genre.toml");
    let album_a = key("album", "/media/A");
    let album_b = key("album", "/media/B");
    let mut index = ExternalIndex::new();

    index_entity(&mut index, &album_a, std::slice::from_ref(&shared_file));
    index_entity(&mut index, &album_b, std::slice::from_ref(&shared_file));
    assert_eq!(
        index[&shared_file],
        keys(&[("album", "/media/A"), ("album", "/media/B")])
    );

    unindex_entity(&mut index, &album_a);
    assert_eq!(index[&shared_file], keys(&[("album", "/media/B")]));
}

/// An `init` with no discovered entity is still watched, otherwise an album
/// pasted into an empty library is never seen.
#[test]
fn init_dirs_include_a_media_root_with_no_entities() {
    let f = fixture();
    let (state, _) = media_state(&f);

    assert!(state.roots.is_empty());
    assert_eq!(
        state.init_dirs(),
        vec![f.lib.canonicalize().expect("canonical init")]
    );
}

// --------------------------------------------------------- two media roots ----

struct TwoTypes {
    _root: TempDir,
    config: PathBuf,
    albums: PathBuf,
    shows: PathBuf,
    cache: PathBuf,
    state_dir: PathBuf,
}

fn two_type_dsl(albums: &Path, shows: &Path, cache: &Path, state_dir: &Path) -> String {
    format!(
        r#"
d.config({{
    storage = {{ cache = "{cache}", state = "{state}" }}
}})

local track = d.media.node("track", {{
    select = function(ctx)
        return d.fs.files(ctx.path, {{ ext = "flac", paths_only = true }})
    end,
}})
track.tag("title", function(ctx) return ctx.location.path end)

local album = d.media.root("album", {{
    init = "{albums}",
    select = function(dir) return d.fs.roots(dir, "album.toml") end,
    id = function(ctx) return ctx.tags.album end,
    nodes = {{ "track" }},
}})
album.tag("album", function(ctx)
    return d.fs.read_toml(ctx.path .. "/album.toml").album
end)

local episode = d.media.node("episode", {{
    select = function(ctx)
        return d.fs.files(ctx.path, {{ ext = "mkv", paths_only = true }})
    end,
}})
episode.tag("title", function(ctx) return ctx.location.path end)

local show = d.media.root("show", {{
    init = "{shows}",
    select = function(dir) return d.fs.roots(dir, "show.toml") end,
    id = function(ctx) return ctx.tags.show end,
    nodes = {{ "episode" }},
}})
show.tag("show", function(ctx)
    return d.fs.read_toml(ctx.path .. "/show.toml").show
end)
"#,
        cache = cache.display(),
        state = state_dir.display(),
        albums = albums.display(),
        shows = shows.display(),
    )
}

fn two_type_fixture() -> TwoTypes {
    let root = TempDir::new().expect("temp");
    let config_dir = root.path().join("config");
    let albums = root.path().join("albums");
    let shows = root.path().join("shows");
    let cache = root.path().join("cache");
    let state_dir = root.path().join("state");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::create_dir_all(&albums).expect("albums");
    std::fs::create_dir_all(&shows).expect("shows");

    let config = config_dir.join("init.lua");
    write(&config, &two_type_dsl(&albums, &shows, &cache, &state_dir));

    TwoTypes {
        _root: root,
        config,
        albums,
        shows,
        cache,
        state_dir,
    }
}

fn two_type_shared(f: &TwoTypes) -> Arc<MediaShared> {
    let (engine, evaluated) = engine_and_data(&f.config);
    let media = read_media_manifest(&engine).expect("media manifest");
    validate_media_schema(&media, &evaluated.manifest, &evaluated.actions)
        .expect("schema");
    let roots = discover_roots(&engine, &media, f.config.parent().expect("dir"))
        .expect("discovery")
        .roots;

    let state = MediaState {
        config_path: f.config.clone(),
        config_dir: f.config.parent().expect("dir").to_path_buf(),
        cache_root: f.cache.clone(),
        state_root: f.state_dir.clone(),
        media,
        logic_manifest: evaluated.manifest.clone(),
        roots,
        dependencies: vec![f.config.clone()],
        actions: HashMap::new(),
        interfaces: evaluated.interfaces,
        mpd: datamile_mpd::MpdConfig::default(),
        compiler_jobs: None,
        port: datamile_server::DEFAULT_PORT,
    };

    let shared = Arc::new(MediaShared::default());
    *shared.state.write().expect("state") = Some(state);
    shared
}

fn make_show(shows: &Path, name: &str) {
    write(
        &shows.join(name).join("show.toml"),
        &format!("show = \"{name}\"\n"),
    );
    write(&shows.join(name).join("E01.mkv"), "video");
}

/// Each media root owns its own state file, so a change under one `init` does
/// not invalidate the other.
#[test]
fn media_roots_with_different_inits_do_not_invalidate_each_other() {
    let f = two_type_fixture();
    make_album(&f.albums, "A", &["01.flac"]);
    make_show(&f.shows, "S");
    let shared = two_type_shared(&f);

    pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Force,
    );

    let album_state = state_path(&f.cache, &f.config, "album", &f.albums);
    let show_state = state_path(&f.cache, &f.config, "show", &f.shows);
    assert!(album_state.exists(), "one state file per media root");
    assert!(show_state.exists());
    assert_ne!(album_state, show_state);

    write(&f.albums.join("A/01.flac"), "changed audio");

    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );
    assert_eq!(result.updated, 1, "only the album root recompiles");
    assert_eq!(
        event_count(&events, |event| {
            matches!(
                event,
                UpdateEvent::Added { media_type, .. }
                    | UpdateEvent::Updated { media_type, .. }
                    | UpdateEvent::Unchanged { media_type, .. }
                    if media_type == "show"
            )
        }),
        0,
        "the show root is clean, so it emits no per-root event"
    );
}

/// A watcher-discovered root is recorded, so the next pass does not report it
/// as `Added` again.
#[tokio::test]
async fn watcher_discovered_root_is_clean_on_the_next_pass() {
    let server = make_server().await;
    make_album(&server._fixture.lib, "B", &["01.flac"]);
    let new_dir = server._fixture.lib.join("B");

    let _ = handle(&server, &[new_dir]).await;

    let shared = server.backend.shared();
    let (result, events) = pass(
        &shared,
        &UpdateScope::All { media_type: None },
        ForceMode::Preserve,
    );

    assert_eq!(result.failed, 0);
    assert_eq!(
        event_count(&events, |event| matches!(
            event,
            UpdateEvent::Added { .. } | UpdateEvent::Updated { .. }
        )),
        0,
        "the watcher already compiled and recorded B"
    );
}

/// The pass and the watcher write one state file through the same serialised
/// writer, so neither loses the other's entry.
#[test]
fn concurrent_state_writers_do_not_lose_entries() {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    make_album(&f.lib, "B", &["01.flac"]);
    let shared = shared_for(&f);

    let state = shared
        .state
        .read()
        .expect("state")
        .clone()
        .expect("media state");

    let dirs = [f.lib.join("A"), f.lib.join("B")];
    std::thread::scope(|scope| {
        for dir in &dirs {
            let shared = Arc::clone(&shared);
            let state = state.clone();
            scope.spawn(move || {
                let files = datamile_cache::walk_files(dir);
                let lock_stat = datamile_cache::FileStat::default();
                shared
                    .record_roots(&state, "album", vec![(dir.clone(), files, lock_stat)])
                    .expect("record");
            });
        }
    });

    let states =
        datamile_cache::load_state(&state_file(&f, "album", &f.lib)).expect("state");
    assert_eq!(states.len(), 2, "both writers' entries survive");
}

// ---------------------------------------------------------------- discovery ----

/// Root entities of one media type may not nest, so every file belongs to at
/// most one root entity of that type.
#[test]
fn discovery_rejects_nested_roots_within_one_media_type() {
    let root = TempDir::new().expect("temp");
    let lib = root.path().join("lib");
    write(&lib.join("album.toml"), "album = \"Outer\"\n");
    write(&lib.join("A/album.toml"), "album = \"Inner\"\n");
    let config = root.path().join("init.lua");
    write(
        &config,
        &format!(
            r#"
            d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                id = function(ctx) return ctx.tags.album end,
            }})
            "#,
            lib = lib.display()
        ),
    );

    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");
    let err =
        discover_roots(&engine, &media, root.path()).expect_err("nesting must fail");
    assert!(err.to_string().contains("nest"), "unexpected: {err}");
}

/// A media type name is used as a path segment, so the character set is
/// restricted.
#[test]
fn schema_rejects_unsafe_media_type_names() {
    let dir = TempDir::new().expect("temp");
    let config = dir.path().join("init.lua");
    write(
        &config,
        r#"
        d.media.root("Albums/../x", {
            init = "/tmp",
            select = function() return {} end,
            id = function() return "x" end
        })
        "#,
    );
    let engine = engine_for(&config);
    let media = read_media_manifest(&engine).expect("manifest");

    let err = validate_schema_alone(&media).expect_err("unsafe names must fail");
    assert!(err.to_string().contains("[a-z0-9_-]+"), "unexpected: {err}");
}
