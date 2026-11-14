use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use libdatamile::lua::LogicManifest;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::server::logic::{LogicEngine, SortOrder, ViewQuery};
use crate::server::state::{AppConfig, AppState};

const DEFAULT_LIBRARY: &str = "library";
const VIEW_NONE: &str = "none";

#[derive(Deserialize)]
pub struct ItemsQuery {
    library: Option<String>,
    library_filter: Option<String>,
    group: Option<String>,
    group_val: Option<String>,
    shelf: Option<String>,
    sort: Option<String>,
    reverse: Option<String>,
    view: Option<String>,
}

#[derive(Deserialize)]
pub struct ItemQuery {
    id: String,
}

#[derive(Deserialize)]
pub struct GroupQuery {
    library: Option<String>,
    library_filter: Option<String>,
}

enum View {
    Full,
    None,
    Sections(Vec<String>),
}

impl View {
    fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::Full;
        };

        let sections: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect();

        if sections.is_empty() || sections == [VIEW_NONE] {
            return Self::None;
        }

        Self::Sections(sections)
    }

    fn sections(&self) -> Option<&[String]> {
        match self {
            Self::Sections(sections) => Some(sections),
            Self::Full | Self::None => None,
        }
    }
}

fn default_library(manifest: &LogicManifest) -> String {
    if manifest.libraries.contains_key(DEFAULT_LIBRARY) {
        return DEFAULT_LIBRARY.to_string();
    }

    manifest
        .libraries_order
        .first()
        .or_else(|| manifest.libraries.keys().next())
        .cloned()
        .unwrap_or_else(|| DEFAULT_LIBRARY.to_string())
}

fn reverse_order(raw: Option<&str>) -> SortOrder {
    if raw == Some("true") {
        SortOrder::Reverse
    } else {
        SortOrder::Forward
    }
}

/// A definition is scoped when its declared `type` matches the requested root
/// type; absent `type` means valid for every root type.
fn defined_for<T>(
    def: &T,
    root_type: Option<&str>,
    type_of: fn(&T) -> Option<&str>,
) -> bool {
    let Some(root_type) = root_type else {
        return true;
    };

    type_of(def).is_none_or(|declared| declared == root_type)
}

fn scoped_map<T: serde::Serialize>(
    map: &indexmap::IndexMap<String, T>,
    root_type: Option<&str>,
    type_of: fn(&T) -> Option<&str>,
) -> Value {
    let mut out = Map::new();

    for (key, def) in map {
        if !defined_for(def, root_type, type_of) {
            continue;
        }
        if let Ok(value) = serde_json::to_value(def) {
            out.insert(key.clone(), value);
        }
    }

    Value::Object(out)
}

fn ordered_keys<T>(
    map: &indexmap::IndexMap<String, T>,
    order: &[String],
    root_type: Option<&str>,
    type_of: fn(&T) -> Option<&str>,
) -> Vec<String> {
    let declared: Vec<String> = if order.is_empty() {
        map.keys().cloned().collect()
    } else {
        order.to_vec()
    };

    declared
        .into_iter()
        .filter(|key| {
            map.get(key)
                .is_some_and(|def| defined_for(def, root_type, type_of))
        })
        .collect()
}

/// Build `<name>` and `<name>_order` in one call.
fn scoped_field<T: serde::Serialize>(
    map: &indexmap::IndexMap<String, T>,
    order: &[String],
    root_type: Option<&str>,
    type_of: fn(&T) -> Option<&str>,
) -> (Value, Vec<String>) {
    (
        scoped_map(map, root_type, type_of),
        ordered_keys(map, order, root_type, type_of),
    )
}

fn logic_payload(manifest: &LogicManifest, root_type: Option<&str>) -> Value {
    let (libraries, libraries_order) = scoped_field(
        &manifest.libraries,
        &manifest.libraries_order,
        root_type,
        |d| d.r#type.as_deref(),
    );
    let (filters, filters_order) =
        scoped_field(&manifest.filters, &manifest.filters_order, root_type, |d| {
            d.r#type.as_deref()
        });
    let (groupers, groupers_order) = scoped_field(
        &manifest.groupers,
        &manifest.groupers_order,
        root_type,
        |d| d.r#type.as_deref(),
    );
    let (orders, orders_order) =
        scoped_field(&manifest.orders, &manifest.orders_order, root_type, |d| {
            d.r#type.as_deref()
        });
    let (shelves, shelves_order) =
        scoped_field(&manifest.shelves, &manifest.shelves_order, root_type, |d| {
            d.r#type.as_deref()
        });
    let (cabinets, cabinets_order) = scoped_field(
        &manifest.cabinets,
        &manifest.cabinets_order,
        root_type,
        |d| d.r#type.as_deref(),
    );

    json!({
        "libraries": libraries,
        "libraries_order": libraries_order,
        "filters": filters,
        "filters_order": filters_order,
        "groupers": groupers,
        "groupers_order": groupers_order,
        "orders": orders,
        "orders_order": orders_order,
        "shelves": shelves,
        "shelves_order": shelves_order,
        "cabinets": cabinets,
        "cabinets_order": cabinets_order,
    })
}

fn actions_payload(config: &AppConfig) -> Value {
    let mut out = Map::new();

    for (key, action) in &config.actions {
        if let Ok(value) = serde_json::to_value(action) {
            out.insert(key.clone(), value);
        }
    }

    Value::Object(out)
}

fn config_payload(config: &AppConfig) -> Value {
    json!({ "covers": config.covers })
}

fn types_payload(logic: &LogicEngine, root_type: Option<&str>) -> Value {
    let mut names: Vec<&String> = logic.media_types.keys().collect();
    names.sort();

    let mut out = Map::new();
    for name in names {
        if root_type.is_some_and(|wanted| wanted != name) {
            continue;
        }

        let Some(schema) = logic.media_types.get(name) else {
            continue;
        };

        out.insert(
            name.clone(),
            json!({
                "dir": schema.dir,
                "count": logic.entity_count(name),
                "children": schema.children,
            }),
        );
    }

    Value::Object(out)
}

fn not_found_type() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Unknown media type" })),
    )
        .into_response()
}

pub async fn list_media(State(state): State<Arc<AppState>>) -> Response {
    let payload = {
        let logic = state.logic.read().await;
        let types = types_payload(&logic, None);
        let logic_json = logic_payload(&logic.manifest, None);
        drop(logic);

        let config = state.config.read().await;
        let actions = actions_payload(&config);
        let config_json = config_payload(&config);
        drop(config);

        json!({
            "types": types,
            "logic": logic_json,
            "actions": actions,
            "config": config_json,
        })
    };

    Json(payload).into_response()
}

pub async fn media_info(
    Path(root_type): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let payload = {
        let logic = state.logic.read().await;
        if !logic.media_types.contains_key(&root_type) {
            return not_found_type();
        }
        let types = types_payload(&logic, Some(&root_type));
        let logic_json = logic_payload(&logic.manifest, Some(&root_type));
        drop(logic);

        let config = state.config.read().await;
        let actions = actions_payload(&config);
        let config_json = config_payload(&config);
        drop(config);

        json!({
            "types": types,
            "logic": logic_json,
            "actions": actions,
            "config": config_json,
        })
    };

    Json(payload).into_response()
}

pub async fn list_items(
    Path(root_type): Path<String>,
    Query(query): Query<ItemsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let payload = {
        let logic = state.logic.read().await;
        if !logic.media_types.contains_key(&root_type) {
            return not_found_type();
        }

        let library = query
            .library
            .clone()
            .unwrap_or_else(|| default_library(&logic.manifest));
        let order = reverse_order(query.reverse.as_deref());

        let ids = if let Some(shelf) = query.shelf.as_deref() {
            let sort = query.sort.as_deref().filter(|key| *key != "original");
            logic.request_shelf_view(&root_type, shelf, sort, order)
        } else {
            let group = query.group.as_deref().zip(query.group_val.as_deref());
            logic.request_view(&ViewQuery {
                root_type: &root_type,
                library: &library,
                library_filter: query.library_filter.as_deref(),
                group,
                sort: query.sort.as_deref(),
                order,
            })
        };

        let view = View::parse(query.view.as_deref());
        let items = match view {
            View::None => Vec::new(),
            _ => logic.view_entries(&root_type, &ids, view.sections()),
        };
        drop(logic);

        json!({ "ids": ids, "items": items })
    };

    Json(payload).into_response()
}

pub async fn media_item(
    Path(root_type): Path<String>,
    Query(query): Query<ItemQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let lock = {
        let logic = state.logic.read().await;
        if !logic.media_types.contains_key(&root_type) {
            return not_found_type();
        }
        logic
            .entity(&root_type, &query.id)
            .map(|record| Arc::clone(&record.lock))
    };

    let Some(lock) = lock else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match serde_json::to_string(&*lock) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(e) => {
            log::error!("Failed to serialize entity '{}': {e}", query.id);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn media_groups(
    Path((root_type, grouper)): Path<(String, String)>,
    Query(query): Query<GroupQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let tree = {
        let logic = state.logic.read().await;
        if !logic.media_types.contains_key(&root_type) {
            return not_found_type();
        }

        let library = query
            .library
            .clone()
            .unwrap_or_else(|| default_library(&logic.manifest));
        logic.request_group(
            &root_type,
            &library,
            query.library_filter.as_deref(),
            &grouper,
        )
    };

    Json(tree).into_response()
}

#[cfg(test)]
mod tests;
