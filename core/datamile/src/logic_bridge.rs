use std::path::Path;

use datamile_logic::{LogicEngine, plain_grouper_formatter};
use datamile_lua_domain::{FormattedGrouperResult, GrouperFormatContext, LogicManifest};
use serde_json::Value;

/// Lua-backed grouper formatter; `None` when the config cannot be loaded.
///
/// `datamile-logic` never touches Lua, so every entry point that needs a
/// formatter goes through this adapter.
struct GrouperLua {
    engine: libdatamile::lua::LuaEngine,
}

impl GrouperLua {
    fn load(config_path: &Path) -> Option<Self> {
        let engine = libdatamile::lua::LuaEngine::new().ok()?;
        engine.evaluate_config(config_path).ok()?;
        Some(Self { engine })
    }

    fn formatter(
        &self,
    ) -> impl Fn(&str, &GrouperFormatContext<'_>) -> FormattedGrouperResult + '_ {
        move |grouper, ctx| self.engine.evaluate_grouper_format(grouper, ctx)
    }
}

pub fn new_engine(config_path: &Path) -> anyhow::Result<LogicEngine> {
    let manifest =
        libdatamile::lua::with_evaluated_lua_vm(
            config_path,
            |_, eval| Ok(eval.manifest),
        )?;
    Ok(LogicEngine::new(config_path.to_path_buf(), manifest))
}

pub fn rebuild_cache(logic: &mut LogicEngine) {
    match GrouperLua::load(&logic.config_path) {
        Some(lua) => {
            let formatter = lua.formatter();
            logic.build_cache(&formatter);
        }
        None => logic.build_cache(&plain_grouper_formatter),
    }
}

pub fn reload_manifest(
    logic: &mut LogicEngine,
    config_path: &Path,
    manifest: LogicManifest,
) {
    if let Some(lua) = GrouperLua::load(config_path) {
        let formatter = lua.formatter();
        let evaluate = |value: &Value| lua.engine.evaluate_album_logic(value);
        logic.reload_manifest(config_path, manifest, &evaluate, &formatter);
        return;
    }

    let evaluate =
        |_: &Value| -> anyhow::Result<Value> { anyhow::bail!("Lua unavailable") };
    logic.reload_manifest(config_path, manifest, &evaluate, &plain_grouper_formatter);
}

pub fn request_group(
    logic: &LogicEngine,
    library: &str,
    library_filter: Option<&str>,
    grouper: &str,
) -> Vec<Value> {
    GrouperLua::load(&logic.config_path).map_or_else(
        || {
            logic.request_group(
                library,
                library_filter,
                grouper,
                &plain_grouper_formatter,
            )
        },
        |lua| {
            let formatter = lua.formatter();
            logic.request_group(library, library_filter, grouper, &formatter)
        },
    )
}
