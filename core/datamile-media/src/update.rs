use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use datamile_cache::{FileStat, hash_files};
use datamile_logic::LogicEngine;
use datamile_server::{MediaBackend, ServerState, UpdateRequest};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::compiler::{CompiledRoot, compile_root};
use crate::deps::{DependencyGraph, MediaShared, PendingUpdate, UpdateScope};
use crate::schema::{DiscoveredRoot, discover_roots};
use crate::state::{
    MediaState, config_hash_path, deps_graph_path, open_engine, stat_cache_path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedRoot {
    media_type: String,
    dir: PathBuf,
}

/// Persisted between passes, so the files map and the root list are kept sorted:
/// a pass that changes nothing must not rewrite the file with new bytes.
#[derive(Debug, Default, Serialize, Deserialize)]
struct LibraryCache {
    files: BTreeMap<String, FileStat>,
    roots: Vec<CachedRoot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Verbose,
    Silent,
}

impl From<bool> for Verbosity {
    fn from(silent: bool) -> Self {
        if silent { Self::Silent } else { Self::Verbose }
    }
}

/// Whether a pass must recompile everything or only dirty roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceMode {
    Force,
    Preserve,
}

impl From<bool> for ForceMode {
    fn from(force: bool) -> Self {
        if force { Self::Force } else { Self::Preserve }
    }
}

pub struct UpdateOutcome {
    pub compiled: Vec<CompiledRoot>,
    pub removed: Vec<(String, PathBuf)>,
}

type PerTypeUpdates = HashMap<
    String,
    (
        HashMap<String, Box<serde_json::value::RawValue>>,
        Vec<String>,
    ),
>;

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn prefix_for(root: &Path) -> String {
    format!("{}/", path_key(root))
}

fn load_cache(path: &Path) -> LibraryCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_cache(cache: &LibraryCache, path: &Path) -> Result<()> {
    let content = serde_json::to_string_pretty(cache)?;
    datamile_fs::write_atomic(path, &content)?;
    Ok(())
}

fn walk_files(root: &Path) -> Vec<(String, FileStat)> {
    walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((path_key(entry.path()), FileStat::from(&meta)))
        })
        .collect()
}

fn external_deps_dirty(
    root: &Path,
    cache: &LibraryCache,
    deps_graph: &DependencyGraph,
) -> bool {
    let Some(deps) = deps_graph.root_to_files.get(root) else {
        return false;
    };

    deps.iter().any(|dep| {
        if dep.starts_with(root) {
            return false;
        }
        dep.metadata().map_or(true, |meta| {
            cache.files.get(&path_key(dep)) != Some(&FileStat::from(&meta))
        })
    })
}

fn root_dirty(
    root: &DiscoveredRoot,
    cache: &LibraryCache,
    deps_graph: &DependencyGraph,
    force: ForceMode,
) -> bool {
    if force == ForceMode::Force {
        return true;
    }

    let lock = root.dir.join(format!("{}.lock.json", root.media_type));
    if !lock.exists() {
        return true;
    }

    let known = cache
        .roots
        .iter()
        .any(|cached| cached.media_type == root.media_type && cached.dir == root.dir);
    if !known {
        return true;
    }

    let files = walk_files(&root.dir);
    if files
        .iter()
        .any(|(key, stat)| cache.files.get(key) != Some(stat))
    {
        return true;
    }

    let prefix = prefix_for(&root.dir);
    let cached_files = cache
        .files
        .keys()
        .filter(|key| key.starts_with(&prefix))
        .count();
    if cached_files != files.len() {
        return true;
    }

    external_deps_dirty(&root.dir, cache, deps_graph)
}

fn in_scope(root: &DiscoveredRoot, scope: &UpdateScope) -> bool {
    match scope {
        UpdateScope::All => true,
        UpdateScope::Paths(paths) => paths
            .iter()
            .any(|path| root.dir.starts_with(path) || path.starts_with(&root.dir)),
    }
}

fn compile_roots(
    state: &MediaState,
    roots: &[DiscoveredRoot],
    jobs: Option<usize>,
    active_writes: &Arc<std::sync::Mutex<HashSet<PathBuf>>>,
) -> Vec<CompiledRoot> {
    let default_jobs =
        std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.unwrap_or(default_jobs).max(1))
        .build();

    let Ok(pool) = pool else {
        log::error!("Failed to build the compile pool");
        return Vec::new();
    };

    pool.install(|| {
        roots
            .par_iter()
            .filter_map(|root| {
                let engine = match open_engine(state) {
                    Ok(engine) => engine,
                    Err(e) => {
                        log::error!("Failed to open a compile VM: {e}");
                        return None;
                    }
                };
                match compile_root(
                    &engine,
                    &root.media_type,
                    &root.dir,
                    &state.cache_root,
                    Some(active_writes),
                ) {
                    Ok(compiled) => Some(compiled),
                    Err(e) => {
                        log::error!("Compilation failed for {}: {e}", root.dir.display());
                        None
                    }
                }
            })
            .collect()
    })
}

/// One incremental verification and compilation pass.
#[allow(
    clippy::too_many_lines,
    reason = "the pass reads as one verify, compile and cache-update sequence"
)]
pub fn run_pass(
    shared: &Arc<MediaShared>,
    scope: &UpdateScope,
    media_type: Option<&str>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: Verbosity,
) -> Result<UpdateOutcome> {
    let state = shared
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .context("Media state is not loaded")?;

    let engine = open_engine(&state)?;
    let discovered: Vec<DiscoveredRoot> =
        discover_roots(&engine, &state.media, &state.config_dir)?
            .into_iter()
            .filter(|root| in_scope(root, scope))
            .filter(|root| media_type.is_none_or(|mt| root.media_type == mt))
            .collect();

    let inits = state.init_dirs();
    let cache_path = stat_cache_path(&state.cache_root, &inits);
    let deps_path = deps_graph_path(&state.cache_root, &inits);
    std::fs::create_dir_all(cache_path.parent().unwrap_or_else(|| Path::new(".")))?;

    let mut cache = load_cache(&cache_path);
    let mut deps_graph = shared
        .deps_graph
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    let config_hash_file = config_hash_path(&state.cache_root);
    let current_hash = hash_files(&state.dependencies);
    let config_changed =
        std::fs::read_to_string(&config_hash_file).unwrap_or_default() != current_hash;
    let force = if config_changed {
        ForceMode::Force
    } else {
        force
    };

    if verbosity == Verbosity::Verbose {
        log::info!("Verifying {} roots...", discovered.len());
    }

    let dirty: Vec<DiscoveredRoot> = discovered
        .iter()
        .filter(|root| root_dirty(root, &cache, &deps_graph, force))
        .cloned()
        .collect();

    let active_writes = Arc::new(std::sync::Mutex::new(HashSet::new()));
    let jobs = jobs.or(state.compiler_jobs);
    let compiled = compile_roots(&state, &dirty, jobs, &active_writes);

    for root in &compiled {
        deps_graph.update_root_deps(
            root.dir.clone(),
            root.dependencies.iter().cloned().collect(),
        );
        cache
            .files
            .retain(|key, _| !key.starts_with(&prefix_for(&root.dir)));
        for (key, stat) in walk_files(&root.dir) {
            cache.files.insert(key, stat);
        }
    }

    let discovered_keys: HashSet<(String, PathBuf)> = discovered
        .iter()
        .map(|root| (root.media_type.clone(), root.dir.clone()))
        .collect();

    let removed: Vec<(String, PathBuf)> = cache
        .roots
        .iter()
        .filter(|cached| {
            !discovered_keys.contains(&(cached.media_type.clone(), cached.dir.clone()))
        })
        .map(|cached| (cached.media_type.clone(), cached.dir.clone()))
        .collect();

    for (_, dir) in &removed {
        deps_graph.remove_root(dir);
        cache
            .files
            .retain(|key, _| !key.starts_with(&prefix_for(dir)));
        if let Some(root) = state.roots.iter().find(|root| &root.dir == dir) {
            let _ =
                std::fs::remove_file(dir.join(format!("{}.lock.json", root.media_type)));
        }
    }

    cache.roots = discovered
        .iter()
        .map(|root| CachedRoot {
            media_type: root.media_type.clone(),
            dir: root.dir.clone(),
        })
        .collect();
    cache.roots.sort_by(|a, b| a.dir.cmp(&b.dir));

    deps_graph.prune();
    save_cache(&cache, &cache_path)?;
    deps_graph.save_to_file(&deps_path)?;
    let _ = std::fs::write(&config_hash_file, &current_hash);

    {
        let mut guard = shared
            .deps_graph
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = deps_graph;
    }
    {
        let mut writes = shared
            .active_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        writes.extend(
            active_writes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned(),
        );
    }

    if verbosity == Verbosity::Verbose {
        log::info!(
            "Update complete: {} checked ({} modified), {} removed.",
            discovered.len(),
            compiled.iter().filter(|root| root.modified).count(),
            removed.len()
        );
    }

    Ok(UpdateOutcome { compiled, removed })
}

fn emit_log(
    message: &str,
    verbosity: Verbosity,
    log_txs: &[tokio::sync::mpsc::Sender<String>],
) {
    if verbosity == Verbosity::Verbose {
        log::info!("{message}");
    }
    for tx in log_txs {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
            tx.try_send(message.to_string())
        {
            log::warn!("Update log receiver queue is full; dropping a message");
        }
    }
}

fn ingest_outcome(
    state: &MediaState,
    logic: &mut LogicEngine,
    outcome: &UpdateOutcome,
) -> PerTypeUpdates {
    let mut per_type = PerTypeUpdates::new();

    for (media_type, dir) in &outcome.removed {
        if let Some(id) = logic.remove_entity_by_path(dir) {
            per_type.entry(media_type.clone()).or_default().1.push(id);
        }
    }

    let Ok(engine) = open_engine(state) else {
        log::error!("Failed to open the media engine for ingestion");
        return per_type;
    };

    for compiled in &outcome.compiled {
        let Some(schema) = state.root_schema(&compiled.media_type) else {
            continue;
        };
        let music_directory = state.mpd_base(&compiled.media_type, &compiled.dir);

        let Ok(parsed) = crate::ingest::parse_lock(
            &compiled.media_type,
            &compiled.dir,
            &compiled.lock_json,
            schema,
            music_directory.as_deref(),
        ) else {
            log::error!("Failed to parse lock for {}", compiled.dir.display());
            continue;
        };
        let Some(parsed) = parsed else {
            continue;
        };

        let eval = serde_json::from_str::<serde_json::Value>(&compiled.lock_json)
            .ok()
            .and_then(|lock| lock.get(&compiled.media_type).cloned())
            .and_then(|entity| engine.evaluate_logic(&compiled.media_type, &entity).ok())
            .unwrap_or_else(|| json!({}));

        if let Err(e) =
            logic.upsert_entity(parsed.entity, parsed.playables, parsed.images, eval)
        {
            log::error!("Failed to index {}: {e}", compiled.dir.display());
            continue;
        }

        let entry = per_type.entry(compiled.media_type.clone()).or_default();
        if let Some(dict) = logic.entity_dict(&compiled.media_type, &compiled.id) {
            entry.0.insert(compiled.id.clone(), dict.to_owned());
        }
    }

    logic.build_cache();
    per_type
}

fn shelves_for(logic: &LogicEngine, media_type: &str) -> HashMap<String, Vec<String>> {
    let mut shelves = HashMap::new();
    for (key, def) in &logic.manifest.shelves {
        if def.media != media_type {
            continue;
        }
        shelves.insert(
            key.clone(),
            logic.request_shelf_view(
                media_type,
                key,
                None,
                datamile_logic::SortOrder::Forward,
            ),
        );
    }
    shelves
}

#[allow(
    clippy::too_many_arguments,
    reason = "the pass mirrors the pending-update fields one to one"
)]
async fn run_update_pass(
    state: Arc<ServerState>,
    shared: Arc<MediaShared>,
    scope: UpdateScope,
    media_type: Option<String>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: Verbosity,
    log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let shared_for_pass = Arc::clone(&shared);
    let media_type_for_pass = media_type.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        run_pass(
            &shared_for_pass,
            &scope,
            media_type_for_pass.as_deref(),
            force,
            jobs,
            verbosity,
        )
    })
    .await??;

    let media_state = shared
        .state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .context("Media state is not loaded")?;

    let logic_arc = Arc::clone(&state.logic);
    let ingest_state = media_state.clone();
    let per_type = tokio::task::spawn_blocking(move || {
        let mut logic = logic_arc.blocking_write();
        let per_type = ingest_outcome(&ingest_state, &mut logic, &outcome);
        drop(logic);
        (per_type, outcome.compiled.len(), outcome.removed.len())
    })
    .await?;

    let (per_type, compiled_count, removed_count) = per_type;

    for (media_type, (updated, removed)) in per_type {
        if updated.is_empty() && removed.is_empty() {
            continue;
        }
        let shelves = {
            let logic = state.logic.read().await;
            shelves_for(&logic, &media_type)
        };
        let _ = state.tx.send(
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

    emit_log(
        &format!("Update complete: {compiled_count} compiled, {removed_count} removed."),
        verbosity,
        &log_txs,
    );

    Ok(())
}

pub async fn run_server_update(
    state: Arc<ServerState>,
    shared: Arc<MediaShared>,
    request: UpdateRequest,
    log_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let scope = request.path.map_or(UpdateScope::All, |path| {
        let mut set = HashSet::new();
        set.insert(datamile_fs::expand_path(&path));
        UpdateScope::Paths(set)
    });
    let verbosity = Verbosity::from(request.silent);

    {
        let mut lock = shared
            .update_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if lock.is_running {
            if let Some(pending) = lock.pending.as_mut() {
                pending.scope.merge(scope);
                pending.force |= request.force;
                pending.jobs = request.jobs.or(pending.jobs);
            } else {
                lock.pending = Some(PendingUpdate {
                    scope,
                    media_type: request.media_type,
                    force: request.force,
                    jobs: request.jobs,
                    silent: request.silent,
                    log_txs: log_tx.into_iter().collect(),
                });
            }
            return Ok(());
        }
        lock.is_running = true;
    }

    let mut scope = scope;
    let mut media_type = request.media_type;
    let mut force = ForceMode::from(request.force);
    let mut jobs = request.jobs;
    let mut log_txs = log_tx.into_iter().collect::<Vec<_>>();

    let mut last;
    loop {
        let result = run_update_pass(
            Arc::clone(&state),
            Arc::clone(&shared),
            scope.clone(),
            media_type.clone(),
            force,
            jobs,
            verbosity,
            log_txs.clone(),
        )
        .await;

        last = result;

        let next = {
            let mut lock = shared
                .update_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock.pending.take().map_or_else(
                || {
                    lock.is_running = false;
                    None
                },
                Some,
            )
        };

        let Some(next) = next else {
            break;
        };
        scope = next.scope;
        media_type = next.media_type;
        force = ForceMode::from(next.force);
        jobs = next.jobs;
        log_txs = next.log_txs;
    }

    last
}

/// Keeps the persisted stat cache in step with a hot reload.
pub(crate) fn sync_cache(
    state: &MediaState,
    logic: &LogicEngine,
    compiled: &[CompiledRoot],
    removed: &[(String, PathBuf)],
) {
    let inits = state.init_dirs();
    let cache_path = stat_cache_path(&state.cache_root, &inits);
    let mut cache = load_cache(&cache_path);

    for root in compiled {
        cache
            .files
            .retain(|key, _| !key.starts_with(&prefix_for(&root.dir)));
        for (key, stat) in walk_files(&root.dir) {
            cache.files.insert(key, stat);
        }
    }

    for (_, dir) in removed {
        cache
            .files
            .retain(|key, _| !key.starts_with(&prefix_for(dir)));
    }

    cache.roots = logic
        .all_entities()
        .map(|record| CachedRoot {
            media_type: record.media_type.clone(),
            dir: record.path.clone(),
        })
        .collect();
    cache.roots.sort_by(|a, b| a.dir.cmp(&b.dir));

    if let Err(e) = save_cache(&cache, &cache_path) {
        log::error!("Failed to persist the stat cache: {e}");
    }
}

/// CLI update: delegates to a running server, otherwise runs locally.
pub async fn run(
    path: Option<PathBuf>,
    media_type: Option<String>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: Verbosity,
) -> Result<()> {
    if is_server_up().await {
        return delegate(path, media_type, force, jobs, verbosity).await;
    }

    let backend = crate::MediaEngineBackend::new();
    let mut logic = LogicEngine::new(
        PathBuf::new(),
        datamile_lua_domain::LogicManifest::default(),
    );
    backend.reload_config_and_logic(&mut logic)?;

    let scope = path.map_or(UpdateScope::All, |path| {
        let mut set = HashSet::new();
        set.insert(datamile_fs::expand_path(&path.to_string_lossy()));
        UpdateScope::Paths(set)
    });

    let shared = backend.shared();
    tokio::task::spawn_blocking(move || {
        run_pass(
            &shared,
            &scope,
            media_type.as_deref(),
            force,
            jobs,
            verbosity,
        )
    })
    .await??;

    Ok(())
}

async fn is_server_up() -> bool {
    reqwest::Client::new()
        .get("http://127.0.0.1:8000/api/update/tracked")
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

async fn delegate(
    path: Option<PathBuf>,
    media_type: Option<String>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: Verbosity,
) -> Result<()> {
    let body = json!({
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "media_type": media_type,
        "force": force == ForceMode::Force,
        "jobs": jobs,
        "silent": verbosity == Verbosity::Silent
    });

    let mut response = reqwest::Client::new()
        .post("http://127.0.0.1:8000/api/update")
        .json(&body)
        .send()
        .await
        .context("Failed to reach the running server")?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Server update failed: {text}");
    }

    let mut buffer = String::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim_end().to_string();
            buffer.drain(..=pos);
            if !line.is_empty() && verbosity == Verbosity::Verbose {
                log::info!("{line}");
            }
        }
    }

    Ok(())
}
