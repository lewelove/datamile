use datamile_logic::LogicEngine;

use crate::lua::LuaEngine;

/// Rebuilds the logic caches with the album Lua grouper formatter.
pub fn rebuild_cache(logic: &mut LogicEngine) {
    match LuaEngine::new() {
        Ok(engine) => {
            let _ = engine.evaluate_config(&logic.config_path);
            let formatter =
                |grouper: &str, ctx: &datamile_lua_domain::GrouperFormatContext<'_>| {
                    engine.evaluate_grouper_format(grouper, ctx)
                };
            logic.build_cache(&formatter);
        }
        Err(e) => {
            log::error!("Failed to initialize Lua engine for cache rebuild: {e}");
            logic.build_cache(&datamile_logic::plain_grouper_formatter);
        }
    }
}
