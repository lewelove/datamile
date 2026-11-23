use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use datamile_logic::SortOrder;
use datamile_server::ServerState;
use rayon::prelude::*;
use serde_json::json;

use crate::deps::{DependencyGraph, LegacyShared};
use crate::inotify::classifier::ChangeFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOrigin {
    Internal,
    External,
}

pub type RecompiledAlbumItem = (
    PathBuf,
    String,
    String,
    Option<serde_json::Value>,
    HashSet<PathBuf>,
);

#[derive(Default)]
struct IngestOutcome {
    removed_ids: Vec<String>,
    updated_dict_entries: HashMap<String, Box<serde_json::value::RawValue>>,
    deps_modified: bool,
}

pub(super) fn filter_reingested_ids<V, S: std::hash::BuildHasher>(
    removed_ids: &mut Vec<String>,
    updated_entries: &HashMap<String, V, S>,
) {
    if removed_ids.is_empty() || updated_entries.is_empty() {
        return;
    }

    removed_ids.retain(|id| !updated_entries.contains_key(id));
}

pub async fn process_events(
    flags: ChangeFlags,
    state: &Arc<ServerState>,
    shared: &LegacyShared,
) {
    if flags.config {
        handle_config_change(state).await;
    }

    if !flags.vanished_paths.is_empty() || !flags.changed_albums.is_empty() {
        handle_album_changes(flags.vanished_paths, flags.changed_albums, state, shared)
            .await;
    }
}

async fn handle_album_changes(
    vanished_paths: HashSet<PathBuf>,
    changed_albums: HashSet<PathBuf>,
    state: &Arc<ServerState>,
    shared: &LegacyShared,
) {
    let config_path = {
        let guard = state.config.read().await;
        guard.config_path.clone()
    };
    log_detected_events(&vanished_paths, &changed_albums, state).await;
    let recompiled_items = recompile_changed_albums(
        changed_albums,
        Arc::clone(&shared.active_writes),
        config_path,
    )
    .await;
    ingest_and_broadcast_albums(
        vanished_paths,
        recompiled_items,
        IngestOrigin::External,
        state,
        shared,
    )
    .await;
}

async fn log_detected_events(
    vanished_paths: &HashSet<PathBuf>,
    changed_albums: &HashSet<PathBuf>,
    state: &Arc<ServerState>,
) {
    let music_directory = {
        let guard = state.config.read().await;
        guard.music_directory.clone()
    };
    let logic = state.logic.read().await;

    let resolve_ids = |paths: &HashSet<PathBuf>| -> Vec<String> {
        paths
            .iter()
            .filter_map(|p| {
                logic
                    .entity_id_by_path(p)
                    .map(ToString::to_string)
                    .or_else(|| {
                        let rel = datamile_fs::rel_path(p, &music_directory);
                        if rel.is_empty() { None } else { Some(rel) }
                    })
            })
            .collect()
    };

    for id in resolve_ids(vanished_paths) {
        log::info!("Detected removal of: {id}");
    }

    for id in resolve_ids(changed_albums) {
        log::info!("Detected change in: {id}");
    }
}

async fn recompile_changed_albums(
    changed_albums: HashSet<PathBuf>,
    active_writes: Arc<Mutex<HashSet<PathBuf>>>,
    config_path: PathBuf,
) -> Vec<RecompiledAlbumItem> {
    let resolved_lua_config =
        if let Ok(Ok(cfg)) = tokio::task::spawn_blocking(crate::config::load).await {
            Arc::new(cfg)
        } else {
            log::error!("Failed to load resolved config for album update");
            return Vec::new();
        };

    tokio::task::spawn_blocking(move || {
        let config_ref = &resolved_lua_config;
        let config_path_ref = &config_path;

        changed_albums
            .into_par_iter()
            .filter(|p| p.exists())
            .map_init(
                || match crate::lua::LuaEngine::new() {
                    Ok(engine) => match engine.evaluate_config(config_path_ref) {
                        Ok(_) => Some(engine),
                        Err(e) => {
                            log::error!(
                                "Failed to evaluate config in inotify handler: {e}"
                            );
                            None
                        }
                    },
                    Err(e) => {
                        log::error!(
                            "Failed to initialize Lua engine in inotify handler: {e}"
                        );
                        None
                    }
                },
                |engine_opt, album_path| {
                    recompile_single_album(
                        &album_path,
                        config_ref,
                        engine_opt.as_ref(),
                        &active_writes,
                    )
                },
            )
            .flatten()
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default()
}

fn recompile_single_album(
    album_path: &Path,
    config: &crate::lua::ResolvedConfig,
    engine_opt: Option<&crate::lua::LuaEngine>,
    active_writes: &Arc<Mutex<HashSet<PathBuf>>>,
) -> Option<RecompiledAlbumItem> {
    let canon = album_path
        .canonicalize()
        .unwrap_or_else(|_| album_path.to_path_buf());
    let canon_display = canon.display();
    let lock_file_path = album_path.join("album.lock.json");
    let lock_canon = canon.join("album.lock.json");

    let Some(engine) = engine_opt else {
        log::error!(
            "Skipping recompilation of {canon_display} due to Lua engine init failure"
        );
        return None;
    };

    let res = match crate::compile::build::build(album_path, config, engine) {
        Ok(res) => res,
        Err(e) => {
            log::error!("Compilation failed for {canon_display}: {e}");
            return None;
        }
    };

    let eval_res = match engine.evaluate_album_logic(&res.lock_json) {
        Ok(eval) => eval,
        Err(e) => {
            log::error!("Logic evaluation failed for {canon_display}: {e}");
            return None;
        }
    };

    let lock_json = match serde_json::to_string_pretty(&res.lock_json) {
        Ok(json) => json,
        Err(e) => {
            log::error!("Failed to serialize lock JSON for {canon_display}: {e}");
            return None;
        }
    };

    let should_write = std::fs::read_to_string(&lock_file_path)
        .map_or(true, |existing| existing != lock_json);

    if should_write {
        if let Ok(mut active) = active_writes.lock() {
            active.insert(lock_canon);
            active.insert(lock_file_path.clone());
        }
        let _ = std::fs::write(&lock_file_path, &lock_json);
        log::info!("Updated lock: {}", res.album_id);
    }

    Some((
        res.album_dir,
        res.album_id,
        lock_json,
        Some(eval_res),
        res.dependencies,
    ))
}

fn remove_vanished_albums(
    vanished_paths: &HashSet<PathBuf>,
    logic: &mut datamile_logic::LogicEngine,
    deps_graph: &mut DependencyGraph,
    origin: IngestOrigin,
    outcome: &mut IngestOutcome,
) {
    let dead_paths: Vec<PathBuf> = logic
        .entities
        .values()
        .map(|record| record.path.clone())
        .filter(|path| {
            vanished_paths.iter().any(|vanished| {
                path.starts_with(vanished)
                    || (vanished.starts_with(path) && !path.join("album.toml").exists())
            })
        })
        .collect();

    for dead_path in dead_paths {
        if let Some(dead_id) = logic.remove_entity_by_path(&dead_path) {
            if origin == IngestOrigin::External {
                log::info!("Removed album: {dead_id}");
            }
            deps_graph.remove_album(&dead_path);
            outcome.deps_modified = true;
            outcome.removed_ids.push(dead_id);
        }
    }
}

fn ingest_recompiled_items(
    items: Vec<RecompiledAlbumItem>,
    music_directory: &Path,
    logic: &mut datamile_logic::LogicEngine,
    deps_graph: &mut DependencyGraph,
    origin: IngestOrigin,
    outcome: &mut IngestOutcome,
) {
    for (album_path, album_id, lock_json, eval_res, dependencies) in items {
        let canon = album_path
            .canonicalize()
            .unwrap_or_else(|_| album_path.clone());

        if let Some(eval) = eval_res {
            let parsed = crate::ingest::parse_album_lock(
                &album_path,
                &album_id,
                &lock_json,
                music_directory,
            );
            let ingest_result = parsed.and_then(|parsed| {
                parsed.map_or(Ok(()), |parsed| {
                    logic.upsert_entity(
                        parsed.entity,
                        parsed.playables,
                        parsed.images,
                        eval,
                    )
                })
            });

            match ingest_result {
                Ok(()) => {
                    deps_graph.update_album_deps(album_path, dependencies);
                    outcome.deps_modified = true;
                    if let Some(entry) = logic.entity_dict(&album_id) {
                        outcome
                            .updated_dict_entries
                            .insert(album_id, entry.to_owned());
                    }
                }
                Err(e) => {
                    let canon_display = canon.display();
                    log::error!("Failed to ingest album at {canon_display}: {e}");
                }
            }
        } else if !album_path.exists()
            && let Some(dead_id) = logic.remove_entity_by_path(&album_path)
        {
            if origin == IngestOrigin::External {
                log::info!("Removed album: {dead_id}");
            }
            deps_graph.remove_album(&album_path);
            outcome.deps_modified = true;
            outcome.removed_ids.push(dead_id);
        }
    }
}

pub async fn ingest_and_broadcast_albums(
    vanished_paths: HashSet<PathBuf>,
    recompiled_items: Vec<RecompiledAlbumItem>,
    origin: IngestOrigin,
    state: &Arc<ServerState>,
    shared: &LegacyShared,
) {
    let (music_directory, cache_root) = {
        let guard = state.config.read().await;
        (guard.music_directory.clone(), guard.cache_root.clone())
    };

    let logic_arc = Arc::clone(&state.logic);
    let deps_graph_arc = Arc::clone(&shared.deps_graph);

    let (outcome, shelves) = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        let mut deps_graph = deps_graph_arc
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut outcome = IngestOutcome::default();

        remove_vanished_albums(
            &vanished_paths,
            &mut logic,
            &mut deps_graph,
            origin,
            &mut outcome,
        );

        ingest_recompiled_items(
            recompiled_items,
            &music_directory,
            &mut logic,
            &mut deps_graph,
            origin,
            &mut outcome,
        );

        filter_reingested_ids(&mut outcome.removed_ids, &outcome.updated_dict_entries);

        let graph_pruned = deps_graph.prune();
        if outcome.deps_modified || graph_pruned {
            let deps_json_path =
                crate::update::cache::deps_graph_path(&cache_root, &music_directory);
            if let Err(e) = deps_graph.save_to_file(&deps_json_path) {
                log::error!("Failed to persist dependency graph: {e}");
            }
        }

        drop(deps_graph);
        crate::lua_bridge::rebuild_cache(&mut logic);

        let mut shelves = HashMap::new();
        for key in logic.manifest.shelves.keys() {
            shelves.insert(
                key.clone(),
                logic.request_shelf_view(key, None, SortOrder::Forward),
            );
        }
        drop(logic);

        (outcome, shelves)
    })
    .await
    .unwrap_or_else(|_| (IngestOutcome::default(), HashMap::new()));

    if !outcome.updated_dict_entries.is_empty() || !outcome.removed_ids.is_empty() {
        let _ = state.tx.send(
            json!({
                "type": "MEDIA_UPDATED",
                "media_type": "album",
                "updated": outcome.updated_dict_entries,
                "removed": outcome.removed_ids,
                "shelves": shelves
            })
            .to_string(),
        );
    }
}

async fn handle_config_change(state: &Arc<ServerState>) {
    log::info!("Filesystem change: reloading config and logic...");

    let mut logic = state.logic.write().await;
    match state.backend.reload_config_and_logic(&mut logic) {
        Ok(new_config) => {
            let manifest = logic.manifest.clone();
            drop(logic);
            *state.config.write().await = new_config;

            let _ = state.tx.send(
                json!({
                    "type": "LOGIC_UPDATE",
                    "manifest": manifest
                })
                .to_string(),
            );
        }
        Err(e) => {
            log::error!("Failed to reload config: {e:?}");
        }
    }
}
