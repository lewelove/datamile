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
    pub root_type: String,
    pub id: String,
    /// Entity target: the album directory, or a single media file.
    pub path: PathBuf,
    pub lock_path: PathBuf,
    pub title: String,
    pub artist: String,
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

/// One compiled entity ready to enter the read model.
pub struct EntityIngest<'a> {
    pub root_type: &'a str,
    pub target: &'a Path,
    pub lock_path: &'a Path,
    pub id: &'a str,
    pub lock_json: &'a str,
    pub eval_res: Value,
    pub media_directory: &'a Path,
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
    pub lock_to_uid: HashMap<PathBuf, u32>,
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
            lock_to_uid: HashMap::new(),
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
                    && let Ok(res) =
                        engine.evaluate_entity_logic(&record.root_type, &parsed)
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
        self.lock_to_uid.clear();
        self.albums.clear();
        self.cover_lookup.clear();
        self.next_uid = 1;
        self.track_lookup.clear();
        self.evaluated_logic.clear();
    }

    pub fn remove_by_lock(&mut self, lock_path: &Path) -> Option<String> {
        let uid = self.lock_to_uid.remove(lock_path)?;
        let id = self.albums.get(&uid)?.id.clone();
        self.remove_uid(uid);
        Some(id)
    }

    fn remove_uid(&mut self, uid: u32) {
        self.evaluated_logic.remove(&uid);

        let Some(record) = self.albums.remove(&uid) else {
            return;
        };

        self.id_to_uid.remove(&record.id);
        self.lock_to_uid.remove(&record.lock_path);

        for uri in &record.track_uris {
            self.track_lookup.remove(uri);
        }

        for (_, addr) in &record.cover_namespaces {
            self.cover_lookup.remove(addr);
        }
    }

    #[must_use]
    pub fn uid_for_id(&self, id: &str) -> Option<u32> {
        self.id_to_uid.get(id).copied()
    }

    fn validate_and_cleanup(
        &mut self,
        lock_path: &Path,
        target: &Path,
        id: &str,
    ) -> Result<()> {
        if let Some(existing_uid) = self.id_to_uid.get(id).copied()
            && let Some(existing) = self.albums.get(&existing_uid)
        {
            if existing.lock_path != lock_path && existing.lock_path.exists() {
                return Err(DatamileError::DuplicateMediaId {
                    id: id.to_string(),
                    path_a: existing.lock_path.clone(),
                    path_b: lock_path.to_path_buf(),
                }
                .into());
            }

            self.remove_uid(existing_uid);
        }

        if let Some(old_uid) = self.lock_to_uid.get(lock_path).copied() {
            self.remove_uid(old_uid);
        }

        let _ = target;

        Ok(())
    }

    fn ingest_tracks_and_lookup(
        &mut self,
        tracks: &[Value],
        album_dir_canon: &Path,
        id: &str,
        media_directory: &Path,
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
            let mpd_uri_raw = abs_track_path.strip_prefix(media_directory).map_or_else(
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
    ) -> Vec<(String, String)> {
        let mut cover_namespaces = Vec::new();

        let Some(images) = album.get("images").and_then(Value::as_object) else {
            return cover_namespaces;
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

        cover_namespaces
    }

    pub fn ingest_entity(&mut self, ingest: EntityIngest<'_>) -> Result<()> {
        let EntityIngest {
            root_type,
            target,
            lock_path,
            id,
            lock_json: metadata_json,
            eval_res,
            media_directory,
        } = ingest;
        let album_dir_canon = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());
        let lock_path = lock_path
            .canonicalize()
            .unwrap_or_else(|_| lock_path.to_path_buf());
        self.validate_and_cleanup(&lock_path, &album_dir_canon, id)?;

        let parsed: Value = serde_json::from_str(metadata_json)?;

        let Some(album) = parsed.get(root_type) else {
            return Ok(());
        };

        let entity_id = album
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let uid = self.next_uid;
        self.next_uid += 1;
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

        let track_uris = child_with_locations(album).map_or_else(Vec::new, |tracks| {
            self.ingest_tracks_and_lookup(
                tracks,
                &album_dir_canon,
                &entity_id,
                media_directory,
            )
        });

        let cover_namespaces = self.ingest_images(album, &album_dir_canon);

        let title = first_tag(&tags, &["album", "title", "name"]);
        let artist = first_tag(&tags, &["albumartist", "author", "artist"]);

        let entry = json!({
            "id": entity_id,
            "tags": tags,
            "objects": objects,
            "images": images,
            "location": album.get("location")
        });

        let dict_str = serde_json::to_string(&entry)?;
        let dict_raw = RawValue::from_string(dict_str)?;

        let record = AlbumRecord {
            root_type: root_type.to_string(),
            id: entity_id.clone(),
            path: album_dir_canon.clone(),
            lock_path: lock_path.clone(),
            title,
            artist,
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
        self.lock_to_uid.insert(lock_path, uid);
        self.evaluated_logic.insert(uid, eval_res);

        Ok(())
    }

    #[must_use]
    pub fn get_album_record(&self, id: &str) -> Option<&AlbumRecord> {
        let uid = self.uid_for_id(id)?;
        self.albums.get(&uid)
    }

    #[must_use]
    pub fn get_entity(&self, root_type: &str, id: &str) -> Option<&AlbumRecord> {
        let uid = self.id_to_uid.get(id)?;
        let record = self.albums.get(uid)?;
        (record.root_type == root_type).then_some(record)
    }

    #[must_use]
    pub fn get_album_path(&self, id: &str) -> Option<&Path> {
        self.get_album_record(id).map(|r| r.path.as_path())
    }

    #[must_use]
    pub fn get_album_id_by_target(&self, target: &Path) -> Option<&str> {
        self.albums
            .values()
            .find(|record| record.path == target)
            .map(|record| record.id.as_str())
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

    #[must_use]
    pub fn get_entity_json(&self, root_type: &str, id: &str) -> Option<Arc<str>> {
        let record = self.get_entity(root_type, id)?;
        Some(Arc::clone(&record.raw_json))
    }

    /// Playback URIs in compiled order, optionally limited to one disc.
    #[must_use]
    pub fn track_uris_for(&self, id: &str, disc: Option<u32>) -> Vec<String> {
        let Some(record) = self.get_album_record(id) else {
            return Vec::new();
        };

        record
            .track_uris
            .iter()
            .filter(|uri| {
                disc.is_none_or(|wanted| {
                    self.track_lookup
                        .get(*uri)
                        .is_some_and(|track| track.disc_no == wanted)
                })
            })
            .cloned()
            .collect()
    }
}

/// First child array whose elements carry their own provenance.
fn child_with_locations(entity: &Value) -> Option<&Vec<Value>> {
    entity.as_object()?.iter().find_map(|(key, value)| {
        if matches!(key.as_str(), "tags" | "objects" | "images" | "location") {
            return None;
        }
        let items = value.as_array()?;
        items
            .iter()
            .any(|item| item.get("location").is_some())
            .then_some(items)
    })
}

fn first_tag(tags: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| tags.get(*key).and_then(Value::as_str))
        .unwrap_or("Unknown")
        .to_string()
}

#[cfg(test)]
mod tests;
