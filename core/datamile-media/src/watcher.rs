use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use datamile_logic::{LogicEngine, SortOrder};
use datamile_server::ServerState;
use serde_json::json;

use crate::compiler::CompiledRoot;
use crate::deps::MediaShared;
use crate::state::{MediaState, open_engine};

/// All hot-reload guards run here, in Rust: self-writes, config reloads,
/// external dependencies, known-root ancestors, vanished markers and new
/// subtrees.
pub async fn handle_fs_events(
    shared: Arc<MediaShared>,
    paths: &[PathBuf],
    server: &Arc<ServerState>,
) {
    let (config_dir, dependencies) = {
        let guard = server.config.read().await;
        (
            guard.config_dir.clone(),
            guard
                .resolved_dependencies
                .iter()
                .cloned()
                .collect::<HashSet<_>>(),
        )
    };

    let relevant = drain_active_writes(&shared, paths);
    if relevant.is_empty() {
        return;
    }

    if relevant
        .iter()
        .any(|path| is_config_change(path, &config_dir, &dependencies))
    {
        reload_config(server).await;
        return;
    }

    let Some(state) = shared
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    else {
        return;
    };

    let logic = server.logic.read().await;

    // Every Lua VM access lives in this block so no non-Send value is alive at
    // the await below.
    let Some((prepared, compiled, removed_roots)) = (|| {
        let Ok(engine) = open_engine(&state) else {
            log::error!("Failed to open the media engine for hot reload");
            return None;
        };

        let mut targeted: HashSet<(String, PathBuf)> = HashSet::new();
        let mut removed_roots: Vec<(String, PathBuf)> = Vec::new();

        let known: HashSet<(String, PathBuf)> = logic
            .all_entities()
            .map(|record| (record.media_type.clone(), record.path.clone()))
            .collect();

        collect_external_dependencies(&shared, &state, &relevant, &mut targeted);
        collect_media_changes(
            &state,
            &engine,
            &logic,
            &relevant,
            &mut targeted,
            &mut removed_roots,
        );
        drop(logic);

        let mut compiled: Vec<CompiledRoot> = Vec::new();
        for (media_type, dir) in &targeted {
            if removed_roots.iter().any(|(_, removed)| removed == dir) {
                continue;
            }
            match crate::compiler::compile_root(
                &engine,
                media_type,
                dir,
                &state.cache_root,
                None,
            ) {
                Ok(root) => {
                    if root.modified
                        && let Ok(mut writes) = shared.active_writes.lock()
                    {
                        let lock = root.dir.join(format!("{media_type}.lock.json"));
                        writes.insert(lock.clone());
                        writes
                            .insert(lock.canonicalize().unwrap_or_else(|_| lock.clone()));
                    }
                    compiled.push(root);
                }
                Err(e) => log::error!("Recompilation failed for {}: {e}", dir.display()),
            }
        }

        // Unchanged bytes on an already indexed root must not broadcast.
        let compiled: Vec<CompiledRoot> = compiled
            .into_iter()
            .filter(|root| {
                root.modified
                    || !known.contains(&(root.media_type.clone(), root.dir.clone()))
            })
            .collect();

        if compiled.is_empty() && removed_roots.is_empty() {
            return None;
        }

        let prepared = prepare_entities(&state, &engine, &compiled);
        Some((prepared, compiled, removed_roots))
    })() else {
        return;
    };

    ingest_and_broadcast(&shared, &state, server, prepared, compiled, removed_roots)
        .await;
}

type PreparedEntity = (
    String,
    String,
    crate::ingest::ParsedEntity,
    serde_json::Value,
);

type PerTypeUpdates = HashMap<
    String,
    (
        HashMap<String, Box<serde_json::value::RawValue>>,
        Vec<String>,
    ),
>;

/// Parses and evaluates compiled roots before any async work, so the Lua VM
/// never has to be held across an await.
fn prepare_entities(
    state: &MediaState,
    engine: &datamile_lua_domain::DomainLuaEngine,
    compiled: &[CompiledRoot],
) -> Vec<PreparedEntity> {
    let mut prepared = Vec::new();

    for root in compiled {
        let Some(schema) = state.root_schema(&root.media_type) else {
            continue;
        };
        let music_directory = state.mpd_base(&root.media_type, &root.dir);

        let Ok(parsed) = crate::ingest::parse_lock(
            &root.media_type,
            &root.dir,
            &root.lock_json,
            schema,
            music_directory.as_deref(),
        ) else {
            log::error!("Failed to parse lock for {}", root.dir.display());
            continue;
        };
        let Some(parsed) = parsed else {
            continue;
        };

        let eval = serde_json::from_str::<serde_json::Value>(&root.lock_json)
            .ok()
            .and_then(|lock| lock.get(&root.media_type).cloned())
            .and_then(|entity| engine.evaluate_logic(&root.media_type, &entity).ok())
            .unwrap_or_else(|| json!({}));

        prepared.push((root.media_type.clone(), root.id.clone(), parsed, eval));
    }

    prepared
}

fn drain_active_writes(shared: &MediaShared, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut active = shared
        .active_writes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut relevant = Vec::new();
    for path in paths {
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if active.remove(&canon) || active.remove(path) {
            continue;
        }
        relevant.push(path.clone());
    }
    drop(active);
    relevant
}

fn is_config_change(
    path: &Path,
    config_dir: &Path,
    dependencies: &HashSet<PathBuf>,
) -> bool {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    dependencies.contains(&canon)
        || dependencies.contains(path)
        || canon.starts_with(config_dir)
        || path.starts_with(config_dir)
}

async fn reload_config(server: &Arc<ServerState>) {
    log::info!("Filesystem change: reloading config and logic...");
    let mut logic = server.logic.write().await;
    match server.backend.reload_config_and_logic(&mut logic) {
        Ok(new_config) => {
            let manifest = logic.manifest.clone();
            let actions = new_config.actions.clone();
            drop(logic);
            *server.config.write().await = new_config;
            let _ = server.tx.send(
                json!({
                    "type": "LOGIC_UPDATE",
                    "manifest": manifest,
                    "actions": actions
                })
                .to_string(),
            );
        }
        Err(e) => log::error!("Failed to reload config: {e:?}"),
    }
}

fn collect_external_dependencies(
    shared: &MediaShared,
    state: &MediaState,
    paths: &[PathBuf],
    targeted: &mut HashSet<(String, PathBuf)>,
) {
    let graph = shared
        .deps_graph
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    for path in paths {
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        let roots = graph
            .file_to_roots
            .get(&canon)
            .or_else(|| graph.file_to_roots.get(path));

        if let Some(roots) = roots {
            for root in roots {
                if let Some(known) = state.roots.iter().find(|known| &known.dir == root) {
                    targeted.insert((known.media_type.clone(), known.dir.clone()));
                }
            }
        }
    }
    drop(graph);
}

fn known_root_ancestor(
    logic: &LogicEngine,
    media_type: &str,
    path: &Path,
) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };

    loop {
        if logic.entity_id_by_path(media_type, &current).is_some() {
            return Some(current);
        }
        current = current.parent()?.to_path_buf();
    }
}

fn select_roots(
    engine: &datamile_lua_domain::DomainLuaEngine,
    media_type: &str,
    dir: &Path,
) -> Result<Vec<PathBuf>> {
    let select_fn: mlua::Function = engine
        .runtime
        .lua
        .globals()
        .get("__DATAMILE_MEDIA_SELECT")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let value: mlua::Value = select_fn
        .call((media_type, dir.to_string_lossy()))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let raw: Vec<String> = engine
        .runtime
        .from_value(value)
        .context("select must return a list of paths")?;

    let mut roots = Vec::new();
    for entry in raw {
        if let Ok(canon) = PathBuf::from(entry).canonicalize()
            && canon.is_dir()
        {
            roots.push(canon);
        }
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn nearest_existing_dir(path: &Path, init: &Path) -> PathBuf {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map_or_else(|| init.to_path_buf(), Path::to_path_buf)
    };

    loop {
        if current.exists() && current.starts_with(init) {
            return current;
        }
        let Some(parent) = current.parent() else {
            return init.to_path_buf();
        };
        if !parent.starts_with(init) {
            return init.to_path_buf();
        }
        current = parent.to_path_buf();
    }
}

fn collect_media_changes(
    state: &MediaState,
    engine: &datamile_lua_domain::DomainLuaEngine,
    logic: &LogicEngine,
    paths: &[PathBuf],
    targeted: &mut HashSet<(String, PathBuf)>,
    removed_roots: &mut Vec<(String, PathBuf)>,
) {
    for path in paths {
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        for init in state.init_dirs() {
            if !canon.starts_with(&init) && !path.starts_with(&init) {
                continue;
            }

            for media_type in media_types_for_init(state, &init) {
                if let Some(known) = known_root_ancestor(logic, &media_type, &canon) {
                    handle_known_root(
                        engine,
                        &media_type,
                        &known,
                        &canon,
                        targeted,
                        removed_roots,
                    );
                    continue;
                }

                // A deleted path may itself be the ancestor of known roots.
                for record in logic
                    .all_entities()
                    .filter(|record| record.media_type == media_type)
                {
                    if record.path.starts_with(&canon) && !canon.exists() {
                        removed_roots.push((media_type.clone(), record.path.clone()));
                    }
                }

                let scan_dir = nearest_existing_dir(&canon, &init);
                match select_roots(engine, &media_type, &scan_dir) {
                    Ok(found) => {
                        for root in found {
                            if logic.entity_id_by_path(&media_type, &root).is_none() {
                                targeted.insert((media_type.clone(), root));
                            }
                        }
                    }
                    Err(e) => log::error!(
                        "select({}) failed for {media_type}: {e}",
                        scan_dir.display()
                    ),
                }
            }
        }
    }
}

fn media_types_for_init(state: &MediaState, init: &Path) -> Vec<String> {
    let mut types: Vec<String> = state
        .media
        .roots_order
        .iter()
        .filter(|name| {
            state
                .media
                .roots
                .get(*name)
                .and_then(|root| root.init.as_deref())
                .is_some_and(|raw| {
                    datamile_fs::resolve_path(raw, &state.config_dir)
                        .canonicalize()
                        .unwrap_or_else(|_| {
                            datamile_fs::resolve_path(raw, &state.config_dir)
                        })
                        == init
                })
        })
        .cloned()
        .collect();
    types.sort();
    types.dedup();
    types
}

fn handle_known_root(
    engine: &datamile_lua_domain::DomainLuaEngine,
    media_type: &str,
    known: &Path,
    changed: &Path,
    targeted: &mut HashSet<(String, PathBuf)>,
    removed_roots: &mut Vec<(String, PathBuf)>,
) {
    if !known.exists() {
        removed_roots.push((media_type.to_string(), known.to_path_buf()));
        return;
    }

    match select_roots(engine, media_type, known) {
        Ok(roots) if roots.iter().any(|root| root == known) => {
            targeted.insert((media_type.to_string(), known.to_path_buf()));
        }
        Ok(roots) => {
            // The marker vanished (or moved): the directory is no longer a root.
            removed_roots.push((media_type.to_string(), known.to_path_buf()));
            for root in roots {
                targeted.insert((media_type.to_string(), root));
            }
        }
        Err(e) => log::error!(
            "select({}) failed for {media_type} after change in {}: {e}",
            known.display(),
            changed.display()
        ),
    }
}

fn shelves_for(logic: &LogicEngine, media_type: &str) -> HashMap<String, Vec<String>> {
    let mut shelves = HashMap::new();
    for (key, def) in &logic.manifest.shelves {
        if def.media != media_type {
            continue;
        }
        shelves.insert(
            key.clone(),
            logic.request_shelf_view(media_type, key, None, SortOrder::Forward),
        );
    }
    shelves
}

#[allow(
    clippy::cognitive_complexity,
    reason = "the removed/prepared/reindex sequence is one atomic ingest step"
)]
async fn ingest_and_broadcast(
    shared: &Arc<MediaShared>,
    state: &MediaState,
    server: &Arc<ServerState>,
    prepared: Vec<PreparedEntity>,
    compiled: Vec<CompiledRoot>,
    removed_roots: Vec<(String, PathBuf)>,
) {
    let mut per_type = PerTypeUpdates::new();

    let deps_path = crate::state::deps_graph_path(&state.cache_root, &state.init_dirs());

    {
        let mut logic = server.logic.write().await;

        for (media_type, dir) in &removed_roots {
            let lock_path = dir.join(format!("{media_type}.lock.json"));
            if lock_path.exists() {
                if let Ok(mut writes) = shared.active_writes.lock() {
                    writes.insert(lock_path.clone());
                    writes.insert(
                        lock_path
                            .canonicalize()
                            .unwrap_or_else(|_| lock_path.clone()),
                    );
                }
                let _ = std::fs::remove_file(&lock_path);
            }
            if let Some(id) = logic.remove_entity_by_path(dir) {
                per_type.entry(media_type.clone()).or_default().1.push(id);
            }
        }

        for (media_type, id, parsed, eval) in prepared {
            if let Err(e) =
                logic.upsert_entity(parsed.entity, parsed.playables, parsed.images, eval)
            {
                log::error!("Failed to index {media_type}/{id}: {e}");
                continue;
            }

            let entry = per_type.entry(media_type.clone()).or_default();
            if let Some(dict) = logic.entity_dict(&media_type, &id) {
                entry.0.insert(id, dict.to_owned());
            }
        }

        logic.build_cache();

        {
            let mut graph = shared
                .deps_graph
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for root in &compiled {
                graph.update_root_deps(
                    root.dir.clone(),
                    root.dependencies.iter().cloned().collect::<BTreeSet<_>>(),
                );
            }
            for (_, dir) in &removed_roots {
                graph.remove_root(dir);
            }
            if let Err(e) = graph.save_to_file(&deps_path) {
                log::error!("Failed to persist dependency graph: {e}");
            }
        }

        crate::update::sync_cache(state, &logic, &compiled, &removed_roots);
        drop(logic);
    }

    for (media_type, (updated, removed)) in per_type {
        if updated.is_empty() && removed.is_empty() {
            continue;
        }
        let shelves = {
            let logic = server.logic.read().await;
            shelves_for(&logic, &media_type)
        };
        let _ = server.tx.send(
            json!({
                "type": "MEDIA_UPDATED",
                "media_type": media_type,
                "updated": updated,
                "removed": removed,
                "shelves": shelves
            })
            .to_string(),
        );
    }
}
