use roaring::RoaringBitmap;
use serde_json::Value;

use crate::engine::{LogicEngine, TypeIndex};
use crate::sort::{SortKey, SortOrder};

fn value_to_display_string(val: &Value) -> String {
    val.as_str()
        .map_or_else(|| val.to_string(), ToString::to_string)
}

/// One flat grouper bucket: the value is both the key and the label.
struct FlatBucket {
    pub value: String,
    pub sort_key: SortKey,
    pub count: u64,
}

impl LogicEngine {
    pub fn build_cache(&mut self) {
        for (media_type, index) in &mut self.types {
            populate_eval_caches(index, media_type, &self.manifest);
            populate_groupers_cache(index);
            populate_shelves_cache(index, &self.manifest);
            populate_orders_cache(index, &self.manifest);
        }
    }

    /// Flat grouper nodes for `grouper_id` intersected with `view_mask`.
    pub(crate) fn build_grouper_items(
        &self,
        index: &TypeIndex,
        grouper_id: &str,
        view_mask: &RoaringBitmap,
    ) -> Vec<Value> {
        let order = if self
            .manifest
            .groupers
            .get(grouper_id)
            .is_some_and(|g| g.reverse)
        {
            SortOrder::Reverse
        } else {
            SortOrder::Forward
        };

        let Some(groups) = index.groupers_cache.get(grouper_id) else {
            return Vec::new();
        };

        let mut buckets: Vec<FlatBucket> = groups
            .iter()
            .filter_map(|(value, bitmap)| {
                let intersection = bitmap & view_mask;
                if intersection.is_empty() {
                    return None;
                }
                Some(FlatBucket {
                    value: value.clone(),
                    sort_key: SortKey::String(value.clone()),
                    count: intersection.len(),
                })
            })
            .collect();

        buckets.sort_by(|a, b| {
            a.sort_key
                .cmp(&b.sort_key)
                .then_with(|| alphanumeric_sort::compare_str(&a.value, &b.value))
        });
        if order == SortOrder::Reverse {
            buckets.reverse();
        }

        buckets
            .into_iter()
            .map(|bucket| {
                serde_json::json!({
                    "value": bucket.value,
                    "label": bucket.value,
                    "count": bucket.count,
                    "children": []
                })
            })
            .collect()
    }
}

fn populate_eval_caches(
    index: &mut TypeIndex,
    media_type: &str,
    manifest: &datamile_lua_domain::LogicManifest,
) {
    index.libraries_cache.clear();
    index.filters_cache.clear();
    index.shelves_cache.clear();

    for (&uid, eval_res) in &index.evaluated_logic {
        if let Some(libs) = eval_res.get("libraries").and_then(Value::as_object) {
            for (lib_id, is_match) in libs {
                if !is_match.as_bool().unwrap_or(false) {
                    continue;
                }
                if manifest
                    .libraries
                    .get(lib_id)
                    .is_some_and(|def| def.media == media_type)
                {
                    index
                        .libraries_cache
                        .entry(lib_id.clone())
                        .or_default()
                        .insert(uid);
                }
            }
        }

        if let Some(filters) = eval_res.get("filters").and_then(Value::as_object) {
            for (filter_id, is_match) in filters {
                if !is_match.as_bool().unwrap_or(false) {
                    continue;
                }
                if manifest
                    .filters
                    .get(filter_id)
                    .is_some_and(|def| def.media == media_type)
                {
                    index
                        .filters_cache
                        .entry(filter_id.clone())
                        .or_default()
                        .insert(uid);
                }
            }
        }

        if let Some(shelves) = eval_res.get("shelves").and_then(Value::as_object) {
            for (shelf_id, is_match) in shelves {
                if !is_match.as_bool().unwrap_or(false) {
                    continue;
                }
                if manifest
                    .shelves
                    .get(shelf_id)
                    .is_some_and(|def| def.media == media_type)
                {
                    index
                        .shelves_cache
                        .entry(shelf_id.clone())
                        .or_default()
                        .push(uid);
                }
            }
        }
    }
}

fn populate_groupers_cache(index: &mut TypeIndex) {
    index.groupers_cache.clear();

    for (&uid, eval_res) in &index.evaluated_logic {
        let Some(groupers) = eval_res.get("groupers").and_then(Value::as_object) else {
            continue;
        };

        for (grouper_id, val) in groupers {
            let entry = index.groupers_cache.entry(grouper_id.clone()).or_default();
            if let Some(arr) = val.as_array() {
                for item in arr {
                    let group_name = value_to_display_string(item);
                    if !group_name.is_empty() {
                        entry.entry(group_name).or_default().insert(uid);
                    }
                }
            } else {
                let group_name = value_to_display_string(val);
                if !group_name.is_empty() {
                    entry.entry(group_name).or_default().insert(uid);
                }
            }
        }
    }
}

fn populate_shelves_cache(
    index: &mut TypeIndex,
    manifest: &datamile_lua_domain::LogicManifest,
) {
    let shelves: Vec<(String, bool)> = manifest
        .shelves
        .iter()
        .filter(|(_, def)| def.media == index.media_type)
        .map(|(key, def)| (key.clone(), def.reverse))
        .collect();

    for (shelf_key, reverse) in shelves {
        let Some(native) = index.shelves_cache.get(&shelf_key) else {
            continue;
        };

        let mut pairs: Vec<(u32, SortKey)> = native
            .iter()
            .map(|&uid| (uid, sort_key_for(index, uid, "shelf_sorts", &shelf_key)))
            .collect();

        pairs.sort_by(|a, b| (&a.1, a.0).cmp(&(&b.1, b.0)));
        if reverse {
            pairs.reverse();
        }

        index
            .shelves_cache
            .insert(shelf_key, pairs.into_iter().map(|(uid, _)| uid).collect());
    }
}

fn populate_orders_cache(
    index: &mut TypeIndex,
    manifest: &datamile_lua_domain::LogicManifest,
) {
    let orders: Vec<(String, bool)> = manifest
        .orders
        .iter()
        .filter(|(_, def)| def.media == index.media_type)
        .map(|(key, def)| (key.clone(), def.reverse))
        .collect();

    for (order_id, reverse) in orders {
        let mut order_pairs: Vec<(u32, SortKey)> = index
            .evaluated_logic
            .keys()
            .map(|&uid| (uid, sort_key_for(index, uid, "orders", &order_id)))
            .collect();

        order_pairs.sort_by(|a, b| (&a.1, a.0).cmp(&(&b.1, b.0)));
        if reverse {
            order_pairs.reverse();
        }

        let sorted_uids: Vec<u32> = order_pairs.into_iter().map(|(uid, _)| uid).collect();
        index.orders_cache.insert(order_id, sorted_uids);
    }
}

fn sort_key_for(index: &TypeIndex, uid: u32, section: &str, key: &str) -> SortKey {
    let default_key = serde_json::json!("");
    let raw_key = index
        .evaluated_logic
        .get(&uid)
        .and_then(|eval| eval.get(section))
        .and_then(|sorts| sorts.get(key))
        .unwrap_or(&default_key);
    crate::sort::value_to_sort_key(raw_key)
}
