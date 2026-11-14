use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::server::state::AppState;

pub(super) struct ChangeFlags {
    pub config: bool,
    pub changed_locks: HashSet<PathBuf>,
    pub changed_paths: HashSet<PathBuf>,
    /// Parent directories whose immediate roots must be rediscovered.
    pub(crate) shallow_paths: HashSet<PathBuf>,
    pub vanished_paths: HashSet<PathBuf>,
}

struct ClassifierContext<'a> {
    config_dir: &'a Path,
    media_roots: &'a [PathBuf],
    deps: &'a HashSet<PathBuf>,
    deps_graph: &'a crate::server::state::DependencyGraph,
}

fn under_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
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
        if !under_roots(ancestor, ctx.media_roots) {
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
    if p_canon.is_dir() && under_roots(p_canon, ctx.media_roots) {
        flags.changed_paths.insert(p_canon.to_path_buf());
        return;
    }

    // A marker dropped into an untracked folder only reveals a root by
    // inspecting that folder's immediate children.
    if !matched
        && under_roots(p_canon, ctx.media_roots)
        && let Some(parent) = p_canon.parent()
    {
        flags.shallow_paths.insert(parent.to_path_buf());
    }
}

pub(super) async fn classify_events(
    paths: &[PathBuf],
    state: &Arc<AppState>,
) -> ChangeFlags {
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
    let media_roots = guard.media_roots.clone();
    let deps: HashSet<PathBuf> = guard.resolved_dependencies.iter().cloned().collect();
    drop(guard);

    let ctx = ClassifierContext {
        config_dir: &config_dir,
        media_roots: &media_roots,
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
mod tests;
