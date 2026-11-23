pub mod functions;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};
use serde::{Deserialize, Serialize};

use crate::config::{ActionDef, AppConfig, InterfaceDef};

const LUA_CORE: &str = include_str!("core.lua");
const LUA_CONFIG: &str = include_str!("config.lua");
const LUA_COMPILER: &str = include_str!("compiler.lua");
const LUA_ACTIONS: &str = include_str!("actions.lua");
const LUA_LOGIC: &str = include_str!("logic.lua");

#[derive(Clone, Copy, Debug, Default)]
pub struct GrouperFormatContext<'a> {
    pub value: &'a str,
    pub count: u64,
    pub pct: f64,
    pub duration_millis: u64,
    pub total_tracks: u64,
    pub total_discs: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FormattedGrouperResult {
    pub label: String,
    pub sublabel: Option<String>,
    pub sort: serde_json::Value,
    pub parent: Option<String>,
}

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

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FilterDef {
    pub label: String,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct GrouperDef {
    pub label: String,
    #[serde(default)]
    pub index: Option<bool>,
    #[serde(default)]
    pub count: Option<bool>,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct OrderDef {
    pub label: String,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LibraryDef {
    pub label: String,
    #[serde(default)]
    pub filters: Vec<String>,
    #[serde(default)]
    pub groupers: Vec<String>,
    #[serde(default)]
    pub orders: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_filters: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_groupers: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_orders: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct ShelfDef {
    pub label: String,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct CabinetDef {
    pub label: String,
    #[serde(default)]
    pub shelves: Vec<String>,
    #[serde(default)]
    pub orders: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_shelves: Vec<String>,
    #[serde(skip_deserializing, default)]
    pub allowed_orders: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LogicManifest {
    pub filters: IndexMap<String, FilterDef>,
    pub groupers: IndexMap<String, GrouperDef>,
    pub orders: IndexMap<String, OrderDef>,
    pub libraries: IndexMap<String, LibraryDef>,
    pub shelves: IndexMap<String, ShelfDef>,
    pub cabinets: IndexMap<String, CabinetDef>,
    #[serde(default)]
    pub filters_order: Vec<String>,
    #[serde(default)]
    pub groupers_order: Vec<String>,
    #[serde(default)]
    pub orders_order: Vec<String>,
    #[serde(default)]
    pub libraries_order: Vec<String>,
    #[serde(default)]
    pub shelves_order: Vec<String>,
    #[serde(default)]
    pub cabinets_order: Vec<String>,
}

fn sort_ordered_map<V>(map: &mut IndexMap<String, V>, order: &mut Vec<String>) {
    if order.is_empty() {
        *order = map.keys().cloned().collect();
    } else {
        map.sort_by_cached_key(|k, _| {
            order.iter().position(|x| x == k).unwrap_or(usize::MAX)
        });
    }
}

impl LogicManifest {
    pub fn normalize(&mut self) {
        self.normalize_ordering();
        self.normalize_grouper_defaults();
        self.resolve_allowed_targets();
    }

    fn normalize_ordering(&mut self) {
        sort_ordered_map(&mut self.filters, &mut self.filters_order);
        sort_ordered_map(&mut self.groupers, &mut self.groupers_order);
        sort_ordered_map(&mut self.orders, &mut self.orders_order);
        sort_ordered_map(&mut self.libraries, &mut self.libraries_order);
        sort_ordered_map(&mut self.shelves, &mut self.shelves_order);
        sort_ordered_map(&mut self.cabinets, &mut self.cabinets_order);
    }

    fn normalize_grouper_defaults(&mut self) {
        for g in self.groupers.values_mut() {
            let idx = g.index.unwrap_or(false);
            g.index = Some(idx);
            if g.count.is_none() {
                g.count = Some(!idx);
            }
        }
    }

    fn resolve_allowed_targets(&mut self) {
        for library in self.libraries.values_mut() {
            library.allowed_filters = library
                .filters
                .iter()
                .filter(|f| self.filters.contains_key(*f))
                .cloned()
                .collect();

            library.allowed_groupers = library
                .groupers
                .iter()
                .filter(|g| self.groupers.contains_key(*g))
                .cloned()
                .collect();

            library.allowed_orders = library
                .orders
                .iter()
                .filter(|o| self.orders.contains_key(*o))
                .cloned()
                .collect();
        }

        for cabinet in self.cabinets.values_mut() {
            cabinet.allowed_shelves = cabinet
                .shelves
                .iter()
                .filter(|s| self.shelves.contains_key(*s))
                .cloned()
                .collect();

            cabinet.allowed_orders = cabinet
                .orders
                .iter()
                .filter(|o| self.orders.contains_key(*o))
                .cloned()
                .collect();
        }
    }
}

pub struct LuaEngine {
    pub lua: Lua,
}

pub struct EvaluatedLuaData {
    pub app: AppConfig,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
}

fn call_getter<T: serde::de::DeserializeOwned>(lua: &Lua, fn_name: &str) -> Result<T> {
    let func: mlua::Function = lua
        .globals()
        .get(fn_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let val: mlua::Value = func.call(()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let parsed: T = lua.from_value(val).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(parsed)
}

fn register_native_functions(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let datamile_table: Table = globals
        .get::<Table>("datamile")
        .or_else(|_| lua.create_table())?;

    let opts = SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);

    functions::register_all(lua, &datamile_table, opts)?;

    globals.set("datamile", datamile_table.clone())?;
    globals.set("d", datamile_table)?;
    Ok(())
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let default_cache = datamile_fs::expand_path("~/.cache/datamile");
        lua.set_app_data(EngineContext::new(default_cache));

        register_native_functions(&lua)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to register native Lua functions")?;
        lua.load(LUA_CORE)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load core.lua")?;
        lua.load(LUA_CONFIG)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load config.lua")?;
        lua.load(LUA_COMPILER)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load compiler.lua")?;
        lua.load(LUA_ACTIONS)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load actions.lua")?;
        lua.load(LUA_LOGIC)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load logic.lua")?;
        Ok(Self { lua })
    }

    pub fn evaluate_config(&self, path: &Path) -> Result<EvaluatedLuaData> {
        let content = std::fs::read_to_string(path)
            .context(format!("Failed to read {}", path.display()))?;

        if let Some(parent) = path.parent() {
            let parent_str = parent.to_string_lossy();
            let code = format!("package.path = package.path .. ';{parent_str}/?.lua'");
            let _: () = self
                .lua
                .load(&code)
                .exec()
                .map_err(|e| anyhow::anyhow!("{e}"))
                .context("Failed to append config directory to package.path")?;

            if let Ok(registry) = self.lua.globals().get::<Table>("REGISTRY") {
                let _ = registry.set("config_dir", parent_str.to_string());
            }
        }

        self.lua
            .load(&content)
            .set_name(path.to_string_lossy())
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to execute init.lua")?;

        let app_config: AppConfig = call_getter(&self.lua, "__DATAMILE_GET_CONFIG")?;

        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let resolved_cache = if app_config.storage.cache.is_empty() {
            datamile_fs::expand_path("~/.cache/datamile")
        } else {
            datamile_fs::resolve_path(&app_config.storage.cache, config_dir)
        };
        if let Some(mut ctx) = self.lua.app_data_mut::<EngineContext>() {
            ctx.cache_root = resolved_cache;
        } else {
            self.lua.set_app_data(EngineContext::new(resolved_cache));
        }

        let interfaces: HashMap<String, InterfaceDef> =
            call_getter(&self.lua, "__DATAMILE_GET_INTERFACES")?;
        let actions: HashMap<String, ActionDef> =
            call_getter(&self.lua, "__DATAMILE_GET_ACTIONS")?;
        let deps_str: Vec<String> =
            call_getter(&self.lua, "__DATAMILE_GET_DEPENDENCIES")?;
        let mut manifest: LogicManifest =
            call_getter(&self.lua, "__DATAMILE_GET_LOGIC_MANIFEST")?;
        manifest.normalize();

        let mut dependencies = Vec::new();
        for d in deps_str {
            let p = datamile_fs::expand_path(&d);
            dependencies.push(p.canonicalize().unwrap_or(p));
        }

        if let Some(ctx) = self.lua.app_data_ref::<EngineContext>() {
            for dep in ctx.take_dependencies() {
                dependencies.push(dep);
            }
        }
        dependencies.sort();
        dependencies.dedup();

        Ok(EvaluatedLuaData {
            app: app_config,
            interfaces,
            actions,
            dependencies,
            manifest,
        })
    }

    pub fn execute_interface(&self, name: &str, ctx: &serde_json::Value) -> Result<()> {
        let globals = self.lua.globals();
        let registry: Table = globals
            .get("REGISTRY")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let interfaces: Table = registry
            .get("interfaces")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let interface_entry: Table = interfaces.get(name).map_err(|_| {
            anyhow::anyhow!("Interface '{name}' is not declared in configuration.")
        })?;
        let run_fn: mlua::Function = interface_entry
            .get("run")
            .map_err(|_| anyhow::anyhow!("Interface '{name}' has no run function."))?;

        let opts = SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);
        let lua_ctx = self
            .lua
            .to_value_with(ctx, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        run_fn
            .call::<()>(lua_ctx)
            .map_err(|e| anyhow::anyhow!("Error executing interface '{name}': {e}"))?;

        Ok(())
    }

    pub fn execute_action(
        &self,
        name: &str,
        intermediary: &serde_json::Value,
    ) -> Result<()> {
        let globals = self.lua.globals();
        let registry: Table = globals
            .get("REGISTRY")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let actions: Table = registry
            .get("actions")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let action_entry: Table = actions.get(name).map_err(|_| {
            anyhow::anyhow!("Action '{name}' is not declared in configuration.")
        })?;
        let run_fn: mlua::Function = action_entry
            .get("run")
            .map_err(|_| anyhow::anyhow!("Action '{name}' has no run function."))?;

        let opts = SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);
        let lua_intermediary = self
            .lua
            .to_value_with(intermediary, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        run_fn
            .call::<()>(lua_intermediary)
            .map_err(|e| anyhow::anyhow!("Error executing action '{name}': {e}"))?;

        Ok(())
    }

    pub fn evaluate_album_logic(
        &self,
        album_val: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let globals = self.lua.globals();
        let eval_fn: mlua::Function = globals
            .get("__DATAMILE_EVALUATE_ALBUM_LOGIC")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let opts = SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);
        let lua_album = self
            .lua
            .to_value_with(album_val, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let res: mlua::Table = eval_fn
            .call(lua_album)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let json_res: serde_json::Value = self
            .lua
            .from_value(mlua::Value::Table(res))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(json_res)
    }

    pub fn execute_dispatcher(
        &self,
        ctx_val: &serde_json::Value,
        manifests_val: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let globals = self.lua.globals();
        let dispatcher: mlua::Function = globals
            .get("__DATAMILE_DISPATCHER")
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let opts = SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);

        let lua_ctx = self
            .lua
            .to_value_with(ctx_val, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let lua_manifests = self
            .lua
            .to_value_with(manifests_val, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let res: mlua::Table = dispatcher
            .call((lua_ctx, lua_manifests))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let json_res: serde_json::Value = self
            .lua
            .from_value(mlua::Value::Table(res))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(json_res)
    }

    #[must_use]
    pub fn evaluate_grouper_format(
        &self,
        grouper_id: &str,
        ctx: &GrouperFormatContext<'_>,
    ) -> FormattedGrouperResult {
        let format_fn: Option<mlua::Function> = (|| {
            let globals = self.lua.globals();
            let registry = globals.get::<Table>("REGISTRY").ok()?;
            let groupers = registry.get::<Table>("groupers").ok()?;
            let grouper_tbl = groupers.get::<Table>(grouper_id).ok()?;
            grouper_tbl.get::<mlua::Function>("format").ok()
        })();

        if let Some(f) = format_fn
            && let Ok(g_tbl) = self.lua.create_table()
        {
            let _ = g_tbl.set("value", ctx.value);
            let _ = g_tbl.set("count", ctx.count);
            let _ = g_tbl.set("pct", ctx.pct);
            let _ = g_tbl.set("duration_millis", ctx.duration_millis);
            let _ = g_tbl.set("total_tracks", ctx.total_tracks);
            let _ = g_tbl.set("total_discs", ctx.total_discs);

            if let Ok(res) = f.call::<mlua::Value>(g_tbl) {
                match res {
                    mlua::Value::Table(tbl) => {
                        let label: String =
                            tbl.get("label").unwrap_or_else(|_| ctx.value.to_string());
                        let sublabel: Option<String> = match tbl
                            .get::<Option<mlua::Value>>("sublabel")
                            .ok()
                            .flatten()
                        {
                            Some(mlua::Value::String(s)) => {
                                s.to_str().ok().as_deref().map(ToString::to_string)
                            }
                            Some(mlua::Value::Integer(i)) => Some(i.to_string()),
                            Some(mlua::Value::Number(n)) => Some(n.to_string()),
                            _ => None,
                        };
                        let sort: serde_json::Value = tbl
                            .get("sort")
                            .ok()
                            .and_then(|v| {
                                self.lua.from_value::<serde_json::Value>(v).ok()
                            })
                            .unwrap_or_else(|| serde_json::json!(label));
                        let parent: Option<String> = tbl
                            .get::<Option<String>>("parent")
                            .ok()
                            .flatten()
                            .filter(|p| !p.is_empty());
                        return FormattedGrouperResult {
                            label,
                            sublabel,
                            sort,
                            parent,
                        };
                    }
                    mlua::Value::String(s) => {
                        if let Ok(str_val) = s.to_str() {
                            let s_owned = str_val.to_string();
                            return FormattedGrouperResult {
                                label: s_owned.clone(),
                                sublabel: None,
                                sort: serde_json::json!(s_owned),
                                parent: None,
                            };
                        }
                    }
                    _ => {}
                }
            }
        }

        FormattedGrouperResult {
            label: ctx.value.to_string(),
            sublabel: None,
            sort: serde_json::json!(ctx.value),
            parent: None,
        }
    }
}

/// Config directory passed on the command line. Process-wide because the server
/// re-loads the config on file changes, long after main parsed the arguments.
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_config_dir(dir: PathBuf) {
    let _ = CONFIG_DIR.set(dir);
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
pub(crate) fn resolve_config(
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

pub fn resolve_config_path() -> Result<PathBuf> {
    let env_dir = std::env::var_os("DATAMILE_CONFIG_PATH").map(PathBuf::from);
    let home_init = dirs::home_dir().map(|h| h.join(".config/datamile/init.lua"));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    resolve_config(
        CONFIG_DIR.get().map(PathBuf::as_path),
        env_dir.as_deref(),
        home_init.as_deref(),
        &cwd,
    )?
    .ok_or_else(|| anyhow::anyhow!("Could not locate init.lua"))
}

#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    pub app: AppConfig,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
    pub path: PathBuf,
}

impl ResolvedConfig {
    pub fn load() -> Result<Self> {
        let path = resolve_config_path()?;
        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let engine = LuaEngine::new()?;
        let mut evaluated = engine.evaluate_config(&path)?;

        if let Some(ref manifests) = evaluated.app.compiler.manifests {
            let validated =
                crate::compiler::manifest::validate_and_filter_manifest_names(manifests)?;
            evaluated.app.compiler.manifests = Some(validated);
        }

        if !evaluated.app.storage.music_directory.is_empty() {
            evaluated.app.storage.music_directory = datamile_fs::resolve_path(
                &evaluated.app.storage.music_directory,
                config_dir,
            )
            .to_string_lossy()
            .to_string();
        }
        if !evaluated.app.storage.cache.is_empty() {
            evaluated.app.storage.cache =
                datamile_fs::resolve_path(&evaluated.app.storage.cache, config_dir)
                    .to_string_lossy()
                    .to_string();
        }
        if !evaluated.app.storage.state.is_empty() {
            evaluated.app.storage.state =
                datamile_fs::resolve_path(&evaluated.app.storage.state, config_dir)
                    .to_string_lossy()
                    .to_string();
        }

        let mut dependencies = evaluated.dependencies;
        dependencies.push(path.clone().canonicalize().unwrap_or_else(|_| path.clone()));

        Ok(Self {
            app: evaluated.app,
            interfaces: evaluated.interfaces,
            actions: evaluated.actions,
            dependencies,
            manifest: evaluated.manifest,
            path,
        })
    }
}

pub fn with_evaluated_lua_vm<F, R>(config_path: &Path, f: F) -> Result<R>
where
    F: FnOnce(&LuaEngine, EvaluatedLuaData) -> Result<R>,
{
    let engine = LuaEngine::new()?;
    let eval = engine.evaluate_config(config_path)?;
    f(&engine, eval)
}
