use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use libdatamile::lua::LuaEngine;

use crate::compile::LogVerbosity;
use crate::compile::media::{RootTarget, discover_targets};
use crate::logging::{self, LogTx};
use crate::server::state::{DependencyGraph, UpdateScope};
use crate::update::cache::{FileStat, LibraryCache};
use crate::update::client::ForceMode;
use crate::update::verify::{CacheIndex, verify_targets};

pub struct WorkQueueContext<'a> {
    pub engine: &'a LuaEngine,
    pub scope: &'a UpdateScope,
    pub media_roots: &'a [PathBuf],
    pub cache: &'a LibraryCache,
    pub deps_graph: &'a DependencyGraph,
    pub force: ForceMode,
    pub effective_jobs: Option<usize>,
    /// Lock paths the server already tracks; avoids a full discovery walk.
    pub tracked: Option<Vec<PathBuf>>,
    pub verbosity: LogVerbosity,
    pub log_txs: &'a [LogTx],
}

pub struct WorkQueue {
    pub roots: Vec<RootTarget>,
    pub missing: Vec<PathBuf>,
}

pub fn resolve_work_queue(ctx: &WorkQueueContext<'_>) -> Result<WorkQueue> {
    let mut roots = match (&ctx.tracked, ctx.force) {
        (Some(tracked), ForceMode::Preserve) => {
            tracked_targets(ctx.engine, tracked, ctx.deps_graph)?
        }
        _ => discover(ctx.engine, ctx.scope, ctx.media_roots)?,
    };
    roots.sort_by(|a, b| a.lock_path.cmp(&b.lock_path));
    roots.dedup_by(|a, b| a.lock_path == b.lock_path);

    let mut missing = match &ctx.tracked {
        Some(tracked) if ctx.force == ForceMode::Preserve => {
            let found: HashSet<&PathBuf> =
                roots.iter().map(|root| &root.lock_path).collect();
            tracked
                .iter()
                .filter(|lock| !found.contains(lock))
                .cloned()
                .collect()
        }
        _ => missing_locks(ctx.deps_graph, ctx.scope, ctx.media_roots, &roots),
    };
    missing.sort();
    missing.dedup();

    if !roots.is_empty() {
        let msg = format!("Verifying {} entities...", roots.len());
        logging::emit(&msg, ctx.verbosity, ctx.log_txs);
    }

    let cache = CacheIndex::new(ctx.cache);
    let dirty =
        verify_targets(roots, &cache, ctx.deps_graph, ctx.force, ctx.effective_jobs)?;

    Ok(WorkQueue {
        roots: dirty,
        missing,
    })
}

fn discover(
    engine: &LuaEngine,
    scope: &UpdateScope,
    _media_roots: &[PathBuf],
) -> Result<Vec<RootTarget>> {
    match scope {
        UpdateScope::All => crate::compile::media::discover_targets_all(engine),
        UpdateScope::Paths(paths) => {
            let mut roots = Vec::new();
            for path in paths {
                roots.extend(discover_targets(engine, path, None)?);
            }
            Ok(roots)
        }
    }
}

fn tracked_targets(
    engine: &LuaEngine,
    tracked: &[PathBuf],
    deps_graph: &DependencyGraph,
) -> Result<Vec<RootTarget>> {
    let mut roots = Vec::new();

    for lock in tracked {
        let Some(meta) = deps_graph.lock_to_meta.get(lock) else {
            continue;
        };

        let found = discover_targets(
            engine,
            &meta.target,
            Some(std::slice::from_ref(&meta.target)),
        )?;
        roots.extend(found.into_iter().filter(|root| &root.lock_path == lock));
    }

    Ok(roots)
}

fn missing_locks(
    deps_graph: &DependencyGraph,
    scope: &UpdateScope,
    media_roots: &[PathBuf],
    discovered: &[RootTarget],
) -> Vec<PathBuf> {
    let found: HashSet<&PathBuf> =
        discovered.iter().map(|root| &root.lock_path).collect();

    let mut missing: Vec<PathBuf> = match scope {
        UpdateScope::All => media_roots
            .iter()
            .flat_map(|root| deps_graph.locks_under(root))
            .collect(),
        UpdateScope::Paths(paths) => paths
            .iter()
            .flat_map(|path| deps_graph.locks_under(path))
            .collect(),
    };

    missing.retain(|lock| !found.contains(lock));
    missing
}

/// Rebuild the baseline from the graph after a pass. Everything the roots
/// inspected is re-statted so the next pass compares against current disk state.
pub fn rebuild_cache(cache: &mut LibraryCache, deps_graph: &DependencyGraph) {
    cache.clear();

    for meta in deps_graph.lock_to_meta.values() {
        for file in &meta.files {
            insert_stat(cache, file);
        }

        for dir in &meta.dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                insert_stat(cache, &entry.path());
            }
        }

        if !meta.target.is_dir() {
            insert_stat(cache, &meta.target);
        }
    }
}

fn insert_stat(cache: &mut LibraryCache, path: &Path) {
    if let Ok(meta) = path.metadata() {
        cache.insert(path.to_path_buf(), FileStat::from(&meta));
    }
}

pub async fn try_get_server_tracked_roots() -> Option<Vec<PathBuf>> {
    let resp = reqwest::Client::new()
        .get("http://127.0.0.1:8000/api/internal/tracked_albums")
        .timeout(std::time::Duration::from_millis(500))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    if data.get("full_rescan").and_then(serde_json::Value::as_bool) != Some(false) {
        return None;
    }

    let tracked = data.get("tracked_roots")?.as_array()?;
    Some(
        tracked
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(PathBuf::from)
            .collect(),
    )
}

pub fn check_lua_config_changed(
    dependencies: &[PathBuf],
    cache_root: &Path,
    verbosity: LogVerbosity,
    log_txs: &[LogTx],
) -> (bool, PathBuf, String) {
    let lua_hash = crate::update::cache::lua_config_hash(dependencies);
    let lua_hash_file = cache_root.join("config.blake3");
    let previous_lua_hash = fs::read_to_string(&lua_hash_file).unwrap_or_default();
    let config_changed = lua_hash != previous_lua_hash;

    if config_changed {
        logging::emit(
            "Lua configuration changed. Re-evaluating entity locks...",
            verbosity,
            log_txs,
        );
    }

    (config_changed, lua_hash_file, lua_hash)
}
