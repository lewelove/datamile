use datamile_mpd::MpdConfig;
use serde::{Deserialize, Serialize};

const DEFAULT_CACHE_DIR: &str = "~/.cache/datamile";
const DEFAULT_STATE_DIR: &str = "~/.local/share/datamile";

/// The whole surface of `d.config`. Every key and section is declared here, so
/// a typo fails the config instead of being carried along unused.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DomainConfig {
    pub storage: StorageConfig,
    pub server: ServerConfig,
    pub mpd: MpdConfig,
    pub compiler: CompilerConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub cache: String,
    pub state: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            cache: DEFAULT_CACHE_DIR.to_string(),
            state: DEFAULT_STATE_DIR.to_string(),
        }
    }
}

/// Compilation knobs; `jobs = nil` means the available parallelism.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CompilerConfig {
    pub jobs: Option<usize>,
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
