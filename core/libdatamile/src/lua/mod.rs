pub mod functions;

#[cfg(test)]
mod tests;

use crate::config::{ActionDef, AppConfig, CoversConfig, CoversRegistry, InterfaceDef};
use crate::error::DatamileError;
use anyhow::{Context, Result};
use indexmap::IndexMap;
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LUA_CORE: &str = include_str!("core.lua");
const LUA_CONFIG: &str = include_str!("config.lua");
const LUA_COMPILER: &str = include_str!("compiler.lua");
const LUA_DEFAULTS: &str = include_str!("defaults.lua");
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
    captured_files: RefCell<HashSet<PathBuf>>,
    captured_dirs: RefCell<HashSet<PathBuf>>,
}

impl EngineContext {
    #[must_use]
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            captured_files: RefCell::new(HashSet::new()),
            captured_dirs: RefCell::new(HashSet::new()),
        }
    }

    /// Record an inspected path. A probe for a missing file records its parent
    /// directory, so creating the file later marks the owner dirty.
    pub fn record_dependency(&self, path: &Path) {
        if let Ok(canon) = path.canonicalize() {
            if canon.is_file() {
                self.captured_files.borrow_mut().insert(canon);
            } else {
                self.captured_dirs.borrow_mut().insert(canon);
            }
            return;
        }

        if let Some(parent) = path.parent()
            && let Ok(canon) = parent.canonicalize()
        {
            self.captured_dirs.borrow_mut().insert(canon);
        }
    }

    pub fn record_dir(&self, path: &Path) {
        if let Ok(canon) = path.canonicalize()
            && canon.is_dir()
        {
            self.captured_dirs.borrow_mut().insert(canon);
        }
    }

    pub fn take_dependencies(&self) -> HashSet<PathBuf> {
        std::mem::take(&mut *self.captured_files.borrow_mut())
    }

    pub fn take_directories(&self) -> HashSet<PathBuf> {
        std::mem::take(&mut *self.captured_dirs.borrow_mut())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FilterDef {
    pub label: String,
    #[serde(default)]
    pub r#type: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct GrouperDef {
    pub label: String,
    #[serde(default)]
    pub r#type: Option<String>,
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
    pub r#type: Option<String>,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct LibraryDef {
    pub label: String,
    #[serde(default)]
    pub r#type: Option<String>,
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
    pub r#type: Option<String>,
    #[serde(default)]
    pub reverse: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct CabinetDef {
    pub label: String,
    #[serde(default)]
    pub r#type: Option<String>,
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
    pub covers: CoversRegistry,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
}

/// Child targets emitted by `__DATAMILE_MEDIA_PREPARE`.
pub struct PreparedChildren {
    pub field: String,
    pub type_name: String,
    pub targets: Vec<serde_json::Value>,
}

pub struct PreparedEntity {
    pub ctx_id: u64,
    pub children: Vec<PreparedChildren>,
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

fn json_str(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("expected string field '{key}' in {value}"))
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

fn validate_audio_extensions(
    exts: &[String],
    section: &str,
) -> Result<Vec<String>, DatamileError> {
    let mut result = Vec::with_capacity(exts.len());
    for ext in exts {
        if !crate::harvest::SUPPORTED_AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            return Err(DatamileError::UnsupportedAudioExtension {
                extension: ext.clone(),
                section: section.to_string(),
            });
        }
        result.push(ext.clone());
    }
    Ok(result)
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let default_cache = crate::utils::expand_path("~/.cache/datamile");
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
        lua.load(LUA_DEFAULTS)
            .exec()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("Failed to load defaults.lua")?;
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

        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));

        if let Some(media_directory) = MEDIA_DIRECTORY_OVERRIDE.get()
            && let Ok(registry) = self.lua.globals().get::<Table>("REGISTRY")
            && let Ok(config) = registry.get::<Table>("config")
            && let Ok(storage) = config.get::<Table>("storage")
        {
            storage
                .set(
                    "music_directory",
                    media_directory.to_string_lossy().as_ref(),
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        self.resolve_music_directory(config_dir)?;

        let app_config: AppConfig = call_getter(&self.lua, "__DATAMILE_GET_CONFIG")?;

        let resolved_cache = if app_config.storage.cache.is_empty() {
            crate::utils::expand_path("~/.cache/datamile")
        } else {
            crate::utils::resolve_path(&app_config.storage.cache, config_dir)
        };
        if let Some(mut ctx) = self.lua.app_data_mut::<EngineContext>() {
            ctx.cache_root = resolved_cache;
        } else {
            self.lua.set_app_data(EngineContext::new(resolved_cache));
        }

        let covers: CoversRegistry = call_getter(&self.lua, "__DATAMILE_GET_COVERS")?;
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
            let p = crate::utils::expand_path(&d);
            dependencies.push(p.canonicalize().unwrap_or(p));
        }

        if let Some(ctx) = self.lua.app_data_ref::<EngineContext>() {
            for dep in ctx.take_dependencies() {
                dependencies.push(dep);
            }
            ctx.take_directories();
        }
        dependencies.sort();
        dependencies.dedup();

        Ok(EvaluatedLuaData {
            app: app_config,
            covers,
            interfaces,
            actions,
            dependencies,
            manifest,
        })
    }

    /// Resolve `storage.music_directory` inside the Lua registry so Lua-side
    /// path math (relative ids, source matching) sees absolute paths, matching
    /// what `discover_roots` hands to the compiler.
    fn resolve_music_directory(&self, config_dir: &Path) -> Result<()> {
        let registry: Table = self
            .lua
            .globals()
            .get("REGISTRY")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let Ok(config) = registry.get::<Table>("config") else {
            return Ok(());
        };
        let Ok(storage) = config.get::<Table>("storage") else {
            return Ok(());
        };

        let Ok(music_directory) = storage.get::<Option<String>>("music_directory") else {
            return Ok(());
        };
        let Some(raw) = music_directory.filter(|value| !value.is_empty()) else {
            return Ok(());
        };

        let resolved = crate::utils::resolve_path(&raw, config_dir);
        let resolved = resolved.canonicalize().unwrap_or(resolved);

        storage
            .set("music_directory", resolved.to_string_lossy().as_ref())
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        Ok(())
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

    pub fn evaluate_entity_logic(
        &self,
        root_type: &str,
        lock_val: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let globals = self.lua.globals();
        let eval_fn: mlua::Function = globals
            .get("__DATAMILE_EVALUATE_ENTITY_LOGIC")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let opts = SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);
        let lua_lock = self
            .lua
            .to_value_with(lock_val, opts)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let res: mlua::Table = eval_fn
            .call((root_type, lua_lock))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let json_res: serde_json::Value = self
            .lua
            .from_value(mlua::Value::Table(res))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(json_res)
    }

    pub fn media_roots(&self) -> Result<Vec<String>> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_ROOTS")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let table: Table = func.call(()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut roots = Vec::new();
        for value in table.sequence_values::<String>() {
            roots.push(value.map_err(|e| anyhow::anyhow!("{e}"))?);
        }
        Ok(roots)
    }

    pub fn media_select(
        &self,
        type_name: &str,
        source: &serde_json::Value,
    ) -> Result<Vec<serde_json::Value>> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_SELECT")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let lua_source = self
            .lua
            .to_value(source)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let table: Table = func
            .call((type_name, lua_source))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut result = Vec::new();
        for value in table.sequence_values::<mlua::Value>() {
            let value = value.map_err(|e| anyhow::anyhow!("{e}"))?;
            result.push(
                self.lua
                    .from_value(value)
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
        }
        Ok(result)
    }

    pub fn media_lock(
        &self,
        type_name: &str,
        source: &serde_json::Value,
    ) -> Result<Option<PathBuf>> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_LOCK")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let lua_source = self
            .lua
            .to_value(source)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let value: mlua::Value = func
            .call((type_name, lua_source))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        match value {
            mlua::Value::String(s) => Ok(Some(PathBuf::from(
                s.to_str().map_err(|e| anyhow::anyhow!("{e}"))?.to_string(),
            ))),
            mlua::Value::Nil => Ok(None),
            other => Err(anyhow::anyhow!(
                "media lock for {type_name} returned unsupported value: {other:?}"
            )),
        }
    }

    /// Stage 1: run inputs and child selectors. The context stays alive in Lua
    /// until `media_finalize` or `media_drop` collects it.
    pub fn media_prepare(
        &self,
        type_name: &str,
        seed: &serde_json::Value,
    ) -> Result<PreparedEntity> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_PREPARE")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let lua_seed = self
            .lua
            .to_value(seed)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let value: mlua::Value = func
            .call((type_name, lua_seed))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let prepared: serde_json::Value = self
            .lua
            .from_value(value)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let ctx_id = prepared
            .get("ctx_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                anyhow::anyhow!("media prepare for '{type_name}' returned no ctx")
            })?;

        let mut children = Vec::new();
        if let Some(items) = prepared
            .get("children")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                children.push(PreparedChildren {
                    field: json_str(item, "field")?,
                    type_name: json_str(item, "type")?,
                    targets: item
                        .get("targets")
                        .and_then(serde_json::Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                });
            }
        }

        Ok(PreparedEntity { ctx_id, children })
    }

    /// Stage 2: run the output boxes with the compiled children attached.
    pub fn media_finalize(
        &self,
        type_name: &str,
        ctx_id: u64,
        children: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_FINALIZE")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let lua_children = self
            .lua
            .to_value(children)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let value: mlua::Value = func
            .call((type_name, ctx_id, lua_children))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        self.lua
            .from_value(value)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn media_drop(&self, ctx_id: u64) -> Result<()> {
        let func: mlua::Function = self
            .lua
            .globals()
            .get("__DATAMILE_MEDIA_DROP")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        func.call::<()>(ctx_id).map_err(|e| anyhow::anyhow!("{e}"))
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

static CONFIG_PATH_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
static MEDIA_DIRECTORY_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

pub fn set_config_path_override(path: PathBuf) {
    let _ = CONFIG_PATH_OVERRIDE.set(path);
}

pub fn set_media_directory_override(path: PathBuf) {
    let _ = MEDIA_DIRECTORY_OVERRIDE.set(path);
}

#[must_use]
pub fn resolve_config_path() -> Option<PathBuf> {
    if let Some(override_path) = CONFIG_PATH_OVERRIDE.get()
        && override_path.exists()
    {
        return Some(override_path.clone());
    }

    if let Some(home_config) =
        dirs::home_dir().map(|h| h.join(".config/datamile/init.lua"))
        && home_config.exists()
    {
        return Some(home_config);
    }

    if let Ok(env_path) = std::env::var("DATAMILE_CONFIG_PATH") {
        let p = PathBuf::from(env_path);
        let p = if p.is_dir() { p.join("init.lua") } else { p };
        if p.exists() {
            return Some(p);
        }
    }

    let mut curr = std::env::current_dir().ok()?;
    loop {
        let local_nested = curr.join("config/init.lua");
        if local_nested.exists() {
            return Some(local_nested);
        }

        let local_root = curr.join("init.lua");
        if local_root.exists() {
            return Some(local_root);
        }

        if let Some(parent) = curr.parent() {
            curr = parent.to_path_buf();
        } else {
            break;
        }
    }

    None
}

#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    pub app: AppConfig,
    pub covers: CoversRegistry,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
    pub path: PathBuf,
}

impl ResolvedConfig {
    pub fn load() -> Result<Self> {
        let path = resolve_config_path().context("Could not locate init.lua")?;
        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let engine = LuaEngine::new()?;
        let mut evaluated = engine.evaluate_config(&path)?;

        if let Some(ref manifests) = evaluated.app.compiler.manifests {
            let validated =
                crate::compiler::manifest::validate_and_filter_manifest_names(manifests)?;
            evaluated.app.compiler.manifests = Some(validated);
        }

        if let Some(ref exts) = evaluated.app.manifest.audio_extensions {
            let validated = validate_audio_extensions(exts, "manifest.audio_extensions")?;
            evaluated.app.manifest.audio_extensions = Some(validated);
        }

        if let Some(ref exts) = evaluated.app.compiler.audio_extensions {
            let validated = validate_audio_extensions(exts, "compiler.audio_extensions")?;
            evaluated.app.compiler.audio_extensions = Some(validated);
        }

        if evaluated.covers.targets.is_empty() {
            evaluated.covers.targets.push(CoversConfig {
                filter: "lanczos".to_string(),
                size: 200,
            });
        }

        if !evaluated.app.storage.music_directory.is_empty() {
            evaluated.app.storage.music_directory = crate::utils::resolve_path(
                &evaluated.app.storage.music_directory,
                config_dir,
            )
            .to_string_lossy()
            .to_string();
        }
        if let Some(media_directory) = MEDIA_DIRECTORY_OVERRIDE.get() {
            evaluated.app.storage.music_directory =
                media_directory.to_string_lossy().to_string();
        }
        if !evaluated.app.storage.cache.is_empty() {
            evaluated.app.storage.cache =
                crate::utils::resolve_path(&evaluated.app.storage.cache, config_dir)
                    .to_string_lossy()
                    .to_string();
        }
        if !evaluated.app.storage.state.is_empty() {
            evaluated.app.storage.state =
                crate::utils::resolve_path(&evaluated.app.storage.state, config_dir)
                    .to_string_lossy()
                    .to_string();
        }

        let mut dependencies = evaluated.dependencies;
        dependencies.push(path.clone().canonicalize().unwrap_or_else(|_| path.clone()));

        Ok(Self {
            app: evaluated.app,
            covers: evaluated.covers,
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
