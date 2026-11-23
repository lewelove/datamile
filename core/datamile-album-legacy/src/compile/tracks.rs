use std::path::Path;

use datamile_fs::HashMode;
use serde_json::{Value, json};

use crate::compile::album::AlbumKind;
use crate::compile::utils::{self, sort_json_keys};
use crate::compiler::validation::validate_track_indices;
use crate::error::DatamileError;
use crate::resolvers;

pub(crate) fn build_ctx_tracks(
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
            let audio = datamile_audio::probe_file_cached(file_path, cache_root)
                .map_err(|source| DatamileError::HarvestError {
                    path: file_path.clone(),
                    source,
                })?;
            let rel_path = datamile_fs::rel_path(file_path, album_root);
            let file_info =
                datamile_fs::read_file_info(file_path, &rel_path, HashMode::Skip)
                    .unwrap_or_else(|_| json!({}));

            ctx_tracks.push(json!({
                "sample_rate": audio.physics.sample_rate,
                "bits_per_sample": audio.physics.bit_depth.unwrap_or(0),
                "bitrate_kbps": audio.physics.audio_bitrate,
                "encoding": audio.physics.format,
                "channels": audio.physics.channels,
                "duration_milliseconds": audio.physics.duration_ms,
                "file": file_info,
                "embedded": audio.tags
            }));

            duration_sum_ms += audio.physics.duration_ms;
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
        "duration_formatted": resolvers::format_ms(ctx_track.get("duration_milliseconds").and_then(Value::as_u64).unwrap_or(0)),
    })
}

/// Filesystem location of a physical track; virtual tracks have none.
pub(crate) fn track_location(ctx_track: &Value) -> Option<Value> {
    let file = ctx_track.get("file")?;
    let path = file.get("path").and_then(Value::as_str)?;

    if path.is_empty() {
        return None;
    }

    Some(json!({
        "kind": "file",
        "path": path,
        "mtime": file.get("mtime").cloned().unwrap_or_else(|| json!(0)),
        "byte_size": file.get("byte_size").cloned().unwrap_or_else(|| json!(0)),
    }))
}

pub(crate) fn build_final_tracks(
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
        if let Some(location) = track_location(ctx_track) {
            t_obj.insert("location".to_string(), location);
        }

        final_tracks.push(Value::Object(t_obj));
    }

    final_tracks
}

pub(crate) fn validate_audio_files(
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
