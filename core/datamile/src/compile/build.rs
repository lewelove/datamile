use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use libdatamile::compiler::manifest::load_manifests;
use libdatamile::error::DatamileError;
use serde::de::Error as _;
use serde_json::{Value, json};

use crate::compile::{album, context, images, tracks, utils};

struct PrimaryBuildData {
    parsed_manifests: serde_json::Map<String, Value>,
    primary_tracks: Vec<Value>,
    prep_ctx: context::PreparedContext,
}

pub struct BuildOutput {
    pub album_dir: PathBuf,
    pub album_id: String,
    pub lock_json: Value,
    pub dependencies: HashSet<PathBuf>,
}

struct DispatcherInput<'a> {
    config: &'a libdatamile::lua::ResolvedConfig,
    album_root: &'a Path,
    music_directory: &'a Path,
    total_discs: u32,
    total_tracks: u32,
    duration_sum_ms: u64,
}

struct AlbumAssemblyInput<'a> {
    id: &'a str,
    tags: Value,
    objects: Value,
    manifests: &'a BTreeMap<String, Value>,
    images: &'a BTreeMap<String, Value>,
}

fn load_primary_and_files(
    album_root: &Path,
    config: &libdatamile::lua::ResolvedConfig,
    cache_root: &Path,
) -> Result<PrimaryBuildData, DatamileError> {
    let manifest_names = config.app.compiler.manifests.as_deref();
    let parsed_manifests = load_manifests(album_root, manifest_names, cache_root)?;

    let primary_manifest = parsed_manifests.get("metadata").ok_or_else(|| {
        DatamileError::MissingPrimaryManifest {
            path: album_root.to_path_buf(),
        }
    })?;
    let primary_tracks = primary_manifest
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| DatamileError::MissingTracksBlock {
            path: album_root.to_path_buf(),
        })?
        .clone();

    let prep_ctx = context::prepare_build_context(config, album_root);
    Ok(PrimaryBuildData {
        parsed_manifests,
        primary_tracks,
        prep_ctx,
    })
}

fn build_dispatcher_context(
    input: &DispatcherInput<'_>,
    ctx_tracks: &[Value],
) -> (Value, String, String, String) {
    let config_json =
        serde_json::to_value(&input.config.app).unwrap_or_else(|_| json!({}));

    let project_root_str = input
        .config
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_string_lossy()
        .into_owned();

    let album_root_canon = input
        .album_root
        .canonicalize()
        .unwrap_or_else(|_| input.album_root.to_path_buf());
    let album_root_str = album_root_canon.to_string_lossy().into_owned();
    let music_directory_str = input.music_directory.to_string_lossy().into_owned();
    let rel_path_str = datamile_fs::rel_path(&album_root_canon, input.music_directory);

    let ctx = json!({
        "config": config_json,
        "paths": {
            "album_root": album_root_str,
            "rel_path": rel_path_str,
            "project_root": project_root_str,
            "music_directory": music_directory_str
        },
        "total_discs": input.total_discs,
        "total_tracks": input.total_tracks,
        "duration_milliseconds": input.duration_sum_ms,
        "tracks": ctx_tracks,
    });

    (ctx, album_root_str, project_root_str, music_directory_str)
}

fn assemble_album_object(
    input: &AlbumAssemblyInput<'_>,
) -> serde_json::Map<String, Value> {
    let mut album_obj = serde_json::Map::new();
    album_obj.insert("id".to_string(), json!(input.id));
    album_obj.insert("tags".to_string(), input.tags.clone());
    album_obj.insert("objects".to_string(), input.objects.clone());
    album_obj.insert("manifests".to_string(), json!(input.manifests));
    album_obj.insert("images".to_string(), json!(input.images));
    album_obj
}

fn run_dispatcher_phase(
    engine: &libdatamile::lua::LuaEngine,
    album_root: &Path,
    parsed_manifests: &serde_json::Map<String, Value>,
    ctx: &Value,
) -> Result<(Value, HashSet<PathBuf>), DatamileError> {
    let manifests_json = Value::Object(parsed_manifests.clone());
    let lua_res = engine
        .execute_dispatcher(ctx, &manifests_json)
        .map_err(|e| DatamileError::ManifestParseError {
            path: album_root.to_path_buf(),
            source: toml::de::Error::custom(e.to_string()),
        })?;

    let mut dependencies = engine
        .lua
        .app_data_ref::<libdatamile::lua::EngineContext>()
        .map_or_else(HashSet::new, |c| c.take_dependencies());

    dependencies.retain(|p| !p.starts_with(album_root));

    Ok((lua_res, dependencies))
}

fn extract_album_id(lua_res: &Value, album_root: &Path) -> Result<String, DatamileError> {
    lua_res
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| DatamileError::TypeMismatch {
            path: album_root.to_path_buf(),
            key: "id".to_string(),
            expected_type: "non-empty string".to_string(),
            found_val: "missing or invalid".to_string(),
        })
}

fn validate_album_header(
    parsed_manifests: &serde_json::Map<String, Value>,
    album_root_canon: &Path,
) -> Result<(), DatamileError> {
    let empty_album = json!({});
    let primary_manifest = parsed_manifests.get("metadata").ok_or_else(|| {
        DatamileError::MissingPrimaryManifest {
            path: album_root_canon.to_path_buf(),
        }
    })?;
    let primary_album = primary_manifest.get("album").unwrap_or(&empty_album);
    album::validate_mandatory_fields(primary_album, album_root_canon)
}

fn build_lock_json(
    assembly_input: &AlbumAssemblyInput<'_>,
    final_tracks: Vec<Value>,
) -> Value {
    let album_obj = assemble_album_object(assembly_input);
    let mut final_json = json!({
        "album": Value::Object(album_obj),
        "tracks": Value::Array(final_tracks),
    });
    utils::strip_empty_values(&mut final_json);
    final_json
}

pub fn build(
    album_root: &Path,
    config: &libdatamile::lua::ResolvedConfig,
    engine: &libdatamile::lua::LuaEngine,
) -> Result<BuildOutput, DatamileError> {
    if let Some(ctx) = engine.lua.app_data_ref::<libdatamile::lua::EngineContext>() {
        ctx.take_dependencies();
    }

    let album_root_canon = album_root
        .canonicalize()
        .unwrap_or_else(|_| album_root.to_path_buf());

    let cache_root = datamile_fs::expand_path(&config.app.storage.cache);
    let build_data = load_primary_and_files(&album_root_canon, config, &cache_root)?;

    // Cheapest check first: no Lua dispatch or image I/O for an album that
    // cannot produce a usable lock anyway.
    validate_album_header(&build_data.parsed_manifests, &album_root_canon)?;

    let album_kind = album::is_virtual_album(&build_data.parsed_manifests);
    tracks::validate_audio_files(
        album_kind,
        &build_data.prep_ctx.audio_files,
        &build_data.primary_tracks,
        &album_root_canon,
    )?;

    let lock_manifests = album::generate_lock_manifests(
        &build_data.parsed_manifests,
        &album_root_canon,
        album_kind,
    );
    let total_discs =
        libdatamile::resolvers::calculate_total_discs(&build_data.primary_tracks);
    let total_tracks = u32::try_from(build_data.primary_tracks.len()).unwrap_or(0);

    let (ctx_tracks, duration_sum_ms) = tracks::build_ctx_tracks(
        album_kind,
        &build_data.primary_tracks,
        &build_data.prep_ctx.audio_files,
        &album_root_canon,
        &cache_root,
    )?;

    let disp_input = DispatcherInput {
        config,
        album_root: &album_root_canon,
        music_directory: &build_data.prep_ctx.music_directory,
        total_discs,
        total_tracks,
        duration_sum_ms,
    };

    let (ctx, _album_root_str, _project_root_str, _music_directory_str) =
        build_dispatcher_context(&disp_input, &ctx_tracks);

    let (lua_res, mut dependencies) = run_dispatcher_phase(
        engine,
        &album_root_canon,
        &build_data.parsed_manifests,
        &ctx,
    )?;

    let images_map = images::process_images(
        &album_root_canon,
        &cache_root,
        lua_res.get("images"),
        &mut dependencies,
    )?;

    let album_id = extract_album_id(&lua_res, &album_root_canon)?;

    let mut album_tags = utils::lua_object(lua_res.get("tags"));
    utils::sort_json_keys(&mut album_tags);

    let mut album_objects = utils::lua_object(lua_res.get("objects"));
    let info_obj = json!({
        "virtual": album_kind == album::AlbumKind::Virtual,
        "total_discs": total_discs,
        "total_tracks": total_tracks,
        "duration_milliseconds": duration_sum_ms,
        "duration_formatted": libdatamile::resolvers::format_ms(duration_sum_ms),
    });
    // `info` is the one object Rust owns, and only until config registers it too.
    if let Some(objects) = album_objects.as_object_mut() {
        objects.entry("info").or_insert(info_obj);
    }
    utils::sort_json_keys(&mut album_objects);

    let final_tracks = tracks::build_final_tracks(
        lua_res.get("tracks").and_then(Value::as_array),
        &ctx_tracks,
    );

    let assembly_input = AlbumAssemblyInput {
        id: &album_id,
        tags: album_tags,
        objects: album_objects,
        manifests: &lock_manifests,
        images: &images_map,
    };

    let lock_json = build_lock_json(&assembly_input, final_tracks);

    Ok(BuildOutput {
        album_dir: album_root_canon,
        album_id,
        lock_json,
        dependencies,
    })
}
