use serde::{Deserialize, Serialize};

const DEFAULT_CACHE_DIR: &str = "~/.cache/datamile";
const DEFAULT_STATE_DIR: &str = "~/.local/share/datamile";

/// Media-agnostic root config. Domain-specific sections (`manifest`, `compiler`,
/// `music_directory`, ...) pass through `extra` untouched so this crate never
/// needs to know their schema.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct DomainConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8000,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct StorageConfig {
    pub cache: String,
    pub state: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            cache: DEFAULT_CACHE_DIR.to_string(),
            state: DEFAULT_STATE_DIR.to_string(),
            extra: serde_json::Map::new(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionDef {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct InterfaceDef {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}
