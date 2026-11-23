use datamile_lua_domain::ServerConfig;
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
pub struct ManifestConfig {
    pub audio_extensions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CompilerConfig {
    pub manifests: Option<Vec<String>>,
    pub audio_extensions: Option<Vec<String>>,
    pub jobs: Option<usize>,
}
