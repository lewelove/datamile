use anyhow::{Context, Result};
use datamile_album_legacy::config;
use datamile_album_legacy::lua::LuaEngine;
use serde_json::json;

pub fn execute(name: Option<String>) -> Result<()> {
    let raw_name = name.unwrap_or_else(|| "default".to_string());
    let name = raw_name.replace('-', "_");
    let config = config::load().context("Failed to load config")?;

    if !config.interfaces.contains_key(&name) {
        anyhow::bail!("Interface '{raw_name}' is not declared in configuration.");
    }

    let host = if config.app.server.host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        config.app.server.host.clone()
    };
    let port = if config.app.server.port == 0 {
        8000
    } else {
        config.app.server.port
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

    let engine = LuaEngine::new()?;
    engine.evaluate_config(&config.path)?;
    engine.execute_interface(&name, &ctx)?;

    Ok(())
}
