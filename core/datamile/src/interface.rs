use anyhow::{Context, Result};
use datamile_lua_domain::DomainLuaEngine;
use datamile_media::state::{COMPILE_ROOT_LUA, MEDIA_LUA};
use serde_json::json;

pub fn execute(name: Option<String>) -> Result<()> {
    let raw_name = name.unwrap_or_else(|| "default".to_string());
    let name = raw_name.replace('-', "_");

    let config_path = datamile_lua_domain::resolve_config_path()
        .context("Failed to resolve init.lua")?;
    let engine = DomainLuaEngine::new()?;
    engine.load_extension("media.lua", MEDIA_LUA)?;
    engine.load_extension("compile_root.lua", COMPILE_ROOT_LUA)?;
    datamile_media::lua_bridge::register_native_helpers(&engine.runtime)?;

    let evaluated = engine.evaluate_config(&config_path)?;
    if !evaluated.interfaces.contains_key(&name) {
        anyhow::bail!("Interface '{raw_name}' is not declared in configuration.");
    }

    let host = if evaluated.config.server.host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        evaluated.config.server.host.clone()
    };
    let port = if evaluated.config.server.port == 0 {
        8000
    } else {
        evaluated.config.server.port
    };

    let url = format!("http://{host}:{port}");
    let ws_url = format!("ws://{host}:{port}/ws");

    let ctx = json!({
        "server": {
            "host": host,
            "port": port,
            "url": url,
            "ws_url": ws_url
        }
    });

    engine.execute_interface(&name, &ctx)
}
