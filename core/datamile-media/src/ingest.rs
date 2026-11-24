use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use datamile_logic::{EntityRecord, ImageLookupItem, PlayableLookupItem, tag_text};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::schema::{MediaNodeSchema, MediaRootSchema};

const DEFAULT_IMAGE_DIM: u32 = 1080;

type ImageEntries = Vec<(String, ImageLookupItem)>;
type ImageNamespaces = Vec<(String, String)>;

pub struct ParsedEntity {
    pub entity: EntityRecord,
    pub playables: Vec<PlayableLookupItem>,
    pub images: ImageEntries,
}

/// Every scalar/list tag value, lowercased, for generic search.
fn search_texts(tags: Option<&Value>) -> Vec<String> {
    let Some(tags) = tags.and_then(Value::as_object) else {
        return Vec::new();
    };
    tags.values()
        .filter_map(|value| tag_text(Some(value)))
        .map(|text| text.to_lowercase())
        .collect()
}

fn mpd_uri(root: &Path, rel: &str, music_directory: Option<&Path>) -> String {
    let absolute = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        root.join(rel)
    };
    let base = music_directory.unwrap_or(root);
    let raw = absolute.strip_prefix(base).map_or_else(
        |_| absolute.to_string_lossy().to_string(),
        |p| p.to_string_lossy().to_string(),
    );
    raw.trim_start_matches('/').to_string()
}

fn playable_meta(node: &Value) -> Value {
    let mut meta = node
        .get("tags")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if let Some(info) = node.pointer("/objects/info") {
        let duration = info.get("duration_formatted").cloned();
        let duration_ms = info.get("duration_milliseconds").cloned();
        meta.insert("info".to_string(), info.clone());
        if let Some(duration) = duration {
            meta.insert("duration".to_string(), duration);
        }
        if let Some(duration) = duration_ms {
            meta.insert("duration_ms".to_string(), duration);
        }
    }

    Value::Object(meta)
}

#[allow(
    clippy::too_many_arguments,
    reason = "walk state is threaded explicitly to avoid a mutable visitor struct"
)]
fn walk_nodes(
    nodes: &Value,
    schema_nodes: &BTreeMap<String, MediaNodeSchema>,
    root: &Path,
    media_type: &str,
    id: &str,
    music_directory: Option<&Path>,
    path_acc: &mut Vec<(String, usize)>,
    out: &mut Vec<PlayableLookupItem>,
) {
    let Some(nodes) = nodes.as_object() else {
        return;
    };

    for (node_name, items) in nodes {
        let Some(schema) = schema_nodes.get(node_name) else {
            continue;
        };
        let Some(items) = items.as_array() else {
            continue;
        };

        for (index, node) in items.iter().enumerate() {
            path_acc.push((node_name.clone(), index));

            if schema.playable
                && node.pointer("/location/kind").and_then(Value::as_str) == Some("file")
                && let Some(rel) = node.pointer("/location/path").and_then(Value::as_str)
                && !rel.is_empty()
            {
                out.push(PlayableLookupItem {
                    media_type: media_type.to_string(),
                    media_id: id.to_string(),
                    node_index: index,
                    node_path: path_acc.clone(),
                    path: mpd_uri(root, rel, music_directory),
                    meta: playable_meta(node),
                });
            }

            if let Some(children) = node.get("nodes") {
                walk_nodes(
                    children,
                    &schema.nodes,
                    root,
                    media_type,
                    id,
                    music_directory,
                    path_acc,
                    out,
                );
            }

            path_acc.pop();
        }
    }
}

fn parse_images(entity: &Value, root: &Path) -> (ImageEntries, ImageNamespaces) {
    let mut images = Vec::new();
    let mut namespaces = Vec::new();

    let Some(entries_by_ns) = entity.get("images").and_then(Value::as_object) else {
        return (images, namespaces);
    };

    for (namespace, entries) in entries_by_ns {
        let Some(entries) = entries.as_array() else {
            continue;
        };

        for entry in entries {
            let Some(address) = entry.get("address").and_then(Value::as_str) else {
                continue;
            };
            let Some(path) = entry.pointer("/source/file/path").and_then(Value::as_str)
            else {
                continue;
            };

            let dimension = |key: &str| {
                entry
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(DEFAULT_IMAGE_DIM)
            };

            images.push((
                address.to_string(),
                ImageLookupItem {
                    entity_dir: root.to_path_buf(),
                    path: path.to_string(),
                    fit: entry
                        .get("fit")
                        .and_then(Value::as_str)
                        .unwrap_or("inside")
                        .to_string(),
                    filter: entry
                        .get("filter")
                        .and_then(Value::as_str)
                        .unwrap_or("catmullrom")
                        .to_string(),
                    target_width: dimension("width"),
                    target_height: dimension("height"),
                },
            ));
            namespaces.push((namespace.clone(), address.to_string()));
        }
    }

    (images, namespaces)
}

/// Turns any `<type>.lock.json` into the media-agnostic index records.
pub fn parse_lock(
    media_type: &str,
    root: &Path,
    lock_json: &str,
    schema: &MediaRootSchema,
    music_directory: Option<&Path>,
) -> Result<Option<ParsedEntity>> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let parsed: Value = serde_json::from_str(lock_json)?;

    let Some(entity) = parsed.get(media_type) else {
        return Ok(None);
    };

    let id = entity
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .context("lock has no id")?
        .to_string();

    let mut playables = Vec::new();
    if let Some(nodes) = entity.get("nodes") {
        walk_nodes(
            nodes,
            &schema.nodes,
            &root,
            media_type,
            &id,
            music_directory,
            &mut Vec::new(),
            &mut playables,
        );
    }

    let (images, image_namespaces) = parse_images(entity, &root);

    let dict = serde_json::json!({
        "id": id,
        "tags": entity.get("tags"),
        "objects": entity.get("objects"),
        "images": entity.get("images"),
    });
    let dict_raw = RawValue::from_string(serde_json::to_string(&dict)?)?;

    let record = EntityRecord {
        media_type: media_type.to_string(),
        id: id.clone(),
        path: root,
        raw_json: std::sync::Arc::from(lock_json),
        dict_raw,
        search_texts: search_texts(entity.get("tags")),
        playable_uris: playables.iter().map(|p| p.path.clone()).collect(),
        image_namespaces,
    };

    Ok(Some(ParsedEntity {
        entity: record,
        playables,
        images,
    }))
}
