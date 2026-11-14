pub mod actions;
pub mod assets;
pub mod media;
pub mod playback;
pub mod state;
pub mod system;
pub mod websocket;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::server::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", get(websocket::ws_handler))
        .route("/api/media", get(media::list_media))
        .route("/api/media/{type}", get(media::get_media))
        .route("/api/media/{type}/items", get(media::list_items))
        .route("/api/media/{type}/item", get(media::get_item))
        .route("/api/media/{type}/groups/{grouper}", get(media::get_groups))
        .route("/api/playback", get(playback::get))
        .route("/api/playback/play", post(playback::play))
        .route("/api/playback/queue", post(playback::queue))
        .route("/api/playback/jump", post(playback::jump))
        .route("/api/playback/next", post(playback::next))
        .route("/api/playback/prev", post(playback::prev))
        .route("/api/playback/stop", post(playback::stop))
        .route("/api/playback/clear", post(playback::clear))
        .route("/api/playback/toggle-pause", post(playback::toggle_pause))
        .route("/api/state/{namespace}", get(state::get_state))
        .route("/api/state/{namespace}", post(state::save_state))
        .route("/api/actions/{name}", post(actions::execute_action))
        .route(
            "/api/internal/notify_force_update",
            post(system::notify_force_update),
        )
        .route("/api/internal/reset", post(system::trigger_full_reset))
        .route("/api/internal/reload", post(system::trigger_reload))
        .route("/api/internal/ingest", post(system::ingest_reload_payloads))
        .route("/api/internal/trigger_update", post(system::trigger_update))
        .route("/api/internal/query", post(system::run_query))
        .route("/api/internal/tracked_albums", get(system::tracked_albums))
        .route(
            "/api/internal/cache_images",
            post(assets::trigger_cache_images),
        )
        .route("/api/images/{file}", get(assets::image))
        .with_state(state)
}
