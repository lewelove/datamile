use crate::server::state::AppState;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct ChangeFlags {
    pub config: bool,
    pub changed_albums: HashSet<PathBuf>,
    pub vanished_paths: HashSet<PathBuf>,
}

struct ClassifierContext<'a> {
    config_dir: &'a Path,
    music_directory: &'a Path,
    deps: &'a HashSet<PathBuf>,
    deps_graph: &'a crate::server::state::DependencyGraph,
}

fn classify_path_change(
    p: &Path,
    p_canon: &Path,
    ctx: &ClassifierContext<'_>,
    flags: &mut ChangeFlags,
) {
    if ctx.deps.contains(p_canon)
        || p_canon.starts_with(ctx.config_dir)
        || p.starts_with(ctx.config_dir)
    {
        flags.config = true;
    }

    if let Some(albums) = ctx
        .deps_graph
        .file_to_albums
        .get(p_canon)
        .or_else(|| ctx.deps_graph.file_to_albums.get(p))
    {
        for album_path in albums {
            flags.changed_albums.insert(album_path.clone());
        }
    }

    if !p.exists() {
        flags.vanished_paths.insert(p_canon.to_path_buf());
        flags.vanished_paths.insert(p.to_path_buf());

        if let Some(parent) = p.parent() {
            let parent_canon = parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf());
            if let Some(album_path) =
                find_enclosing_album_dir(&parent_canon, ctx.music_directory)
            {
                flags.changed_albums.insert(album_path);
            }
        }
    } else if p_canon.starts_with(ctx.music_directory) {
        if let Some(album_path) = find_enclosing_album_dir(p_canon, ctx.music_directory) {
            flags.changed_albums.insert(album_path);
        } else if p_canon.is_dir()
            && let Ok(found) = libdatamile::scanner::find_target_albums(p_canon)
        {
            for album_path in found {
                flags.changed_albums.insert(album_path);
            }
        }
    }
}

pub async fn classify_events(paths: &[PathBuf], state: &Arc<AppState>) -> ChangeFlags {
    let mut flags = ChangeFlags {
        config: false,
        changed_albums: HashSet::new(),
        vanished_paths: HashSet::new(),
    };

    let deps_graph = state.deps_graph.read().await;
    let guard = state.config.read().await;
    let config_dir = guard.config_dir.clone();
    let music_directory = guard.music_directory.clone();
    let deps: HashSet<PathBuf> = guard.resolved_dependencies.iter().cloned().collect();
    drop(guard);

    let ctx = ClassifierContext {
        config_dir: &config_dir,
        music_directory: &music_directory,
        deps: &deps,
        deps_graph: &deps_graph,
    };

    let mut active = state
        .active_writes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    for p in paths {
        let p_canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        if active.remove(&p_canon) || active.remove(p) {
            continue;
        }
        classify_path_change(p, &p_canon, &ctx, &mut flags);
    }
    drop(deps_graph);
    drop(active);

    if !flags.changed_albums.is_empty()
        && let Ok(mut tracked) = state.tracked_albums.lock()
    {
        for album_path in &flags.changed_albums {
            tracked.insert(album_path.to_string_lossy().to_string());
        }
    }

    flags
}

fn find_enclosing_album_dir(path: &Path, music_directory: &Path) -> Option<PathBuf> {
    let mut curr = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    while curr.starts_with(music_directory) && curr != music_directory {
        if curr.join("album.toml").exists() {
            return Some(curr);
        }
        let Some(parent) = curr.parent() else {
            break;
        };
        curr = parent.to_path_buf();
    }
    None
}
