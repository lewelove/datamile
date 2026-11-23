use anyhow::Result;
use datamile_lua_domain::ServerConfig;
use serde::{Deserialize, Serialize};

use crate::lua::ResolvedConfig;

/// Loads `init.lua` and runs the album-level checks every command relies on,
/// including hot reloads, so no entry point can run with an unfiltered or
/// unsupported config.
pub fn load() -> Result<ResolvedConfig> {
    let mut config = ResolvedConfig::load()?;
    crate::compile::context::validate_config(&mut config)?;
    Ok(config)
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub compiler: CompilerConfig,
    #[serde(default)]
    pub server: ServerConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct StorageConfig {
    pub music_directory: String,
    pub cache: String,
    pub state: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            music_directory: String::new(),
            cache: "~/.cache/datamile".to_string(),
            state: "~/.local/share/datamile".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CompilerConfig {
    pub manifests: Option<Vec<String>>,
    pub audio_extensions: Option<Vec<String>>,
    pub jobs: Option<usize>,
}
