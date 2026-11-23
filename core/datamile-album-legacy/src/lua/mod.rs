#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use datamile_lua::LuaRuntime;
use datamile_lua_domain::{
    ActionDef, DomainConfig, DomainLuaEngine, FormattedGrouperResult,
    GrouperFormatContext, InterfaceDef, LogicManifest, ResolvedDomainConfig,
};
use serde_json::Value;

use crate::config::AppConfig;

const LUA_COMPILER: &str = include_str!("compiler.lua");
const LUA_ALBUM_ACTIONS: &str = include_str!("album_actions.lua");

const ALBUM_EXTENSIONS: &[(&str, &str)] = &[
    ("compiler.lua", LUA_COMPILER),
    ("album_actions.lua", LUA_ALBUM_ACTIONS),
];

pub struct EvaluatedLuaData {
    pub app: AppConfig,
    pub interfaces: HashMap<String, InterfaceDef>,
    pub actions: HashMap<String, ActionDef>,
    pub dependencies: Vec<PathBuf>,
    pub manifest: LogicManifest,
}

/// Album-domain adapter over [`DomainLuaEngine`]: adds the album compiler and
/// the album built-in actions, and unwraps `{ album, tracks }` for logic.
pub struct LuaEngine {
    pub domain: DomainLuaEngine,
}

fn deserialize_app_config(config: &DomainConfig) -> Result<AppConfig> {
    let value = serde_json::to_value(config)?;
    serde_json::from_value(value).context("Failed to deserialize Datamile config")
}

/// Mirrors the legacy Lua unwrap: when the lock has an `album` table, evaluate
/// it with `nodes.track` also exposed as `tracks` so album filters keep working.
fn album_entity(album_val: &Value) -> Value {
    let Some(album) = album_val.get("album").filter(|v| v.is_object()) else {
        return album_val.clone();
    };

    let mut entity = album.clone();
    let tracks = album
        .pointer("/nodes/track")
        .filter(|t| !t.is_null() && !matches!(t, Value::Bool(false)));

    if let Some(tracks) = tracks
        && let Some(obj) = entity.as_object_mut()
        && obj.get("tracks").is_none_or(Value::is_null)
    {
        obj.insert("tracks".to_string(), tracks.clone());
    }

    entity
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        let domain = DomainLuaEngine::new()?;
        for (name, script) in ALBUM_EXTENSIONS {
            domain.load_extension(name, script)?;
        }
        Ok(Self { domain })
    }

    #[must_use]
    pub const fn runtime(&self) -> &LuaRuntime {
        &self.domain.runtime
    }

    pub fn evaluate_config(&self, path: &Path) -> Result<EvaluatedLuaData> {
        let evaluated = self.domain.evaluate_config(path)?;

        Ok(EvaluatedLuaData {
            app: deserialize_app_config(&evaluated.config)?,
            interfaces: evaluated.interfaces,
            actions: evaluated.actions,
            dependencies: evaluated.dependencies,
            manifest: evaluated.manifest,
        })
    }

    pub fn execute_interface(&self, name: &str, ctx: &serde_json::Value) -> Result<()> {
        self.domain.execute_interface(name, ctx)
    }

    pub fn execute_action(
        &self,
        name: &str,
        intermediary: &serde_json::Value,
    ) -> Result<()> {
        self.domain.execute_action(name, intermediary)
    }

    pub fn evaluate_album_logic(&self, album_val: &Value) -> Result<Value> {
        self.domain.evaluate_logic(&album_entity(album_val))
    }

    pub fn execute_dispatcher(
        &self,
        ctx_val: &Value,
        manifests_val: &Value,
    ) -> Result<Value> {
        let dispatcher: mlua::Function = self
            .runtime()
            .lua
            .globals()
            .get("__DATAMILE_DISPATCHER")
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        let lua_ctx = self.runtime().to_value(ctx_val)?;
        let lua_manifests = self.runtime().to_value(manifests_val)?;

        let res: mlua::Table = dispatcher
            .call((lua_ctx, lua_manifests))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        self.runtime().from_value(mlua::Value::Table(res))
    }

    #[must_use]
    pub fn evaluate_grouper_format(
        &self,
        grouper_id: &str,
        ctx: &GrouperFormatContext<'_>,
    ) -> FormattedGrouperResult {
        self.domain.evaluate_grouper_format(grouper_id, ctx)
    }
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
        let domain = ResolvedDomainConfig::load_with_extensions(ALBUM_EXTENSIONS)?;
        let config_dir = domain
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let mut app = deserialize_app_config(&domain.config)?;
        if !app.storage.music_directory.is_empty() {
            app.storage.music_directory =
                datamile_fs::resolve_path(&app.storage.music_directory, &config_dir)
                    .to_string_lossy()
                    .to_string();
        }

        Ok(Self {
            app,
            interfaces: domain.interfaces,
            actions: domain.actions,
            dependencies: domain.dependencies,
            manifest: domain.manifest,
            path: domain.path,
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
