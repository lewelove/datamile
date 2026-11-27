use std::collections::BTreeMap;
use std::net::IpAddr;

use datamile_mpd::MpdConfig;
use serde::{Deserialize, Serialize};

const DEFAULT_CACHE_DIR: &str = "~/.cache/datamile";
const DEFAULT_STATE_DIR: &str = "~/.local/share/datamile";
const IPV4_PREFIX_MAX: u8 = 32;
const IPV6_PREFIX_MAX: u8 = 128;

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
    /// Extra hostnames or `ip/prefix` CIDR blocks allowed to reach the daemon,
    /// on top of the built-in loopback and `192.168.0.0/16` allowlist.
    pub allowed_origins: Vec<String>,
}

impl ServerConfig {
    /// An allowlist entry is compared against request hosts, so each one must
    /// be a hostname or a CIDR block the daemon can parse.
    pub fn validate_allowed_origins(&self) -> Result<(), String> {
        for entry in &self.allowed_origins {
            if !is_host_or_cidr(entry) {
                return Err(format!(
                    "server.allowed_origins entry '{entry}' is not a hostname or ip/prefix CIDR"
                ));
            }
        }
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8000,
            allowed_origins: Vec::new(),
        }
    }
}

fn is_host_or_cidr(entry: &str) -> bool {
    if entry.is_empty() {
        return false;
    }

    if let Some((address, prefix)) = entry.split_once('/') {
        let Ok(address) = address.parse::<IpAddr>() else {
            return false;
        };
        let max = if address.is_ipv4() {
            IPV4_PREFIX_MAX
        } else {
            IPV6_PREFIX_MAX
        };
        return prefix.parse::<u8>().is_ok_and(|prefix| prefix <= max);
    }

    // Bracketed IPv6 (`[::1]`) names the same address as `::1`. A port never
    // belongs here: the guard strips request ports before it compares.
    let host = entry
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(entry);

    if host.parse::<IpAddr>().is_ok() {
        return true;
    }

    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
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

/// Kept only until the daemon switches `ServerConfig.actions` to `ActionMap`.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionDef {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Declared media types per canonical action name, as validated before any
/// `label`/`description` callback runs. An empty list means a generic action.
pub type ActionDeclarations = BTreeMap<String, Vec<String>>;

/// One `label`/`description` pair resolved for one media type.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionPresentation {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One action after its presentation callbacks were evaluated. A generic action
/// carries `label`/`description` directly; a scoped one carries `media`.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionEntry {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<BTreeMap<String, ActionPresentation>>,
}

pub type ActionMap = BTreeMap<String, ActionEntry>;

/// The runtime table passed to `run`. `lock` and `media` are absent when the
/// action runs targetless.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCtx {
    pub lock: Option<serde_json::Value>,
    pub media: Option<String>,
    pub options: Vec<String>,
    pub isatty: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct InterfaceDef {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}
