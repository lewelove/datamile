use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use datamile_cache::EntityKey;
use datamile_logic::LogicEngine;
use datamile_server::ServerState;
use serde_json::json;

use crate::deps::{MediaShared, UpdateScope};
use crate::state::{MediaState, open_engine};
use crate::update::{ForceMode, canonical_or_self};

/// Turns filesystem events into update scopes. The watcher resolves targets
/// here and hands them to the serialized update pipeline; it never compiles,
/// writes state or broadcasts on its own.
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
        // A config change may invalidate every lock, so the reload submits one
        // full pass.
        if reload_config(server).await {
            submit(server, &shared, UpdateScope::All { media_type: None }).await;
        }
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

    // Resolution reads the Lua selectors and the logic index; both are held
    // only until the scope is built, never across the update await.
    let scope = {
        let logic = server.logic.read().await;
        let Ok(engine) = open_engine(&state) else {
            log::error!("Failed to open the media engine for hot reload");
            return;
        };
        resolve_targets(&shared, &state, &engine, &logic, &relevant)
    };

    if let Some(scope) = scope {
        submit(server, &shared, scope).await;
    }
}

/// The scope a set of changed paths asks for, or `None` when nothing indexed is
/// affected. Anything that may have removed an entity widens to a paths scope,
/// so the pass can report the removal instead of failing on a stale target.
#[must_use]
pub(crate) fn resolve_targets(
    shared: &MediaShared,
    state: &MediaState,
    engine: &datamile_lua_domain::DomainLuaEngine,
    logic: &LogicEngine,
    paths: &[PathBuf],
) -> Option<UpdateScope> {
    let mut entities: BTreeSet<EntityKey> = BTreeSet::new();
    let mut vanished: HashSet<PathBuf> = HashSet::new();

    for path in paths {
        let canon = canonical_or_self(path);
        collect_external_dependencies(shared, &canon, &mut entities);
        collect_media_changes(
            state,
            engine,
            logic,
            &canon,
            path,
            &mut entities,
            &mut vanished,
        );
    }

    if !vanished.is_empty() {
        // A paths scope carries no per-entity media type, so a batch that both
        // removed and edited widens to paths. A clean sibling in a shared dir
        // is still only verified, never recompiled.
        vanished.extend(entities.into_iter().map(|key| key.dir));
        return Some(UpdateScope::Paths {
            paths: vanished,
            media_type: None,
        });
    }

    if entities.is_empty() {
        return None;
    }

    Some(UpdateScope::Entities(entities))
}

async fn submit(
    server: &Arc<ServerState>,
    shared: &Arc<MediaShared>,
    scope: UpdateScope,
) {
    if let Err(e) = crate::update::run_scope_update(
        Arc::clone(server),
        Arc::clone(shared),
        scope,
        ForceMode::Preserve,
        None,
        Vec::new(),
    )
    .await
    {
        log::error!("Watcher update failed: {e}");
    }
}

fn drain_active_writes(shared: &MediaShared, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut active = shared
        .active_writes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut relevant = Vec::new();
    for path in paths {
        let canon = canonical_or_self(path);
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
    let canon = canonical_or_self(path);
    dependencies.contains(&canon)
        || dependencies.contains(path)
        || canon.starts_with(config_dir)
        || path.starts_with(config_dir)
}

/// Reloads config and manifest; `true` when the new config applied.
async fn reload_config(server: &Arc<ServerState>) -> bool {
    log::info!("Filesystem change: reloading config and logic...");
    let mut logic = server.logic.write().await;

    match server.backend.reload_config_and_logic(&mut logic) {
        Ok(new_config) => {
            let manifest = logic.manifest.clone();
            let actions = new_config.actions.clone();
            let new_port = new_config.port;
            drop(logic);

            let old_port = server.config.read().await.port;
            *server.config.write().await = new_config;
            *server.reload_error.write().await = None;

            let _ = server.tx.send(
                json!({
                    "type": "LOGIC_UPDATE",
                    "manifest": manifest,
                    "actions": actions
                })
                .to_string(),
            );

            // A bound port cannot change in place, and a daemon on the old port
            // would be unreachable by a CLI reading the new one.
            if new_port != old_port {
                log::error!(
                    "server.port changed from {old_port} to {new_port}; exiting so the next start binds it"
                );
                std::process::exit(0);
            }

            true
        }
        Err(e) => {
            log::error!("Failed to reload config: {e:?}");
            *server.reload_error.write().await = Some(format!("{e:?}"));
            false
        }
    }
}

/// External dependency hits read the reverse index directly: those entities are
/// already known, so nothing has to be resolved through `select`.
fn collect_external_dependencies(
    shared: &MediaShared,
    canon: &Path,
    entities: &mut BTreeSet<EntityKey>,
) {
    let index = shared
        .external_index
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(keys) = index.get(canon) {
        entities.extend(keys.iter().cloned());
    }
}

fn collect_media_changes(
    state: &MediaState,
    engine: &datamile_lua_domain::DomainLuaEngine,
    logic: &LogicEngine,
    canon: &Path,
    raw: &Path,
    entities: &mut BTreeSet<EntityKey>,
    vanished: &mut HashSet<PathBuf>,
) {
    for (media_type, init) in state.media_roots() {
        if !canon.starts_with(&init) && !raw.starts_with(&init) {
            continue;
        }

        if let Some(known) = known_root_ancestor(logic, &media_type, canon) {
            handle_known_root(engine, &media_type, &known, entities, vanished);
            continue;
        }

        // A deleted path may itself be the ancestor of indexed roots.
        let deleted = !canon.exists();
        for record in logic
            .all_entities()
            .filter(|record| record.media_type == media_type)
        {
            if deleted && record.path.starts_with(canon) {
                vanished.insert(canon.to_path_buf());
            }
        }

        let scan_dir = nearest_existing_dir(canon, &init);
        match select_roots(engine, &media_type, &scan_dir) {
            Ok(found) => {
                for dir in found {
                    if logic.entity_id_by_path(&media_type, &dir).is_none() {
                        entities.insert(EntityKey {
                            media_type: media_type.clone(),
                            dir,
                        });
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

fn handle_known_root(
    engine: &datamile_lua_domain::DomainLuaEngine,
    media_type: &str,
    known: &Path,
    entities: &mut BTreeSet<EntityKey>,
    vanished: &mut HashSet<PathBuf>,
) {
    if !known.exists() {
        vanished.insert(known.to_path_buf());
        return;
    }

    match select_roots(engine, media_type, known) {
        Ok(roots) if roots.iter().any(|root| root == known) => {
            entities.insert(EntityKey {
                media_type: media_type.to_string(),
                dir: known.to_path_buf(),
            });
        }
        Ok(roots) => {
            // The marker vanished (or moved): the directory is no longer a root.
            vanished.insert(known.to_path_buf());
            for dir in roots {
                entities.insert(EntityKey {
                    media_type: media_type.to_string(),
                    dir,
                });
            }
        }
        Err(e) => log::error!("select({}) failed for {media_type}: {e}", known.display()),
    }
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
