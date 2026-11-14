pub mod cache;
pub mod query;
pub mod sort;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use libdatamile::error::DatamileError;
use libdatamile::images::{FilterType, FitMode, parse_filter, parse_fit_mode};
use libdatamile::lua::{LogicManifest, MediaSchema, with_evaluated_lua_vm};
pub use query::ViewQuery;
use roaring::RoaringBitmap;
use serde::Serialize;
use serde_json::{Map, Value, json};
pub use sort::{SortKey, SortOrder, value_to_sort_key};

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
    /// Parsed lockfile, cached so views never re-parse raw JSON.
    pub lock: Arc<Value>,
    pub total_discs: u64,
    pub total_tracks: u64,
    pub duration_ms: u64,
    /// Playable HTTP URIs and their child ids, in compiled order.
    pub track_uris: Vec<String>,
    pub track_ids: Vec<String>,
    pub cover_namespaces: Vec<(String, String)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackLookupItem {
    pub path: String,
    #[serde(rename = "rootType")]
    pub root_type: String,
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
    pub media_roots: &'a [PathBuf],
}

pub struct LogicEngine {
    pub config_path: PathBuf,
    pub manifest: LogicManifest,
    pub media_types: HashMap<String, MediaSchema>,
    pub(crate) libraries_cache: HashMap<String, RoaringBitmap>,
    pub(crate) filters_cache: HashMap<String, RoaringBitmap>,
    pub(crate) groupers_cache: HashMap<String, HashMap<String, RoaringBitmap>>,
    pub(crate) orders_cache: HashMap<String, Vec<u32>>,
    pub(crate) shelves_cache: HashMap<String, Vec<u32>>,
    pub(crate) next_uid: u32,
    pub albums: HashMap<u32, AlbumRecord>,
    pub id_to_uid: HashMap<(String, String), u32>,
    pub lock_to_uid: HashMap<PathBuf, u32>,
    pub cover_lookup: HashMap<String, CoverLookupItem>,
    pub track_lookup: HashMap<String, TrackLookupItem>,
    pub(crate) evaluated_logic: HashMap<u32, Value>,
}

impl LogicEngine {
    pub fn new() -> Result<Self> {
        let config_path = libdatamile::lua::resolve_config_path().unwrap_or_default();
        let (manifest, media_types) =
            with_evaluated_lua_vm(&config_path, |engine, eval| {
                Ok((eval.manifest, engine.media_schema()?))
            })?;

        Ok(Self {
            config_path,
            manifest,
            media_types,
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
        let (manifest, media_types) =
            with_evaluated_lua_vm(config_path, |engine, eval| {
                let mut new_evaluated = HashMap::new();
                for (&uid, record) in &self.albums {
                    let Some(parsed) = record.lock.get(&record.root_type) else {
                        continue;
                    };
                    if let Ok(res) =
                        engine.evaluate_entity_logic(&record.root_type, parsed)
                    {
                        new_evaluated.insert(uid, res);
                    }
                }
                self.evaluated_logic = new_evaluated;
                Ok((eval.manifest, engine.media_schema()?))
            })?;
        self.config_path = config_path.to_path_buf();
        self.manifest = manifest;
        self.media_types = media_types;
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

    pub fn remove_by_lock(&mut self, lock_path: &Path) -> Option<(String, String)> {
        let uid = self.lock_to_uid.remove(lock_path)?;
        let target = {
            let record = self.albums.get(&uid)?;
            (record.root_type.clone(), record.id.clone())
        };
        self.remove_uid(uid);
        Some(target)
    }

    fn remove_uid(&mut self, uid: u32) {
        self.evaluated_logic.remove(&uid);

        let Some(record) = self.albums.remove(&uid) else {
            return;
        };

        self.id_to_uid
            .remove(&(record.root_type.clone(), record.id.clone()));
        self.lock_to_uid.remove(&record.lock_path);

        for uri in &record.track_uris {
            self.track_lookup.remove(uri);
        }

        for (_, addr) in &record.cover_namespaces {
            self.cover_lookup.remove(addr);
        }
    }

    #[must_use]
    pub fn uid_for_id(&self, root_type: &str, id: &str) -> Option<u32> {
        self.id_to_uid
            .get(&(root_type.to_string(), id.to_string()))
            .copied()
    }

    fn validate_and_cleanup(
        &mut self,
        root_type: &str,
        lock_path: &Path,
        target: &Path,
        id: &str,
    ) -> Result<()> {
        let key = (root_type.to_string(), id.to_string());
        if let Some(existing_uid) = self.id_to_uid.get(&key).copied()
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
        root_type: &str,
        id: &str,
        media_roots: &[PathBuf],
    ) -> (Vec<String>, Vec<String>) {
        let mut track_uris = Vec::new();
        let mut track_ids = Vec::new();

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
            let mpd_uri_raw = media_roots
                .iter()
                .find_map(|root| abs_track_path.strip_prefix(root).ok())
                .map_or_else(
                    || abs_track_path.to_string_lossy().to_string(),
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
                root_type: root_type.to_string(),
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
            track_ids.push(
                track
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            );
            track_uris.push(mpd_uri);
        }

        (track_uris, track_ids)
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
            media_roots,
        } = ingest;
        let album_dir_canon = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());
        let lock_path = lock_path
            .canonicalize()
            .unwrap_or_else(|_| lock_path.to_path_buf());
        self.validate_and_cleanup(root_type, &lock_path, &album_dir_canon, id)?;

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

        let duration_ms = info
            .get("duration_milliseconds")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total_tracks = info
            .get("total_tracks")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total_discs = info.get("total_discs").and_then(Value::as_u64).unwrap_or(1);

        let (track_uris, track_ids) =
            child_with_locations(album).map_or_else(Default::default, |tracks| {
                self.ingest_tracks_and_lookup(
                    tracks,
                    &album_dir_canon,
                    root_type,
                    &entity_id,
                    media_roots,
                )
            });

        let cover_namespaces = self.ingest_images(album, &album_dir_canon);

        let title = first_tag(&tags, &["album", "title", "name"]);
        let artist = first_tag(&tags, &["albumartist", "author", "artist"]);

        let record = AlbumRecord {
            root_type: root_type.to_string(),
            id: entity_id.clone(),
            path: album_dir_canon.clone(),
            lock_path: lock_path.clone(),
            title,
            artist,
            lock: Arc::new(parsed),
            total_discs,
            total_tracks,
            duration_ms,
            track_uris,
            track_ids,
            cover_namespaces,
        };

        self.albums.insert(uid, record);
        self.id_to_uid
            .insert((root_type.to_string(), entity_id), uid);
        self.lock_to_uid.insert(lock_path, uid);
        self.evaluated_logic.insert(uid, eval_res);

        Ok(())
    }

    #[must_use]
    pub fn entity_count(&self, root_type: &str) -> usize {
        self.albums
            .values()
            .filter(|record| record.root_type == root_type)
            .count()
    }

    #[must_use]
    pub fn entity(&self, root_type: &str, id: &str) -> Option<&AlbumRecord> {
        let uid = self.uid_for_id(root_type, id)?;
        self.albums.get(&uid)
    }

    /// Parsed lockfile `{ "<type>": { ... } }` for one entity.
    #[must_use]
    pub fn lock(&self, root_type: &str, id: &str) -> Option<&Value> {
        let record = self.entity(root_type, id)?;
        Some(record.lock.as_ref())
    }

    /// Playback URIs in compiled order, optionally narrowed to child ids.
    #[must_use]
    pub fn track_uris_for(
        &self,
        root_type: &str,
        id: &str,
        track_ids: Option<&[String]>,
    ) -> Vec<String> {
        let Some(record) = self.entity(root_type, id) else {
            return Vec::new();
        };

        let Some(wanted) = track_ids else {
            return record.track_uris.clone();
        };

        wanted
            .iter()
            .filter_map(|wanted_id| {
                record
                    .track_ids
                    .iter()
                    .position(|track_id| track_id == wanted_id)
                    .map(|index| record.track_uris[index].clone())
            })
            .collect()
    }

    /// Resolve an MPD URI back to its owning root type and entity id.
    #[must_use]
    pub fn track_target(&self, uri: &str) -> Option<(String, String)> {
        let clean = uri.trim_start_matches('/');
        let track = self
            .track_lookup
            .get(clean)
            .or_else(|| self.track_lookup.get(uri))?;

        Some((track.root_type.clone(), track.album_id.clone()))
    }

    /// Filtered lock entries shaped as `{ "<type>": { ... } }`.
    #[must_use]
    pub fn view_entries(
        &self,
        root_type: &str,
        ids: &[String],
        view: Option<&[String]>,
    ) -> Vec<Value> {
        ids.iter()
            .filter_map(|id| {
                let record = self.entity(root_type, id)?;
                let entity = record.lock.get(root_type)?;
                let filtered = view.map_or_else(
                    || entity.clone(),
                    |sections| filter_sections(entity, sections),
                );

                Some(json!({ root_type: filtered }))
            })
            .collect()
    }
}

fn filter_sections(entity: &Value, sections: &[String]) -> Value {
    let mut out = Map::new();
    if let Some(id) = entity.get("id") {
        out.insert("id".to_string(), id.clone());
    }

    for section in sections {
        if let Some(value) = entity.get(section) {
            out.insert(section.clone(), value.clone());
        }
    }

    Value::Object(out)
}

/// First child array whose elements carry their own provenance.
fn child_with_locations(entity: &Value) -> Option<&Vec<Value>> {
    entity.as_object()?.iter().find_map(|(key, value)| {
        if matches!(
            key.as_str(),
            "tags" | "objects" | "images" | "location" | "files"
        ) {
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
