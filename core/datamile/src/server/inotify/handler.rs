use crate::server::inotify::classifier::ChangeFlags;
use crate::server::logic::SortOrder;
use crate::server::state::AppState;
use rayon::prelude::*;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

pub async fn process_events(flags: ChangeFlags, state: &Arc<AppState>) {
    if flags.config {
        handle_config_change(state).await;
    }

    if !flags.vanished_paths.is_empty() || !flags.changed_albums.is_empty() {
        handle_album_changes(flags.vanished_paths, flags.changed_albums, state).await;
    }
}

async fn handle_album_changes(
    vanished_paths: HashSet<PathBuf>,
    changed_albums: HashSet<PathBuf>,
    state: &Arc<AppState>,
) {
    let config_path = {
        let guard = state.config.read().await;
        guard.config_path.clone()
    };
    log_detected_events(&vanished_paths, &changed_albums, state).await;
    let recompiled_items = recompile_changed_albums(
        changed_albums,
        Arc::clone(&state.active_writes),
        config_path,
    )
    .await;
    ingest_and_broadcast_albums(
        vanished_paths,
        recompiled_items,
        IngestOrigin::External,
        state,
    )
    .await;
}

async fn log_detected_events(
    vanished_paths: &HashSet<PathBuf>,
    changed_albums: &HashSet<PathBuf>,
    state: &Arc<AppState>,
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
                    .get_album_id_by_path(p)
                    .map(ToString::to_string)
                    .or_else(|| {
                        let rel = libdatamile::resolvers::rel_path(p, &music_directory);
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
    let resolved_lua_config = if let Ok(Ok(cfg)) =
        tokio::task::spawn_blocking(libdatamile::lua::ResolvedConfig::load).await
    {
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
                || match libdatamile::lua::LuaEngine::new() {
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
    config: &libdatamile::lua::ResolvedConfig,
    engine_opt: Option<&libdatamile::lua::LuaEngine>,
    active_writes: &Arc<Mutex<HashSet<PathBuf>>>,
) -> Option<RecompiledAlbumItem> {
    let canon = album_path
        .canonicalize()
        .unwrap_or_else(|_| album_path.to_path_buf());
    let canon_display = canon.display();

    let Some(engine) = engine_opt else {
        log::error!(
            "Skipping recompilation of {canon_display} due to Lua engine init failure"
        );
        return None;
    };

    let roots = match crate::compile::media::discover_roots(
        engine,
        &canon,
        Some(std::slice::from_ref(&canon)),
    ) {
        Ok(roots) => roots,
        Err(e) => {
            log::error!("Root discovery failed for {canon_display}: {e}");
            return None;
        }
    };

    let (root_type, root_source) = roots.first()?;

    let cache_root = libdatamile::utils::expand_path(&config.app.storage.cache);
    let res = match crate::compile::media::compile_root(
        engine,
        root_type,
        root_source,
        &cache_root,
    ) {
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

    let should_write = std::fs::read_to_string(&res.lock_path)
        .map_or(true, |existing| existing != lock_json);

    if should_write {
        if let Ok(mut active) = active_writes.lock() {
            active.insert(res.lock_path.clone());
        }
        if let Some(parent) = res.lock_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&res.lock_path, &lock_json);
        log::info!("Updated lock: {}", res.id);
    }

    let entity_dir = if res.path.is_dir() {
        res.path.clone()
    } else {
        res.path
            .parent()
            .map_or_else(|| res.path.clone(), PathBuf::from)
    };

    Some((
        entity_dir,
        res.id,
        lock_json,
        Some(eval_res),
        res.dependencies,
    ))
}

fn remove_vanished_albums(
    vanished_paths: &HashSet<PathBuf>,
    logic: &mut crate::server::logic::LogicEngine,
    deps_graph: &mut crate::server::state::DependencyGraph,
    origin: IngestOrigin,
    outcome: &mut IngestOutcome,
) {
    let dead_paths: Vec<PathBuf> = logic
        .albums
        .values()
        .map(|a| a.path.clone())
        .filter(|path| {
            vanished_paths.iter().any(|vanished| {
                path.starts_with(vanished)
                    || (vanished.starts_with(path) && !path.join("album.toml").exists())
            })
        })
        .collect();

    for dead_path in dead_paths {
        if let Some(dead_id) = logic.remove_album_by_path(&dead_path) {
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
    logic: &mut crate::server::logic::LogicEngine,
    deps_graph: &mut crate::server::state::DependencyGraph,
    origin: IngestOrigin,
    outcome: &mut IngestOutcome,
) {
    for (album_path, album_id, lock_json, eval_res, dependencies) in items {
        let canon = album_path
            .canonicalize()
            .unwrap_or_else(|_| album_path.clone());
        if let Some(eval) = eval_res {
            match logic.ingest_pre_evaluated(
                &album_path,
                &album_id,
                &lock_json,
                eval,
                music_directory,
            ) {
                Ok(()) => {
                    deps_graph.update_album_deps(album_path, dependencies);
                    outcome.deps_modified = true;
                    if let Some(entry) = logic.get_album_dict(&album_id) {
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
            && let Some(dead_id) = logic.remove_album_by_path(&album_path)
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
    state: &Arc<AppState>,
) {
    let (music_directory, cache_root) = {
        let guard = state.config.read().await;
        (guard.music_directory.clone(), guard.cache_root.clone())
    };

    let logic_arc = Arc::clone(&state.logic);
    let deps_graph_arc = Arc::clone(&state.deps_graph);

    let (outcome, shelves) = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        let mut deps_graph = deps_graph_arc.blocking_write();
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
        logic.build_cache();

        let mut s = HashMap::new();
        for key in logic.manifest.shelves.keys() {
            s.insert(
                key.clone(),
                logic.request_shelf_view(key, None, SortOrder::Forward),
            );
        }
        drop(logic);

        (outcome, s)
    })
    .await
    .unwrap_or_else(|_| (IngestOutcome::default(), HashMap::new()));

    if !outcome.updated_dict_entries.is_empty() || !outcome.removed_ids.is_empty() {
        let _ = state.tx.send(
            json!({
                "type": "ALBUMS_UPDATED",
                "updated": outcome.updated_dict_entries,
                "removed": outcome.removed_ids,
                "shelves": shelves
            })
            .to_string(),
        );
    }
}

async fn handle_config_change(state: &Arc<AppState>) {
    log::info!("Filesystem change: reloading config and logic...");

    match libdatamile::lua::ResolvedConfig::load() {
        Ok(new_config) => {
            let covers = new_config.covers.clone();
            let new_interfaces = new_config.interfaces.clone();
            let new_actions = new_config.actions.clone();
            let dependencies = new_config.dependencies.clone();
            let config_path = new_config.path.clone();

            {
                let mut config_guard = state.config.write().await;
                config_guard.covers.clone_from(&covers);
                config_guard.interfaces.clone_from(&new_interfaces);
                config_guard.actions.clone_from(&new_actions);
                config_guard.resolved_dependencies.clone_from(&dependencies);
                config_guard.config_path.clone_from(&config_path);
            }

            let manifest = {
                let mut logic = state.logic.write().await;
                if let Err(e) = logic.reload_manifest(&config_path) {
                    log::error!("Failed to reload logic manifest: {e}");
                }
                logic.manifest.clone()
            };

            let _ = state.tx.send(
                json!({
                    "type": "CONFIG_UPDATE",
                    "config": {
                        "covers": covers
                    }
                })
                .to_string(),
            );

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
