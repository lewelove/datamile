use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use datamile_cache::{FileStat, hash_files};

use crate::compile::LogVerbosity;
use crate::deps::{DependencyGraph, UpdateScope};
use crate::update::client::ForceMode;
use crate::update::verify::{find_missing_paths, verify_albums_parallel};

pub(crate) struct WorkQueueContext<'a> {
    pub scope: &'a UpdateScope,
    pub music_directory: &'a Path,
    pub cache: &'a HashMap<String, FileStat>,
    pub deps_graph: &'a DependencyGraph,
    pub force: ForceMode,
    pub effective_jobs: Option<usize>,
    pub tracked: Option<(Vec<PathBuf>, Vec<PathBuf>)>,
    pub verbosity: LogVerbosity,
}

fn filter_dirty_albums(
    albums: Vec<PathBuf>,
    ctx: &WorkQueueContext<'_>,
) -> Result<Vec<PathBuf>> {
    let results = verify_albums_parallel(
        albums,
        ctx.cache,
        ctx.deps_graph,
        ctx.force,
        ctx.effective_jobs,
        ctx.music_directory,
    )?;
    Ok(results
        .into_iter()
        .filter_map(|(path, is_dirty)| if is_dirty { Some(path) } else { None })
        .collect())
}

fn scan_scoped_paths(
    paths: &HashSet<PathBuf>,
    music_dir: &Path,
    cache: &HashMap<String, FileStat>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut albums_set = HashSet::new();
    let mut missing_set = HashSet::new();

    for p in paths {
        let p_canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        if let Ok(found) = crate::scanner::find_target_albums(&p_canon) {
            for album in found {
                albums_set.insert(album);
            }
        }
    }

    let all_found: Vec<PathBuf> = albums_set.iter().cloned().collect();

    for p in paths {
        let p_canon = p.canonicalize().unwrap_or_else(|_| p.clone());
        let target_albums = if p_canon.exists() {
            &all_found[..]
        } else {
            &[][..]
        };
        let missing = find_missing_paths(target_albums, music_dir, &p_canon, cache);
        for m in missing {
            missing_set.insert(m);
        }
    }

    let mut albums_vec: Vec<PathBuf> = albums_set.into_iter().collect();
    albums_vec.sort();
    let mut missing_vec: Vec<PathBuf> = missing_set.into_iter().collect();
    missing_vec.sort();
    (albums_vec, missing_vec)
}

fn discover_scope_albums(
    scope: &UpdateScope,
    music_dir: &Path,
    cache: &HashMap<String, FileStat>,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    match scope {
        UpdateScope::All => {
            let albums = crate::scanner::find_target_albums(music_dir)?;
            let missing = find_missing_paths(&albums, music_dir, music_dir, cache);
            Ok((albums, missing))
        }
        UpdateScope::Paths(paths) => Ok(scan_scoped_paths(paths, music_dir, cache)),
    }
}

pub(crate) fn resolve_work_queue(
    mut ctx: WorkQueueContext<'_>,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    if ctx.force == ForceMode::Preserve
        && let Some((wq, mp)) = ctx.tracked.take()
    {
        if ctx.verbosity == LogVerbosity::Verbose && !wq.is_empty() {
            log::info!("Verifying {} tracked albums...", wq.len());
        }
        let verified_wq = filter_dirty_albums(wq, &ctx)?;
        return Ok((verified_wq, mp));
    }

    let (all_albums, mp) =
        discover_scope_albums(ctx.scope, ctx.music_directory, ctx.cache)?;

    if ctx.verbosity == LogVerbosity::Verbose && !all_albums.is_empty() {
        log::info!("Verifying {} albums...", all_albums.len());
    }

    let wq = filter_dirty_albums(all_albums, &ctx)?;
    Ok((wq, mp))
}

pub(crate) fn update_cache_entries(
    cache: &mut HashMap<String, FileStat>,
    deps_graph: &DependencyGraph,
    work_queue: &[PathBuf],
    missing_paths: &[PathBuf],
    music_directory: &Path,
    verbosity: LogVerbosity,
) {
    for album_root in work_queue {
        let album_root_canon = album_root
            .canonicalize()
            .unwrap_or_else(|_| album_root.clone());
        let album_id = datamile_fs::rel_path(&album_root_canon, music_directory);
        let prefix = if album_id.is_empty() {
            String::new()
        } else {
            format!("{album_id}/")
        };

        cache.retain(|k, _| {
            if prefix.is_empty() {
                k.contains('/')
            } else {
                !k.starts_with(&prefix)
            }
        });

        for entry in walkdir::WalkDir::new(&album_root_canon)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            let p = entry.path();
            if p.is_file() {
                let rel = datamile_fs::rel_path(p, music_directory);
                if let Ok(m) = entry.metadata() {
                    cache.insert(rel, FileStat::from(&m));
                }
            }
        }

        if let Some(external_deps) = deps_graph.album_to_files.get(&album_root_canon) {
            for dep_path in external_deps {
                if !dep_path.starts_with(&album_root_canon)
                    && let Ok(m) = dep_path.metadata()
                {
                    let rel = datamile_fs::rel_path(dep_path, music_directory);
                    cache.insert(rel, FileStat::from(&m));
                }
            }
        }
    }

    for missing in missing_paths {
        let album_id = datamile_fs::rel_path(missing, music_directory);
        let prefix = format!("{album_id}/");

        if verbosity == LogVerbosity::Verbose {
            log::info!("Removed album: {album_id}");
        }

        cache.retain(|k, _| !k.starts_with(&prefix));
    }
}

pub(crate) async fn try_get_server_tracked_albums(
    music_directory: &Path,
) -> Option<(Vec<PathBuf>, Vec<PathBuf>)> {
    let resp = reqwest::Client::new()
        .get("http://127.0.0.1:8000/api/update/tracked")
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

    let tracked_arr = data.get("tracked_items")?.as_array()?;
    let mut work_queue = Vec::new();
    let mut missing_paths = Vec::new();

    for item in tracked_arr {
        if let Some(p_str) = item.as_str() {
            let p = if Path::new(p_str).is_absolute() {
                PathBuf::from(p_str)
            } else {
                music_directory.join(p_str)
            };
            if p.exists() {
                work_queue.push(p);
            } else {
                missing_paths.push(p);
            }
        }
    }

    Some((work_queue, missing_paths))
}

pub(crate) fn check_lua_config_changed(
    dependencies: &[PathBuf],
    cache_root: &Path,
    verbosity: LogVerbosity,
) -> (bool, PathBuf, String) {
    let lua_hash = hash_files(dependencies);
    let lua_hash_file = cache_root.join("config.blake3");
    let previous_lua_hash = fs::read_to_string(&lua_hash_file).unwrap_or_default();
    let config_changed = lua_hash != previous_lua_hash;

    if config_changed && verbosity == LogVerbosity::Verbose {
        log::info!("Lua configuration changed. Re-evaluating album locks...");
    }

    (config_changed, lua_hash_file, lua_hash)
}
