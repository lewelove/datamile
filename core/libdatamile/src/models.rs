use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HashInfo {
    pub string: String,
    pub address: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileInfo {
    #[serde(default)]
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub byte_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub file: FileInfo,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub format: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverSource {
    pub file: FileInfo,
    pub info: ImageInfo,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoverEntry {
    pub address: String,
    pub width: u32,
    pub height: u32,
    pub fit: String,
    pub filter: String,
    pub source: CoverSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlbumInfo {
    pub total_discs: u32,
    pub total_tracks: u32,
    #[serde(default)]
    pub duration_milliseconds: u64,
    #[serde(default)]
    pub duration_formatted: String,
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlbumLock {
    pub albumartist: String,
    pub album: String,
    pub date: String,
    pub id: String,
    #[serde(default)]
    pub keys: BTreeMap<String, serde_json::Value>,
    pub info: AlbumInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<serde_json::Value>,
    #[serde(default)]
    pub manifests: BTreeMap<String, ManifestEntry>,
    #[serde(default)]
    pub covers: BTreeMap<String, Vec<CoverEntry>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LyricsData {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackInfo {
    #[serde(default)]
    pub sample_rate: u32,
    #[serde(default)]
    pub bits_per_sample: u8,
    #[serde(default)]
    pub bitrate_kbps: u32,
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub channels: u8,
    #[serde(default)]
    pub duration_milliseconds: u64,
    #[serde(default)]
    pub duration_formatted: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackLock {
    pub discnumber: u32,
    pub tracknumber: u32,
    pub artist: String,
    pub title: String,
    #[serde(default)]
    pub keys: BTreeMap<String, serde_json::Value>,
    pub info: TrackInfo,
    pub file: FileInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<LyricsData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub album: AlbumLock,
    pub tracks: Vec<TrackLock>,
}
