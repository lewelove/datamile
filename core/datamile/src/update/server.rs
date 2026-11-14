use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::compile::{self, LogVerbosity};
use crate::logging;
use crate::server::inotify::handler::{IngestOrigin, RecompiledAlbumItem};
use crate::server::state::{AppState, PendingUpdateParams, UpdateScope};
use crate::update::cache::{
    deps_graph_path, library_cache_dir, library_key, load_cache, save_cache,
    validate_library_root,
};
use crate::update::client::ForceMode;
use crate::update::queue::{
    WorkQueueContext, check_lua_config_changed, rebuild_cache, resolve_work_queue,
    try_get_server_tracked_roots,
};

pub struct CompilePassContext<'a> {
    pub scope: &'a UpdateScope,
    pub media_roots: &'a [PathBuf],
    pub roots: &'a [crate::compile::media::RootTarget],
    pub missing: &'a [PathBuf],
    pub effective_jobs: Option<usize>,
    pub verbosity: LogVerbosity,
}

pub fn spawn_server_ingest_handler(
    state: Arc<AppState>,
) -> (
    tokio::sync::mpsc::Sender<crate::server::api::system::EntityIngestPayload>,
    tokio::task::JoinHandle<()>,
) {
    let (ingest_tx, mut ingest_rx) = tokio::sync::mpsc::channel::<
        crate::server::api::system::EntityIngestPayload,
    >(512);

    let ingest_handle = tokio::spawn(async move {
        while let Some(first_payload) = ingest_rx.recv().await {
            let mut batch = vec![first_payload];

            while let Ok(payload) = ingest_rx.try_recv() {
                batch.push(payload);
            }

            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            while let Ok(payload) = ingest_rx.try_recv() {
                batch.push(payload);
            }

            let mut vanished = HashSet::new();
            let mut items = Vec::with_capacity(batch.len());

            for payload in batch {
                if payload.eval_res.is_none() {
                    vanished.insert(payload.lock_path);
                    continue;
                }

                items.push(RecompiledAlbumItem::from(payload));
            }

            crate::server::inotify::handler::ingest_and_broadcast_albums(
                vanished,
                items,
                IngestOrigin::Internal,
                &state,
            )
            .await;
        }
    });

    (ingest_tx, ingest_handle)
}

pub async fn run_server_update(
    state: Arc<AppState>,
    target_path: Option<PathBuf>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
    log_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let incoming_scope = target_path.map_or_else(
        || UpdateScope::All,
        |p| {
            let mut set = HashSet::new();
            set.insert(p);
            UpdateScope::Paths(set)
        },
    );

    let mut current_scope = incoming_scope;
    let mut current_force = force;
    let mut current_jobs = jobs;
    let mut current_verbosity = verbosity;
    let mut current_log_txs = log_tx.into_iter().collect::<Vec<_>>();

    {
        let mut lock = state
            .update_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if lock.is_running {
            if let Some(ref mut pending) = lock.pending {
                pending.scope.merge(current_scope);
                pending.force = pending.force || current_force == ForceMode::Force;
                pending.jobs = current_jobs.or(pending.jobs);
                pending.silent =
                    pending.silent && current_verbosity == LogVerbosity::Silent;
                pending.log_txs.extend(current_log_txs);
            } else {
                lock.pending = Some(PendingUpdateParams {
                    scope: current_scope,
                    force: current_force == ForceMode::Force,
                    jobs: current_jobs,
                    silent: current_verbosity == LogVerbosity::Silent,
                    log_txs: current_log_txs,
                });
            }
            return Ok(());
        }

        lock.is_running = true;
    }

    let mut last_res;

    loop {
        let res = run_server_update_pass(
            Arc::clone(&state),
            current_scope.clone(),
            current_force,
            current_jobs,
            current_verbosity,
            current_log_txs.clone(),
        )
        .await;

        if let Err(ref e) = res {
            let err_msg = format!("Update failed: {e}");
            logging::emit(&err_msg, current_verbosity, &current_log_txs);
        }

        last_res = res;

        let next_params = {
            let mut lock = state
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

        if let Some(next) = next_params {
            current_scope = next.scope;
            current_force = if next.force {
                ForceMode::Force
            } else {
                ForceMode::Preserve
            };
            current_jobs = next.jobs;
            current_verbosity = if next.silent {
                LogVerbosity::Silent
            } else {
                LogVerbosity::Verbose
            };
            current_log_txs = next.log_txs;
        } else {
            break;
        }
    }

    last_res
}

async fn run_server_update_pass(
    state: Arc<AppState>,
    scope: UpdateScope,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
    log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let config = state.config.read().await.app.clone();
    let config_path = state.config.read().await.config_path.clone();
    let media_roots = state.config.read().await.media_roots.clone();
    let cache_root = state.config.read().await.cache_root.clone();
    let dependencies = state.config.read().await.resolved_dependencies.clone();

    let effective_jobs = jobs.or(config.compiler.jobs);
    let is_full_library = scope == UpdateScope::All;

    let base_cache_dir = library_cache_dir(&cache_root, &media_roots);
    fs::create_dir_all(&base_cache_dir)?;

    let lib_hash = library_key(&media_roots);
    validate_library_root(&base_cache_dir, &lib_hash).await?;

    let mut cache = load_cache(&base_cache_dir.join("library.json"));

    let (config_changed, lua_hash_file, lua_hash) =
        check_lua_config_changed(&dependencies, &cache_root, verbosity, &log_txs);
    let effective_force = match force {
        ForceMode::Force => ForceMode::Force,
        ForceMode::Preserve => {
            if config_changed {
                ForceMode::Force
            } else {
                ForceMode::Preserve
            }
        }
    };

    let tracked = if effective_force == ForceMode::Preserve && is_full_library {
        try_get_server_tracked_roots().await
    } else {
        None
    };

    let deps_graph = state.deps_graph.read().await.clone();
    let queue = {
        let engine =
            libdatamile::lua::LuaEngine::new().context("Failed to create Lua engine")?;
        engine
            .evaluate_config(&config_path)
            .context("Failed to evaluate config in Lua engine")?;

        resolve_work_queue(&WorkQueueContext {
            engine: &engine,
            scope: &scope,
            media_roots: &media_roots,
            cache: &cache,
            deps_graph: &deps_graph,
            force: effective_force,
            effective_jobs,
            tracked,
            verbosity,
            log_txs: &log_txs,
        })?
    };

    if queue.roots.is_empty() && queue.missing.is_empty() {
        logging::emit("Library is up to date.", verbosity, &log_txs);
        let _ = fs::write(&lua_hash_file, &lua_hash);
        save_cache(&cache, &base_cache_dir.join("library.json"))?;
        let deps_json_path = deps_graph_path(&cache_root, &media_roots);
        let deps_graph_arc = Arc::clone(&state.deps_graph);
        tokio::task::spawn_blocking(move || {
            let mut guard = deps_graph_arc.blocking_write();
            guard.prune();
            if let Err(e) = guard.save_to_file(&deps_json_path) {
                log::error!("Failed to persist dependency graph: {e}");
            }
        })
        .await?;
        return Ok(());
    }

    let checked_count = queue.roots.len();
    let missing_count = queue.missing.len();
    let start_time = std::time::Instant::now();

    let report = process_missing_and_compile(
        &state,
        CompilePassContext {
            scope: &scope,
            media_roots: &media_roots,
            roots: &queue.roots,
            missing: &queue.missing,
            effective_jobs,
            verbosity,
        },
        &log_txs,
    )
    .await?;

    let deps_graph_guard = state.deps_graph.read().await;
    rebuild_cache(&mut cache, &deps_graph_guard);
    drop(deps_graph_guard);

    let elapsed = start_time.elapsed().as_millis();
    logging::emit(
        &format!(
            "Update complete: {checked_count} checked ({} modified), {missing_count} removed. Finished in {elapsed}ms.",
            report.written
        ),
        verbosity,
        &log_txs,
    );

    let _ = fs::write(&lua_hash_file, &lua_hash);
    save_cache(&cache, &base_cache_dir.join("library.json"))?;

    report.into_result().map(|_| ())
}

async fn process_missing_and_compile(
    state: &Arc<AppState>,
    ctx: CompilePassContext<'_>,
    log_txs: &[tokio::sync::mpsc::Sender<String>],
) -> Result<compile::CompileReport> {
    let (ingest_tx, ingest_handle) = spawn_server_ingest_handler(Arc::clone(state));

    for lock in ctx.missing {
        let id = lock
            .file_stem()
            .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
        logging::emit(&format!("Removed entity: {id}"), ctx.verbosity, log_txs);
        let _ = ingest_tx
            .send(crate::server::api::system::EntityIngestPayload {
                root_type: String::new(),
                target: lock.clone(),
                lock_path: lock.clone(),
                id,
                lock_json: String::new(),
                eval_res: None,
                files: Vec::new(),
                dirs: Vec::new(),
                modified: false,
            })
            .await;
    }

    let fallback = ctx.media_roots.first().cloned().unwrap_or_default();
    let scan_root = match ctx.scope {
        UpdateScope::All => fallback.clone(),
        UpdateScope::Paths(paths) => {
            if paths.len() == 1 {
                paths.iter().next().cloned().unwrap_or_default()
            } else {
                fallback.clone()
            }
        }
    };

    let compile_options = compile::CompileOptions {
        target_path: scan_root,
        specific_paths: Some(ctx.roots.iter().map(|root| root.target.clone()).collect()),
        jobs: ctx.effective_jobs,
        compile_flags: compile::CompileFlags {
            mode: compile::CompileMode::Standard,
            target: compile::ExportTarget::File,
            pretty: false,
        },
        ingest_tx: Some(ingest_tx),
        active_writes: Some(Arc::clone(&state.active_writes)),
        log_txs: log_txs.to_vec(),
        verbosity: ctx.verbosity,
    };

    let report = compile::run(compile_options).await?;

    let _ = ingest_handle.await;

    Ok(report)
}
