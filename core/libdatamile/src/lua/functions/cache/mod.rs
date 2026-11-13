#[cfg(test)]
mod tests;

use crate::lua::EngineContext;
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table, Value};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn cache_key(namespace: &str, path: &Path, size: u64, mtime: u128) -> String {
    let raw = format!("{namespace}:{}:{size}:{mtime}", path.display());
    blake3::hash(raw.as_bytes()).to_hex().to_string()
}

fn probe_path(cache_root: &Path, key: &str) -> PathBuf {
    cache_root.join("probes").join(format!("{key}.json"))
}

fn file_stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    Some((meta.len(), mtime))
}

fn read_cached(lua: &Lua, file: &Path, opts: SerializeOptions) -> Option<Value> {
    let content = std::fs::read_to_string(file).ok()?;
    let json: JsonValue = serde_json::from_str(&content).ok()?;

    lua.to_value_with(&json, opts).ok()
}

fn write_cached(lua: &Lua, file: &Path, result: &Value) -> mlua::Result<()> {
    let json: JsonValue = lua.from_value(result.clone())?;
    let content = serde_json::to_string(&json).map_err(mlua::Error::external)?;
    crate::utils::write_atomic_cache_file(file, &content).map_err(mlua::Error::external)
}

pub fn cache_file(
    lua: &Lua,
    opts: SerializeOptions,
    (namespace, path_str, func): (String, Option<String>, mlua::Function),
) -> mlua::Result<Value> {
    if namespace.trim().is_empty() {
        return Err(mlua::Error::runtime("namespace must be a non-empty string"));
    }

    let Some(path_str) = path_str.filter(|p| !p.trim().is_empty()) else {
        return Ok(Value::Nil);
    };

    let path = crate::utils::expand_path(&path_str);
    let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
        return Err(mlua::Error::external(anyhow::anyhow!(
            "EngineContext is not initialized"
        )));
    };
    ctx.record_dependency(&path);

    let Some((size, mtime)) = file_stamp(&path) else {
        return Ok(Value::Nil);
    };

    let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
    let key = cache_key(&namespace, &canon, size, mtime);
    let file = probe_path(&ctx.cache_root, &key);

    if let Some(cached) = read_cached(lua, &file, opts) {
        return Ok(cached);
    }

    let result: Value = func.call(canon.to_string_lossy().to_string())?;
    write_cached(lua, &file, &result)?;

    Ok(result)
}

pub fn register(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    let cache_table: Table = match datamile_tbl.get("cache") {
        Ok(table) => table,
        Err(_) => lua.create_table()?,
    };
    cache_table.set(
        "file",
        lua.create_function(move |lua, args| cache_file(lua, opts, args))?,
    )?;
    datamile_tbl.set("cache", cache_table)
}
