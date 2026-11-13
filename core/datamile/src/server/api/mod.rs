pub mod actions;
pub mod assets;
pub mod playback;
pub mod system;
pub mod websocket;

use crate::server::state::AppState;
use axum::{
    Router,
    routing::{any, get, post},
};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", get(websocket::ws_handler))
        .route("/api/state", post(system::update_state))
        .route(
            "/api/internal/notify_force_update",
            post(system::notify_force_update),
        )
        .route("/api/internal/reset", post(system::trigger_full_reset))
        .route("/api/internal/reload", post(system::trigger_reload))
        .route("/api/internal/ingest", post(system::ingest_reload_payloads))
        .route("/api/internal/trigger_update", post(system::trigger_update))
        .route("/api/internal/query", post(system::run_query))
        .route(
            "/api/internal/tracked_albums",
            get(system::get_tracked_albums),
        )
        .route(
            "/api/internal/cache_images",
            post(assets::trigger_cache_images),
        )
        .route("/api/images/{file}", get(assets::get_image))
        .route("/api/album/{*id}", get(assets::get_album_metadata))
        .route("/api/assets/cover/{*id}", get(assets::get_album_cover))
        .route("/api/play/{*id}", post(playback::play_album))
        .route("/api/play-disc/{*id}", post(playback::play_disc))
        .route("/api/queue/{*id}", post(playback::queue_album))
        .route("/api/jump/{index}", post(playback::jump_to_index))
        .route("/api/next", post(playback::next_track))
        .route("/api/prev", post(playback::prev_track))
        .route("/api/stop", post(playback::stop_playback))
        .route("/api/clear", post(playback::clear_queue))
        .route("/api/toggle-pause", post(playback::toggle_pause))
        .route("/api/update-album/{*id}", post(system::force_update_album))
        .route("/api/actions/{name}", any(actions::execute_action))
        .with_state(state)
}
