pub mod api;
pub mod backend;
pub mod instance;
pub mod mpd_bridge;
pub mod state;
pub mod watcher;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
pub use backend::{MediaBackend, PlaybackSlice, UpdateRequest};
pub use datamile_cache::EntityKey;
use datamile_logic::LogicEngine;
use datamile_lua_domain::LogicManifest;
pub use datamile_lua_domain::{ActionCtx, ActionEntry, ActionMap};
pub use instance::{Identity, IdentityMismatch, InstanceLock, VERSION};
use serde_json::json;
pub use state::{
    ImageClaim, ImageTaskGuard, ImageTaskRegistry, ServerConfig, ServerState,
};

/// Port used when the config leaves `server.port` at 0.
pub const DEFAULT_PORT: u16 = 8000;
use tokio::sync::{RwLock, broadcast};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

fn create_directories(cache_root: &Path, state_root: &Path) {
    std::fs::create_dir_all(cache_root).ok();
    std::fs::create_dir_all(cache_root.join("objects")).ok();
    std::fs::create_dir_all(state_root).ok();
}

fn load_state(state_root: &Path) -> serde_json::Value {
    let state_file = state_root.join("state.json");
    if state_file.exists() {
        let data = std::fs::read_to_string(&state_file).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_else(|_| default_state())
    } else {
        default_state()
    }
}

fn default_state() -> serde_json::Value {
    serde_json::json!({
        "activeTab": "home",
        "homeSubView": "libraries",
        "activeLibrary": "library",
        "activeCabinet": "default",
        "sortKey": "default",
        "sortOrder": "default",
        "groupKey": "genre",
        "filter": {
            "key": null,
            "val": null
        },
        "activeShelf": null,
        "activeShelfOrder": "original",
        "activeShelfOrderReverse": false,
        "sidebarWidth": 280
    })
}

/// Boots the HTTP/WS server around a media backend.
///
/// The backend owns every domain decision: what a media type is, how a lock
/// maps to playable URIs, and how an update runs.
pub async fn run(backend: Arc<dyn MediaBackend>) -> Result<()> {
    let mut logic_engine = LogicEngine::new(PathBuf::new(), LogicManifest::default());
    let config = backend
        .reload_config_and_logic(&mut logic_engine)
        .context("Failed to load application configuration")?;

    create_directories(&config.cache_root, &config.state_root);
    backend.scan_library(&mut logic_engine);

    let ui_state = load_state(&config.state_root);
    let logic = Arc::new(RwLock::new(logic_engine));
    let (tx, _) = broadcast::channel(1024);

    let mpd_enabled = config.mpd.enabled;
    let (mpd_engine, mpd_status_tx) = if mpd_enabled {
        datamile_mpd::start_actor(config.mpd.addr())
    } else {
        // A disabled MPD still needs a command sink; the receiver is dropped, so
        // every command is a no-op.
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let (status_tx, _) = broadcast::channel(1);
        (datamile_mpd::MpdEngine::new(tx), status_tx)
    };

    let port = if config.port == 0 {
        DEFAULT_PORT
    } else {
        config.port
    };
    let host: IpAddr = config
        .host
        .parse()
        .with_context(|| format!("server.host '{}' is not an IP address", config.host))?;

    let state = Arc::new(ServerState {
        logic,
        ui_state: RwLock::new(ui_state),
        tx: tx.clone(),
        config: RwLock::new(config),
        mpd_engine,
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: RwLock::new(String::new()),
        reload_error: RwLock::new(None),
        backend: Arc::clone(&backend),
    });

    if mpd_enabled {
        mpd_bridge::start_status_bridge(&mpd_status_tx, Arc::clone(&state));
    }
    watcher::start(Arc::clone(&state));

    if let Err(e) =
        backend.trigger_update(Arc::clone(&state), UpdateRequest::preserve(), Vec::new())
    {
        log::error!("Startup auto-update error: {e}");
    }

    let app = secured_router(Arc::clone(&state)).await;

    let addr = SocketAddr::new(host, port);
    log::info!("Datamile Server listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}

/// The API router with the `Host`/`Origin` guard and the matching CORS policy.
/// One allowlist is built at startup and shared by both, so they never disagree.
pub async fn secured_router(state: Arc<ServerState>) -> Router {
    let allow = {
        let config = state.config.read().await;
        AllowList::from_config(&config.host, &config.allowed_origins)
    };
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate({
            let allow = allow.clone();
            move |origin, _parts| allow.origin_allowed(origin)
        }))
        .allow_methods(Any)
        .allow_headers([header::CONTENT_TYPE]);

    api::router(state)
        .layer(axum::middleware::from_fn_with_state(allow, guard))
        .layer(cors)
}

/// Built-in `192.168.0.0/16` allowance: the LAN range the daemon trusts without
/// an explicit `server.allowed_origins` entry.
const LAN_OCTETS: (u8, u8) = (192, 168);
const IPV4_PREFIX_MAX: u8 = 32;
const IPV6_PREFIX_MAX: u8 = 128;
const BITS_PER_BYTE: u8 = 8;
const LOOPBACK_HOST: &str = "localhost";

/// One parsed `server.allowed_origins` entry.
#[derive(Clone)]
enum AllowEntry {
    Host(String),
    Cidr(IpAddr, u8),
}

/// Everything a `Host` or `Origin` header may name.
#[derive(Clone)]
struct AllowList {
    bound: Option<IpAddr>,
    extra: Vec<AllowEntry>,
}

impl AllowList {
    fn from_config(host: &str, allowed_origins: &[String]) -> Self {
        Self {
            bound: host.parse().ok(),
            extra: allowed_origins
                .iter()
                .filter_map(|entry| parse_allow_entry(entry))
                .collect(),
        }
    }

    /// `host` arrives with any port already stripped.
    fn host_allowed(&self, host: &str) -> bool {
        let host = host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host);

        if host.eq_ignore_ascii_case(LOOPBACK_HOST) {
            return true;
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            return ip.is_loopback()
                || is_lan(ip)
                || Some(ip) == self.bound
                || self.cidr_allows(ip);
        }

        self.extra
            .iter()
            .any(|entry| matches!(entry, AllowEntry::Host(name) if name.eq_ignore_ascii_case(host)))
    }

    fn cidr_allows(&self, ip: IpAddr) -> bool {
        self.extra.iter().any(|entry| match entry {
            AllowEntry::Cidr(network, prefix) => cidr_contains(*network, *prefix, ip),
            AllowEntry::Host(_) => false,
        })
    }

    /// A full `Origin` value (`scheme://host[:port]`) names an allowed host.
    fn origin_allowed(&self, origin: &HeaderValue) -> bool {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        let Some((_, rest)) = origin.split_once("://") else {
            return false;
        };
        let host = rest.split('/').next().unwrap_or(rest);

        self.host_allowed(strip_port(host))
    }
}

fn parse_allow_entry(entry: &str) -> Option<AllowEntry> {
    let Some((address, prefix)) = entry.split_once('/') else {
        return Some(AllowEntry::Host(entry.to_string()));
    };
    let address = address.parse::<IpAddr>().ok()?;
    let prefix = prefix.parse::<u8>().ok()?;

    Some(AllowEntry::Cidr(address, prefix))
}

const fn is_lan(ip: IpAddr) -> bool {
    let IpAddr::V4(ip) = ip else {
        return false;
    };
    let octets = ip.octets();

    octets[0] == LAN_OCTETS.0 && octets[1] == LAN_OCTETS.1
}

fn cidr_contains(network: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (network, ip) {
        (IpAddr::V4(network), IpAddr::V4(ip)) => {
            prefix <= IPV4_PREFIX_MAX
                && masked(u32::from(network), prefix) == masked(u32::from(ip), prefix)
        }
        (IpAddr::V6(network), IpAddr::V6(ip)) => {
            prefix <= IPV6_PREFIX_MAX
                && masked_bytes(&network.octets(), prefix)
                    == masked_bytes(&ip.octets(), prefix)
        }
        _ => false,
    }
}

fn masked(value: u32, prefix: u8) -> u32 {
    if prefix == 0 {
        return 0;
    }

    value & (u32::MAX << (u32::BITS - u32::from(prefix)))
}

fn masked_bytes(value: &[u8], prefix: u8) -> Vec<u8> {
    let mut out = value.to_vec();
    let full = usize::from(prefix / BITS_PER_BYTE);
    let bits = prefix % BITS_PER_BYTE;

    if full < out.len() {
        if bits == 0 {
            out[full..].fill(0);
        } else {
            out[full] &= 0xFF << (BITS_PER_BYTE - bits);
            out[full + 1..].fill(0);
        }
    }

    out
}

/// `[::1]:8000` becomes `[::1]`; `127.0.0.1:8000` becomes `127.0.0.1`.
fn strip_port(value: &str) -> &str {
    if value.starts_with('[') {
        return value.find(']').map_or(value, |end| &value[..=end]);
    }

    match value.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':')
                && !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            host
        }
        _ => value,
    }
}

async fn guard(State(allow): State<AllowList>, request: Request, next: Next) -> Response {
    if is_mutating_api(request.method(), request.uri().path())
        && let Err(error) = check_request(&allow, request.headers())
    {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": error }))).into_response();
    }

    next.run(request).await
}

fn is_mutating_api(method: &Method, path: &str) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::DELETE)
        && path.starts_with("/api/")
}

fn check_request(allow: &AllowList, headers: &HeaderMap) -> Result<(), String> {
    let Some(host) = headers.get(header::HOST) else {
        return Err("missing Host header".to_string());
    };
    let Ok(host) = host.to_str() else {
        return Err("invalid Host header".to_string());
    };
    if !allow.host_allowed(strip_port(host)) {
        return Err(format!("Host '{host}' is not allowed"));
    }

    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    if !allow.origin_allowed(origin) {
        return Err("Origin is not allowed".to_string());
    }

    Ok(())
}

/// Directory holding the watcher's extra non-recursive watch targets.
#[must_use]
pub fn dependency_dirs(dependencies: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = HashSet::new();
    for dep in dependencies {
        if let Some(parent) = dep.parent() {
            dirs.insert(parent.to_path_buf());
        }
    }
    dirs.into_iter().collect()
}
