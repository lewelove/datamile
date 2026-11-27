use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datamile_cache::EntityKey;
use datamile_lua_domain::LogicManifest;
use roaring::RoaringBitmap;
use serde::Serialize;
use serde_json::Value;
use serde_json::value::RawValue;

/// Rendering parameters shared by every owner of one content address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSpecMeta {
    pub fit: String,
    pub filter: String,
    pub target_width: u32,
    pub target_height: u32,
}

/// The cold-cache recovery data for one content address: how to render it and
/// every source image that produced it.
#[derive(Clone, Debug)]
pub struct CachedImageLookup {
    pub spec: ImageSpecMeta,
    pub sources: Vec<(EntityKey, PathBuf)>,
}

/// One image upsert: content address, rendering spec, absolute source path.
pub type ImageUpsert = (String, ImageSpecMeta, PathBuf);

/// One indexed media item, whatever its media type is.
#[derive(Clone, Debug)]
pub struct EntityRecord {
    pub media_type: String,
    pub id: String,
    pub path: PathBuf,
    pub raw_json: Arc<str>,
    pub dict_raw: Box<RawValue>,
    /// Lowercased strings the search filter matches against.
    pub search_texts: Vec<String>,
    pub playable_uris: Vec<String>,
    pub image_namespaces: Vec<(String, String)>,
}

/// One playable node inside an entity, keyed in the lookup by its MPD URI.
#[derive(Clone, Debug, Serialize)]
pub struct PlayableLookupItem {
    pub media_type: String,
    pub media_id: String,
    pub node_index: usize,
    /// Schema path from the root, e.g. `[("track", 0)]`.
    pub node_path: Vec<(String, usize)>,
    pub path: String,
    /// Node tags plus `objects.info`, so consumers need no lock parsing.
    pub meta: serde_json::Value,
}

/// Everything the engine knows about one media type. Ids, paths, bitmaps and
/// Lua results never leak across types.
#[derive(Default)]
pub struct TypeIndex {
    pub media_type: String,
    pub entities: HashMap<u32, EntityRecord>,
    pub id_to_uid: HashMap<String, u32>,
    pub path_to_uid: HashMap<PathBuf, u32>,
    pub libraries_cache: HashMap<String, RoaringBitmap>,
    pub filters_cache: HashMap<String, RoaringBitmap>,
    pub groupers_cache: HashMap<String, HashMap<String, RoaringBitmap>>,
    pub orders_cache: HashMap<String, Vec<u32>>,
    pub shelves_cache: HashMap<String, Vec<u32>>,
    pub evaluated_logic: HashMap<u32, Value>,
}

pub struct LogicEngine {
    pub config_path: PathBuf,
    pub manifest: LogicManifest,
    pub types: HashMap<String, TypeIndex>,
    pub image_lookup: HashMap<String, CachedImageLookup>,
    /// Every owner of one playable URI: the same file may compile under several
    /// media types.
    pub playable_lookup: HashMap<String, Vec<PlayableLookupItem>>,
    pub(crate) next_uid: u32,
}

impl LogicEngine {
    #[must_use]
    pub fn new(config_path: PathBuf, manifest: LogicManifest) -> Self {
        Self {
            config_path,
            manifest,
            types: HashMap::new(),
            image_lookup: HashMap::new(),
            playable_lookup: HashMap::new(),
            next_uid: 1,
        }
    }

    /// Swaps the config manifest and re-runs per-entity logic.
    ///
    /// `evaluate` is the caller's Lua hook; a failing entity is simply skipped
    /// so one broken lock cannot take the whole index down.
    pub fn reload_manifest(
        &mut self,
        config_path: &Path,
        manifest: LogicManifest,
        evaluate: &dyn Fn(&str, &Value) -> anyhow::Result<Value>,
    ) {
        for (media_type, index) in &mut self.types {
            let mut new_evaluated = HashMap::new();
            for (&uid, record) in &index.entities {
                // The compile pass evaluates the inner `<media_type>` entity;
                // `raw_json` is the whole lock file, wrapper included.
                let Ok(parsed) = serde_json::from_str::<Value>(&record.raw_json) else {
                    continue;
                };
                let Some(entity) = parsed.get(media_type) else {
                    continue;
                };
                if let Ok(res) = evaluate(media_type, entity) {
                    new_evaluated.insert(uid, res);
                }
            }
            index.evaluated_logic = new_evaluated;
        }

        self.config_path = config_path.to_path_buf();
        self.manifest = manifest;
        self.build_cache();
    }

    pub fn clear(&mut self) {
        self.types.clear();
        self.image_lookup.clear();
        self.playable_lookup.clear();
        self.next_uid = 1;
    }

    #[must_use]
    pub fn media_types(&self) -> Vec<(String, usize)> {
        let mut types: Vec<(String, usize)> = self
            .types
            .iter()
            .map(|(media_type, index)| (media_type.clone(), index.entities.len()))
            .collect();
        types.sort();
        types
    }

    /// Every entity across every media type, for whole-library payloads.
    pub fn all_entities(&self) -> impl Iterator<Item = &EntityRecord> {
        self.types
            .values()
            .flat_map(|index| index.entities.values())
    }

    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.types.values().map(|index| index.entities.len()).sum()
    }

    pub fn remove_entity_by_type_and_path(
        &mut self,
        media_type: &str,
        path: &Path,
    ) -> Option<String> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let index = self.types.get(media_type)?;
        let uid = *index.path_to_uid.get(&canon)?;
        let id = index.entities.get(&uid)?.id.clone();

        self.remove_entity_by_id(media_type, &id);
        Some(id)
    }

    pub fn remove_entity_by_id(&mut self, media_type: &str, id: &str) {
        let Some(index) = self.types.get_mut(media_type) else {
            return;
        };
        let Some(uid) = index.id_to_uid.remove(id) else {
            return;
        };

        index.evaluated_logic.remove(&uid);

        if let Some(record) = index.entities.remove(&uid) {
            index.path_to_uid.remove(&record.path);

            let key = EntityKey {
                media_type: media_type.to_string(),
                dir: record.path.clone(),
            };

            for uri in &record.playable_uris {
                self.forget_playable_owner(uri, media_type, &record.id);
            }

            for (_, addr) in &record.image_namespaces {
                self.forget_image_owner(addr, &key);
            }
        }
    }

    /// Drops one media type's ownership of a playable URI, keeping the other
    /// owners of the same file.
    fn forget_playable_owner(&mut self, uri: &str, media_type: &str, id: &str) {
        let Some(items) = self.playable_lookup.get_mut(uri) else {
            return;
        };

        items.retain(|item| item.media_type != media_type || item.media_id != id);
        if items.is_empty() {
            self.playable_lookup.remove(uri);
        }
    }

    /// Drops one entity's source from a content address, keeping every other
    /// owner that renders the same address.
    fn forget_image_owner(&mut self, address: &str, key: &EntityKey) {
        let Some(entry) = self.image_lookup.get_mut(address) else {
            return;
        };

        entry.sources.retain(|(owner, _)| owner != key);
        if entry.sources.is_empty() {
            self.image_lookup.remove(address);
        }
    }

    /// Drops any record that would collide with the incoming id or path.
    fn validate_and_cleanup_paths(
        &mut self,
        media_type: &str,
        entity_dir_canon: &Path,
        id: &str,
    ) -> anyhow::Result<()> {
        let existing_path = self.types.get(media_type).and_then(|index| {
            index
                .id_to_uid
                .get(id)
                .and_then(|uid| index.entities.get(uid))
                .map(|record| record.path.clone())
        });

        if let Some(existing_path) = existing_path
            && existing_path != entity_dir_canon
        {
            if existing_path.exists() {
                anyhow::bail!(
                    "Duplicate media id '{id}' for '{media_type}': defined in both {} and {}",
                    existing_path.display(),
                    entity_dir_canon.display()
                );
            }
            self.remove_entity_by_type_and_path(media_type, &existing_path);
        }

        let old_id = self.types.get(media_type).and_then(|index| {
            index
                .path_to_uid
                .get(entity_dir_canon)
                .and_then(|uid| index.entities.get(uid))
                .map(|record| record.id.clone())
        });
        if let Some(old_id) = old_id
            && old_id != id
        {
            self.remove_entity_by_id(media_type, &old_id);
        }

        self.remove_entity_by_id(media_type, id);

        Ok(())
    }

    /// Inserts or replaces one entity plus its playable and image lookups.
    ///
    /// `images` is keyed by rendered address; `entity.image_namespaces` keeps the
    /// namespace-to-address mapping so a namespace wipe can remove its addresses.
    pub fn upsert_entity(
        &mut self,
        entity: EntityRecord,
        playable_items: Vec<PlayableLookupItem>,
        images: Vec<ImageUpsert>,
        eval_res: Value,
    ) -> anyhow::Result<()> {
        let media_type = entity.media_type.clone();
        self.validate_and_cleanup_paths(&media_type, &entity.path, &entity.id)?;

        let uid = self.next_uid;
        self.next_uid += 1;

        for item in playable_items {
            let owners = self.playable_lookup.entry(item.path.clone()).or_default();
            let same_owner = owners.iter_mut().find(|owner| {
                owner.media_type == item.media_type && owner.media_id == item.media_id
            });
            match same_owner {
                Some(owner) => *owner = item,
                None => owners.push(item),
            }
        }

        let key = EntityKey {
            media_type: media_type.clone(),
            dir: entity.path.clone(),
        };
        // The address digests the rendering spec, so an address that already
        // exists was rendered with the same parameters; only its owners grow.
        for (address, spec, source) in images {
            let entry =
                self.image_lookup
                    .entry(address)
                    .or_insert_with(|| CachedImageLookup {
                        spec,
                        sources: Vec::new(),
                    });

            match entry.sources.iter_mut().find(|(owner, _)| *owner == key) {
                Some(slot) => slot.1 = source,
                None => entry.sources.push((key.clone(), source)),
            }
        }

        let index = self
            .types
            .entry(media_type.clone())
            .or_insert_with(|| TypeIndex {
                media_type: media_type.clone(),
                ..TypeIndex::default()
            });
        index.id_to_uid.insert(entity.id.clone(), uid);
        index.path_to_uid.insert(entity.path.clone(), uid);
        index.evaluated_logic.insert(uid, eval_res);
        index.entities.insert(uid, entity);

        Ok(())
    }

    #[must_use]
    pub fn entity_record(&self, media_type: &str, id: &str) -> Option<&EntityRecord> {
        let index = self.types.get(media_type)?;
        let uid = index.id_to_uid.get(id)?;
        index.entities.get(uid)
    }

    #[must_use]
    pub fn entity_path(&self, media_type: &str, id: &str) -> Option<&Path> {
        self.entity_record(media_type, id).map(|r| r.path.as_path())
    }

    #[must_use]
    pub fn entity_id_by_path(&self, media_type: &str, path: &Path) -> Option<&str> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let index = self.types.get(media_type)?;
        let uid = index.path_to_uid.get(&canon)?;
        index.entities.get(uid).map(|r| r.id.as_str())
    }

    #[must_use]
    pub fn entity_dict(&self, media_type: &str, id: &str) -> Option<&RawValue> {
        self.entity_record(media_type, id).map(|r| &*r.dict_raw)
    }

    #[must_use]
    pub fn dict_snapshot_by_type(
        &self,
        media_type: &str,
    ) -> HashMap<String, Box<RawValue>> {
        self.types
            .get(media_type)
            .map_or_else(HashMap::new, |index| {
                index
                    .entities
                    .values()
                    .map(|record| (record.id.clone(), record.dict_raw.clone()))
                    .collect()
            })
    }

    #[must_use]
    pub fn entity_json(&self, media_type: &str, id: &str) -> Option<Arc<str>> {
        let record = self.entity_record(media_type, id)?;
        Some(Arc::clone(&record.raw_json))
    }
}
