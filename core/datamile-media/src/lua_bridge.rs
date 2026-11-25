use std::path::{Path, PathBuf};

use anyhow::Result;
use datamile_lua::{EngineContext, LuaRuntime, read_with_entity_memo};
use mlua::{Lua, Value};

/// Registers the native helpers `media.lua` and `compile_root.lua` call, plus
/// the audio reader injected into `d.fs`.
pub fn register_native_helpers(runtime: &LuaRuntime) -> Result<()> {
    let lua = &runtime.lua;
    let lua_err = |e: mlua::Error| anyhow::anyhow!("{e}");

    let dir_fn = lua
        .create_function(|lua, root: String| dir_location(lua, &root))
        .map_err(lua_err)?;
    lua.globals()
        .set("__DATAMILE_DIR_LOCATION", dir_fn)
        .map_err(lua_err)?;

    let node_fn = lua
        .create_function(|lua, (root, raw): (String, String)| {
            node_location(lua, &root, &raw)
        })
        .map_err(lua_err)?;
    lua.globals()
        .set("__DATAMILE_NODE_LOCATION", node_fn)
        .map_err(lua_err)?;

    runtime.register_fs_function("read_audio", |lua, path_str| {
        let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
            return Err(mlua::Error::external(anyhow::anyhow!(
                "EngineContext is not initialized"
            )));
        };
        let Some(raw) = path_str else {
            return Ok(Value::Nil);
        };
        let path = datamile_fs::expand_path(&raw);
        let cache_root = ctx.cache_root.clone();

        read_with_entity_memo(lua, "audio", &path, |probe_path| {
            let probe = datamile_audio::probe_file_cached(probe_path, &cache_root)?;
            Ok(serde_json::to_value(probe)?)
        })
    })?;

    Ok(())
}

fn resolve_target(root: &Path, raw: &str) -> PathBuf {
    let expanded = datamile_fs::expand_path(raw);
    if expanded.is_absolute() {
        expanded
    } else {
        root.join(expanded)
    }
}

fn record_external(lua: &Lua, canonical: &Path) {
    if let Some(ctx) = lua.app_data_ref::<EngineContext>() {
        ctx.record_dependency(canonical);
    }
}

fn mtime_seconds(meta: &std::fs::Metadata) -> u64 {
    datamile_fs::file_meta(meta).map_or(0, |m| m.mtime)
}

fn dir_location(lua: &Lua, root: &str) -> mlua::Result<Value> {
    // No mtime or byte_size: writing `<type>.lock.json` bumps the directory's
    // mtime, so any value here would make every recompile produce new lock
    // bytes. Only files carry stats, and only where a cache keys on them.
    let location = lua.create_table()?;
    location.set("kind", "dir")?;
    location.set("path", root)?;
    Ok(Value::Table(location))
}

/// Resolves one `select` result and returns `{ path, location }`, or `nil` when
/// the target no longer exists.
fn node_location(lua: &Lua, root: &str, raw: &str) -> mlua::Result<Value> {
    let root_path = datamile_fs::expand_path(root);
    let root_canon = root_path
        .canonicalize()
        .unwrap_or_else(|_| root_path.clone());
    let target = resolve_target(&root_path, raw);

    let Ok(canonical) = target.canonicalize() else {
        return Ok(Value::Nil);
    };
    let Ok(meta) = std::fs::metadata(&canonical) else {
        return Ok(Value::Nil);
    };

    let kind = if meta.is_dir() { "dir" } else { "file" };
    let location_path = canonical.strip_prefix(&root_canon).map_or_else(
        |_| {
            record_external(lua, &canonical);
            canonical.to_string_lossy().to_string()
        },
        |rel| rel.to_string_lossy().to_string(),
    );

    let location = lua.create_table()?;
    location.set("kind", kind)?;
    location.set("path", location_path)?;
    if kind == "file" {
        location.set("mtime", mtime_seconds(&meta))?;
        location.set("byte_size", meta.len())?;
    }

    let result = lua.create_table()?;
    result.set("path", canonical.to_string_lossy().to_string())?;
    result.set("location", location)?;
    Ok(Value::Table(result))
}
