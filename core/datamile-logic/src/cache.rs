use std::collections::{HashMap, HashSet};

use roaring::RoaringBitmap;
use serde_json::{Value, json};

use crate::engine::{EntityRecord, GrouperFormatter, LogicEngine};
use crate::sort::{SortKey, SortOrder, value_to_sort_key};

fn value_to_display_string(val: &Value) -> String {
    val.as_str()
        .map_or_else(|| val.to_string(), ToString::to_string)
}

fn sort_val_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Array(arr) => arr
            .iter()
            .map(sort_val_to_string)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(k1, _), (k2, _)| {
                let n1 = k1.parse::<usize>().ok();
                let n2 = k2.parse::<usize>().ok();
                match (n1, n2) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => k1.cmp(k2),
                }
            });
            entries
                .into_iter()
                .map(|(_, v)| sort_val_to_string(v))
                .collect::<Vec<_>>()
                .join(" ")
        }
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

struct RawGrouperNode {
    value: String,
    label: String,
    sublabel: Option<String>,
    sort_key: SortKey,
    sort_str: String,
    count: u64,
    pct: f64,
    duration_millis: u64,
    total_tracks: u64,
    total_discs: u64,
    parent: Option<String>,
}

fn calculate_mask_metrics(
    mask: &RoaringBitmap,
    entities: &HashMap<u32, EntityRecord>,
) -> (u64, u64, u64) {
    let (mut dur, mut trk, mut dsc) = (0u64, 0u64, 0u64);
    for uid in mask {
        if let Some(record) = entities.get(&uid) {
            dur += record.duration_ms;
            trk += record.total_tracks;
            dsc += record.total_discs;
        }
    }
    (dur, trk, dsc)
}

fn calculate_pct(count: u64, view_total: f64) -> f64 {
    if view_total > 0.0 {
        let count_f = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
        (count_f / view_total * 1000.0).floor() / 10.0
    } else {
        0.0
    }
}

fn build_node_json(
    node: &RawGrouperNode,
    children_map: &mut HashMap<Option<String>, Vec<RawGrouperNode>>,
    order: SortOrder,
) -> Value {
    let mut children_json = Vec::new();
    if let Some(mut children) = children_map.remove(&Some(node.value.clone())) {
        children.sort_by(|a, b| {
            a.sort_key
                .cmp(&b.sort_key)
                .then_with(|| alphanumeric_sort::compare_str(&a.value, &b.value))
        });
        if order == SortOrder::Reverse {
            children.reverse();
        }
        for child in &children {
            children_json.push(build_node_json(child, children_map, order));
        }
    }

    json!({
        "value": node.value,
        "label": node.label,
        "sublabel": node.sublabel,
        "sort": node.sort_str,
        "count": node.count,
        "pct": node.pct,
        "duration_millis": node.duration_millis,
        "total_tracks": node.total_tracks,
        "total_discs": node.total_discs,
        "parent": node.parent,
        "children": children_json
    })
}

fn discover_parent_hierarchy(
    grouper_id: &str,
    node_bitmaps: &mut HashMap<String, RoaringBitmap>,
    view_total: f64,
    entities: &HashMap<u32, EntityRecord>,
    formatter: &GrouperFormatter<'_>,
) -> HashMap<String, String> {
    let mut parent_links: HashMap<String, String> = HashMap::new();
    let mut queue: Vec<String> = node_bitmaps.keys().cloned().collect();
    let mut visited: HashSet<String> = HashSet::new();

    while let Some(current_val) = queue.pop() {
        if !visited.insert(current_val.clone()) {
            continue;
        }

        let direct_mask = node_bitmaps.get(&current_val).cloned().unwrap_or_default();
        let count = direct_mask.len();
        let pct = calculate_pct(count, view_total);
        let (dur, trk, dsc) = calculate_mask_metrics(&direct_mask, entities);

        let fmt_ctx = datamile_lua_domain::GrouperFormatContext {
            value: &current_val,
            count,
            pct,
            duration_millis: dur,
            total_tracks: trk,
            total_discs: dsc,
        };

        let fmt_res = formatter(grouper_id, &fmt_ctx);

        if let Some(parent) = fmt_res.parent
            && parent != current_val
        {
            parent_links.insert(current_val.clone(), parent.clone());
            if !node_bitmaps.contains_key(&parent) {
                node_bitmaps.insert(parent.clone(), RoaringBitmap::new());
                queue.push(parent);
            }
        }
    }

    parent_links
}

fn propagate_child_bitmaps(
    parent_links: &HashMap<String, String>,
    node_bitmaps: &mut HashMap<String, RoaringBitmap>,
) {
    let initial_nodes: Vec<(String, RoaringBitmap)> = node_bitmaps
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    for (child_val, child_bm) in initial_nodes {
        let mut curr = parent_links.get(&child_val);
        let mut path_seen = HashSet::new();
        while let Some(parent) = curr {
            if !path_seen.insert(parent.clone()) {
                break;
            }
            if let Some(parent_bm) = node_bitmaps.get_mut(parent) {
                *parent_bm |= &child_bm;
            }
            curr = parent_links.get(parent);
        }
    }
}

fn build_raw_grouper_nodes(
    grouper_id: &str,
    node_bitmaps: &HashMap<String, RoaringBitmap>,
    view_total: f64,
    entities: &HashMap<u32, EntityRecord>,
    formatter: &GrouperFormatter<'_>,
) -> Vec<RawGrouperNode> {
    let mut raw_nodes: Vec<RawGrouperNode> = Vec::new();

    for (node_val, node_bm) in node_bitmaps {
        let count = node_bm.len();
        if count == 0 {
            continue;
        }

        let pct = calculate_pct(count, view_total);
        let (dur, trk, dsc) = calculate_mask_metrics(node_bm, entities);

        let fmt_ctx = datamile_lua_domain::GrouperFormatContext {
            value: node_val,
            count,
            pct,
            duration_millis: dur,
            total_tracks: trk,
            total_discs: dsc,
        };

        let fmt_res = formatter(grouper_id, &fmt_ctx);

        let sort_key = value_to_sort_key(&fmt_res.sort);
        let sort_str = sort_val_to_string(&fmt_res.sort);
        let valid_parent = fmt_res
            .parent
            .filter(|p| p != node_val && node_bitmaps.contains_key(p));

        raw_nodes.push(RawGrouperNode {
            value: node_val.clone(),
            label: fmt_res.label,
            sublabel: fmt_res.sublabel,
            sort_key,
            sort_str,
            count,
            pct,
            duration_millis: dur,
            total_tracks: trk,
            total_discs: dsc,
            parent: valid_parent,
        });
    }

    raw_nodes
}

fn assemble_grouper_tree(raw_nodes: Vec<RawGrouperNode>, order: SortOrder) -> Vec<Value> {
    let mut children_map: HashMap<Option<String>, Vec<RawGrouperNode>> = HashMap::new();
    for node in raw_nodes {
        children_map
            .entry(node.parent.clone())
            .or_default()
            .push(node);
    }

    let mut roots = children_map.remove(&None).unwrap_or_default();
    roots.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| alphanumeric_sort::compare_str(&a.value, &b.value))
    });

    if order == SortOrder::Reverse {
        roots.reverse();
    }

    roots
        .iter()
        .map(|r| build_node_json(r, &mut children_map, order))
        .collect()
}

impl LogicEngine {
    pub fn build_cache(&mut self, formatter: &GrouperFormatter<'_>) {
        self.libraries_cache.clear();
        self.filters_cache.clear();
        self.groupers_cache.clear();
        self.orders_cache.clear();
        self.shelves_cache.clear();

        self.populate_eval_caches();
        self.populate_groupers_cache(formatter);
        self.populate_shelves_cache();
        self.populate_orders_cache();
    }

    fn populate_eval_caches(&mut self) {
        for (&uid, eval_res) in &self.evaluated_logic {
            if let Some(libs) = eval_res.get("libraries").and_then(Value::as_object) {
                for (lib_id, is_match) in libs {
                    if is_match.as_bool().unwrap_or(false) {
                        self.libraries_cache
                            .entry(lib_id.clone())
                            .or_default()
                            .insert(uid);
                    }
                }
            }

            if let Some(filters) = eval_res.get("filters").and_then(Value::as_object) {
                for (filter_id, is_match) in filters {
                    if is_match.as_bool().unwrap_or(false) {
                        self.filters_cache
                            .entry(filter_id.clone())
                            .or_default()
                            .insert(uid);
                    }
                }
            }

            if let Some(shelves) = eval_res.get("shelves").and_then(Value::as_object) {
                for (shelf_id, is_match) in shelves {
                    if is_match.as_bool().unwrap_or(false) {
                        self.shelves_cache
                            .entry(shelf_id.clone())
                            .or_default()
                            .push(uid);
                    }
                }
            }
        }
    }

    fn collect_raw_groupers(&mut self) {
        for (&uid, eval_res) in &self.evaluated_logic {
            if let Some(groupers) = eval_res.get("groupers").and_then(Value::as_object) {
                for (grouper_id, val) in groupers {
                    let grouper_entry =
                        self.groupers_cache.entry(grouper_id.clone()).or_default();
                    if let Some(arr) = val.as_array() {
                        for item in arr {
                            let group_name = value_to_display_string(item);
                            if !group_name.is_empty() {
                                grouper_entry.entry(group_name).or_default().insert(uid);
                            }
                        }
                    } else {
                        let group_name = value_to_display_string(val);
                        if !group_name.is_empty() {
                            grouper_entry.entry(group_name).or_default().insert(uid);
                        }
                    }
                }
            }
        }
    }

    fn propagate_global_parent_unions(&mut self, formatter: &GrouperFormatter<'_>) {
        for (grouper_id, group_map) in &mut self.groupers_cache {
            let mut parent_links: HashMap<String, String> = HashMap::new();
            let mut queue: Vec<String> = group_map.keys().cloned().collect();
            let mut visited: HashSet<String> = HashSet::new();

            while let Some(current_val) = queue.pop() {
                if !visited.insert(current_val.clone()) {
                    continue;
                }

                let ctx = datamile_lua_domain::GrouperFormatContext {
                    value: &current_val,
                    count: 0,
                    pct: 0.0,
                    duration_millis: 0,
                    total_tracks: 0,
                    total_discs: 0,
                };

                let fmt_res = formatter(grouper_id, &ctx);

                if let Some(parent) = fmt_res.parent
                    && parent != current_val
                {
                    parent_links.insert(current_val.clone(), parent.clone());
                    if !group_map.contains_key(&parent) {
                        group_map.insert(parent.clone(), RoaringBitmap::new());
                        queue.push(parent);
                    }
                }
            }

            let initial_entries: Vec<(String, RoaringBitmap)> = group_map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            for (child_val, child_bm) in initial_entries {
                let mut curr = parent_links.get(&child_val);
                let mut path_seen = HashSet::new();
                while let Some(parent) = curr {
                    if !path_seen.insert(parent.clone()) {
                        break;
                    }
                    if let Some(parent_bm) = group_map.get_mut(parent) {
                        *parent_bm |= &child_bm;
                    }
                    curr = parent_links.get(parent);
                }
            }
        }
    }

    pub(crate) fn resolve_view_mask(
        &self,
        library: &str,
        filter: Option<&str>,
    ) -> RoaringBitmap {
        let empty_bitmap = RoaringBitmap::new();
        let library_mask = if self.manifest.libraries.is_empty() && library == "library" {
            let mut all_uids = RoaringBitmap::new();
            for &uid in self.entities.keys() {
                all_uids.insert(uid);
            }
            all_uids
        } else {
            self.libraries_cache
                .get(library)
                .unwrap_or(&empty_bitmap)
                .clone()
        };

        if let Some(f_name) = filter
            && let Some(f_mask) = self.filters_cache.get(f_name)
        {
            library_mask & f_mask
        } else {
            library_mask
        }
    }

    pub(crate) fn build_grouper_items(
        &self,
        formatter: &GrouperFormatter<'_>,
        grouper_id: &str,
        view_mask: &RoaringBitmap,
    ) -> Vec<Value> {
        let grouper_def = self.manifest.groupers.get(grouper_id);
        let order = if grouper_def.is_some_and(|g| g.reverse) {
            SortOrder::Reverse
        } else {
            SortOrder::Forward
        };
        let view_total = f64::from(u32::try_from(view_mask.len()).unwrap_or(0));

        let Some(groups) = self.groupers_cache.get(grouper_id) else {
            return Vec::new();
        };

        let mut node_bitmaps: HashMap<String, RoaringBitmap> = HashMap::new();
        for (group_val, group_bitmap) in groups {
            let intersection = group_bitmap & view_mask;
            if !intersection.is_empty() {
                node_bitmaps.insert(group_val.clone(), intersection);
            }
        }

        if node_bitmaps.is_empty() {
            return Vec::new();
        }

        let parent_links = discover_parent_hierarchy(
            grouper_id,
            &mut node_bitmaps,
            view_total,
            &self.entities,
            formatter,
        );

        propagate_child_bitmaps(&parent_links, &mut node_bitmaps);

        let raw_nodes = build_raw_grouper_nodes(
            grouper_id,
            &node_bitmaps,
            view_total,
            &self.entities,
            formatter,
        );

        assemble_grouper_tree(raw_nodes, order)
    }

    fn populate_groupers_cache(&mut self, formatter: &GrouperFormatter<'_>) {
        self.collect_raw_groupers();
        self.propagate_global_parent_unions(formatter);
    }

    fn populate_shelves_cache(&mut self) {
        let shelves: Vec<(String, bool)> = self
            .manifest
            .shelves
            .iter()
            .map(|(key, def)| (key.clone(), def.reverse))
            .collect();

        for (shelf_key, reverse) in shelves {
            let Some(native) = self.shelves_cache.get(&shelf_key) else {
                continue;
            };

            let mut pairs: Vec<(u32, SortKey)> = native
                .iter()
                .map(|&uid| (uid, self.sort_key_for(uid, "shelf_sorts", &shelf_key)))
                .collect();

            pairs.sort_by(|a, b| (&a.1, a.0).cmp(&(&b.1, b.0)));
            if reverse {
                pairs.reverse();
            }

            self.shelves_cache
                .insert(shelf_key, pairs.into_iter().map(|(uid, _)| uid).collect());
        }
    }

    fn populate_orders_cache(&mut self) {
        let orders: Vec<(String, bool)> = self
            .manifest
            .orders
            .iter()
            .map(|(key, def)| (key.clone(), def.reverse))
            .collect();

        for (order_id, reverse) in orders {
            let mut order_pairs: Vec<(u32, SortKey)> = Vec::new();
            for &uid in self.evaluated_logic.keys() {
                let sort_key = self.sort_key_for(uid, "orders", &order_id);
                order_pairs.push((uid, sort_key));
            }

            order_pairs.sort_by(|a, b| (&a.1, a.0).cmp(&(&b.1, b.0)));
            if reverse {
                order_pairs.reverse();
            }

            let sorted_uids: Vec<u32> =
                order_pairs.into_iter().map(|(uid, _)| uid).collect();
            self.orders_cache.insert(order_id, sorted_uids);
        }
    }
}
