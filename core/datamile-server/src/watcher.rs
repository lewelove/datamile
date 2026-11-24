use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::state::ServerState;

fn setup_watcher(tx: UnboundedSender<Vec<PathBuf>>) -> RecommendedWatcher {
    RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res
                && (event.kind.is_modify()
                    || event.kind.is_create()
                    || event.kind.is_remove())
            {
                let _ = tx.send(event.paths);
            }
        },
        notify::Config::default(),
    )
    .expect("Failed to create inotify watcher")
}

pub fn start(state: Arc<ServerState>) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut fs_watcher = setup_watcher(tx);

    tokio::spawn(async move {
        run_loop(rx, &mut fs_watcher, state).await;
    });
}

async fn run_loop(
    mut rx: UnboundedReceiver<Vec<PathBuf>>,
    watcher: &mut RecommendedWatcher,
    state: Arc<ServerState>,
) {
    let mut watched_dirs = HashSet::new();
    sync_watches(watcher, &mut watched_dirs, &state).await;

    while let Some(mut paths) = rx.recv().await {
        tokio::time::sleep(Duration::from_millis(100)).await;
        while let Ok(more_paths) = rx.try_recv() {
            paths.extend(more_paths);
        }

        paths.sort();
        paths.dedup();

        state.backend.handle_fs_events(&paths, &state).await;

        sync_watches(watcher, &mut watched_dirs, &state).await;
    }
}

fn apply_watch_set(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut HashSet<PathBuf>,
    dirs: HashSet<PathBuf>,
    mode: RecursiveMode,
) {
    for dir in dirs {
        if dir.exists() && watched_dirs.insert(dir.clone()) {
            let _ = watcher.watch(&dir, mode);
        }
    }
}

async fn sync_watches(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut HashSet<PathBuf>,
    state: &Arc<ServerState>,
) {
    let (config_dir, watch_roots, dependencies) = {
        let guard = state.config.read().await;
        (
            guard.config_dir.clone(),
            guard.watch_roots.clone(),
            guard.resolved_dependencies.clone(),
        )
    };

    let (extra_recursive, extra_non_recursive) = state.backend.extra_watch_dirs(state);

    let mut needed_recursive: HashSet<PathBuf> = HashSet::from([config_dir]);
    needed_recursive.extend(watch_roots);
    needed_recursive.extend(extra_recursive);

    let mut needed_non_recursive: HashSet<PathBuf> =
        crate::dependency_dirs(&dependencies).into_iter().collect();
    needed_non_recursive.extend(extra_non_recursive);

    apply_watch_set(
        watcher,
        watched_dirs,
        needed_recursive,
        RecursiveMode::Recursive,
    );
    apply_watch_set(
        watcher,
        watched_dirs,
        needed_non_recursive,
        RecursiveMode::NonRecursive,
    );
}
