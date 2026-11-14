use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rayon::prelude::*;

use crate::compile::media::RootTarget;
use crate::server::state::DependencyGraph;
use crate::update::cache::{FileStat, LibraryCache};
use crate::update::client::ForceMode;

/// Cache stats indexed by parent directory so a directory listing can be
/// compared to the previous pass without scanning the whole cache.
pub struct CacheIndex<'a> {
    pub stats: &'a LibraryCache,
    by_dir: HashMap<PathBuf, HashSet<PathBuf>>,
}

impl<'a> CacheIndex<'a> {
    pub fn new(stats: &'a LibraryCache) -> Self {
        let mut by_dir: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();

        for path in stats.keys() {
            let Some(parent) = path.parent() else {
                continue;
            };
            by_dir
                .entry(parent.to_path_buf())
                .or_default()
                .insert(path.clone());
        }

        Self { stats, by_dir }
    }
}

pub fn verify_targets(
    roots: Vec<RootTarget>,
    cache: &CacheIndex<'_>,
    deps_graph: &DependencyGraph,
    force: ForceMode,
    jobs: Option<usize>,
) -> Result<Vec<RootTarget>> {
    if force == ForceMode::Force {
        return Ok(roots);
    }

    let default_parallelism =
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.unwrap_or(default_parallelism))
        .build()?;

    Ok(pool.install(|| {
        roots
            .into_par_iter()
            .filter(|root| is_dirty(root, cache, deps_graph))
            .collect()
    }))
}

fn is_dirty(root: &RootTarget, cache: &CacheIndex<'_>, graph: &DependencyGraph) -> bool {
    if !root.lock_path.exists() {
        return true;
    }

    let Some(meta) = graph.lock_to_meta.get(&root.lock_path) else {
        return true;
    };

    if meta.root_type != root.root_type || meta.target != root.target {
        return true;
    }

    if !meta.target.is_dir() && file_changed(&meta.target, cache) {
        return true;
    }

    for file in &meta.files {
        if file_changed(file, cache) {
            return true;
        }
    }

    meta.dirs.iter().any(|dir| dir_dirty(dir, cache))
}

fn file_changed(path: &Path, cache: &CacheIndex<'_>) -> bool {
    let Ok(meta) = path.metadata() else {
        return true;
    };

    cache.stats.get(path) != Some(&FileStat::from(&meta))
}

/// A recorded directory is dirty when an entry was added, removed, or
/// modified since the last pass, including immediate subdirectories.
fn dir_dirty(dir: &Path, cache: &CacheIndex<'_>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };

    let mut listed = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            return true;
        };
        if cache.stats.get(&path) != Some(&FileStat::from(&meta)) {
            return true;
        }
        listed.insert(path);
    }

    cache
        .by_dir
        .get(dir)
        .is_some_and(|known| known.iter().any(|path| !listed.contains(path)))
}
