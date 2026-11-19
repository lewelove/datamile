use std::path::Path;

use libdatamile::compiler::manifest::extract_strict_u32;
use libdatamile::compiler::validation::validate_track_indices;
use libdatamile::error::DatamileError;
use libdatamile::utils::HashMode;
use serde_json::{Value, json};

use crate::compile::album::AlbumKind;
use crate::compile::utils::sort_json_keys;
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
                libdatamile::utils::get_file_info(file_path, &rel_path, HashMode::Skip)
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

pub fn build_final_tracks(
    primary_tracks: &[Value],
    albumartist: &str,
    track_keys_array: Option<&Vec<Value>>,
    ctx_tracks: &[Value],
    album_root: &Path,
) -> Result<Vec<Value>, DatamileError> {
    let mut final_tracks = Vec::new();

    for (i, t_val) in primary_tracks.iter().enumerate() {
        let discnumber =
            extract_strict_u32(t_val.get("discnumber"), "discnumber", Some(1))?;
        let tracknumber =
            extract_strict_u32(t_val.get("tracknumber"), "tracknumber", None)?;

        let t_artist = t_val
            .get("artist")
            .and_then(Value::as_str)
            .unwrap_or(albumartist)
            .to_string();
        let t_title = t_val
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| DatamileError::TypeMismatch {
                path: album_root.to_path_buf(),
                key: "title".to_string(),
                expected_type: "string".to_string(),
                found_val: "missing".to_string(),
            })?
            .to_string();

        let mut t_keys = track_keys_array
            .and_then(|arr| arr.get(i))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let lyrics_val = t_keys.as_object_mut().and_then(|m| m.remove("lyrics"));
        sort_json_keys(&mut t_keys);

        let ctx_track = &ctx_tracks[i];

        let mut t_obj = serde_json::Map::new();
        t_obj.insert("discnumber".to_string(), json!(discnumber));
        t_obj.insert("tracknumber".to_string(), json!(tracknumber));
        t_obj.insert("artist".to_string(), json!(t_artist));
        t_obj.insert("title".to_string(), json!(t_title));
        t_obj.insert("keys".to_string(), t_keys);

        let t_info = json!({
            "sample_rate": ctx_track.get("sample_rate").cloned().unwrap_or_else(|| json!(0)),
            "bits_per_sample": ctx_track.get("bits_per_sample").cloned().unwrap_or_else(|| json!(0)),
            "bitrate_kbps": ctx_track.get("bitrate_kbps").cloned().unwrap_or_else(|| json!(0)),
            "encoding": ctx_track.get("encoding").cloned().unwrap_or_else(|| json!("")),
            "channels": ctx_track.get("channels").cloned().unwrap_or_else(|| json!(0)),
            "duration_milliseconds": ctx_track.get("duration_milliseconds").cloned().unwrap_or_else(|| json!(0)),
            "duration_formatted": libdatamile::resolvers::format_ms(ctx_track.get("duration_milliseconds").and_then(Value::as_u64).unwrap_or(0)),
        });
        t_obj.insert("info".to_string(), t_info);
        t_obj.insert(
            "file".to_string(),
            ctx_track.get("file").cloned().unwrap_or_else(|| json!({})),
        );

        if let Some(mut l_val) = lyrics_val {
            sort_json_keys(&mut l_val);
            t_obj.insert("lyrics".to_string(), l_val);
        }

        final_tracks.push(Value::Object(t_obj));
    }

    Ok(final_tracks)
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
