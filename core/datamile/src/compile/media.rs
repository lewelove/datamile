use crate::compile::images;
use anyhow::{Context, Result};
use libdatamile::lua::{EngineContext, LuaEngine};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

pub struct MediaBuild {
    pub root_type: String,
    pub path: PathBuf,
    pub id: String,
    pub lock_path: PathBuf,
    pub lock_json: Value,
    pub dependencies: HashSet<PathBuf>,
}

/// Collects prepared contexts whose children failed to compile.
struct CtxGuard<'a> {
    engine: &'a LuaEngine,
    ctx_id: u64,
}

impl Drop for CtxGuard<'_> {
    fn drop(&mut self) {
        let _ = self.engine.media_drop(self.ctx_id);
    }
}

pub fn discover_roots(
    engine: &LuaEngine,
    target: &Path,
    specific: Option<&[PathBuf]>,
) -> Result<Vec<(String, Value)>> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for root_type in engine.media_roots()? {
        let candidates: Vec<Value> = match specific {
            Some(paths) => {
                let mut found = Vec::new();
                for path in paths {
                    found.extend(engine.media_select(&root_type, &recursive_scope(path))?);
                }
                found
            }
            None => engine.media_select(&root_type, &recursive_scope(target))?,
        };

        for source in candidates {
            let path = source_path(&source).with_context(|| {
                format!("root media type '{root_type}' returned a source without a path")
            })?;
            let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

            if seen.insert((root_type.clone(), canon.clone())) {
                roots.push((root_type.clone(), path_source(&canon)));
            }
        }
    }

    roots.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| source_sort_key(&a.1).cmp(source_sort_key(&b.1)))
    });
    Ok(roots)
}

pub fn compile_root(
    engine: &LuaEngine,
    root_type: &str,
    source: &Value,
    cache_root: &Path,
) -> Result<MediaBuild> {
    let root_path = source_path(source)
        .with_context(|| format!("media type '{root_type}' has no source path"))?;

    take_dependencies(engine);

    let mut dependencies = HashSet::new();
    let value = compile_entity(engine, root_type, source, cache_root, &mut dependencies)?;
    dependencies.extend(take_dependencies(engine));

    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let lock_path = engine.media_lock(root_type, source)?.with_context(|| {
        format!(
            "media type '{root_type}' has no lock path for {}",
            root_path.display()
        )
    })?;

    let lock_json = json!({
        root_type: value
    });

    Ok(MediaBuild {
        root_type: root_type.to_string(),
        path: root_path.to_path_buf(),
        id,
        lock_path,
        lock_json,
        dependencies,
    })
}

/// Bottom-up compilation: children fully compile before the parent finalizes.
fn compile_entity(
    engine: &LuaEngine,
    type_name: &str,
    source: &Value,
    cache_root: &Path,
    dependencies: &mut HashSet<PathBuf>,
) -> Result<Value> {
    let prepared = engine.media_prepare(type_name, source).with_context(|| {
        format!("failed to prepare {type_name} at {}", source_label(source))
    })?;
    let guard = CtxGuard {
        engine,
        ctx_id: prepared.ctx_id,
    };

    let mut children = Map::new();
    for child in prepared.children {
        let mut compiled = Vec::with_capacity(child.targets.len());
        for target in child.targets {
            compiled.push(compile_entity(
                engine,
                &child.type_name,
                &target,
                cache_root,
                dependencies,
            )?);
        }
        children.insert(child.field, Value::Array(compiled));
    }

    let raw = engine
        .media_finalize(type_name, prepared.ctx_id, &Value::Object(children.clone()))
        .with_context(|| {
            format!("failed to finalize {type_name} at {}", source_label(source))
        })?;
    drop(guard);

    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map_or_else(
            || {
                source_path(source)
                    .and_then(Path::to_str)
                    .unwrap_or_default()
                    .to_string()
            },
            ToString::to_string,
        );

    let image_map = match source_path(source) {
        Some(path) => {
            let entity_root = if path.is_dir() {
                path
            } else {
                path.parent().unwrap_or(path)
            };
            images::process_images(
                entity_root,
                cache_root,
                raw.get("images"),
                dependencies,
            )?
        }
        None => BTreeMap::new(),
    };

    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(id));
    out.insert("tags".to_string(), sorted_object(raw.get("tags")));
    out.insert("objects".to_string(), sorted_object(raw.get("objects")));
    out.insert("images".to_string(), json!(image_map));

    if let Some(location) = raw.get("location") {
        out.insert("location".to_string(), sort_maps(location));
    }

    for (field, compiled) in children {
        out.insert(field, compiled);
    }

    Ok(Value::Object(out))
}

fn recursive_scope(path: &Path) -> Value {
    json!({
        "path": path.to_string_lossy(),
        "recursive": true,
    })
}

fn path_source(path: &Path) -> Value {
    json!({ "path": path.to_string_lossy() })
}

fn source_path(source: &Value) -> Option<&Path> {
    source.get("path").and_then(Value::as_str).map(Path::new)
}

fn source_label(source: &Value) -> String {
    source_path(source).map_or_else(
        || "<virtual>".to_string(),
        |path| path.display().to_string(),
    )
}

fn source_sort_key(source: &Value) -> &str {
    source.get("path").and_then(Value::as_str).unwrap_or("")
}

fn sorted_object(value: Option<&Value>) -> Value {
    let mut sorted = BTreeMap::new();

    if let Some(object) = value.and_then(Value::as_object) {
        for (key, value) in object {
            sorted.insert(key.clone(), sort_maps(value));
        }
    }

    let mut out = Map::new();
    for (key, value) in sorted {
        out.insert(key, value);
    }
    Value::Object(out)
}

fn sort_maps(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut sorted: Vec<_> = object.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));

            let mut out = Map::new();
            for (key, value) in sorted {
                out.insert(key.clone(), sort_maps(value));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_maps).collect()),
        other => other.clone(),
    }
}

fn take_dependencies(engine: &LuaEngine) -> HashSet<PathBuf> {
    engine
        .lua
        .app_data_ref::<EngineContext>()
        .map_or_else(HashSet::new, |ctx| ctx.take_dependencies())
}
