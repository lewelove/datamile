use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use datamile_cache::{calculate_path_hash, load_stat_cache, save_stat_cache};
use datamile_server::{ServerState, UpdateRequest};

use crate::compile::{self, AlbumIngestPayload, LogVerbosity};
use crate::deps::{LegacyShared, PendingUpdateParams, UpdateScope};
use crate::inotify::handler::IngestOrigin;
use crate::update::cache::{deps_graph_path, library_cache_dir, validate_library_root};
use crate::update::client::ForceMode;
use crate::update::queue::{
    WorkQueueContext, check_lua_config_changed, resolve_work_queue,
    try_get_server_tracked_albums, update_cache_entries,
};

pub(crate) struct CompilePassContext<'a> {
    pub scope: &'a UpdateScope,
    pub music_directory: &'a Path,
    pub work_queue: &'a [PathBuf],
    pub missing_paths: &'a [PathBuf],
    pub effective_jobs: Option<usize>,
    pub verbosity: LogVerbosity,
}

pub(crate) fn emit_log(
    msg: &str,
    verbosity: LogVerbosity,
    log_txs: &[tokio::sync::mpsc::Sender<String>],
) {
    if verbosity == LogVerbosity::Verbose {
        log::info!("{msg}");
    }
    for tx in log_txs {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
            tx.try_send(msg.to_string())
        {
            log::warn!(
                "Update log receiver queue full; dropping message for stream subscriber."
            );
        }
    }
}

pub(crate) fn spawn_server_ingest_handler(
    state: Arc<ServerState>,
    shared: LegacyShared,
    verbosity: LogVerbosity,
    log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> (
    tokio::sync::mpsc::Sender<AlbumIngestPayload>,
    tokio::task::JoinHandle<()>,
) {
    let (ingest_tx, mut ingest_rx) =
        tokio::sync::mpsc::channel::<AlbumIngestPayload>(512);

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

            let mut vanished_paths = HashSet::new();
            let mut items = Vec::with_capacity(batch.len());
            let mut logs = Vec::new();

            for payload in batch {
                if payload.eval_res.is_none() && !payload.album_dir.exists() {
                    vanished_paths.insert(payload.album_dir);
                } else if let Some(eval) = payload.eval_res {
                    if payload.modified
                        && !payload.lock_json.is_empty()
                        && !payload.id.is_empty()
                    {
                        logs.push(format!("Updated lock: {}", payload.id));
                    }
                    items.push((
                        payload.album_dir,
                        payload.id,
                        payload.lock_json,
                        Some(eval),
                        payload.dependencies.into_iter().collect(),
                    ));
                }
            }

            crate::inotify::handler::ingest_and_broadcast_albums(
                vanished_paths,
                items,
                IngestOrigin::Internal,
                &state,
                &shared,
            )
            .await;

            for log_msg in &logs {
                emit_log(log_msg, verbosity, &log_txs);
            }
        }
    });

    (ingest_tx, ingest_handle)
}

#[allow(
    clippy::too_many_lines,
    reason = "the update orchestration reads as one sequence; splitting hides it"
)]
pub async fn run_server_update(
    state: Arc<ServerState>,
    shared: LegacyShared,
    request: UpdateRequest,
    log_tx: Option<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let target_path = request.path.map(PathBuf::from);
    let force = if request.force {
        ForceMode::Force
    } else {
        ForceMode::Preserve
    };
    let jobs = request.jobs;
    let verbosity = if request.silent {
        LogVerbosity::Silent
    } else {
        LogVerbosity::Verbose
    };

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
        let mut lock = shared
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
            shared.clone(),
            current_scope.clone(),
            current_force,
            current_jobs,
            current_verbosity,
            current_log_txs.clone(),
        )
        .await;

        if let Err(ref e) = res {
            let err_msg = format!("Update failed: {e}");
            emit_log(&err_msg, current_verbosity, &current_log_txs);
        }

        last_res = res;

        let next_params = {
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

#[allow(
    clippy::too_many_lines,
    reason = "the update pass reads as one sequence; splitting hides it"
)]
async fn run_server_update_pass(
    state: Arc<ServerState>,
    shared: LegacyShared,
    scope: UpdateScope,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
    log_txs: Vec<tokio::sync::mpsc::Sender<String>>,
) -> Result<()> {
    let (music_directory, cache_root, dependencies, compiler_jobs) = {
        let guard = state.config.read().await;
        (
            guard.music_directory.clone(),
            guard.cache_root.clone(),
            guard.resolved_dependencies.clone(),
            guard.compiler_jobs,
        )
    };

    let effective_jobs = jobs.or(compiler_jobs);
    let is_full_library = scope == UpdateScope::All;

    let base_cache_dir = library_cache_dir(&cache_root, &music_directory);
    fs::create_dir_all(&base_cache_dir)?;

    let lib_hash = calculate_path_hash(&music_directory);
    validate_library_root(&base_cache_dir, &lib_hash).await?;

    let mut cache = load_stat_cache(&base_cache_dir.join("library.json"));

    let (config_changed, lua_hash_file, lua_hash) =
        check_lua_config_changed(&dependencies, &cache_root, verbosity);
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
        try_get_server_tracked_albums(&music_directory).await
    } else {
        None
    };

    let deps_graph = shared
        .deps_graph
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let (work_queue, missing_paths) = resolve_work_queue(WorkQueueContext {
        scope: &scope,
        music_directory: &music_directory,
        cache: &cache,
        deps_graph: &deps_graph,
        force: effective_force,
        effective_jobs,
        tracked,
        verbosity,
    })?;

    if work_queue.is_empty() && missing_paths.is_empty() {
        emit_log("Library is up to date.", verbosity, &log_txs);
        let _ = fs::write(&lua_hash_file, &lua_hash);
        save_stat_cache(&cache, &base_cache_dir.join("library.json"))?;
        let deps_json_path = deps_graph_path(&cache_root, &music_directory);
        let deps_graph_arc = Arc::clone(&shared.deps_graph);
        tokio::task::spawn_blocking(move || {
            let mut guard = deps_graph_arc
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.prune();
            if let Err(e) = guard.save_to_file(&deps_json_path) {
                log::error!("Failed to persist dependency graph: {e}");
            }
        })
        .await?;
        return Ok(());
    }

    let checked_count = work_queue.len();
    let missing_count = missing_paths.len();
    let start_time = std::time::Instant::now();

    let written_count = process_missing_and_compile(
        &state,
        &shared,
        CompilePassContext {
            scope: &scope,
            music_directory: &music_directory,
            work_queue: &work_queue,
            missing_paths: &missing_paths,
            effective_jobs,
            verbosity,
        },
        &log_txs,
    )
    .await?;

    let deps_graph_guard = shared
        .deps_graph
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update_cache_entries(
        &mut cache,
        &deps_graph_guard,
        &work_queue,
        &missing_paths,
        &music_directory,
        verbosity,
    );
    drop(deps_graph_guard);

    let elapsed = start_time.elapsed().as_millis();
    emit_log(
        &format!(
            "Update complete: {checked_count} checked ({written_count} modified), {missing_count} removed. Finished in {elapsed}ms."
        ),
        verbosity,
        &log_txs,
    );

    let _ = fs::write(&lua_hash_file, &lua_hash);
    save_stat_cache(&cache, &base_cache_dir.join("library.json"))?;

    Ok(())
}

async fn process_missing_and_compile(
    state: &Arc<ServerState>,
    shared: &LegacyShared,
    ctx: CompilePassContext<'_>,
    log_txs: &[tokio::sync::mpsc::Sender<String>],
) -> Result<usize> {
    let (ingest_tx, ingest_handle) = spawn_server_ingest_handler(
        Arc::clone(state),
        shared.clone(),
        ctx.verbosity,
        log_txs.to_vec(),
    );

    for missing in ctx.missing_paths {
        let album_id = datamile_fs::rel_path(missing, ctx.music_directory);
        emit_log(
            &format!("Removed album: {album_id}"),
            ctx.verbosity,
            log_txs,
        );
        let _ = ingest_tx
            .send(AlbumIngestPayload {
                album_dir: missing.clone(),
                id: String::new(),
                artist: String::new(),
                album: String::new(),
                lock_json: String::new(),
                eval_res: None,
                dependencies: Vec::new(),
                modified: false,
            })
            .await;
    }

    if !ctx.work_queue.is_empty() {
        let checked_count = ctx.work_queue.len();
        emit_log(
            &format!("Verifying {checked_count} albums..."),
            ctx.verbosity,
            log_txs,
        );
    }

    let scan_root = match ctx.scope {
        UpdateScope::All => ctx.music_directory.to_path_buf(),
        UpdateScope::Paths(paths) => match paths.iter().next() {
            Some(only) if paths.len() == 1 => only.clone(),
            _ => ctx.music_directory.to_path_buf(),
        },
    };

    let compile_options = compile::CompileOptions {
        target_path: scan_root,
        flags: vec!["default".to_string()],
        specific_albums: Some(ctx.work_queue.to_vec()),
        jobs: ctx.effective_jobs,
        compile_flags: compile::CompileFlags {
            mode: compile::CompileMode::Standard,
            target: compile::ExportTarget::File,
            pretty: false,
        },
        ingest_tx: Some(ingest_tx),
        active_writes: Some(Arc::clone(&shared.active_writes)),
        verbosity: ctx.verbosity,
    };

    let written_count = compile::run(compile_options).await?;

    let _ = ingest_handle.await;

    Ok(written_count)
}
