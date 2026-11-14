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

fn scoped_map<T>(map: &indexmap::IndexMap<String, T>, root_type: Option<&str>) -> Value
where
    T: serde::Serialize,
{
    let mut out = Map::new();

    for (key, def) in map {
        if !defined_for(def, root_type) {
            continue;
        }
        if let Ok(value) = serde_json::to_value(def) {
            out.insert(key.clone(), value);
        }
    }

    Value::Object(out)
}

fn defined_for<T>(def: &T, root_type: Option<&str>) -> bool
where
    T: serde::Serialize,
{
    let Some(root_type) = root_type else {
        return true;
    };

    let Ok(Value::Object(object)) = serde_json::to_value(def) else {
        return true;
    };

    object
        .get("type")
        .and_then(Value::as_str)
        .is_none_or(|declared| declared == root_type)
}

fn ordered_keys<T>(
    map: &indexmap::IndexMap<String, T>,
    order: &[String],
    root_type: Option<&str>,
) -> Vec<String>
where
    T: serde::Serialize,
{
    let declared: Vec<String> = if order.is_empty() {
        map.keys().cloned().collect()
    } else {
        order.to_vec()
    };

    declared
        .into_iter()
        .filter(|key| map.get(key).is_some_and(|def| defined_for(def, root_type)))
        .collect()
}

fn logic_payload(manifest: &LogicManifest, root_type: Option<&str>) -> Value {
    json!({
        "libraries": scoped_map(&manifest.libraries, root_type),
        "libraries_order": ordered_keys(&manifest.libraries, &manifest.libraries_order, root_type),
        "filters": scoped_map(&manifest.filters, root_type),
        "filters_order": ordered_keys(&manifest.filters, &manifest.filters_order, root_type),
        "groupers": scoped_map(&manifest.groupers, root_type),
        "groupers_order": ordered_keys(&manifest.groupers, &manifest.groupers_order, root_type),
        "orders": scoped_map(&manifest.orders, root_type),
        "orders_order": ordered_keys(&manifest.orders, &manifest.orders_order, root_type),
        "shelves": scoped_map(&manifest.shelves, root_type),
        "shelves_order": ordered_keys(&manifest.shelves, &manifest.shelves_order, root_type),
        "cabinets": scoped_map(&manifest.cabinets, root_type),
        "cabinets_order": ordered_keys(&manifest.cabinets, &manifest.cabinets_order, root_type),
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

fn ensure_root_type(logic: &LogicEngine, root: &str) -> Result<(), Response> {
    if logic.media_types.contains_key(root) {
        return Ok(());
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "Unknown media type" })),
    )
        .into_response())
}

pub async fn list_media(State(state): State<Arc<AppState>>) -> Response {
    let logic = state.logic.read().await;
    let config = state.config.read().await;

    Json(json!({
        "types": types_payload(&logic, None),
        "logic": logic_payload(&logic.manifest, None),
        "actions": actions_payload(&config),
        "config": config_payload(&config),
    }))
    .into_response()
}

pub async fn get_media(
    Path(root_type): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let logic = state.logic.read().await;
    if let Err(response) = ensure_root_type(&logic, &root_type) {
        return response;
    }

    let config = state.config.read().await;

    Json(json!({
        "types": types_payload(&logic, Some(&root_type)),
        "logic": logic_payload(&logic.manifest, Some(&root_type)),
        "actions": actions_payload(&config),
        "config": config_payload(&config),
    }))
    .into_response()
}

pub async fn list_items(
    Path(root_type): Path<String>,
    Query(query): Query<ItemsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let logic = state.logic.read().await;
    if let Err(response) = ensure_root_type(&logic, &root_type) {
        return response;
    }

    let library = query
        .library
        .clone()
        .unwrap_or_else(|| default_library(&logic.manifest));
    let order = reverse_order(query.reverse.as_deref());

    let ids = match query.shelf.as_deref() {
        Some(shelf) => {
            let sort = query.sort.as_deref().filter(|key| *key != "original");
            logic.request_shelf_view(&root_type, shelf, sort, order)
        }
        None => {
            let group = query.group.as_deref().zip(query.group_val.as_deref());
            logic.request_view(&ViewQuery {
                root_type: &root_type,
                library: &library,
                library_filter: query.library_filter.as_deref(),
                group,
                sort: query.sort.as_deref(),
                order,
            })
        }
    };

    let view = View::parse(query.view.as_deref());
    let items = match view {
        View::None => Vec::new(),
        _ => logic.view_entries(&root_type, &ids, view.sections()),
    };

    Json(json!({ "ids": ids, "items": items })).into_response()
}

pub async fn get_item(
    Path(root_type): Path<String>,
    Query(query): Query<ItemQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let logic = state.logic.read().await;
    if let Err(response) = ensure_root_type(&logic, &root_type) {
        return response;
    }

    let Some(json) = logic.entity_json(&root_type, &query.id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    (
        [(header::CONTENT_TYPE, "application/json")],
        json.to_string(),
    )
        .into_response()
}

pub async fn get_groups(
    Path((root_type, grouper)): Path<(String, String)>,
    Query(query): Query<GroupQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let logic = state.logic.read().await;
    if let Err(response) = ensure_root_type(&logic, &root_type) {
        return response;
    }

    let library = query
        .library
        .clone()
        .unwrap_or_else(|| default_library(&logic.manifest));
    let tree = logic.request_group(
        &root_type,
        &library,
        query.library_filter.as_deref(),
        &grouper,
    );

    Json(tree).into_response()
}
