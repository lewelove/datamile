mod classifier;
pub mod handler;
#[cfg(test)]
mod tests;
mod watcher;

use std::sync::Arc;

use crate::server::state::AppState;

pub fn start(state: Arc<AppState>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut fs_watcher = watcher::setup_watcher(tx);

    tokio::spawn(async move {
        watcher::run_loop(rx, &mut fs_watcher, state).await;
    });
}
