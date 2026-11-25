use serde::{Deserialize, Serialize};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 6600;

/// MPD transport settings declared under `d.config({ mpd = { ... } })`.
///
/// `music_directory` must match MPD's own setting: playable URIs are the locks'
/// absolute paths with this base subtracted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MpdConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub music_directory: Option<String>,
}

impl Default for MpdConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
            music_directory: None,
        }
    }
}

impl MpdConfig {
    /// `host:port`, the address the actor and one-shot queries connect to.
    #[must_use]
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Applies `MPD_HOST` / `MPD_PORT` overrides on top of the Lua config.
    #[must_use]
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(host) = std::env::var("MPD_HOST")
            && !host.is_empty()
        {
            self.host = host;
        }
        if let Ok(port) = std::env::var("MPD_PORT")
            && let Ok(port) = port.parse()
        {
            self.port = port;
        }
        self
    }
}
