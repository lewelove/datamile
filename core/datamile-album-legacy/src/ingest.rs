use std::path::Path;

use datamile_logic::{EntityRecord, ImageLookupItem, PlayableLookupItem, tag_text};
use serde_json::Value;
use serde_json::value::RawValue;

#[cfg(test)]
mod tests;

/// Render size assumed when an image entry omits its rendered dimensions.
const DEFAULT_IMAGE_DIM: u32 = 1080;

const MEDIA_TYPE: &str = "album";

pub struct ParsedAlbum {
    pub entity: EntityRecord,
    pub playables: Vec<PlayableLookupItem>,
    pub images: Vec<(String, ImageLookupItem)>,
}

fn number_field(value: &Value, key: &str, fallback: u64) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(fallback)
}

fn tag_number(value: Option<&Value>, fallback: u32) -> u32 {
    match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(fallback),
        Some(Value::String(s)) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}

/// MPD addresses tracks relative to the music directory, without a leading `/`.
fn mpd_uri(album_dir: &Path, rel: &str, music_directory: &Path) -> String {
    let absolute = album_dir.join(rel);
    let raw = absolute.strip_prefix(music_directory).map_or_else(
        |_| absolute.to_string_lossy().to_string(),
        |p| p.to_string_lossy().to_string(),
    );
    raw.trim_start_matches('/').to_string()
}

fn parse_playables(
    tracks: &[Value],
    album_dir: &Path,
    id: &str,
    music_directory: &Path,
) -> Vec<PlayableLookupItem> {
    let mut playables = Vec::new();

    for (node_index, track) in tracks.iter().enumerate() {
        let Some(info) = track.pointer("/objects/info") else {
            continue;
        };

        let rel = track
            .pointer("/location/path")
            .and_then(Value::as_str)
            .unwrap_or("");
        if rel.is_empty() {
            continue;
        }

        playables.push(PlayableLookupItem {
            media_type: MEDIA_TYPE.to_string(),
            media_id: id.to_string(),
            node_index,
            track_no: tag_number(track.pointer("/tags/tracknumber"), 0),
            disc_no: tag_number(track.pointer("/tags/discnumber"), 1),
            title: tag_text(track.pointer("/tags/title"))
                .unwrap_or_else(|| "Unknown".to_string()),
            artist: tag_text(track.pointer("/tags/artist"))
                .unwrap_or_else(|| "Unknown".to_string()),
            duration: info
                .get("duration_formatted")
                .and_then(Value::as_str)
                .unwrap_or("0:00")
                .to_string(),
            duration_ms: number_field(info, "duration_milliseconds", 0),
            path: mpd_uri(album_dir, rel, music_directory),
        });
    }

    playables
}

struct ParsedImages {
    main_image_path: Option<String>,
    images: Vec<(String, ImageLookupItem)>,
    namespaces: Vec<(String, String)>,
}

fn parse_images(album: &Value, album_dir: &Path) -> ParsedImages {
    let main_image_path = album
        .pointer("/images/main/0/source/file/path")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut images = Vec::new();
    let mut namespaces = Vec::new();

    let Some(entries_by_ns) = album.get("images").and_then(Value::as_object) else {
        return ParsedImages {
            main_image_path,
            images,
            namespaces,
        };
    };

    for (namespace, entries) in entries_by_ns {
        let Some(entries) = entries.as_array() else {
            continue;
        };

        for entry in entries {
            let Some(address) = entry.get("address").and_then(Value::as_str) else {
                continue;
            };
            let Some(path) = entry.pointer("/source/file/path").and_then(Value::as_str)
            else {
                continue;
            };

            let width = entry
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(DEFAULT_IMAGE_DIM);
            let height = entry
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(DEFAULT_IMAGE_DIM);

            images.push((
                address.to_string(),
                ImageLookupItem {
                    entity_dir: album_dir.to_path_buf(),
                    path: path.to_string(),
                    fit: entry
                        .get("fit")
                        .and_then(Value::as_str)
                        .unwrap_or("inside")
                        .to_string(),
                    filter: entry
                        .get("filter")
                        .and_then(Value::as_str)
                        .unwrap_or("catmullrom")
                        .to_string(),
                    target_width: width,
                    target_height: height,
                },
            ));
            namespaces.push((namespace.clone(), address.to_string()));
        }
    }

    ParsedImages {
        main_image_path,
        images,
        namespaces,
    }
}

/// Turns a compiled `album.lock.json` into the media-agnostic index records.
///
/// Returns `Ok(None)` for a lock without the album object or its `info` block:
/// such a file cannot produce a usable record and is skipped quietly.
pub fn parse_album_lock(
    album_dir: &Path,
    id: &str,
    lock_json: &str,
    music_directory: &Path,
) -> anyhow::Result<Option<ParsedAlbum>> {
    let album_dir = album_dir
        .canonicalize()
        .unwrap_or_else(|_| album_dir.to_path_buf());
    let parsed: Value = serde_json::from_str(lock_json)?;

    let Some(album) = parsed.get("album") else {
        return Ok(None);
    };
    let Some(info) = album.pointer("/objects/info") else {
        return Ok(None);
    };

    let playables = album
        .pointer("/nodes/track")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |tracks| {
            parse_playables(tracks, &album_dir, id, music_directory)
        });

    let parsed_images = parse_images(album, &album_dir);

    let entry = serde_json::json!({
        "id": id,
        "tags": album.get("tags"),
        "objects": album.get("objects"),
        "images": album.get("images"),
    });
    let dict_raw = RawValue::from_string(serde_json::to_string(&entry)?)?;

    let entity = EntityRecord {
        media_type: MEDIA_TYPE.to_string(),
        id: id.to_string(),
        path: album_dir,
        title: tag_text(album.pointer("/tags/album"))
            .unwrap_or_else(|| "Unknown".to_string()),
        subtitle: tag_text(album.pointer("/tags/albumartist"))
            .unwrap_or_else(|| "Unknown".to_string()),
        main_image_path: parsed_images.main_image_path,
        raw_json: std::sync::Arc::from(lock_json),
        dict_raw,
        total_discs: number_field(info, "total_discs", 1),
        total_tracks: number_field(info, "total_tracks", 0),
        duration_ms: number_field(info, "duration_milliseconds", 0),
        playable_uris: playables.iter().map(|p| p.path.clone()).collect(),
        image_namespaces: parsed_images.namespaces,
    };

    Ok(Some(ParsedAlbum {
        entity,
        playables,
        images: parsed_images.images,
    }))
}
