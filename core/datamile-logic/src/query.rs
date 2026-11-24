use roaring::RoaringBitmap;
use serde_json::Value;

use crate::engine::{LogicEngine, TypeIndex};
use crate::sort::SortOrder;

impl LogicEngine {
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the view request mirrors the WebSocket fields one to one"
    )]
    pub fn request_view(
        &self,
        media_type: &str,
        library: &str,
        library_filter: Option<&str>,
        sort: &str,
        filter_key: Option<&str>,
        filter_val: Option<&str>,
        order: SortOrder,
    ) -> Vec<String> {
        let Some(index) = self.types.get(media_type) else {
            return Vec::new();
        };

        let mut final_mask = view_mask(index, library);

        if let Some(lf) = library_filter
            && let Some(f_mask) = index.filters_cache.get(lf)
        {
            final_mask &= f_mask;
        }

        if let (Some(fk), Some(fv)) = (filter_key, filter_val) {
            if fk == "search" {
                let needle = fv.to_lowercase();
                let mut searched = RoaringBitmap::new();
                for uid in &final_mask {
                    if let Some(record) = index.entities.get(&uid)
                        && entity_matches(record, &needle)
                    {
                        searched.insert(uid);
                    }
                }
                final_mask = searched;
            } else if let Some(grouper_vals) = index.groupers_cache.get(fk) {
                if let Some(grouper_mask) = grouper_vals.get(fv) {
                    final_mask &= grouper_mask;
                } else {
                    final_mask.clear();
                }
            }
        }

        let fallback: Vec<u32> = index.entities.keys().copied().collect();
        let sorted_uids = index.orders_cache.get(sort).unwrap_or(&fallback);

        let mut res: Vec<String> = sorted_uids
            .iter()
            .filter(|uid| final_mask.contains(**uid))
            .filter_map(|uid| index.entities.get(uid).map(|record| record.id.clone()))
            .collect();

        if order == SortOrder::Reverse {
            res.reverse();
        }
        res
    }

    #[must_use]
    pub fn request_shelf_view(
        &self,
        media_type: &str,
        shelf_key: &str,
        order_key: Option<&str>,
        order: SortOrder,
    ) -> Vec<String> {
        let Some(index) = self.types.get(media_type) else {
            return Vec::new();
        };
        let Some(native_uids) = index.shelves_cache.get(shelf_key) else {
            return Vec::new();
        };

        let mut uids = order_key.map_or_else(
            || native_uids.clone(),
            |ok| {
                if ok != "original"
                    && let Some(sorted_uids) = index.orders_cache.get(ok)
                {
                    let mut set = RoaringBitmap::new();
                    for &uid in native_uids {
                        set.insert(uid);
                    }
                    return sorted_uids
                        .iter()
                        .filter(|uid| set.contains(**uid))
                        .copied()
                        .collect();
                }
                native_uids.clone()
            },
        );

        if order == SortOrder::Reverse {
            uids.reverse();
        }

        uids.iter()
            .filter_map(|uid| index.entities.get(uid).map(|record| record.id.clone()))
            .collect()
    }

    #[must_use]
    pub fn request_group(
        &self,
        media_type: &str,
        library: &str,
        library_filter: Option<&str>,
        grouper: &str,
    ) -> Vec<Value> {
        let Some(index) = self.types.get(media_type) else {
            return Vec::new();
        };

        let mut view_mask = view_mask(index, library);
        if let Some(lf) = library_filter
            && let Some(f_mask) = index.filters_cache.get(lf)
        {
            view_mask &= f_mask;
        }
        if view_mask.is_empty() {
            return Vec::new();
        }

        self.build_grouper_items(index, grouper, &view_mask)
    }

    #[must_use]
    pub fn find_ids(&self, media_type: &str, query_str: &str) -> Vec<String> {
        let Some(index) = self.types.get(media_type) else {
            return Vec::new();
        };

        let q = query_str.trim();
        if q.is_empty() {
            return index.entities.values().map(|a| a.id.clone()).collect();
        }

        if let Some(uids) = index.libraries_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| index.entities.get(&uid).map(|a| a.id.clone()))
                .collect();
        }
        if let Some(uids) = index.filters_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| index.entities.get(&uid).map(|a| a.id.clone()))
                .collect();
        }
        if let Some(uids) = index.shelves_cache.get(q) {
            return uids
                .iter()
                .filter_map(|uid| index.entities.get(uid).map(|a| a.id.clone()))
                .collect();
        }

        let needle = q.to_lowercase();
        index
            .entities
            .values()
            .filter(|record| entity_matches(record, &needle))
            .map(|record| record.id.clone())
            .collect()
    }
}

/// Library mask, falling back to every entity of the type for the default
/// `library` view when the manifest declares no libraries at all.
fn view_mask(index: &TypeIndex, library: &str) -> RoaringBitmap {
    if index.libraries_cache.is_empty() && library == "library" {
        return index.entities.keys().copied().collect();
    }

    index
        .libraries_cache
        .get(library)
        .cloned()
        .unwrap_or_default()
}

fn entity_matches(record: &crate::engine::EntityRecord, needle: &str) -> bool {
    record.id.to_lowercase().contains(needle)
        || record.search_texts.iter().any(|text| text.contains(needle))
}
