use crate::chapters::Chapter;
use anyhow::{Context, Result};
use lofty::config::ParseOptions;
use lofty::file::AudioFile;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemValue, TagType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SUPPORTED_AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "wav", "wave", "aif", "aiff", "aifc", "afc", "alac", "mp3", "mp2", "mp1",
    "m4a", "m4b", "m4p", "m4r", "m4v", "mp4", "aac", "3gp", "ogg", "oga", "opus", "spx",
    "ape", "wv", "mpc", "mp+", "mpp",
];

const HARVEST_CACHE_VERSION: u8 = 3;

#[must_use]
pub fn is_matching_extension<S: AsRef<str>>(ext: &str, candidates: &[S]) -> bool {
    candidates
        .iter()
        .any(|c| c.as_ref().eq_ignore_ascii_case(ext))
}

#[must_use]
pub fn is_audio_file<S: AsRef<str>>(path: &Path, candidates: &[S]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_matching_extension(ext, candidates))
}

#[derive(Serialize, Deserialize)]
pub struct TrackJson {
    pub path: PathBuf,
    pub tags: HashMap<String, serde_json::Value>,
    pub physics: PhysicsData,
    #[serde(default)]
    pub chapters: Option<Vec<Chapter>>,
}

#[derive(Serialize, Deserialize)]
pub struct PhysicsData {
    pub file_size: u64,
    pub mtime: u64,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub bit_depth: Option<u8>,
    pub channels: u8,
    pub audio_bitrate: u32,
    pub overall_bitrate: u32,
    pub format: String,
}

#[must_use]
pub fn sanitize_key(key: &str) -> String {
    let mut out = String::new();
    let mut last_was_under = false;
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_under = false;
        } else if !last_was_under {
            out.push('_');
            last_was_under = true;
        }
    }
    out.trim_matches('_').to_string()
}

pub fn harvest_file(path: &Path) -> Result<TrackJson> {
    let metadata = fs::metadata(path)?;
    let tagged_file = Probe::open(path)
        .context("Failed to open audio file")?
        .options(ParseOptions::new().read_cover_art(false))
        .guess_file_type()
        .context("Failed to guess file type")?
        .read()
        .context("Failed to read audio metadata")?;

    if tagged_file.file_type() == lofty::file::FileType::Flac
        && tagged_file.contains_tag_type(TagType::Id3v2)
    {
        log::warn!(
            "ID3v2 tag encountered in FLAC (incompatible with standards): {}",
            path.display()
        );
    }

    let physics = extract_physics(&metadata, &tagged_file);

    let mut tags = HashMap::new();
    extract_tags(&tagged_file, &mut tags);

    let chapters = crate::chapters::extract(path).map(|chapters| {
        crate::chapters::close_last_chapter(chapters, physics.duration_ms)
    });

    Ok(TrackJson {
        path: path.to_path_buf(),
        tags,
        physics,
        chapters,
    })
}

pub(crate) fn cache_key(path: &Path, metadata: &fs::Metadata) -> String {
    let mtime = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let key_str = format!(
        "{HARVEST_CACHE_VERSION}:{}:{}:{mtime}",
        path.display(),
        metadata.len()
    );

    blake3::hash(key_str.as_bytes()).to_hex().to_string()
}

pub fn harvest_file_cached(path: &Path, cache_root: &Path) -> Result<TrackJson> {
    let metadata = fs::metadata(path)?;
    let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = cache_key(&canon_path, &metadata);

    let cache_dir = cache_root.join("harvest");
    let cache_file = cache_dir.join(format!("{key}.json"));

    if let Ok(content) = fs::read_to_string(&cache_file) {
        if let Ok(cached_track) = serde_json::from_str::<TrackJson>(&content) {
            return Ok(cached_track);
        }
        let _ = fs::remove_file(&cache_file);
    }

    let harvested = harvest_file(path)?;

    if let Ok(json_str) = serde_json::to_string(&harvested) {
        let _ = crate::utils::write_atomic_cache_file(&cache_file, &json_str);
    }

    Ok(harvested)
}

fn extract_physics(
    metadata: &std::fs::Metadata,
    tagged_file: &lofty::file::TaggedFile,
) -> PhysicsData {
    let file_size = metadata.len();
    let mtime = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let properties = tagged_file.properties();
    let file_type = tagged_file.file_type();

    PhysicsData {
        file_size,
        mtime,
        duration_ms: u64::try_from(properties.duration().as_millis()).unwrap_or(u64::MAX),
        sample_rate: properties.sample_rate().unwrap_or(0),
        bit_depth: properties.bit_depth(),
        channels: properties.channels().unwrap_or(0),
        audio_bitrate: properties.audio_bitrate().unwrap_or(0),
        overall_bitrate: properties.overall_bitrate().unwrap_or(0),
        format: format!("{file_type:?}"),
    }
}

fn extract_tags(
    tagged_file: &lofty::file::TaggedFile,
    tags: &mut HashMap<String, serde_json::Value>,
) {
    let Some(tag) = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
    else {
        return;
    };

    let source_tag_type = tag.tag_type();
    for item in tag.items() {
        let item_key = item.key();
        let key_raw = item_key.map_key(TagType::VorbisComments).map_or_else(
            || {
                item_key
                    .map_key(source_tag_type)
                    .map_or_else(|| format!("{item_key:?}"), ToString::to_string)
            },
            ToString::to_string,
        );

        let key = sanitize_key(&key_raw);
        let value_raw = match item.value() {
            ItemValue::Text(text) | ItemValue::Locator(text) => text.as_str(),
            ItemValue::Binary(_) => continue,
        };

        let value = value_raw.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }

        match tags.get_mut(&key) {
            Some(serde_json::Value::String(existing_str)) => {
                if existing_str != value {
                    let prev = existing_str.clone();
                    tags.insert(
                        key,
                        serde_json::Value::Array(vec![
                            serde_json::Value::String(prev),
                            serde_json::Value::String(value.to_string()),
                        ]),
                    );
                }
            }
            Some(serde_json::Value::Array(arr)) => {
                if !arr.iter().any(|v| v.as_str() == Some(value)) {
                    arr.push(serde_json::Value::String(value.to_string()));
                }
            }
            _ => {
                tags.insert(key, serde_json::Value::String(value.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests;
