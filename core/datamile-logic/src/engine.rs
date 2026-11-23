use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datamile_lua_domain::{FormattedGrouperResult, GrouperFormatContext, LogicManifest};
use roaring::RoaringBitmap;
use serde::Serialize;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::sort::SortKey;

/// Grouper formatter supplied by the caller, so this crate stays free of Lua.
pub type GrouperFormatter<'a> =
    dyn Fn(&str, &GrouperFormatContext<'_>) -> FormattedGrouperResult + 'a;

/// Fallback used when no Lua engine is available: the raw value is the label.
#[must_use]
pub fn plain_grouper_formatter(
    _grouper: &str,
    ctx: &GrouperFormatContext<'_>,
) -> FormattedGrouperResult {
    FormattedGrouperResult {
        label: ctx.value.to_string(),
        sublabel: None,
        sort: serde_json::json!(ctx.value),
        parent: None,
    }
}

#[derive(Clone, Debug)]
pub struct ImageLookupItem {
    pub entity_dir: PathBuf,
    pub path: String,
    pub fit: String,
    pub filter: String,
    pub target_width: u32,
    pub target_height: u32,
}

/// One indexed media item, whatever its media type is.
#[derive(Clone, Debug)]
pub struct EntityRecord {
    pub media_type: String,
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub subtitle: String,
    pub main_image_path: Option<String>,
    pub raw_json: Arc<str>,
    pub dict_raw: Box<RawValue>,
    pub total_discs: u64,
    pub total_tracks: u64,
    pub duration_ms: u64,
    pub playable_uris: Vec<String>,
    pub image_namespaces: Vec<(String, String)>,
}

/// One playable node inside an entity, keyed in the lookup by its MPD URI.
#[derive(Clone, Debug, Serialize)]
pub struct PlayableLookupItem {
    pub media_type: String,
    #[serde(rename = "albumId")]
    pub media_id: String,
    #[serde(rename = "trackIndex")]
    pub node_index: usize,
    pub track_no: u32,
    pub disc_no: u32,
    pub title: String,
    pub artist: String,
    pub duration: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    pub path: String,
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
    pub entities: HashMap<u32, EntityRecord>,
    pub id_to_uid: HashMap<String, u32>,
    pub path_to_uid: HashMap<PathBuf, u32>,
    pub image_lookup: HashMap<String, ImageLookupItem>,
    pub playable_lookup: HashMap<String, PlayableLookupItem>,
    pub(crate) evaluated_logic: HashMap<u32, Value>,
}

impl LogicEngine {
    #[must_use]
    pub fn new(config_path: PathBuf, manifest: LogicManifest) -> Self {
        Self {
            config_path,
            manifest,
            libraries_cache: HashMap::new(),
            filters_cache: HashMap::new(),
            groupers_cache: HashMap::new(),
            orders_cache: HashMap::new(),
            shelves_cache: HashMap::new(),
            next_uid: 1,
            entities: HashMap::new(),
            id_to_uid: HashMap::new(),
            path_to_uid: HashMap::new(),
            image_lookup: HashMap::new(),
            playable_lookup: HashMap::new(),
            evaluated_logic: HashMap::new(),
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
        evaluate: &dyn Fn(&Value) -> anyhow::Result<Value>,
        formatter: &GrouperFormatter<'_>,
    ) {
        let mut new_evaluated = HashMap::new();
        for (&uid, record) in &self.entities {
            if let Ok(parsed) = serde_json::from_str::<Value>(&record.raw_json)
                && let Ok(res) = evaluate(&parsed)
            {
                new_evaluated.insert(uid, res);
            }
        }

        self.evaluated_logic = new_evaluated;
        self.config_path = config_path.to_path_buf();
        self.manifest = manifest;
        self.build_cache(formatter);
    }

    pub fn clear(&mut self) {
        self.libraries_cache.clear();
        self.filters_cache.clear();
        self.groupers_cache.clear();
        self.orders_cache.clear();
        self.shelves_cache.clear();
        self.id_to_uid.clear();
        self.path_to_uid.clear();
        self.entities.clear();
        self.image_lookup.clear();
        self.next_uid = 1;
        self.playable_lookup.clear();
        self.evaluated_logic.clear();
    }

    pub fn remove_entity_by_path(&mut self, path: &Path) -> Option<String> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let uid = self.path_to_uid.remove(&canon)?;
        let id = self.entities.get(&uid)?.id.clone();
        self.remove_entity_by_id(&id);
        Some(id)
    }

    pub fn remove_entity_by_id(&mut self, id: &str) {
        let Some(uid) = self.id_to_uid.remove(id) else {
            return;
        };

        self.evaluated_logic.remove(&uid);

        if let Some(record) = self.entities.remove(&uid) {
            self.path_to_uid.remove(&record.path);

            for uri in &record.playable_uris {
                self.playable_lookup.remove(uri);
            }

            for (_, addr) in &record.image_namespaces {
                self.image_lookup.remove(addr);
            }
        }
    }

    /// Drops any record that would collide with the incoming id or path.
    fn validate_and_cleanup_paths(
        &mut self,
        entity_dir_canon: &Path,
        id: &str,
    ) -> anyhow::Result<()> {
        if let Some(existing_record) = self.entity_record(id)
            && existing_record.path != entity_dir_canon
        {
            if existing_record.path.exists() {
                anyhow::bail!(
                    "Duplicate media id '{id}': defined in both {} and {}",
                    existing_record.path.display(),
                    entity_dir_canon.display()
                );
            }
            let old_path = existing_record.path.clone();
            self.remove_entity_by_path(&old_path);
        }

        if let Some(old_uid) = self.path_to_uid.get(entity_dir_canon)
            && let Some(old_record) = self.entities.get(old_uid)
            && old_record.id != id
        {
            let old_id = old_record.id.clone();
            self.remove_entity_by_id(&old_id);
        }

        self.remove_entity_by_id(id);

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
        images: Vec<(String, ImageLookupItem)>,
        eval_res: Value,
    ) -> anyhow::Result<()> {
        self.validate_and_cleanup_paths(&entity.path, &entity.id)?;

        let uid = self.next_uid;
        self.next_uid += 1;

        for item in playable_items {
            self.playable_lookup.insert(item.path.clone(), item);
        }
        for (address, item) in images {
            self.image_lookup.insert(address, item);
        }

        self.id_to_uid.insert(entity.id.clone(), uid);
        self.path_to_uid.insert(entity.path.clone(), uid);
        self.evaluated_logic.insert(uid, eval_res);
        self.entities.insert(uid, entity);

        Ok(())
    }

    #[must_use]
    pub fn entity_record(&self, id: &str) -> Option<&EntityRecord> {
        let uid = self.id_to_uid.get(id)?;
        self.entities.get(uid)
    }

    #[must_use]
    pub fn entity_path(&self, id: &str) -> Option<&Path> {
        self.entity_record(id).map(|r| r.path.as_path())
    }

    #[must_use]
    pub fn entity_id_by_path(&self, path: &Path) -> Option<&str> {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let uid = self.path_to_uid.get(&canon)?;
        self.entities.get(uid).map(|r| r.id.as_str())
    }

    #[must_use]
    pub fn entity_dict(&self, id: &str) -> Option<&RawValue> {
        self.entity_record(id).map(|r| &*r.dict_raw)
    }

    #[must_use]
    pub fn dict_snapshot(&self) -> HashMap<String, Box<RawValue>> {
        self.entities
            .values()
            .map(|a| (a.id.clone(), a.dict_raw.clone()))
            .collect()
    }

    #[must_use]
    pub fn entity_json(&self, id: &str) -> Option<Arc<str>> {
        let record = self.entity_record(id)?;
        Some(Arc::clone(&record.raw_json))
    }

    /// Sort key of a shelf/order entry, defaulting to an empty string.
    pub(crate) fn sort_key_for(&self, uid: u32, section: &str, key: &str) -> SortKey {
        let default_key = serde_json::json!("");
        let raw_key = self
            .evaluated_logic
            .get(&uid)
            .and_then(|eval| eval.get(section))
            .and_then(|sorts| sorts.get(key))
            .unwrap_or(&default_key);
        crate::sort::value_to_sort_key(raw_key)
    }
}
