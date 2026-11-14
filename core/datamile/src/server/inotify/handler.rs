use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use serde_json::json;

use crate::server::inotify::classifier::ChangeFlags;
use crate::server::state::{AppState, RootMeta};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOrigin {
    Internal,
    External,
}

#[derive(Debug, Clone)]
pub struct RecompiledAlbumItem {
    pub root_type: String,
    pub target: PathBuf,
    pub lock_path: PathBuf,
    pub id: String,
    pub lock_json: String,
    pub eval_res: Option<serde_json::Value>,
    pub files: HashSet<PathBuf>,
    pub dirs: HashSet<PathBuf>,
}

impl From<crate::server::api::system::EntityIngestPayload> for RecompiledAlbumItem {
    fn from(payload: crate::server::api::system::EntityIngestPayload) -> Self {
        Self {
            root_type: payload.root_type,
            target: payload.target,
            lock_path: payload.lock_path,
            id: payload.id,
            lock_json: payload.lock_json,
            eval_res: payload.eval_res,
            files: payload.files.into_iter().collect(),
            dirs: payload.dirs.into_iter().collect(),
        }
    }
}

#[derive(Default)]
struct IngestOutcome {
    removed: Vec<(String, String)>,
    updated: Vec<(String, String)>,
    deps_modified: bool,
}

pub(super) fn filter_reingested_ids(
    removed: &mut Vec<(String, String)>,
    updated: &[(String, String)],
) {
    if removed.is_empty() || updated.is_empty() {
        return;
    }

    removed.retain(|target| !updated.contains(target));
}

pub(super) async fn process_events(flags: ChangeFlags, state: &Arc<AppState>) {
    if flags.config {
        handle_config_change(state).await;
    }

    if flags.changed_locks.is_empty()
        && flags.changed_paths.is_empty()
        && flags.shallow_paths.is_empty()
        && flags.vanished_paths.is_empty()
    {
        return;
    }

    let config_path = state.config.read().await.config_path.clone();
    let targets = resolve_targets(&flags, state).await;
    let recompiled =
        recompile_targets(targets, Arc::clone(&state.active_writes), config_path).await;

    let vanished_locks = {
        let graph = state.deps_graph.read().await;
        let mut locks = HashSet::new();
        for path in &flags.vanished_paths {
            locks.extend(graph.locks_under(path));
        }

        drop(graph);

        // A marker or target that no longer yields a lock removes the entity.
        if recompiled.complete {
            for lock in &flags.changed_locks {
                if !recompiled.discovered.contains(lock) {
                    locks.insert(lock.clone());
                }
            }
        }

        locks
    };

    ingest_and_broadcast_albums(
        vanished_locks,
        recompiled.items,
        IngestOrigin::External,
        state,
    )
    .await;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DiscoveryMode {
    Recursive,
    Shallow,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DiscoveryTarget {
    path: PathBuf,
    mode: DiscoveryMode,
}

fn add_target(
    targets: &mut HashMap<PathBuf, DiscoveryMode>,
    path: PathBuf,
    mode: DiscoveryMode,
) {
    match targets.get(&path) {
        Some(DiscoveryMode::Recursive) => {}
        _ => {
            targets.insert(path, mode);
        }
    }
}

async fn resolve_targets(
    flags: &ChangeFlags,
    state: &Arc<AppState>,
) -> HashSet<DiscoveryTarget> {
    let mut targets = HashMap::new();

    {
        let graph = state.deps_graph.read().await;
        for lock in &flags.changed_locks {
            if let Some(meta) = graph.lock_to_meta.get(lock) {
                add_target(&mut targets, meta.target.clone(), DiscoveryMode::Recursive);
            }
        }
    }

    for path in &flags.changed_paths {
        add_target(&mut targets, path.clone(), DiscoveryMode::Recursive);
    }
    for path in &flags.shallow_paths {
        add_target(&mut targets, path.clone(), DiscoveryMode::Shallow);
    }

    targets
        .into_iter()
        .map(|(path, mode)| DiscoveryTarget { path, mode })
        .collect()
}

#[derive(Default)]
struct TargetOutcome {
    items: Vec<RecompiledAlbumItem>,
    discovered: Vec<PathBuf>,
}

#[derive(Default)]
struct RecompileOutcome {
    items: Vec<RecompiledAlbumItem>,
    /// Locks the selectors still yield for the attempted targets.
    discovered: HashSet<PathBuf>,
    /// False when discovery never ran, so absence proves nothing.
    complete: bool,
}

async fn recompile_targets(
    targets: HashSet<DiscoveryTarget>,
    active_writes: Arc<Mutex<HashSet<PathBuf>>>,
    config_path: PathBuf,
) -> RecompileOutcome {
    if targets.is_empty() {
        return RecompileOutcome::default();
    }

    let resolved_lua_config = if let Ok(Ok(cfg)) =
        tokio::task::spawn_blocking(libdatamile::lua::ResolvedConfig::load).await
    {
        Arc::new(cfg)
    } else {
        log::error!("Failed to load resolved config for entity update");
        return RecompileOutcome::default();
    };

    let outcomes = tokio::task::spawn_blocking(move || {
        let config_ref = &resolved_lua_config;

        targets
            .into_par_iter()
            .filter(|target| target.path.exists())
            .map_init(
                || {
                    libdatamile::lua::LuaEngine::new().ok().and_then(
                        |engine| match engine.evaluate_config(&config_path) {
                            Ok(_) => Some(engine),
                            Err(e) => {
                                log::error!(
                                    "Failed to evaluate config in inotify handler: {e}"
                                );
                                None
                            }
                        },
                    )
                },
                |engine_opt, target| {
                    let mut outcome = TargetOutcome::default();
                    let Some(engine) = engine_opt.as_ref() else {
                        return outcome;
                    };

                    let discovered = match target.mode {
                        DiscoveryMode::Recursive => {
                            crate::compile::media::discover_targets(
                                engine,
                                &target.path,
                                Some(std::slice::from_ref(&target.path)),
                            )
                        }
                        DiscoveryMode::Shallow => {
                            crate::compile::media::discover_targets_shallow(
                                engine,
                                &target.path,
                            )
                        }
                    };
                    let Ok(roots) = discovered else {
                        log::error!(
                            "Root discovery failed for {}",
                            target.path.display()
                        );
                        return outcome;
                    };

                    for root in roots {
                        outcome.discovered.push(root.lock_path.clone());
                        if let Some(item) =
                            recompile_root(&root, config_ref, engine, &active_writes)
                        {
                            outcome.items.push(item);
                        }
                    }

                    outcome
                },
            )
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();

    let mut merged = RecompileOutcome {
        complete: true,
        ..RecompileOutcome::default()
    };
    for outcome in outcomes {
        merged.items.extend(outcome.items);
        merged.discovered.extend(outcome.discovered);
    }
    merged
}

fn recompile_root(
    root: &crate::compile::media::RootTarget,
    config: &libdatamile::lua::ResolvedConfig,
    engine: &libdatamile::lua::LuaEngine,
    active_writes: &Arc<Mutex<HashSet<PathBuf>>>,
) -> Option<RecompiledAlbumItem> {
    let cache_root = libdatamile::utils::expand_path(&config.app.storage.cache);
    let res = match crate::compile::media::compile_root(engine, root, &cache_root) {
        Ok(res) => res,
        Err(e) => {
            log::error!("Compilation failed for {}: {e}", root.target.display());
            return None;
        }
    };

    let eval_res = match engine.evaluate_entity_logic(&root.root_type, &res.lock_json) {
        Ok(eval) => eval,
        Err(e) => {
            log::error!("Logic evaluation failed for {}: {e}", root.target.display());
            return None;
        }
    };

    let lock_json = match serde_json::to_string_pretty(&res.lock_json) {
        Ok(json) => json,
        Err(e) => {
            log::error!(
                "Failed to serialize lock JSON for {}: {e}",
                root.target.display()
            );
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

    Some(RecompiledAlbumItem {
        root_type: res.root_type,
        target: res.target,
        lock_path: res.lock_path,
        id: res.id,
        lock_json,
        eval_res: Some(eval_res),
        files: res.files,
        dirs: res.dirs,
    })
}

fn remove_vanished_locks(
    vanished_locks: &HashSet<PathBuf>,
    logic: &mut crate::server::logic::LogicEngine,
    deps_graph: &mut crate::server::state::DependencyGraph,
    outcome: &mut IngestOutcome,
) {
    for lock in vanished_locks {
        if let Some(target) = logic.remove_by_lock(lock) {
            outcome.removed.push(target);
        }
        deps_graph.remove_lock(lock);
        outcome.deps_modified = true;
    }
}

fn ingest_recompiled_items(
    items: Vec<RecompiledAlbumItem>,
    media_roots: &[PathBuf],
    logic: &mut crate::server::logic::LogicEngine,
    deps_graph: &mut crate::server::state::DependencyGraph,
    outcome: &mut IngestOutcome,
) {
    for item in items {
        let Some(eval) = item.eval_res else {
            continue;
        };

        let ingest = logic.ingest_entity(crate::server::logic::EntityIngest {
            root_type: &item.root_type,
            target: &item.target,
            lock_path: &item.lock_path,
            id: &item.id,
            lock_json: &item.lock_json,
            eval_res: eval,
            media_roots,
        });

        if let Err(e) = ingest {
            log::error!("Failed to ingest entity at {}: {e}", item.target.display());
            continue;
        }

        let root_type = item.root_type.clone();

        deps_graph.update_lock(
            item.lock_path,
            RootMeta {
                root_type: item.root_type,
                target: item.target,
                files: item.files,
                dirs: item.dirs,
            },
        );
        outcome.deps_modified = true;

        outcome.updated.push((root_type, item.id));
    }
}

pub async fn ingest_and_broadcast_albums(
    vanished_locks: HashSet<PathBuf>,
    recompiled_items: Vec<RecompiledAlbumItem>,
    origin: IngestOrigin,
    state: &Arc<AppState>,
) {
    let (media_roots, cache_root) = {
        let guard = state.config.read().await;
        (guard.media_roots.clone(), guard.cache_root.clone())
    };

    let logic_arc = Arc::clone(&state.logic);
    let deps_graph_arc = Arc::clone(&state.deps_graph);

    let outcome = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        let mut deps_graph = deps_graph_arc.blocking_write();
        let mut outcome = IngestOutcome::default();

        remove_vanished_locks(&vanished_locks, &mut logic, &mut deps_graph, &mut outcome);

        ingest_recompiled_items(
            recompiled_items,
            &media_roots,
            &mut logic,
            &mut deps_graph,
            &mut outcome,
        );

        filter_reingested_ids(&mut outcome.removed, &outcome.updated);

        let graph_pruned = deps_graph.prune();
        if outcome.deps_modified || graph_pruned {
            let deps_json_path =
                crate::update::cache::deps_graph_path(&cache_root, &media_roots);
            if let Err(e) = deps_graph.save_to_file(&deps_json_path) {
                log::error!("Failed to persist dependency graph: {e}");
            }
        }

        drop(deps_graph);
        logic.build_cache();
        drop(logic);

        outcome
    })
    .await
    .unwrap_or_default();

    if origin == IngestOrigin::External {
        for (root_type, id) in &outcome.removed {
            log::info!("Removed {root_type}: {id}");
        }
    }

    broadcast_media_updates(&outcome, state);
}

/// Thin invalidation: group changed ids per root type, never payloads.
fn broadcast_media_updates(outcome: &IngestOutcome, state: &Arc<AppState>) {
    if outcome.updated.is_empty() && outcome.removed.is_empty() {
        return;
    }

    let mut groups: BTreeMap<&str, (Vec<&str>, Vec<&str>)> = BTreeMap::new();
    for (root_type, id) in &outcome.updated {
        groups.entry(root_type).or_default().0.push(id);
    }
    for (root_type, id) in &outcome.removed {
        groups.entry(root_type).or_default().1.push(id);
    }

    for (root_type, (updated, removed)) in groups {
        let _ = state.tx.send(
            json!({
                "event": "media.updated",
                "type": root_type,
                "updated": updated,
                "removed": removed
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

            {
                let mut logic = state.logic.write().await;
                if let Err(e) = logic.reload_manifest(&config_path) {
                    log::error!("Failed to reload logic manifest: {e}");
                }
            }

            let _ = state.tx.send(
                json!({
                    "event": "config.updated",
                    "config": {
                        "covers": covers
                    }
                })
                .to_string(),
            );

            let _ = state
                .tx
                .send(json!({ "event": "logic.updated" }).to_string());
        }
        Err(e) => {
            log::error!("Failed to reload config: {e:?}");
        }
    }
}
