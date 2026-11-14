pub mod api;
pub mod inotify;
pub mod library;
pub mod logic;
pub mod mpd;
pub mod state;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use libdatamile::utils::expand_path;
use tokio::sync::{RwLock, broadcast};
use tower_http::cors::{Any, CorsLayer};

use self::state::{AppConfig as ServerConfig, AppState};

fn create_directories(cache_root: &Path, state_root: &Path) {
    std::fs::create_dir_all(cache_root).ok();
    std::fs::create_dir_all(cache_root.join("images").join("master")).ok();
    std::fs::create_dir_all(cache_root.join("images").join("static")).ok();
    std::fs::create_dir_all(cache_root.join("images").join("dynamic")).ok();
    std::fs::create_dir_all(cache_root.join("images").join("metadata")).ok();
    std::fs::create_dir_all(cache_root.join("harvest")).ok();
    std::fs::create_dir_all(cache_root.join("libraries")).ok();
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

pub async fn run(port: u16) -> Result<()> {
    let config = libdatamile::lua::ResolvedConfig::load()
        .context("Failed to load application configuration")?;
    let config_dir = config
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let media_roots: Vec<_> = config
        .root_dirs()
        .into_iter()
        .map(|path| path.canonicalize().unwrap_or(path))
        .collect();
    if media_roots.is_empty() {
        anyhow::bail!("No root media type declares a dir");
    }

    let cache_root = expand_path(&config.app.storage.cache);
    let state_root = expand_path(&config.app.storage.state);

    create_directories(&cache_root, &state_root);

    let covers = config.covers.clone();
    let interfaces = config.interfaces.clone();
    let actions = config.actions.clone();
    let resolved_dependencies = config.dependencies.clone();

    let mut logic_engine = logic::LogicEngine::new()?;

    let server_config = ServerConfig {
        media_roots: media_roots.clone(),
        cache_root: cache_root.clone(),
        state_root: state_root.clone(),
        resolved_dependencies,
        covers,
        interfaces,
        actions,
        config_dir: config_dir.clone(),
        config_path: config.path.clone(),
        app: config.app.clone(),
    };

    let ui_state_val = load_state(&state_root);

    let lib_scanner = library::scanner::Library::new(media_roots.clone());
    lib_scanner.scan(&mut logic_engine);

    let logic_arc = Arc::new(RwLock::new(logic_engine));
    let (tx, _) = broadcast::channel(1024);

    let mpd_engine = mpd::start_actor(
        tx.clone(),
        Arc::clone(&logic_arc),
        Arc::new(server_config.clone()),
    );

    let deps_json_path = crate::update::cache::deps_graph_path(&cache_root, &media_roots);
    let initial_deps_graph = state::DependencyGraph::load_from_file(&deps_json_path);

    let app_state = Arc::new(AppState {
        logic: Arc::clone(&logic_arc),
        ui_state: RwLock::new(ui_state_val),
        tx,
        config: RwLock::new(server_config),
        mpd_engine,
        tracked_roots: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        full_rescan_needed: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        active_writes: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        cover_tasks: Arc::new(state::CoverTaskRegistry::default()),
        update_lock: Arc::new(std::sync::Mutex::new(state::UpdateState::default())),
        deps_graph: Arc::new(RwLock::new(initial_deps_graph)),
    });

    inotify::start(Arc::clone(&app_state));

    let app_state_clone = Arc::clone(&app_state);
    tokio::spawn(async move {
        if let Err(e) = crate::update::run_server_update(
            app_state_clone,
            None,
            crate::update::client::ForceMode::Preserve,
            None,
            crate::compile::LogVerbosity::Silent,
            None,
        )
        .await
        {
            log::error!("Startup auto-update error: {e}");
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = api::router(Arc::clone(&app_state)).layer(cors);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log::info!("Datamile Server listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}
