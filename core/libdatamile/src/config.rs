use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub manifest: ManifestConfig,
    #[serde(default)]
    pub compiler: CompilerConfig,
    #[serde(default)]
    pub server: ServerConfig,
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

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct StorageConfig {
    pub cache: String,
    pub state: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            cache: "~/.cache/datamile".to_string(),
            state: "~/.local/share/datamile".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ManifestConfig {
    pub audio_extensions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CompilerConfig {
    pub manifests: Option<Vec<String>>,
    pub audio_extensions: Option<Vec<String>>,
    pub jobs: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq, Hash, Default)]
pub struct CoversConfig {
    pub filter: String,
    pub size: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct CoversRegistry {
    pub master: CoversConfig,
    pub targets: Vec<CoversConfig>,
}

impl Default for CoversRegistry {
    fn default() -> Self {
        Self {
            master: CoversConfig {
                filter: "mitchell".to_string(),
                size: 1080,
            },
            targets: Vec::new(),
        }
    }
}
