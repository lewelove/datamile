//! Contract of the schema-driven media engine: DSL validation, discovery,
//! single-pass compilation and every hot-reload guard.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datamile_logic::LogicEngine;
use datamile_lua_domain::{DomainLuaEngine, LogicManifest};
use datamile_mpd::MpdEngine;
use datamile_server::{ImageTaskRegistry, MediaBackend, ServerConfig, ServerState};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::backend::MediaEngineBackend;
use crate::compiler::compile_root;
use crate::deps::UpdateScope;
use crate::lua_bridge::register_native_helpers;
use crate::schema::{discover_roots, read_media_manifest, validate_media_schema};
use crate::state::{COMPILE_ROOT_LUA, MEDIA_LUA, MediaState};
use crate::update::{ForceMode, Verbosity, run_pass};

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
    storage = {{ cache = "{cache}", state = "{state}" }},
    mpd = {{ music_directory = "{lib}" }}
}})

local track = d.media.node("track", {{
    playable = true,
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
    validate_media_schema(&media, &evaluated.manifest).expect("schema");
    let roots = discover_roots(&engine, &media, f.config.parent().expect("dir"))
        .expect("discovery");

    let state = MediaState {
        config_path: f.config.clone(),
        config_dir: f.config.parent().expect("dir").to_path_buf(),
        cache_root: f.cache.clone(),
        state_root: f.state_dir.clone(),
        media,
        logic_manifest: evaluated.manifest.clone(),
        roots,
        dependencies: vec![f.config.clone()],
        actions: evaluated.actions,
        interfaces: evaluated.interfaces,
        mpd: datamile_mpd::MpdConfig {
            music_directory: Some(f.lib.to_string_lossy().to_string()),
            ..datamile_mpd::MpdConfig::default()
        },
        compiler_jobs: None,
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

    let err = validate_media_schema(&media, &LogicManifest::default())
        .expect_err("missing init must fail");
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

    let err = validate_media_schema(&media, &LogicManifest::default())
        .expect_err("missing select/id must fail");
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

    let err = validate_media_schema(&media, &evaluated.manifest)
        .expect_err("unknown media must fail");
    assert!(err.to_string().contains("movie"), "unexpected: {err}");
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

    let err = validate_media_schema(&media, &LogicManifest::default())
        .expect_err("duplicate names must fail");
    assert!(err.to_string().contains("duplicate"), "unexpected: {err}");
}

#[test]
fn discovery_rejects_two_types_claiming_one_directory() {
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
    let err =
        discover_roots(&engine, &media, root.path()).expect_err("collision must fail");
    assert!(err.to_string().contains("claimed"), "unexpected: {err}");
}

// ------------------------------------------------------------ compilation ----

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
    assert_eq!(album["location"]["path"], json!("."));

    let tracks = album["nodes"]["track"].as_array().expect("tracks");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["location"]["kind"], json!("file"));
    assert_eq!(tracks[0]["location"]["path"], json!("01.flac"));
    assert!(tracks[0]["location"]["byte_size"].as_u64().unwrap_or(0) > 0);

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
            local tracks = d.media.node("tracks", {{
                ctx = function(ctx)
                    ctx.marker = ctx.parent.marker .. "-track"
                end,
                select = function(ctx)
                    return d.fs.files(ctx.path, {{ ext = "flac", paths_only = true }})
                end,
            }})
            tracks.tag("title", function(ctx) return ctx.marker end)

            local album = d.media.root("album", {{
                init = "{lib}",
                select = function(dir) return d.fs.roots(dir, "album.toml") end,
                ctx = function(ctx) ctx.marker = "album" end,
                id = function(ctx) return ctx.marker end,
                nodes = {{ "tracks" }},
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
        lock["album"]["nodes"]["tracks"][0]["tags"]["title"],
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
                playable = true,
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
    server: Arc<ServerState>,
    backend: Arc<MediaEngineBackend>,
    _fixture: Fixture,
}

async fn make_server() -> Server {
    let f = fixture();
    make_album(&f.lib, "A", &["01.flac"]);
    // Exists before the first compile so editing it does not change the root's
    // directory mtime; the lock bytes must stay identical.
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
        }),
        mpd_engine: MpdEngine::new(mpd_tx),
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: tokio::sync::RwLock::new(String::new()),
        backend: backend.clone(),
    });

    let shared = backend.shared();
    run_pass(
        &shared,
        &UpdateScope::All,
        None,
        ForceMode::Force,
        None,
        Verbosity::Silent,
    )
    .expect("initial compile");

    let mut logic = server.logic.write().await;
    backend.scan_library(&mut logic);
    drop(logic);

    Server {
        server,
        backend,
        _fixture: f,
    }
}

async fn handle(server: &Server, paths: &[PathBuf]) -> Vec<Value> {
    let mut rx = server.server.tx.subscribe();
    server.backend.handle_fs_events(paths, &server.server).await;

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

#[tokio::test]
async fn unrelated_edit_does_not_broadcast_an_update() {
    let server = make_server().await;
    let notes = server._fixture.lib.join("A/notes.txt");
    write(&notes, "nothing the schema reads");

    let messages = handle(&server, &[notes]).await;
    assert!(media_updated(&messages).is_none(), "no bogus update");
}

#[tokio::test]
async fn deleting_the_marker_removes_the_root_and_its_lock() {
    let server = make_server().await;
    let marker = server._fixture.lib.join("A/album.toml");
    std::fs::remove_file(&marker).expect("remove marker");

    let messages = handle(&server, &[marker]).await;
    let update = media_updated(&messages).expect("MEDIA_UPDATED");
    assert_eq!(update["removed"], json!(["Artist - A"]));
    assert!(!lock_path(&server._fixture.lib, "A").exists());

    let logic = server.server.logic.read().await;
    assert!(logic.entity_record("album", "Artist - A").is_none());
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
    init.push_str(
        "\nd.library('all', { media = 'album', match = function() return true end })\n",
    );
    write(&server._fixture.config, &init);

    let messages = handle(&server, &[server._fixture.config.clone()]).await;
    let update = messages
        .iter()
        .find(|message| message["type"] == json!("LOGIC_UPDATE"))
        .expect("LOGIC_UPDATE");
    assert!(update["manifest"]["libraries"].get("all").is_some());

    let logic = server.server.logic.read().await;
    assert!(logic.manifest.libraries.contains_key("all"));
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
