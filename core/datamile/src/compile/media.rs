use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use libdatamile::lua::{EngineContext, LuaEngine, MediaKind};
use serde_json::{Map, Value, json};

use crate::compile::images;

#[cfg(test)]
mod tests;

/// A root entity discovered from Lua. `lock_path` is the canonical identity:
/// two roots may share a directory (file-backed types) without colliding.
#[derive(Clone, Debug)]
pub struct RootTarget {
    pub root_type: String,
    pub target: PathBuf,
    pub lock_path: PathBuf,
    pub root_dir: PathBuf,
    pub source: Value,
}

#[derive(Debug)]
pub struct MediaBuild {
    pub root_type: String,
    pub target: PathBuf,
    pub id: String,
    pub lock_path: PathBuf,
    pub lock_json: Value,
    pub files: HashSet<PathBuf>,
    pub dirs: HashSet<PathBuf>,
}

struct EntityScope<'a> {
    root_dir: &'a Path,
    lock_root: &'a Path,
    cache_root: &'a Path,
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
    engine.media_validate()?;

    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for (root_type, root_dir) in engine.media_root_dirs()? {
        let scopes = root_scopes(&root_dir, target, specific);
        for scope in scopes {
            let candidates = engine.media_select(
                &root_type,
                &Value::String(scope.to_string_lossy().to_string()),
            )?;
            for candidate in candidates {
                let canon =
                    validate_root_target(&root_type, &root_dir, &scope, &candidate)?;
                if seen.insert((root_type.clone(), canon.clone())) {
                    roots.push((root_type.clone(), path_source(&canon)));
                }
            }
        }
    }

    roots.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| source_sort_key(&a.1).cmp(source_sort_key(&b.1)))
    });
    Ok(roots)
}

/// Scan every declared root directory.
pub fn discover_targets_all(engine: &LuaEngine) -> Result<Vec<RootTarget>> {
    engine.media_validate()?;

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for (root_type, root_dir) in engine.media_root_dirs()? {
        let scopes = root_scopes(&root_dir, &root_dir, None);
        for scope in scopes {
            let found = engine.media_select(
                &root_type,
                &Value::String(scope.to_string_lossy().to_string()),
            )?;
            for candidate in found {
                let canon =
                    validate_root_target(&root_type, &root_dir, &scope, &candidate)?;
                if seen.insert((root_type.clone(), canon.clone())) {
                    candidates.push((root_type.clone(), path_source(&canon)));
                }
            }
        }
    }
    resolve_targets(candidates, engine)
}

/// Resolve root targets, including their canonical lock paths.
pub fn discover_targets(
    engine: &LuaEngine,
    target: &Path,
    specific: Option<&[PathBuf]>,
) -> Result<Vec<RootTarget>> {
    resolve_targets(discover_roots(engine, target, specific)?, engine)
}

/// Discover roots that anchor directly at `target`, never its descendants.
pub fn discover_targets_shallow(
    engine: &LuaEngine,
    target: &Path,
) -> Result<Vec<RootTarget>> {
    let anchor = canonical_or_owned(target);
    let mut roots = resolve_targets(discover_roots(engine, target, None)?, engine)?;
    roots.retain(|root| root.target == anchor);

    Ok(roots)
}

fn root_scopes(
    root_dir: &Path,
    target: &Path,
    specific: Option<&[PathBuf]>,
) -> Vec<PathBuf> {
    let root = canonical_or_owned(root_dir);
    let mut scopes = Vec::new();

    let mut push_scope = |path: &Path| {
        let path = canonical_or_owned(path);
        if path == root || path.starts_with(&root) || root.starts_with(&path) {
            let scope = if path.starts_with(&root) {
                path
            } else {
                root.clone()
            };
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    };

    match specific {
        Some(paths) => {
            for path in paths {
                push_scope(path);
            }
        }
        None => push_scope(target),
    }

    scopes
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn validate_root_target(
    root_type: &str,
    root_dir: &Path,
    scope: &Path,
    candidate: &Value,
) -> Result<PathBuf> {
    let Some(raw) = candidate.as_str() else {
        bail!(
            "root media type '{root_type}' select must return directory path strings, got {candidate}"
        );
    };

    if raw.is_empty() {
        bail!("root media type '{root_type}' returned an empty directory path");
    }

    let path = resolve_selection_path(raw, scope);
    let canon = path.canonicalize().with_context(|| {
        format!("root media type '{root_type}' selected missing path {raw}")
    })?;

    if !canon.is_dir() {
        bail!(
            "root media type '{root_type}' selected {}, which is not an existing directory",
            canon.display()
        );
    }

    let root = canonical_or_owned(root_dir);
    if !canon.starts_with(&root) {
        bail!(
            "root media type '{root_type}' selected {}, outside its declared dir {}",
            canon.display(),
            root.display()
        );
    }

    Ok(canon)
}

fn resolve_targets(
    roots: Vec<(String, Value)>,
    engine: &LuaEngine,
) -> Result<Vec<RootTarget>> {
    let dirs: std::collections::HashMap<String, PathBuf> =
        engine.media_root_dirs()?.into_iter().collect();

    let mut out = Vec::new();
    for (root_type, source) in roots {
        let target_path = source_path(&source)
            .with_context(|| format!("media type '{root_type}' has no source path"))?
            .to_path_buf();

        if !target_path.is_dir() {
            bail!(
                "root media type '{root_type}' selected {}, which is not an existing directory",
                target_path.display()
            );
        }

        let root_dir = dirs
            .get(&root_type)
            .cloned()
            .with_context(|| format!("media type '{root_type}' has no declared dir"))?;
        let lock_path = target_path.join(format!("{root_type}.lock.json"));

        out.push(RootTarget {
            root_type,
            target: target_path,
            lock_path,
            root_dir,
            source,
        });
    }
    Ok(out)
}

pub fn compile_root(
    engine: &LuaEngine,
    root: &RootTarget,
    cache_root: &Path,
) -> Result<MediaBuild> {
    engine.media_validate()?;
    take_dependencies(engine);
    take_directories(engine);

    let mut files = HashSet::new();
    let scope = EntityScope {
        root_dir: &root.root_dir,
        lock_root: &root.target,
        cache_root,
    };
    let value = compile_entity(
        engine,
        &root.root_type,
        &root.source,
        &scope,
        None,
        &mut files,
    )?;
    files.extend(take_dependencies(engine));
    let dirs = take_directories(engine);

    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let lock_json = json!({ root.root_type.clone(): value });

    Ok(MediaBuild {
        root_type: root.root_type.clone(),
        target: root.target.clone(),
        id,
        lock_path: root.lock_path.clone(),
        lock_json,
        files,
        dirs,
    })
}

/// Bottom-up compilation: children fully compile before the parent finalizes.
fn compile_entity(
    engine: &LuaEngine,
    type_name: &str,
    source: &Value,
    scope: &EntityScope<'_>,
    parent_ctx_id: Option<u64>,
    dependencies: &mut HashSet<PathBuf>,
) -> Result<Value> {
    let kind = engine.media_kind(type_name)?;

    // The entity's own directory is always watched so new files (a dropped
    // track or cover) mark it dirty even when no hook listed it.
    if let Some(path) = source_path(source) {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        if let Some(ctx) = engine.lua.app_data_ref::<EngineContext>() {
            ctx.record_dir(dir);
        }
    }

    let prepared = engine
        .media_prepare(type_name, source, parent_ctx_id)
        .with_context(|| {
            format!("failed to prepare {type_name} at {}", source_label(source))
        })?;
    let guard = CtxGuard {
        engine,
        ctx_id: prepared.ctx_id,
    };

    let parent_path = source_path(source);
    let parent_is_virtual = parent_path.is_none_or(Path::is_file);

    let mut children = Map::new();
    for child in prepared.children {
        let child_kind = engine.media_kind(&child.type_name)?;
        if parent_is_virtual && !child_kind.kinds.iter().all(|kind| kind == "virtual") {
            bail!(
                "media type '{type_name}' is file-backed or virtual and can only have virtual children"
            );
        }
        let mut compiled = Vec::with_capacity(child.targets.len());
        for target in child.targets {
            let child_source =
                validate_child_target(&child.type_name, &child_kind, &target, source)?;
            compiled.push(compile_entity(
                engine,
                &child.type_name,
                &child_source,
                scope,
                Some(prepared.ctx_id),
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

    let path = parent_path;
    let id = resolve_id(type_name, &kind, path, scope.root_dir, raw.get("id"))?;
    let location = path
        .map(|path| auto_location(path, scope.lock_root))
        .transpose()?;
    let files = process_files(
        engine,
        raw.get("files"),
        path,
        scope.lock_root,
        dependencies,
    );

    let image_map = match path {
        Some(path) => {
            let entity_root = if path.is_dir() {
                path
            } else {
                path.parent().unwrap_or(path)
            };
            images::process_images(
                entity_root,
                scope.cache_root,
                raw.get("images"),
                dependencies,
            )?
        }
        None => BTreeMap::new(),
    };

    let mut out = Map::new();
    out.insert("id".to_string(), Value::String(id));
    if let Some(location) = location {
        out.insert("location".to_string(), sort_maps(&location));
    }
    out.insert("tags".to_string(), sorted_object(raw.get("tags")));
    out.insert("files".to_string(), files);
    out.insert("images".to_string(), json!(image_map));
    out.insert("objects".to_string(), sorted_object(raw.get("objects")));

    for (field, compiled) in children {
        out.insert(field, compiled);
    }

    Ok(Value::Object(out))
}

fn resolve_id(
    type_name: &str,
    kind: &MediaKind,
    path: Option<&Path>,
    root_dir: &Path,
    raw_id: Option<&Value>,
) -> Result<String> {
    if let Some(id) = raw_id.and_then(Value::as_str).filter(|id| !id.is_empty()) {
        return Ok(id.to_string());
    }

    if let Some(path) = path {
        return Ok(relpath_string(path, root_dir));
    }

    if kind.has("virtual") {
        bail!("media type '{type_name}' produced a virtual entity without an id");
    }

    bail!("media type '{type_name}' produced an entity without an id or path");
}

fn validate_child_target(
    type_name: &str,
    kind: &MediaKind,
    target: &Value,
    parent: &Value,
) -> Result<Value> {
    if let Some(raw) = target.as_str() {
        return validate_physical_target(type_name, kind, raw, parent);
    }

    let Some(object) = target.as_object() else {
        bail!("media type '{type_name}' select returned an invalid target: {target}");
    };

    if !kind.has("virtual") {
        bail!(
            "media type '{type_name}' rejected a table target because it is not virtual"
        );
    }

    for key in ["path", "parent", "id", "tags", "objects", "files", "images"] {
        if object.contains_key(key) {
            bail!(
                "media type '{type_name}' virtual target contains reserved key '{key}'"
            );
        }
    }

    Ok(target.clone())
}

fn validate_physical_target(
    type_name: &str,
    kind: &MediaKind,
    raw: &str,
    parent: &Value,
) -> Result<Value> {
    if !kind.has("dir") && !kind.has("file") {
        bail!(
            "media type '{type_name}' rejected a path target because it is virtual-only"
        );
    }

    let Some(parent_path) = source_path(parent) else {
        bail!("media type '{type_name}' rejected a physical child of a virtual parent");
    };

    let base = if parent_path.is_dir() {
        parent_path.to_path_buf()
    } else {
        parent_path
            .parent()
            .map(Path::to_path_buf)
            .context("physical parent has no directory")?
    };
    let base = canonical_or_owned(&base);

    let path = resolve_selection_path(raw, &base);
    let canon = path.canonicalize().with_context(|| {
        format!("media type '{type_name}' selected missing path {raw}")
    })?;

    if !canon.starts_with(&base) {
        bail!(
            "media type '{type_name}' selected {}, outside its parent {}",
            canon.display(),
            base.display()
        );
    }

    if canon.is_dir() {
        if !kind.has("dir") {
            bail!(
                "media type '{type_name}' selected a directory but does not declare kind 'dir'"
            );
        }
    } else if canon.is_file() {
        if !kind.has("file") {
            bail!(
                "media type '{type_name}' selected a file but does not declare kind 'file'"
            );
        }
    } else {
        bail!(
            "media type '{type_name}' selected unsupported path {}",
            canon.display()
        );
    }

    Ok(path_source(&canon))
}

fn auto_location(path: &Path, root_dir: &Path) -> Result<Value> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?;
    let mtime = meta
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rel = relpath_string(path, root_dir);

    if meta.is_dir() {
        return Ok(json!({ "kind": "dir", "path": rel, "mtime": mtime }));
    }
    if meta.is_file() {
        return Ok(json!({
            "kind": "file",
            "path": rel,
            "mtime": mtime,
            "byte_size": meta.len()
        }));
    }

    bail!("unsupported physical entity at {}", path.display())
}

fn process_files(
    engine: &LuaEngine,
    raw_files: Option<&Value>,
    entity_path: Option<&Path>,
    root_dir: &Path,
    dependencies: &mut HashSet<PathBuf>,
) -> Value {
    let entity_dir = entity_path.map(|path| {
        if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        }
    });

    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    let Some(files) = raw_files.and_then(Value::as_object) else {
        return Value::Object(Map::new());
    };

    for (name, value) in files {
        let processed = match value {
            Value::Array(items) => {
                let mut list = Vec::new();
                for item in items {
                    if let Some(entry) = process_file_entry(
                        engine,
                        item,
                        entity_dir.as_deref(),
                        root_dir,
                        dependencies,
                    ) {
                        list.push(entry);
                    }
                }
                if list.is_empty() {
                    continue;
                }
                Value::Array(list)
            }
            other => {
                let Some(entry) = process_file_entry(
                    engine,
                    other,
                    entity_dir.as_deref(),
                    root_dir,
                    dependencies,
                ) else {
                    continue;
                };
                entry
            }
        };
        out.insert(name.clone(), processed);
    }

    let mut map = Map::new();
    for (key, value) in out {
        map.insert(key, sort_maps(&value));
    }
    Value::Object(map)
}

fn process_file_entry(
    engine: &LuaEngine,
    value: &Value,
    entity_dir: Option<&Path>,
    root_dir: &Path,
    dependencies: &mut HashSet<PathBuf>,
) -> Option<Value> {
    let raw = value.get("path").and_then(Value::as_str)?;
    let path = resolve_attached_path(raw, entity_dir)?;
    let canon = path.canonicalize().ok()?;
    if !canon.is_file() {
        return None;
    }

    dependencies.insert(canon.clone());
    if let Some(parent) = canon.parent()
        && let Some(ctx) = engine.lua.app_data_ref::<EngineContext>()
    {
        ctx.record_dir(parent);
    }

    let meta = std::fs::metadata(&canon).ok()?;
    let mtime = meta
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rel = relpath_string(&canon, root_dir);

    Some(json!({
        "path": rel,
        "mtime": mtime,
        "byte_size": meta.len()
    }))
}

fn resolve_attached_path(raw: &str, entity_dir: Option<&Path>) -> Option<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }

    entity_dir.map(|base| base.join(path))
}

fn resolve_selection_path(raw: &str, base: &Path) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn relpath_string(path: &Path, root_dir: &Path) -> String {
    let canon = canonical_or_owned(path);
    let root = canonical_or_owned(root_dir);
    canon.strip_prefix(&root).map_or_else(
        |_| canon.to_string_lossy().to_string(),
        |rel| {
            if rel.as_os_str().is_empty() {
                ".".to_string()
            } else {
                rel.to_string_lossy().to_string()
            }
        },
    )
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

fn take_directories(engine: &LuaEngine) -> HashSet<PathBuf> {
    engine
        .lua
        .app_data_ref::<EngineContext>()
        .map_or_else(HashSet::new, |ctx| ctx.take_directories())
}
