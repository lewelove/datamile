use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default)]
pub struct GrouperFormatContext<'a> {
    pub value: &'a str,
    pub count: u64,
    pub pct: f64,
    pub duration_millis: u64,
    pub total_tracks: u64,
    pub total_discs: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FormattedGrouperResult {
    pub label: String,
    pub sublabel: Option<String>,
    pub sort: serde_json::Value,
    pub parent: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FilterDef {
    pub label: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct GrouperDef {
    pub label: String,
    #[serde(default)]
    pub index: Option<bool>,
    #[serde(default)]
    pub count: Option<bool>,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct OrderDef {
    pub label: String,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LibraryDef {
    pub label: String,
    #[serde(default)]
    pub filters: Vec<String>,
    #[serde(default)]
    pub groupers: Vec<String>,
    #[serde(default)]
    pub orders: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_filters: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_groupers: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_orders: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct ShelfDef {
    pub label: String,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct CabinetDef {
    pub label: String,
    #[serde(default)]
    pub shelves: Vec<String>,
    #[serde(default)]
    pub orders: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_shelves: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_orders: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LogicManifest {
    pub filters: IndexMap<String, FilterDef>,
    pub groupers: IndexMap<String, GrouperDef>,
    pub orders: IndexMap<String, OrderDef>,
    pub libraries: IndexMap<String, LibraryDef>,
    pub shelves: IndexMap<String, ShelfDef>,
    pub cabinets: IndexMap<String, CabinetDef>,
    #[serde(default)]
    pub filters_order: Vec<String>,
    #[serde(default)]
    pub groupers_order: Vec<String>,
    #[serde(default)]
    pub orders_order: Vec<String>,
    #[serde(default)]
    pub libraries_order: Vec<String>,
    #[serde(default)]
    pub shelves_order: Vec<String>,
    #[serde(default)]
    pub cabinets_order: Vec<String>,
}

fn sort_ordered_map<V>(map: &mut IndexMap<String, V>, order: &mut Vec<String>) {
    if order.is_empty() {
        *order = map.keys().cloned().collect();
    } else {
        map.sort_by_cached_key(|k, _| {
            order.iter().position(|x| x == k).unwrap_or(usize::MAX)
        });
    }
}

impl LogicManifest {
    pub fn normalize(&mut self) {
        self.normalize_ordering();
        self.normalize_grouper_defaults();
        self.resolve_allowed_targets();
    }

    fn normalize_ordering(&mut self) {
        sort_ordered_map(&mut self.filters, &mut self.filters_order);
        sort_ordered_map(&mut self.groupers, &mut self.groupers_order);
        sort_ordered_map(&mut self.orders, &mut self.orders_order);
        sort_ordered_map(&mut self.libraries, &mut self.libraries_order);
        sort_ordered_map(&mut self.shelves, &mut self.shelves_order);
        sort_ordered_map(&mut self.cabinets, &mut self.cabinets_order);
    }

    fn normalize_grouper_defaults(&mut self) {
        for g in self.groupers.values_mut() {
            let idx = g.index.unwrap_or(false);
            g.index = Some(idx);
            if g.count.is_none() {
                g.count = Some(!idx);
            }
        }
    }

    fn resolve_allowed_targets(&mut self) {
        for library in self.libraries.values_mut() {
            library.allowed_filters = library
                .filters
                .iter()
                .filter(|f| self.filters.contains_key(*f))
                .cloned()
                .collect();

            library.allowed_groupers = library
                .groupers
                .iter()
                .filter(|g| self.groupers.contains_key(*g))
                .cloned()
                .collect();

            library.allowed_orders = library
                .orders
                .iter()
                .filter(|o| self.orders.contains_key(*o))
                .cloned()
                .collect();
        }

        for cabinet in self.cabinets.values_mut() {
            cabinet.allowed_shelves = cabinet
                .shelves
                .iter()
                .filter(|s| self.shelves.contains_key(*s))
                .cloned()
                .collect();

            cabinet.allowed_orders = cabinet
                .orders
                .iter()
                .filter(|o| self.orders.contains_key(*o))
                .cloned()
                .collect();
        }
    }
}
