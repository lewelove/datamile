pub mod cache;
pub mod query;
pub mod sort;

pub use sort::{SortKey, SortOrder, value_to_sort_key};

use anyhow::Result;
use libdatamile::error::DatamileError;
use libdatamile::images::{FilterType, FitMode, parse_filter, parse_fit_mode};
use libdatamile::lua::{LogicManifest, with_evaluated_lua_vm};
use roaring::RoaringBitmap;
use serde::Serialize;
use serde_json::{Value, json, value::RawValue};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct CoverLookupItem {
    pub album_dir: PathBuf,
    pub path: String,
    pub fit_mode: FitMode,
    pub target_width: u32,
    pub target_height: u32,
    pub filter: FilterType,
}

#[derive(Clone, Debug)]
pub struct AlbumRecord {
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub cover_path: String,
    pub raw_json: Arc<str>,
    pub dict_raw: Box<RawValue>,
    pub total_discs: u64,
    pub total_tracks: u64,
    pub duration_ms: u64,
    pub track_uris: Vec<String>,
    pub cover_namespaces: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackLookupItem {
    pub path: String,
    #[serde(rename = "trackIndex")]
    pub track_index: usize,
    #[serde(rename = "trackNo")]
    pub track_no: u32,
    #[serde(rename = "discNo")]
    pub disc_no: u32,
    pub title: String,
    pub artist: String,
    pub duration: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    #[serde(rename = "albumId")]
    pub album_id: String,
}

pub struct LogicEngine {
    pub config_path: PathBuf,
    pub manifest: LogicManifest,
    pub(crate) libraries_cache: HashMap<String, RoaringBitmap>,
    pub(crate) filters_cache: HashMap<String, RoaringBitmap>,
    pub(crate) groupers_cache: HashMap<String, HashMap<String, RoaringBitmap>>,
    pub(crate) orders_cache: HashMap<String, Vec<u32>>,
    pub(crate) shelves_cache: HashMap<String, Vec<u32>>,
    pub(crate) next_uid: u32,
    pub albums: HashMap<u32, AlbumRecord>,
    pub id_to_uid: HashMap<String, u32>,
    pub path_to_uid: HashMap<PathBuf, u32>,
    pub cover_lookup: HashMap<String, CoverLookupItem>,
    pub track_lookup: HashMap<String, TrackLookupItem>,
    pub(crate) evaluated_logic: HashMap<u32, Value>,
}

impl LogicEngine {
    pub fn new() -> Result<Self> {
        let config_path = libdatamile::lua::resolve_config_path().unwrap_or_default();
        let manifest = with_evaluated_lua_vm(&config_path, |_, eval| Ok(eval.manifest))?;

        Ok(Self {
            config_path,
            manifest,
            libraries_cache: HashMap::new(),
            filters_cache: HashMap::new(),
            groupers_cache: HashMap::new(),
            orders_cache: HashMap::new(),
            shelves_cache: HashMap::new(),
            next_uid: 1,
            albums: HashMap::new(),
            id_to_uid: HashMap::new(),
            path_to_uid: HashMap::new(),
            cover_lookup: HashMap::new(),
            track_lookup: HashMap::new(),
            evaluated_logic: HashMap::new(),
        })
    }

    pub fn reload_manifest(&mut self, config_path: &Path) -> Result<()> {
        let manifest = with_evaluated_lua_vm(config_path, |engine, eval| {
            let mut new_evaluated = HashMap::new();
            for (&uid, record) in &self.albums {
                if let Ok(parsed) = serde_json::from_str::<Value>(&record.raw_json)
                    && let Ok(res) = engine.evaluate_album_logic(&parsed)
                {
                    new_evaluated.insert(uid, res);
                }
            }
            self.evaluated_logic = new_evaluated;
            Ok(eval.manifest)
        })?;
        self.config_path = config_path.to_path_buf();
        self.manifest = manifest;
        self.build_cache();
        Ok(())
    }

    pub fn clear(&mut self) {
        self.libraries_cache.clear();
        self.filters_cache.clear();
        self.groupers_cache.clear();
        self.orders_cache.clear();
        self.shelves_cache.clear();
        self.id_to_uid.clear();
        self.path_to_uid.clear();
        self.albums.clear();
        self.cover_lookup.clear();
        self.next_uid = 1;
        self.track_lookup.clear();
        self.evaluated_logic.clear();
    }

    pub fn remove_album_by_path(&mut self, path: &Path) -> Option<String> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let uid = self.path_to_uid.remove(&canon)?;
        let id = self.albums.get(&uid)?.id.clone();
        self.remove_album_by_id(&id);
        Some(id)
    }

    pub fn remove_album_by_id(&mut self, id: &str) {
        let Some(uid) = self.id_to_uid.remove(id) else {
            return;
        };

        self.evaluated_logic.remove(&uid);

        if let Some(record) = self.albums.remove(&uid) {
            self.path_to_uid.remove(&record.path);

            for uri in &record.track_uris {
                self.track_lookup.remove(uri);
            }

            for (_, addr) in &record.cover_namespaces {
                self.cover_lookup.remove(addr);
            }
        }
    }

    fn validate_and_cleanup_paths(
        &mut self,
        album_dir_canon: &Path,
        id: &str,
    ) -> Result<()> {
        if let Some(existing_record) = self.get_album_record(id)
            && existing_record.path != album_dir_canon
        {
            if existing_record.path.exists() {
                return Err(DatamileError::DuplicateAlbumId {
                    id: id.to_string(),
                    path_a: existing_record.path.clone(),
                    path_b: album_dir_canon.to_path_buf(),
                }
                .into());
            }
            let old_path = existing_record.path.clone();
            self.remove_album_by_path(&old_path);
        }

        if let Some(old_uid) = self.path_to_uid.get(album_dir_canon)
            && let Some(old_record) = self.albums.get(old_uid)
            && old_record.id != id
        {
            let old_id = old_record.id.clone();
            self.remove_album_by_id(&old_id);
        }

        self.remove_album_by_id(id);

        Ok(())
    }

    fn ingest_tracks_and_lookup(
        &mut self,
        tracks: &[Value],
        album_dir_canon: &Path,
        id: &str,
        music_directory: &Path,
    ) -> Vec<String> {
        let mut track_uris = Vec::new();

        for (track_idx, track) in tracks.iter().enumerate() {
            let tags = track.get("tags");
            let objects = track.get("objects");
            let info = objects.and_then(|o| o.get("info"));

            let rel = track
                .get("location")
                .and_then(|l| l.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if rel.is_empty() {
                continue;
            }

            let abs_track_path = if Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                album_dir_canon.join(rel)
            };
            let mpd_uri_raw = abs_track_path.strip_prefix(music_directory).map_or_else(
                |_| abs_track_path.to_string_lossy().to_string(),
                |p| p.to_string_lossy().to_string(),
            );
            let mpd_uri = mpd_uri_raw.trim_start_matches('/').to_string();

            let tag_u32 = |key: &str, default: u32| -> u32 {
                match tags.and_then(|t| t.get(key)) {
                    Some(Value::Number(n)) => n
                        .as_u64()
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(default),
                    Some(Value::String(s)) => s.parse().unwrap_or(default),
                    _ => default,
                }
            };

            let track_no = tag_u32("tracknumber", 0);
            let disc_no = tag_u32("discnumber", 1);
            let title = tags
                .and_then(|t| t.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_string();
            let artist = tags
                .and_then(|t| t.get("artist"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_string();
            let duration = info
                .and_then(|i| i.get("duration_formatted"))
                .and_then(Value::as_str)
                .unwrap_or("0:00")
                .to_string();
            let duration_ms = info
                .and_then(|i| i.get("duration_milliseconds"))
                .and_then(Value::as_u64)
                .unwrap_or(0);

            let track_item = TrackLookupItem {
                path: mpd_uri.clone(),
                track_index: track_idx,
                track_no,
                disc_no,
                title,
                artist,
                duration,
                duration_ms,
                album_id: id.to_string(),
            };

            self.track_lookup.insert(mpd_uri.clone(), track_item);
            track_uris.push(mpd_uri);
        }

        track_uris
    }

    fn ingest_images(
        &mut self,
        album: &Value,
        album_dir_canon: &Path,
    ) -> (Option<Value>, Option<Value>, Vec<(String, String)>) {
        let mut cover_namespaces = Vec::new();
        let main_cover = album
            .get("images")
            .and_then(|c| c.get("main"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first());

        let cover_path_val = main_cover
            .and_then(|m| m.pointer("/source/file/path"))
            .cloned();
        let cover_hash_val = main_cover.and_then(|m| m.get("address")).cloned();

        let Some(images) = album.get("images").and_then(Value::as_object) else {
            return (cover_path_val, cover_hash_val, cover_namespaces);
        };

        for (ns, entries) in images {
            let Some(entries_arr) = entries.as_array() else {
                continue;
            };
            for entry in entries_arr {
                let Some(addr) = entry.get("address").and_then(Value::as_str) else {
                    continue;
                };
                let Some(p_str) =
                    entry.pointer("/source/file/path").and_then(Value::as_str)
                else {
                    continue;
                };
                let fit_str =
                    entry.get("fit").and_then(Value::as_str).unwrap_or("inside");
                let filter_str = entry
                    .get("filter")
                    .and_then(Value::as_str)
                    .unwrap_or("catmullrom");
                let width = entry
                    .get("width")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(1080);
                let height = entry
                    .get("height")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(1080);

                let fit_mode = parse_fit_mode(fit_str).unwrap_or(FitMode::Inside);
                let filter = parse_filter(filter_str).unwrap_or(FilterType::CatmullRom);

                self.cover_lookup.insert(
                    addr.to_string(),
                    CoverLookupItem {
                        album_dir: album_dir_canon.to_path_buf(),
                        path: p_str.to_string(),
                        fit_mode,
                        target_width: width,
                        target_height: height,
                        filter,
                    },
                );
                cover_namespaces.push((ns.clone(), addr.to_string()));
            }
        }

        (cover_path_val, cover_hash_val, cover_namespaces)
    }

    pub fn ingest_pre_evaluated(
        &mut self,
        album_dir: &Path,
        id: &str,
        metadata_json: &str,
        eval_res: Value,
        music_directory: &Path,
    ) -> Result<()> {
        let album_dir_canon = album_dir
            .canonicalize()
            .unwrap_or_else(|_| album_dir.to_path_buf());
        self.validate_and_cleanup_paths(&album_dir_canon, id)?;

        let uid = self.next_uid;
        self.next_uid += 1;

        let parsed: Value = serde_json::from_str(metadata_json)?;

        let Some(album) = parsed.get("album") else {
            return Ok(());
        };

        let entity_id = album
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let tags = album.get("tags").cloned().unwrap_or_else(|| json!({}));
        let objects = album.get("objects").cloned().unwrap_or_else(|| json!({}));
        let info = objects.get("info").cloned().unwrap_or_else(|| json!({}));
        let images = album.get("images").cloned().unwrap_or_else(|| json!({}));

        let duration_ms = info
            .get("duration_milliseconds")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total_tracks = info
            .get("total_tracks")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total_discs = info.get("total_discs").and_then(Value::as_u64).unwrap_or(1);

        let track_uris = album.get("tracks").and_then(Value::as_array).map_or_else(
            Vec::new,
            |tracks| {
                self.ingest_tracks_and_lookup(
                    tracks,
                    &album_dir_canon,
                    &entity_id,
                    music_directory,
                )
            },
        );

        let (cover_path_val, cover_hash_val, cover_namespaces) =
            self.ingest_images(album, &album_dir_canon);

        let cover_path = cover_path_val
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or("cover.jpg")
            .to_string();

        let title = tags
            .get("album")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string();

        let artist = tags
            .get("albumartist")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string();

        let entry = json!({
            "id": entity_id,
            "album": tags.get("album"),
            "albumartist": tags.get("albumartist"),
            "date": tags.get("date"),
            "cover_path": cover_path_val,
            "cover_hash": cover_hash_val,
            "covers": images,
            "images": images,
            "duration_formatted": info.get("duration_formatted"),
            "total_discs": info.get("total_discs"),
            "total_tracks": info.get("total_tracks"),
            "virtual": info.get("virtual"),
            "keys": tags,
            "tags": tags,
            "objects": objects,
            "colors": album.get("colors")
        });

        let dict_str = serde_json::to_string(&entry)?;
        let dict_raw = RawValue::from_string(dict_str)?;

        let record = AlbumRecord {
            id: entity_id.clone(),
            path: album_dir_canon.clone(),
            title,
            artist,
            cover_path,
            raw_json: Arc::from(metadata_json),
            dict_raw,
            total_discs,
            total_tracks,
            duration_ms,
            track_uris,
            cover_namespaces,
        };

        self.albums.insert(uid, record);
        self.id_to_uid.insert(entity_id, uid);
        self.path_to_uid.insert(album_dir_canon, uid);
        self.evaluated_logic.insert(uid, eval_res);

        Ok(())
    }

    #[must_use]
    pub fn get_album_record(&self, id: &str) -> Option<&AlbumRecord> {
        let uid = self.id_to_uid.get(id)?;
        self.albums.get(uid)
    }

    #[must_use]
    pub fn get_album_path(&self, id: &str) -> Option<&Path> {
        self.get_album_record(id).map(|r| r.path.as_path())
    }

    #[must_use]
    pub fn get_album_id_by_path(&self, path: &Path) -> Option<&str> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let uid = self.path_to_uid.get(&canon)?;
        self.albums.get(uid).map(|r| r.id.as_str())
    }

    #[must_use]
    pub fn get_album_dict(&self, id: &str) -> Option<&RawValue> {
        self.get_album_record(id).map(|r| &*r.dict_raw)
    }

    #[must_use]
    pub fn dict_snapshot(&self) -> HashMap<String, Box<RawValue>> {
        self.albums
            .values()
            .map(|a| (a.id.clone(), a.dict_raw.clone()))
            .collect()
    }

    #[must_use]
    pub fn get_album_json(&self, id: &str) -> Option<Arc<str>> {
        let record = self.get_album_record(id)?;
        Some(Arc::clone(&record.raw_json))
    }
}
