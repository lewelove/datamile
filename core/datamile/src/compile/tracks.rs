use std::path::Path;

use libdatamile::compiler::validation::validate_track_indices;
use libdatamile::error::DatamileError;
use libdatamile::utils::HashMode;
use serde_json::{Value, json};

use crate::compile::album::AlbumKind;
use crate::compile::utils::{self, sort_json_keys};
use crate::harvest;

pub fn build_ctx_tracks(
    kind: AlbumKind,
    primary_tracks: &[Value],
    audio_files: &[std::path::PathBuf],
    album_root: &Path,
    cache_root: &Path,
) -> Result<(Vec<Value>, u64), DatamileError> {
    let mut ctx_tracks = Vec::new();
    let mut duration_sum_ms = 0;

    for (idx, _track_val) in primary_tracks.iter().enumerate() {
        if kind == AlbumKind::Virtual {
            ctx_tracks.push(json!({
                "sample_rate": 0,
                "bits_per_sample": 0,
                "bitrate_kbps": 0,
                "encoding": "",
                "channels": 0,
                "duration_milliseconds": 0,
                "file": {
                    "path": "",
                    "mtime": 0,
                    "byte_size": 0
                },
                "embedded": {}
            }));
        } else {
            let file_path = &audio_files[idx];
            let harvest = harvest::harvest_file_cached(file_path, cache_root).map_err(
                |source| DatamileError::HarvestError {
                    path: file_path.clone(),
                    source,
                },
            )?;
            let rel_path = libdatamile::resolvers::rel_path(file_path, album_root);
            let file_info =
                libdatamile::utils::read_file_info(file_path, &rel_path, HashMode::Skip)
                    .unwrap_or_else(|_| json!({}));

            ctx_tracks.push(json!({
                "sample_rate": harvest.physics.sample_rate,
                "bits_per_sample": harvest.physics.bit_depth.unwrap_or(0),
                "bitrate_kbps": harvest.physics.audio_bitrate,
                "encoding": harvest.physics.format,
                "channels": harvest.physics.channels,
                "duration_milliseconds": harvest.physics.duration_ms,
                "file": file_info,
                "embedded": harvest.tags
            }));

            duration_sum_ms += harvest.physics.duration_ms;
        }
    }

    Ok((ctx_tracks, duration_sum_ms))
}

fn track_info(ctx_track: &Value) -> Value {
    json!({
        "sample_rate": ctx_track.get("sample_rate").cloned().unwrap_or_else(|| json!(0)),
        "bits_per_sample": ctx_track.get("bits_per_sample").cloned().unwrap_or_else(|| json!(0)),
        "bitrate_kbps": ctx_track.get("bitrate_kbps").cloned().unwrap_or_else(|| json!(0)),
        "encoding": ctx_track.get("encoding").cloned().unwrap_or_else(|| json!("")),
        "channels": ctx_track.get("channels").cloned().unwrap_or_else(|| json!(0)),
        "duration_milliseconds": ctx_track.get("duration_milliseconds").cloned().unwrap_or_else(|| json!(0)),
        "duration_formatted": libdatamile::resolvers::format_ms(ctx_track.get("duration_milliseconds").and_then(Value::as_u64).unwrap_or(0)),
    })
}

pub fn build_final_tracks(
    lua_tracks: Option<&Vec<Value>>,
    ctx_tracks: &[Value],
) -> Vec<Value> {
    let mut final_tracks = Vec::new();

    for (i, ctx_track) in ctx_tracks.iter().enumerate() {
        let lua_track = lua_tracks.and_then(|arr| arr.get(i));

        let mut tags = utils::lua_object(lua_track.and_then(|t| t.get("tags")));
        sort_json_keys(&mut tags);

        let mut objects = utils::lua_object(lua_track.and_then(|t| t.get("objects")));
        // `info` is the one object Rust owns, and only until config registers it too.
        if let Some(map) = objects.as_object_mut() {
            map.entry("info").or_insert_with(|| track_info(ctx_track));
        }
        sort_json_keys(&mut objects);

        let mut t_obj = serde_json::Map::new();
        t_obj.insert("tags".to_string(), tags);
        t_obj.insert("objects".to_string(), objects);
        t_obj.insert(
            "file".to_string(),
            ctx_track.get("file").cloned().unwrap_or_else(|| json!({})),
        );

        final_tracks.push(Value::Object(t_obj));
    }

    final_tracks
}

pub fn validate_audio_files(
    kind: AlbumKind,
    audio_files: &[std::path::PathBuf],
    primary_tracks: &[Value],
    album_root: &Path,
) -> Result<(), DatamileError> {
    if kind == AlbumKind::Physical && audio_files.len() != primary_tracks.len() {
        return Err(DatamileError::PhysicalInventoryMismatch {
            path: album_root.to_path_buf(),
            files_count: audio_files.len(),
            tracks_count: primary_tracks.len(),
        });
    }
    validate_track_indices(primary_tracks, album_root)?;
    Ok(())
}
