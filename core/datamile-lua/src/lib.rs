pub mod functions;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};
use serde::Serialize;
use serde::de::DeserializeOwned;

const LUA_RUNTIME: &str = include_str!("runtime.lua");
const DEFAULT_CACHE_ROOT: &str = "~/.cache/datamile";

/// Memo key for one reader result: reader kind plus canonical file path.
type MemoKey = (String, PathBuf);

/// Per-VM state: the cache directory `d.fs.read_json`/`read_toml` resolve
/// against, every script the config loaded, and the per-entity reader memo.
///
/// The memo holds Lua registry keys rather than JSON so a file read by several
/// hooks of the same entity is converted to Lua exactly once.
pub struct EngineContext {
    pub cache_root: PathBuf,
    pub captured_deps: RefCell<HashSet<PathBuf>>,
    memo: RefCell<HashMap<MemoKey, mlua::Value>>,
    compile_depth: Arc<AtomicU32>,
    globals_baseline: RefCell<Option<mlua::Table>>,
}

impl EngineContext {
    #[must_use]
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            captured_deps: RefCell::new(HashSet::new()),
            memo: RefCell::new(HashMap::new()),
            compile_depth: Arc::new(AtomicU32::new(0)),
            globals_baseline: RefCell::new(None),
        }
    }

    /// Records the globals the config itself declared.
    fn set_globals_baseline(&self, baseline: mlua::Table) {
        *self.globals_baseline.borrow_mut() = Some(baseline);
    }

    /// The globals a compile must leave as `init.lua` left them.
    ///
    /// Cloning a Lua table handle is cheap: it takes a registry reference, not
    /// a copy of the table.
    #[must_use]
    fn globals_baseline(&self) -> Option<mlua::Table> {
        self.globals_baseline.borrow().clone()
    }

    /// Starts a compile scope. Every guard counts, so nested compiles stay
    /// guarded until the outermost one ends.
    #[must_use]
    pub fn compile_guard(&self) -> CompileGuard {
        self.compile_depth.fetch_add(1, Ordering::Acquire);
        CompileGuard {
            depth: Arc::clone(&self.compile_depth),
        }
    }

    #[must_use]
    pub fn is_compiling(&self) -> bool {
        self.compile_depth.load(Ordering::Acquire) > 0
    }

    fn memo_key(kind: &str, path: &Path) -> MemoKey {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        (kind.to_string(), canon)
    }

    /// Returns the memoized Lua value for `(kind, path)`, if any.
    #[must_use]
    pub fn memo_get(&self, kind: &str, path: &Path) -> Option<mlua::Value> {
        let key = Self::memo_key(kind, path);
        self.memo.borrow().get(&key).cloned()
    }

    pub fn memo_insert(&self, kind: &str, path: &Path, value: mlua::Value) {
        self.memo
            .borrow_mut()
            .insert(Self::memo_key(kind, path), value);
    }

    /// Drops every memoized reader result. Called between root entities.
    pub fn clear_entity_memo(&self) {
        self.memo.borrow_mut().clear();
    }

    /// Records `path` as a hot-reload dependency.
    ///
    /// During a compile a `*.lock.json` path is an error instead: a lock is
    /// always an output, so reading one must never feed the next lock.
    pub fn record_dependency(&self, path: &Path) -> mlua::Result<()> {
        if self.is_compiling() && datamile_cache::stat::is_lock_file(path) {
            return Err(mlua::Error::external(anyhow::anyhow!(
                "compile cannot read lock file '{}'; a lock is never an input",
                path.display()
            )));
        }

        if let Ok(canon) = path.canonicalize()
            && canon.is_file()
        {
            self.captured_deps.borrow_mut().insert(canon);
        }

        Ok(())
    }

    pub fn take_dependencies(&self) -> HashSet<PathBuf> {
        std::mem::take(&mut *self.captured_deps.borrow_mut())
    }
}

/// Marks the VM as compiling for as long as it is alive.
///
/// While any guard lives, a lock is not an input: every `d.fs` reader refuses a
/// `*.lock.json` path and the enumerators hide them. Being an RAII value means
/// a compile that returns early with `?` still reopens the VM.
pub struct CompileGuard {
    depth: Arc<AtomicU32>,
}

impl Drop for CompileGuard {
    fn drop(&mut self) {
        self.depth.fetch_sub(1, Ordering::Release);
    }
}

/// Reads `path` through the entity memo.
///
/// The Lua value is computed at most once per `(kind, path)` while the memo is
/// alive, and the path is always recorded as a dependency so hot reload reacts
/// to external files.
pub fn read_with_entity_memo<F>(
    lua: &Lua,
    kind: &str,
    path: &Path,
    compute: F,
) -> mlua::Result<mlua::Value>
where
    F: FnOnce(&Path) -> anyhow::Result<serde_json::Value>,
{
    let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
        return Err(mlua::Error::external(anyhow::anyhow!(
            "EngineContext is not initialized"
        )));
    };
    ctx.record_dependency(path)?;

    if let Some(value) = ctx.memo_get(kind, path) {
        return Ok(value);
    }

    let json_val = compute(path).map_err(mlua::Error::external)?;
    let value = lua.to_value_with(&json_val, LuaRuntime::serialize_options())?;
    ctx.memo_insert(kind, path, value.clone());

    Ok(value)
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

    pub fn clear_entity_memo(&self) {
        if let Some(ctx) = self.lua.app_data_ref::<EngineContext>() {
            ctx.clear_entity_memo();
        }
    }

    /// Records the globals `init.lua` left behind as the baseline for
    /// [`Self::reset_globals`].
    pub fn capture_globals(&self) -> Result<()> {
        let baseline = self
            .lua
            .create_table()
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        for pair in self.lua.globals().pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair.map_err(|e| anyhow::anyhow!("{e}"))?;
            baseline
                .raw_set(key, value)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        // An engine with no `EngineContext` (`Lua::new`) cannot hold a
        // baseline; every engine built by `with_cache_root` installs one.
        if let Some(ctx) = self.lua.app_data_ref::<EngineContext>() {
            ctx.set_globals_baseline(baseline);
        }
        Ok(())
    }

    /// Puts `_G` back to the state `init.lua` left it in, so the next entity
    /// starts clean.
    ///
    /// This covers every top-level change a hook can make: a new key (of any
    /// type), an overwritten value, a deleted one. It does not reach inside a
    /// config table a hook mutates in place, and it cannot undo a closure
    /// upvalue captured at `init.lua` time. A failed compile reopens the VM
    /// instead of relying on this.
    pub fn reset_globals(&self) -> Result<()> {
        // No baseline means `init.lua` never ran on this VM, so nothing it
        // creates can be a leak yet.
        let Some(baseline) = self
            .lua
            .app_data_ref::<EngineContext>()
            .and_then(|ctx| ctx.globals_baseline())
        else {
            return Ok(());
        };

        let globals = self.lua.globals();
        let mut present = Vec::new();
        for pair in globals.pairs::<mlua::Value, mlua::Value>() {
            let (key, _) = pair.map_err(|e| anyhow::anyhow!("{e}"))?;
            present.push(key);
        }

        // Raw writes on purpose: a hook that sets an `_G` metatable must not
        // see the restore through `__newindex`.
        for key in present {
            globals
                .raw_set(key, mlua::Value::Nil)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        for pair in baseline.pairs::<mlua::Value, mlua::Value>() {
            let (key, value) = pair.map_err(|e| anyhow::anyhow!("{e}"))?;
            globals
                .raw_set(key, value)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Ok(())
    }

    /// Starts a compile scope on this VM.
    pub fn compile_guard(&self) -> Result<CompileGuard> {
        self.lua
            .app_data_ref::<EngineContext>()
            .map(|ctx| ctx.compile_guard())
            .context("EngineContext is not initialized")
    }

    /// Injects a reader under `d.fs.<name>` (and `datamile.fs.<name>`).
    ///
    /// `datamile-lua` stays blind to media crates: higher layers supply readers
    /// such as `read_audio` that use the same dependency and memo plumbing.
    pub fn register_fs_function<F>(&self, name: &str, func: F) -> Result<()>
    where
        F: Fn(&Lua, Option<String>) -> mlua::Result<mlua::Value> + 'static,
    {
        let fs_table: Table = self
            .lua
            .globals()
            .get::<Table>("datamile")
            .and_then(|d| d.get::<Table>("fs"))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        fs_table
            .set(
                name,
                self.lua
                    .create_function(func)
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
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

fn record_dependency(lua: &Lua, raw: Option<String>) -> mlua::Result<()> {
    let Some(raw) = raw else {
        return Ok(());
    };
    let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
        return Ok(());
    };

    ctx.record_dependency(&datamile_fs::expand_path(&raw))
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
        lua.create_function(record_dependency)?,
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
