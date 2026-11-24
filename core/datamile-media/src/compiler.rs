use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use datamile_images::{
    FitMode, ImageError, ImageSpec, MAX_IMAGE_DIMENSION, parse_filter, parse_fit_mode,
    render_master_cached,
};
use datamile_lua_domain::DomainLuaEngine;
use serde_json::{Value, json};

/// One compiled root, ready to write and ingest.
#[derive(Debug, Clone)]
pub struct CompiledRoot {
    pub media_type: String,
    pub dir: PathBuf,
    pub id: String,
    pub lock_json: String,
    pub dependencies: Vec<PathBuf>,
    pub modified: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteMode {
    Write,
    Preview,
}

/// Compiles one root directory in a single Lua pass.
#[allow(
    clippy::implicit_hasher,
    reason = "active writes always use the default hasher"
)]
pub fn compile_root(
    engine: &DomainLuaEngine,
    media_type: &str,
    root_dir: &Path,
    cache_root: &Path,
    active_writes: Option<&Arc<Mutex<HashSet<PathBuf>>>>,
) -> Result<CompiledRoot> {
    compile_root_impl(
        engine,
        media_type,
        root_dir,
        cache_root,
        active_writes,
        WriteMode::Write,
    )
}

/// Renders a root without touching its lock file, for `compile --stdout`.
pub fn preview_root(
    engine: &DomainLuaEngine,
    media_type: &str,
    root_dir: &Path,
    cache_root: &Path,
) -> Result<String> {
    Ok(compile_root_impl(
        engine,
        media_type,
        root_dir,
        cache_root,
        None,
        WriteMode::Preview,
    )?
    .lock_json)
}

fn compile_root_impl(
    engine: &DomainLuaEngine,
    media_type: &str,
    root_dir: &Path,
    cache_root: &Path,
    active_writes: Option<&Arc<Mutex<HashSet<PathBuf>>>>,
    mode: WriteMode,
) -> Result<CompiledRoot> {
    let root_canon = root_dir
        .canonicalize()
        .unwrap_or_else(|_| root_dir.to_path_buf());

    engine.runtime.clear_entity_memo();
    let _ = engine.runtime.take_dependencies();

    let compile_fn: mlua::Function = engine
        .runtime
        .lua
        .globals()
        .get("__DATAMILE_COMPILE_ROOT")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let lua_value: mlua::Value = compile_fn
        .call((media_type, root_canon.to_string_lossy()))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut value: Value = engine
        .runtime
        .from_value(lua_value)
        .context("Compiled root is not representable as JSON")?;

    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .context("Compiled root has no id")?
        .to_string();

    let mut external_deps: Vec<PathBuf> = engine
        .runtime
        .take_dependencies()
        .into_iter()
        .filter(|dep| !dep.starts_with(&root_canon))
        .collect();
    external_deps.sort();

    if let Some(images) = value.get_mut("images") {
        process_images(&root_canon, cache_root, images, &mut external_deps)?;
    }

    strip_empty_values(&mut value);
    sort_json_keys(&mut value);

    let mut lock_value = json!({ media_type: value });
    strip_empty_values(&mut lock_value);
    sort_json_keys(&mut lock_value);

    let lock_json = serde_json::to_string_pretty(&lock_value)?;
    let lock_path = root_canon.join(format!("{media_type}.lock.json"));
    let modified =
        std::fs::read_to_string(&lock_path).map_or(true, |old| old != lock_json);

    if modified && mode == WriteMode::Write {
        if let Some(active) = active_writes
            && let Ok(mut active) = active.lock()
        {
            active.insert(lock_path.clone());
            active.insert(
                lock_path
                    .canonicalize()
                    .unwrap_or_else(|_| lock_path.clone()),
            );
        }
        datamile_fs::write_atomic(&lock_path, &lock_json)?;
    }

    engine.runtime.clear_entity_memo();

    Ok(CompiledRoot {
        media_type: media_type.to_string(),
        dir: root_canon,
        id,
        lock_json,
        dependencies: external_deps,
        modified,
    })
}

fn parse_fit(spec: &Value, namespace: &str) -> Result<(FitMode, u32, u32)> {
    let fit = spec.get("fit").and_then(Value::as_array).context(format!(
        "image '{namespace}': 'fit' must be a [mode, w, h] tuple"
    ))?;
    if fit.len() != 3 {
        bail!("image '{namespace}': 'fit' must contain exactly 3 elements");
    }

    let mode = fit[0]
        .as_str()
        .and_then(parse_fit_mode)
        .context(format!("image '{namespace}': unknown fit mode"))?;

    let dimension = |value: &Value, axis: &str| -> Result<u32> {
        value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (1..=MAX_IMAGE_DIMENSION).contains(n))
            .context(format!("image '{namespace}': invalid {axis} dimension"))
    };

    Ok((
        mode,
        dimension(&fit[1], "width")?,
        dimension(&fit[2], "height")?,
    ))
}

fn source_list(spec: &Value) -> Vec<String> {
    match spec.get("sources") {
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn render_entries(
    root: &Path,
    cache_root: &Path,
    namespace: &str,
    spec: &Value,
    deps: &mut Vec<PathBuf>,
) -> Result<Vec<Value>> {
    let (fit_mode, width, height) = parse_fit(spec, namespace)?;
    let filter = spec
        .get("filter")
        .and_then(Value::as_str)
        .and_then(parse_filter)
        .context(format!("image '{namespace}': missing or unknown filter"))?;

    let render = ImageSpec {
        fit_mode,
        target_width: width,
        target_height: height,
        filter_type: filter,
    };

    let mut entries = Vec::new();
    for raw in source_list(spec) {
        let target = if Path::new(&raw).is_absolute() {
            PathBuf::from(&raw)
        } else {
            root.join(&raw)
        };
        let Ok(canonical) = target.canonicalize() else {
            continue;
        };
        if !canonical.is_file() {
            continue;
        }

        let master = match render_master_cached(&canonical, &render, cache_root) {
            Ok(master) => master,
            Err(ImageError::Decode { .. } | ImageError::SourceRead { .. }) => continue,
            Err(error) => return Err(error.into()),
        };

        let path_for_json = canonical.strip_prefix(root).map_or_else(
            |_| canonical.to_string_lossy().to_string(),
            |rel| rel.to_string_lossy().to_string(),
        );
        if !canonical.starts_with(root) && !deps.contains(&canonical) {
            deps.push(canonical);
        }

        entries.push(json!({
            "address": master.address,
            "width": master.rendered_width,
            "height": master.rendered_height,
            "fit": master.fit,
            "filter": master.filter,
            "source": {
                "file": {
                    "path": path_for_json,
                    "mtime": master.source_mtime,
                    "byte_size": master.source_byte_size
                }
            }
        }));
    }

    Ok(entries)
}

fn process_images(
    root: &Path,
    cache_root: &Path,
    images: &mut Value,
    deps: &mut Vec<PathBuf>,
) -> Result<()> {
    let Some(namespaces) = images.as_object_mut() else {
        return Ok(());
    };

    for (namespace, spec) in namespaces.iter_mut() {
        let specs: Vec<Value> = if spec.is_array() {
            spec.as_array().cloned().unwrap_or_default()
        } else {
            vec![spec.clone()]
        };

        let mut entries = Vec::new();
        for one in &specs {
            entries.extend(render_entries(root, cache_root, namespace, one, deps)?);
        }
        *spec = Value::Array(entries);
    }

    Ok(())
}

/// Drops empty strings, nulls, empty objects and empty arrays.
pub fn strip_empty_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                strip_empty_values(v);
            }
            map.retain(|_, v| match v {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                strip_empty_values(v);
            }
            items.retain(|v| match v {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                Value::Object(m) => !m.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            });
        }
        _ => {}
    }
}

/// Root and node primitives, in the order a reader of a lock file expects.
const PRIMITIVE_KEYS: [&str; 5] = ["id", "tags", "objects", "images", "nodes"];

/// Primitives rank first, every other key ranks last and sorts alphabetically.
fn key_rank(key: &str) -> usize {
    PRIMITIVE_KEYS
        .iter()
        .position(|primitive| *primitive == key)
        .unwrap_or(PRIMITIVE_KEYS.len())
}

/// Deterministic key order so lock bytes are stable across runs.
pub fn sort_json_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                std::mem::take(map).into_iter().collect();
            entries.sort_by(|(a, _), (b, _)| {
                key_rank(a).cmp(&key_rank(b)).then_with(|| a.cmp(b))
            });
            for (key, mut v) in entries {
                sort_json_keys(&mut v);
                map.insert(key, v);
            }
        }
        Value::Array(items) => {
            for v in items {
                sort_json_keys(v);
            }
        }
        _ => {}
    }
}
