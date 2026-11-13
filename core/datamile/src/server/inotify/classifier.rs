use crate::server::state::AppState;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct ChangeFlags {
    pub config: bool,
    pub changed_locks: HashSet<PathBuf>,
    pub changed_paths: HashSet<PathBuf>,
    /// Parent directories whose immediate roots must be rediscovered.
    pub shallow_paths: HashSet<PathBuf>,
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

    let mut matched = false;
    if let Some(locks) = ctx.deps_graph.locks_for_file(p_canon) {
        matched = true;
        flags.changed_locks.extend(locks.iter().cloned());
    }

    for ancestor in p_canon.ancestors() {
        if !ancestor.starts_with(ctx.music_directory) {
            break;
        }
        if let Some(locks) = ctx.deps_graph.locks_for_dir(ancestor) {
            matched = true;
            flags.changed_locks.extend(locks.iter().cloned());
        }
    }

    if !p.exists() {
        flags.vanished_paths.insert(p_canon.to_path_buf());
        flags.vanished_paths.insert(p.to_path_buf());
        return;
    }

    // A new directory may hold a root whose marker appeared with it.
    if p_canon.is_dir() && p_canon.starts_with(ctx.music_directory) {
        flags.changed_paths.insert(p_canon.to_path_buf());
        return;
    }

    // A marker dropped into an untracked folder only reveals a root by
    // inspecting that folder's immediate children.
    if !matched
        && p_canon.starts_with(ctx.music_directory)
        && let Some(parent) = p_canon.parent()
    {
        flags.shallow_paths.insert(parent.to_path_buf());
    }
}

pub async fn classify_events(paths: &[PathBuf], state: &Arc<AppState>) -> ChangeFlags {
    let mut flags = ChangeFlags {
        config: false,
        changed_locks: HashSet::new(),
        changed_paths: HashSet::new(),
        shallow_paths: HashSet::new(),
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

    if let Ok(mut tracked) = state.tracked_roots.lock() {
        for lock in &flags.changed_locks {
            tracked.insert(lock.to_string_lossy().to_string());
        }
    }

    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::RootMeta;

    /// Untracked files mark their parent for shallow discovery; files inside a
    /// watched directory stay on the existing lock path.
    #[test]
    fn untracked_file_marks_parent_for_shallow_discovery() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let music = tmp.path().join("music");
        let untracked = music.join("untracked");
        let tracked = music.join("tracked");
        std::fs::create_dir_all(&untracked).expect("untracked");
        std::fs::create_dir_all(&tracked).expect("tracked");
        let marker = untracked.join("album.toml");
        std::fs::write(&marker, b"").expect("marker");

        let lock = tracked.join("album.lock.json");
        let mut graph = crate::server::state::DependencyGraph::default();
        graph.update_lock(
            lock,
            RootMeta {
                root_type: "album".to_string(),
                target: tracked.clone(),
                files: HashSet::new(),
                dirs: HashSet::from([tracked.clone()]),
            },
        );

        let empty_deps = HashSet::new();
        let ctx = ClassifierContext {
            config_dir: Path::new("/config"),
            music_directory: &music,
            deps: &empty_deps,
            deps_graph: &graph,
        };
        let mut flags = ChangeFlags {
            config: false,
            changed_locks: HashSet::new(),
            changed_paths: HashSet::new(),
            shallow_paths: HashSet::new(),
            vanished_paths: HashSet::new(),
        };

        let canon = marker.canonicalize().expect("canonicalize");
        classify_path_change(&marker, &canon, &ctx, &mut flags);

        assert_eq!(
            flags.shallow_paths,
            HashSet::from([untracked.canonicalize().expect("canonicalize")])
        );
        assert!(flags.changed_locks.is_empty());
    }
}
