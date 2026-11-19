use roaring::RoaringBitmap;
use serde_json::Value;

use super::{LogicEngine, SortOrder};

impl LogicEngine {
    pub fn request_view(
        &self,
        library: &str,
        library_filter: Option<&str>,
        sort: &str,
        filter_key: Option<&str>,
        filter_val: Option<&str>,
        order: SortOrder,
    ) -> Vec<String> {
        let empty_bitmap = RoaringBitmap::new();
        let library_mask = self.libraries_cache.get(library).unwrap_or(&empty_bitmap);
        let mut final_mask = library_mask.clone();

        if let Some(lf) = library_filter
            && let Some(f_mask) = self.filters_cache.get(lf)
        {
            final_mask &= f_mask;
        }

        if let (Some(fk), Some(fv)) = (filter_key, filter_val) {
            if fk == "search" {
                let needle = fv.to_lowercase();
                let mut searched = RoaringBitmap::new();
                for uid in &final_mask {
                    if let Some(record) = self.albums.get(&uid) {
                        let title = record.title.to_lowercase();
                        let artist = record.artist.to_lowercase();
                        if title.contains(&needle) || artist.contains(&needle) {
                            searched.insert(uid);
                        }
                    }
                }
                final_mask = searched;
            } else if let Some(grouper_vals) = self.groupers_cache.get(fk) {
                if let Some(grouper_mask) = grouper_vals.get(fv) {
                    final_mask &= grouper_mask;
                } else {
                    final_mask.clear();
                }
            }
        }

        let fallback_storage = self.albums.keys().copied().collect::<Vec<_>>();
        let sorted_uids = self.orders_cache.get(sort).unwrap_or(&fallback_storage);

        let mut res: Vec<String> = sorted_uids
            .iter()
            .filter(|uid| final_mask.contains(**uid))
            .filter_map(|uid| self.albums.get(uid).map(|a| a.id.clone()))
            .collect();

        if order == SortOrder::Reverse {
            res.reverse();
        }
        res
    }

    pub fn request_shelf_view(
        &self,
        shelf_key: &str,
        order_key: Option<&str>,
        order: SortOrder,
    ) -> Vec<String> {
        let empty_vec = Vec::new();
        let native_uids = self.shelves_cache.get(shelf_key).unwrap_or(&empty_vec);

        let mut uids = order_key.map_or_else(
            || native_uids.clone(),
            |ok| {
                if ok != "original" && self.orders_cache.contains_key(ok) {
                    let sorted_uids = &self.orders_cache[ok];
                    let mut set = roaring::RoaringBitmap::new();
                    for &uid in native_uids {
                        set.insert(uid);
                    }
                    sorted_uids
                        .iter()
                        .filter(|uid| set.contains(**uid))
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
            .filter_map(|uid| self.albums.get(uid).map(|a| a.id.clone()))
            .collect()
    }

    pub fn request_group(
        &self,
        library: &str,
        library_filter: Option<&str>,
        grouper: &str,
    ) -> Vec<Value> {
        let view_mask = self.resolve_view_mask(library, library_filter);
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

    pub fn find_ids(&self, query_str: &str) -> Vec<String> {
        let q = query_str.trim();
        if q.is_empty() {
            return self.albums.values().map(|a| a.id.clone()).collect();
        }

        if let Some(uids) = self.libraries_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| self.albums.get(&uid).map(|a| a.id.clone()))
                .collect();
        }
        if let Some(uids) = self.filters_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| self.albums.get(&uid).map(|a| a.id.clone()))
                .collect();
        }
        if let Some(uids) = self.shelves_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| self.albums.get(uid).map(|a| a.id.clone()))
                .collect();
        }

        let needle = q.to_lowercase();
        self.albums
            .values()
            .filter_map(|record| {
                if record.id.to_lowercase().contains(&needle)
                    || record.title.to_lowercase().contains(&needle)
                    || record.artist.to_lowercase().contains(&needle)
                {
                    return Some(record.id.clone());
                }
                None
            })
            .collect()
    }
}
