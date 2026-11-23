pub mod functions;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};
use serde::Serialize;
use serde::de::DeserializeOwned;

const LUA_RUNTIME: &str = include_str!("runtime.lua");
const DEFAULT_CACHE_ROOT: &str = "~/.cache/datamile";

/// Per-VM state: the cache directory `d.fs.read_json`/`read_toml` resolve
/// against, plus every script the config loaded.
#[derive(Clone, Debug)]
pub struct EngineContext {
    pub cache_root: PathBuf,
    pub captured_deps: RefCell<HashSet<PathBuf>>,
}

impl EngineContext {
    #[must_use]
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            captured_deps: RefCell::new(HashSet::new()),
        }
    }

    pub fn record_dependency(&self, path: &Path) {
        if let Ok(canon) = path.canonicalize()
            && canon.is_file()
        {
            self.captured_deps.borrow_mut().insert(canon);
        }
    }

    pub fn take_dependencies(&self) -> HashSet<PathBuf> {
        std::mem::take(&mut *self.captured_deps.borrow_mut())
    }
}

/// One Lua VM with the `d.*` standard library installed and every loaded
/// script recorded in its [`EngineContext`].
pub struct LuaRuntime {
    pub lua: Lua,
}

impl LuaRuntime {
    pub fn new() -> Result<Self> {
        Self::with_cache_root(datamile_fs::expand_path(DEFAULT_CACHE_ROOT))
    }

    pub fn with_cache_root(cache_root: PathBuf) -> Result<Self> {
        let lua = Lua::new();
        lua.set_app_data(EngineContext::new(cache_root));

        register_native_functions(&lua)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to register native Lua functions")?;
        lua.load(LUA_RUNTIME)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to install Lua runtime plumbing")?;

        Ok(Self { lua })
    }

    #[must_use]
    pub const fn serialize_options() -> SerializeOptions {
        SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false)
    }

    pub fn set_cache_root(&self, cache_root: PathBuf) {
        if let Some(mut ctx) = self.lua.app_data_mut::<EngineContext>() {
            ctx.cache_root = cache_root;
        } else {
            self.lua.set_app_data(EngineContext::new(cache_root));
        }
    }

    pub fn append_package_path(&self, dir: &Path) -> Result<()> {
        let package: Table = self
            .lua
            .globals()
            .get("package")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let current: String = package.get("path").map_err(|e| anyhow::anyhow!("{e}"))?;
        let updated = format!("{current};{}/?.lua", dir.display());
        package
            .set("path", updated)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn exec_chunk(&self, code: &str, name: &str) -> Result<()> {
        self.lua
            .load(code)
            .set_name(name)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn take_dependencies(&self) -> HashSet<PathBuf> {
        self.lua
            .app_data_ref::<EngineContext>()
            .map_or_else(HashSet::new, |ctx| ctx.take_dependencies())
    }

    pub fn to_value<T: Serialize>(&self, val: &T) -> Result<mlua::Value> {
        self.lua
            .to_value_with(val, Self::serialize_options())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn from_value<T: DeserializeOwned>(&self, val: mlua::Value) -> Result<T> {
        self.lua.from_value(val).map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn call_global<T: DeserializeOwned>(&self, fn_name: &str) -> Result<T> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get(fn_name)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let val: mlua::Value = func.call(()).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.from_value(val)
    }
}

fn record_dependency(lua: &Lua, raw: Option<String>) {
    let Some(raw) = raw else {
        return;
    };
    let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
        return;
    };
    ctx.record_dependency(&datamile_fs::expand_path(&raw));
}

fn register_native_functions(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let datamile_table: Table = globals
        .get::<Table>("datamile")
        .or_else(|_| lua.create_table())?;

    functions::register_all(lua, &datamile_table, LuaRuntime::serialize_options())?;

    globals.set("datamile", datamile_table.clone())?;
    globals.set("d", datamile_table)?;
    globals.set(
        "__DATAMILE_RECORD_DEPENDENCY",
        lua.create_function(|lua, raw: Option<String>| {
            record_dependency(lua, raw);
            Ok(())
        })?,
    )?;
    Ok(())
}

fn init_lua_in(dir: &Path) -> Result<PathBuf> {
    if !dir.exists() {
        anyhow::bail!("config directory does not exist: {}", dir.display());
    }

    if !dir.is_dir() {
        anyhow::bail!(
            "config path must be a directory containing init.lua, got a file: {}",
            dir.display()
        );
    }

    let init = dir.join("init.lua");
    if !init.is_file() {
        anyhow::bail!("no init.lua in config directory: {}", dir.display());
    }

    Ok(init)
}

/// Explicit override, then `DATAMILE_CONFIG_PATH`, then `~/.config/datamile`,
/// then the nearest `config/init.lua` or `init.lua` above the working directory.
///
/// An explicit override that does not resolve is an error, never a silent
/// fallback: running against a different config than the one asked for is worse
/// than not running at all.
pub fn resolve_config(
    flag_dir: Option<&Path>,
    env_dir: Option<&Path>,
    home_init: Option<&Path>,
    cwd: &Path,
) -> Result<Option<PathBuf>> {
    if let Some(dir) = flag_dir {
        return init_lua_in(dir).map(Some);
    }

    if let Some(dir) = env_dir {
        return init_lua_in(dir).map(Some);
    }

    if home_init.is_some_and(Path::is_file) {
        return Ok(home_init.map(Path::to_path_buf));
    }

    let mut curr = cwd;
    loop {
        let local_nested = curr.join("config/init.lua");
        if local_nested.is_file() {
            return Ok(Some(local_nested));
        }

        let local_root = curr.join("init.lua");
        if local_root.is_file() {
            return Ok(Some(local_root));
        }

        let Some(parent) = curr.parent() else {
            return Ok(None);
        };
        curr = parent;
    }
}
