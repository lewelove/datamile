use std::borrow::Cow;

use roaring::RoaringBitmap;
use serde_json::Value;

use super::{LogicEngine, SortOrder};

/// One browse request, always scoped to a single root type.
pub struct ViewQuery<'a> {
    pub root_type: &'a str,
    pub library: &'a str,
    pub library_filter: Option<&'a str>,
    pub group: Option<(&'a str, &'a str)>,
    pub sort: Option<&'a str>,
    pub order: SortOrder,
}

impl LogicEngine {
    pub fn request_view(&self, query: &ViewQuery<'_>) -> Vec<String> {
        let mut mask = self.resolve_view_mask(query.library, query.library_filter);

        if let Some((grouper, value)) = query.group {
            match self
                .groupers_cache
                .get(grouper)
                .and_then(|vals| vals.get(value))
            {
                Some(group_mask) => {
                    mask &= group_mask;
                }
                None => {
                    mask.clear();
                }
            }
        }

        let sorted_uids: Cow<'_, [u32]> = query
            .sort
            .and_then(|sort| self.orders_cache.get(sort))
            .map_or_else(
                || Cow::Owned(self.uids_for_type(query.root_type)),
                |uids| Cow::Borrowed(uids.as_slice()),
            );

        let mut ids: Vec<String> = sorted_uids
            .iter()
            .filter(|uid| mask.contains(**uid))
            .filter_map(|uid| self.ids_for_uid(query.root_type, *uid))
            .collect();

        if query.order == SortOrder::Reverse {
            ids.reverse();
        }

        ids
    }

    pub fn request_shelf_view(
        &self,
        root_type: &str,
        shelf_key: &str,
        order_key: Option<&str>,
        order: SortOrder,
    ) -> Vec<String> {
        let empty = Vec::new();
        let native_uids = self.shelves_cache.get(shelf_key).unwrap_or(&empty);

        let mut uids = order_key.map_or_else(
            || native_uids.clone(),
            |key| {
                if key != "original" && self.orders_cache.contains_key(key) {
                    let sorted = &self.orders_cache[key];
                    let native: RoaringBitmap = native_uids.iter().copied().collect();
                    sorted
                        .iter()
                        .filter(|uid| native.contains(**uid))
                        .copied()
                        .collect()
                } else {
                    native_uids.clone()
                }
            },
        );

        if order == SortOrder::Reverse {
            uids.reverse();
        }

        uids.iter()
            .filter_map(|uid| self.ids_for_uid(root_type, *uid))
            .collect()
    }

    pub fn request_group(
        &self,
        root_type: &str,
        library: &str,
        library_filter: Option<&str>,
        grouper: &str,
    ) -> Vec<Value> {
        let view_mask = self.resolve_view_mask(library, library_filter)
            & self.uid_mask_for_type(root_type);
        if view_mask.is_empty() {
            return Vec::new();
        }

        let lua_engine = libdatamile::lua::LuaEngine::new().ok();
        if let Some(ref engine) = lua_engine
            && self.config_path.exists()
        {
            let _ = engine.evaluate_config(&self.config_path);
        }

        self.build_grouper_items(lua_engine.as_ref(), grouper, &view_mask)
    }

    /// Exact id match across every root type.
    pub fn targets_for_id(&self, id: &str) -> Vec<(String, String)> {
        self.albums
            .values()
            .filter(|record| record.id == id)
            .map(|record| (record.root_type.clone(), record.id.clone()))
            .collect()
    }

    /// Library/filter/shelf keys first, then a case-insensitive text match.
    pub fn find_targets(&self, query_str: &str) -> Vec<(String, String)> {
        let query = query_str.trim();
        if query.is_empty() {
            return self.all_targets();
        }

        if let Some(uids) = self.libraries_cache.get(query) {
            return self.targets_for_uids(uids);
        }
        if let Some(uids) = self.filters_cache.get(query) {
            return self.targets_for_uids(uids);
        }
        if let Some(uids) = self.shelves_cache.get(query) {
            return self.targets_for_slice(uids);
        }

        let needle = query.to_lowercase();
        let mut targets: Vec<(String, String)> = self
            .albums
            .values()
            .filter(|record| {
                record.id.to_lowercase().contains(&needle)
                    || record.title.to_lowercase().contains(&needle)
                    || record.artist.to_lowercase().contains(&needle)
            })
            .map(|record| (record.root_type.clone(), record.id.clone()))
            .collect();
        targets.sort();

        targets
    }

    fn all_targets(&self) -> Vec<(String, String)> {
        let mut targets: Vec<(String, String)> = self
            .albums
            .values()
            .map(|record| (record.root_type.clone(), record.id.clone()))
            .collect();
        targets.sort();

        targets
    }

    fn targets_for_uids(&self, uids: &RoaringBitmap) -> Vec<(String, String)> {
        uids.iter()
            .filter_map(|uid| self.albums.get(&uid))
            .map(|record| (record.root_type.clone(), record.id.clone()))
            .collect()
    }

    fn targets_for_slice(&self, uids: &[u32]) -> Vec<(String, String)> {
        uids.iter()
            .filter_map(|uid| self.albums.get(uid))
            .map(|record| (record.root_type.clone(), record.id.clone()))
            .collect()
    }

    fn ids_for_uid(&self, root_type: &str, uid: u32) -> Option<String> {
        let record = self.albums.get(&uid)?;
        (record.root_type == root_type).then(|| record.id.clone())
    }

    fn uids_for_type(&self, root_type: &str) -> Vec<u32> {
        let mut uids: Vec<(String, u32)> = self
            .albums
            .iter()
            .filter(|(_, record)| record.root_type == root_type)
            .map(|(uid, record)| (record.id.clone(), *uid))
            .collect();
        uids.sort();

        uids.into_iter().map(|(_, uid)| uid).collect()
    }

    fn uid_mask_for_type(&self, root_type: &str) -> RoaringBitmap {
        self.albums
            .iter()
            .filter(|(_, record)| record.root_type == root_type)
            .map(|(uid, _)| *uid)
            .collect()
    }
}
