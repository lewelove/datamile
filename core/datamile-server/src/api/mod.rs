pub mod actions;
pub mod images;
pub mod media;
pub mod playback;
pub mod system;
pub mod tree;
pub mod websocket;

use std::sync::Arc;

use axum::Router;
use axum::routing::{any, get, post};

use crate::state::ServerState;

pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/ws", get(websocket::ws_handler))
        .route("/api/media", get(media::list_types))
        .route("/api/media/{media_type}", get(media::list_media))
        .route("/api/media/{media_type}/{*id}", get(media::media_by_id))
        .route(
            "/api/media/{media_type}/update/{*id}",
            post(media::update_media),
        )
        .route(
            "/api/media/{media_type}/play/{*id}",
            post(playback::play_media),
        )
        .route(
            "/api/media/{media_type}/queue/{*id}",
            post(playback::queue_media),
        )
        .route("/api/playback/current", get(playback::current_playback))
        .route("/api/playback/jump/{index}", post(playback::jump_to_index))
        .route("/api/playback/next", post(playback::next_track))
        .route("/api/playback/prev", post(playback::prev_track))
        .route("/api/playback/stop", post(playback::stop_playback))
        .route("/api/playback/clear", post(playback::clear_queue))
        .route("/api/playback/toggle-pause", post(playback::toggle_pause))
        .route("/api/actions/{name}", any(actions::execute_action))
        .route("/api/images/{file}", get(images::image))
        .route("/api/images/cache", post(images::trigger_cache_images))
        .route("/api/state", post(system::update_state))
        .route("/api/update", post(system::trigger_update))
        .route("/api/update/tracked", get(system::tracked_changes))
        .route("/api/reset", post(system::trigger_reset))
        .with_state(state)
}
