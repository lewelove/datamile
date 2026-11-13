use crate::server::state::DependencyGraph;
use crate::update::cache::FileStat;
use crate::update::client::ForceMode;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn find_missing_paths(
    all_albums: &[PathBuf],
    library_root: &Path,
    scan_root: &Path,
    cache: &HashMap<String, FileStat>,
) -> Vec<PathBuf> {
    let mut missing_paths = Vec::new();
    let current_album_ids: HashSet<String> = all_albums
        .iter()
        .map(|p| libdatamile::resolvers::rel_path(p, library_root))
        .collect();

    let mut cached_album_ids = HashSet::new();
    for rel_path in cache.keys() {
        if rel_path == "album.toml" {
            cached_album_ids.insert(String::new());
        } else if let Some(album_id) = rel_path.strip_suffix("/album.toml") {
            cached_album_ids.insert(album_id.to_string());
        }
    }

    for cached_id in cached_album_ids {
        let full_cached_path = library_root.join(&cached_id);
        if full_cached_path.starts_with(scan_root)
            && !current_album_ids.contains(&cached_id)
        {
            missing_paths.push(full_cached_path);
        }
    }
    missing_paths
}

pub fn verify_albums_parallel(
    albums: Vec<PathBuf>,
    cache: &HashMap<String, FileStat>,
    deps_graph: &DependencyGraph,
    force: ForceMode,
    jobs: Option<usize>,
    library_root: &Path,
) -> Result<Vec<(PathBuf, bool)>> {
    let default_parallelism =
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.unwrap_or(default_parallelism))
        .build()?;

    Ok(pool.install(|| {
        albums
            .into_par_iter()
            .map(|album_root| {
                let is_dirty = force == ForceMode::Force
                    || is_album_dirty(&album_root, cache, deps_graph, library_root);
                (album_root, is_dirty)
            })
            .collect()
    }))
}

fn is_album_dirty(
    album_root: &Path,
    cache: &HashMap<String, FileStat>,
    deps_graph: &DependencyGraph,
    library_root: &Path,
) -> bool {
    let album_root_canon = album_root
        .canonicalize()
        .unwrap_or_else(|_| album_root.to_path_buf());

    let lock_path = album_root_canon.join("album.lock.json");
    if !lock_path.exists() {
        return true;
    }

    if !album_root_canon.join("album.toml").exists() {
        return true;
    }

    let album_id = libdatamile::resolvers::rel_path(&album_root_canon, library_root);
    let prefix = if album_id.is_empty() {
        String::new()
    } else {
        format!("{album_id}/")
    };

    if deep_verify_dirty(&album_root_canon, cache, &prefix, library_root) {
        return true;
    }

    if let Some(external_deps) = deps_graph.album_to_files.get(&album_root_canon) {
        for dep_path in external_deps {
            if !dep_path.starts_with(&album_root_canon) {
                let Ok(m) = dep_path.metadata() else {
                    return true;
                };
                let stat = FileStat::from(&m);

                let rel_dep = libdatamile::resolvers::rel_path(dep_path, library_root);
                if cache.get(&rel_dep) != Some(&stat) {
                    return true;
                }
            }
        }
    }

    false
}

fn deep_verify_dirty(
    album_root: &Path,
    cache: &HashMap<String, FileStat>,
    prefix: &str,
    library_root: &Path,
) -> bool {
    let mut album_files_changed = false;
    let mut seen_paths = HashSet::new();

    for entry in walkdir::WalkDir::new(album_root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if p.is_file() {
            let rel = libdatamile::resolvers::rel_path(p, library_root);
            let Ok(m) = entry.metadata() else {
                return true;
            };
            let stat = FileStat::from(&m);

            if cache.get(&rel) != Some(&stat) {
                album_files_changed = true;
                break;
            }
            seen_paths.insert(rel);
        }
    }

    if !album_files_changed {
        for k in cache.keys() {
            let matches_album = if prefix.is_empty() {
                !k.contains('/') && !k.starts_with('/')
            } else {
                k.starts_with(prefix)
            };
            if matches_album && !seen_paths.contains(k) {
                return true;
            }
        }
    }

    album_files_changed
}
