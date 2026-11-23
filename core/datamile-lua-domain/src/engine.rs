use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use datamile_lua::LuaRuntime;
use mlua::{LuaSerdeExt, Table};

use crate::config::{ActionDef, DomainConfig, InterfaceDef};
use crate::logic::{FormattedGrouperResult, GrouperFormatContext, LogicManifest};

const LUA_CORE: &str = include_str!("core.lua");
const LUA_CONFIG: &str = include_str!("config.lua");
const LUA_ACTIONS: &str = include_str!("actions.lua");
const LUA_LOGIC: &str = include_str!("logic.lua");

const DEFAULT_CACHE_ROOT: &str = "~/.cache/datamile";

/// Config directory passed on the command line. Process-wide because the server
/// re-loads the config on file changes, long after main parsed the arguments.
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_config_dir(dir: PathBuf) {
    let _ = CONFIG_DIR.set(dir);
}

pub fn resolve_config_path() -> Result<PathBuf> {
    let env_dir = std::env::var_os("DATAMILE_CONFIG_PATH").map(PathBuf::from);
    let home_init = dirs::home_dir().map(|h| h.join(".config/datamile/init.lua"));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    datamile_lua::resolve_config(
        CONFIG_DIR.get().map(PathBuf::as_path),
        env_dir.as_deref(),
        home_init.as_deref(),
        &cwd,
    )?
    .ok_or_else(|| anyhow::anyhow!("Could not locate init.lua"))
}

/// The media-agnostic Datamile Lua API: `config`, `interface`, `action`,
/// `filter`, `grouper`, `order`, `library`, `shelf`, `cabinet`.
pub struct DomainLuaEngine {
    pub runtime: LuaRuntime,
}

pub struct EvaluatedDomainData {
    pub config: DomainConfig,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
}

impl DomainLuaEngine {
    pub fn new() -> Result<Self> {
        let runtime =
            LuaRuntime::with_cache_root(datamile_fs::expand_path(DEFAULT_CACHE_ROOT))?;
        let engine = Self { runtime };

        for (name, script) in [
            ("core.lua", LUA_CORE),
            ("config.lua", LUA_CONFIG),
            ("actions.lua", LUA_ACTIONS),
            ("logic.lua", LUA_LOGIC),
        ] {
            engine.load_extension(name, script)?;
        }

        Ok(engine)
    }

    /// Attaches a domain script (e.g. an album compiler) before `init.lua` runs.
    pub fn load_extension(&self, name: &str, script: &str) -> Result<()> {
        self.runtime
            .exec_chunk(script, name)
            .context(format!("Failed to load {name}"))
    }

    pub fn evaluate_config(&self, path: &Path) -> Result<EvaluatedDomainData> {
        let content = std::fs::read_to_string(path)
            .context(format!("Failed to read {}", path.display()))?;

        if let Some(parent) = path.parent() {
            self.runtime.append_package_path(parent)?;
            self.registry()?
                .set("config_dir", parent.to_string_lossy().to_string())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }

        let name = path.to_string_lossy();
        self.runtime
            .exec_chunk(&content, &name)
            .context("Failed to execute init.lua")?;

        let config: DomainConfig = self.runtime.call_global("__DATAMILE_GET_CONFIG")?;

        let config_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let resolved_cache = if config.storage.cache.is_empty() {
            datamile_fs::expand_path(DEFAULT_CACHE_ROOT)
        } else {
            datamile_fs::resolve_path(&config.storage.cache, config_dir)
        };
        self.runtime.set_cache_root(resolved_cache);

        let interfaces: HashMap<String, InterfaceDef> =
            self.runtime.call_global("__DATAMILE_GET_INTERFACES")?;
        let actions: HashMap<String, ActionDef> =
            self.runtime.call_global("__DATAMILE_GET_ACTIONS")?;
        let mut manifest: LogicManifest =
            self.runtime.call_global("__DATAMILE_GET_LOGIC_MANIFEST")?;
        manifest.normalize();

        let mut dependencies: Vec<PathBuf> =
            self.runtime.take_dependencies().into_iter().collect();
        dependencies.sort();

        Ok(EvaluatedDomainData {
            config,
            interfaces,
            actions,
            dependencies,
            manifest,
        })
    }

    pub fn execute_interface(&self, name: &str, ctx: &serde_json::Value) -> Result<()> {
        let registry = self.registry()?;
        let interfaces: Table = registry
            .get("interfaces")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let interface_entry: Table = interfaces.get(name).map_err(|_| {
            anyhow::anyhow!("Interface '{name}' is not declared in configuration.")
        })?;
        let run_fn: mlua::Function = interface_entry
            .get("run")
            .map_err(|_| anyhow::anyhow!("Interface '{name}' has no run function."))?;

        let lua_ctx = self.runtime.to_value(ctx)?;
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
        let registry = self.registry()?;
        let actions: Table = registry
            .get("actions")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let action_entry: Table = actions.get(name).map_err(|_| {
            anyhow::anyhow!("Action '{name}' is not declared in configuration.")
        })?;
        let run_fn: mlua::Function = action_entry
            .get("run")
            .map_err(|_| anyhow::anyhow!("Action '{name}' has no run function."))?;

        let lua_intermediary = self.runtime.to_value(intermediary)?;
        run_fn
            .call::<()>(lua_intermediary)
            .map_err(|e| anyhow::anyhow!("Error executing action '{name}': {e}"))?;

        Ok(())
    }

    pub fn evaluate_logic(
        &self,
        entity: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let eval_fn: mlua::Function = self
            .runtime
            .lua
            .globals()
            .get("__DATAMILE_EVALUATE_LOGIC")
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let lua_entity = self.runtime.to_value(entity)?;
        let res: mlua::Table = eval_fn
            .call(lua_entity)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        self.runtime.from_value(mlua::Value::Table(res))
    }

    #[must_use]
    pub fn evaluate_grouper_format(
        &self,
        grouper_id: &str,
        ctx: &GrouperFormatContext<'_>,
    ) -> FormattedGrouperResult {
        let lua = &self.runtime.lua;
        let format_fn: Option<mlua::Function> = (|| {
            let globals = lua.globals();
            let registry = globals.get::<Table>("REGISTRY").ok()?;
            let groupers = registry.get::<Table>("groupers").ok()?;
            let grouper_tbl = groupers.get::<Table>(grouper_id).ok()?;
            grouper_tbl.get::<mlua::Function>("format").ok()
        })();

        if let Some(f) = format_fn
            && let Ok(g_tbl) = lua.create_table()
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
                            .and_then(|v| lua.from_value::<serde_json::Value>(v).ok())
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

    fn registry(&self) -> Result<Table> {
        self.runtime
            .lua
            .globals()
            .get("REGISTRY")
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

pub struct ResolvedDomainConfig {
    pub config: DomainConfig,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
    pub path: PathBuf,
}

impl ResolvedDomainConfig {
    pub fn load() -> Result<Self> {
        Self::load_with_extensions(&[])
    }

    pub fn load_with_extensions(extensions: &[(&str, &str)]) -> Result<Self> {
        let path = resolve_config_path()?;
        let config_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let engine = DomainLuaEngine::new()?;
        for (name, script) in extensions {
            engine.load_extension(name, script)?;
        }

        let mut evaluated = engine.evaluate_config(&path)?;

        if !evaluated.config.storage.cache.is_empty() {
            evaluated.config.storage.cache =
                resolve_against(&evaluated.config.storage.cache, &config_dir);
        }
        if !evaluated.config.storage.state.is_empty() {
            evaluated.config.storage.state =
                resolve_against(&evaluated.config.storage.state, &config_dir);
        }

        let mut dependencies = evaluated.dependencies;
        dependencies.push(path.canonicalize().unwrap_or_else(|_| path.clone()));

        Ok(Self {
            config: evaluated.config,
            interfaces: evaluated.interfaces,
            actions: evaluated.actions,
            dependencies,
            manifest: evaluated.manifest,
            path,
        })
    }
}

fn resolve_against(path: &str, base: &Path) -> String {
    datamile_fs::resolve_path(path, base)
        .to_string_lossy()
        .to_string()
}

pub fn with_evaluated_domain_vm<F, R>(
    config_path: &Path,
    extensions: &[(&str, &str)],
    f: F,
) -> Result<R>
where
    F: FnOnce(&DomainLuaEngine, EvaluatedDomainData) -> Result<R>,
{
    let engine = DomainLuaEngine::new()?;
    for (name, script) in extensions {
        engine.load_extension(name, script)?;
    }
    let eval = engine.evaluate_config(config_path)?;
    f(&engine, eval)
}
