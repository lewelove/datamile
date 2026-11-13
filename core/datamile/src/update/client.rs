use crate::compile::{self, LogVerbosity};
use crate::server::state::RootMeta;
use crate::server::state::{DependencyGraph, UpdateScope};
use crate::update::cache::{
    calculate_path_hash, deps_graph_path, library_cache_dir, load_cache, save_cache,
    validate_library_root,
};
use crate::update::queue::{
    WorkQueueContext, check_lua_config_changed, rebuild_cache, resolve_work_queue,
};
use anyhow::{Context, Result};
use libdatamile::utils::expand_path;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceMode {
    Force,
    Preserve,
}

pub async fn run(
    target_path: Option<PathBuf>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
) -> Result<()> {
    if is_server_up(8000).await {
        return run_delegated_update(target_path, force, jobs, verbosity, 8000).await;
    }

    let config =
        libdatamile::lua::ResolvedConfig::load().context("Failed to load config")?;
    let music_directory = expand_path(&config.app.storage.music_directory)
        .canonicalize()
        .context("Invalid music_directory")?;

    let effective_jobs = jobs.or(config.app.compiler.jobs);

    let cache_root = expand_path(&config.app.storage.cache);
    let base_cache_dir = library_cache_dir(&cache_root, &music_directory);
    fs::create_dir_all(&base_cache_dir)?;

    let lib_hash = calculate_path_hash(&music_directory);
    validate_library_root(&base_cache_dir, &lib_hash).await?;

    let mut cache = load_cache(&base_cache_dir.join("library.json"));
    let deps_json_path = deps_graph_path(&cache_root, &music_directory);
    let mut deps_graph = DependencyGraph::load_from_file(&deps_json_path);

    let (config_changed, lua_hash_file, lua_hash) =
        check_lua_config_changed(&config.dependencies, &cache_root, verbosity);
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

    let (scan_root, scope) = resolve_target_scope(target_path.as_ref(), &music_directory);

    let queue = {
        let engine =
            libdatamile::lua::LuaEngine::new().context("Failed to create Lua engine")?;
        engine
            .evaluate_config(&config.path)
            .context("Failed to evaluate config in Lua engine")?;

        resolve_work_queue(&WorkQueueContext {
            engine: &engine,
            scope: &scope,
            media_directory: &music_directory,
            cache: &cache,
            deps_graph: &deps_graph,
            force: effective_force,
            effective_jobs,
            tracked: None,
            verbosity,
        })?
    };

    if queue.roots.is_empty() && queue.missing.is_empty() {
        if verbosity == LogVerbosity::Verbose {
            log::info!("Library is up to date.");
        }
        let _ = fs::write(&lua_hash_file, &lua_hash);
        save_cache(&cache, &base_cache_dir.join("library.json"))?;
        deps_graph.prune();
        if let Err(e) = deps_graph.save_to_file(&deps_json_path) {
            log::error!("Failed to persist dependency graph: {e}");
        }
        return Ok(());
    }

    let checked_count = queue.roots.len();
    let missing_count = queue.missing.len();
    let start_time = std::time::Instant::now();

    let report = execute_compile_pass(
        scan_root,
        &queue.roots,
        &mut deps_graph,
        effective_jobs,
        verbosity,
    )
    .await?;

    apply_missing(&queue.missing, &mut deps_graph);
    rebuild_cache(&mut cache, &deps_graph);

    let elapsed = start_time.elapsed().as_millis();
    if verbosity == LogVerbosity::Verbose {
        log::info!(
            "Update complete: {checked_count} checked ({} modified), {missing_count} removed. Finished in {elapsed}ms.",
            report.written
        );
    }

    let _ = fs::write(&lua_hash_file, &lua_hash);
    save_cache(&cache, &base_cache_dir.join("library.json"))?;
    deps_graph.prune();
    if let Err(e) = deps_graph.save_to_file(&deps_json_path) {
        log::error!("Failed to persist dependency graph: {e}");
    }

    report.into_result().map(|_| ())
}

async fn execute_compile_pass(
    scan_root: PathBuf,
    roots: &[crate::compile::media::RootTarget],
    deps_graph: &mut DependencyGraph,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
) -> Result<compile::CompileReport> {
    let (ingest_tx, mut ingest_rx) = tokio::sync::mpsc::channel::<
        crate::server::api::system::EntityIngestPayload,
    >(512);

    let ingest_handle = tokio::spawn(async move {
        let mut items = Vec::new();
        while let Some(payload) = ingest_rx.recv().await {
            items.push(payload);
        }
        items
    });

    let compile_options = compile::CompileOptions {
        target_path: scan_root,
        specific_paths: Some(roots.iter().map(|root| root.target.clone()).collect()),
        jobs,
        compile_flags: compile::CompileFlags {
            mode: compile::CompileMode::Standard,
            target: compile::ExportTarget::File,
            pretty: false,
        },
        ingest_tx: Some(ingest_tx),
        active_writes: None,
        verbosity,
    };

    let report = compile::run(compile_options).await?;
    let ingested_payloads = ingest_handle.await.unwrap_or_default();

    apply_ingested_deps(ingested_payloads, deps_graph);

    Ok(report)
}

fn apply_ingested_deps(
    payloads: Vec<crate::server::api::system::EntityIngestPayload>,
    deps_graph: &mut DependencyGraph,
) {
    for payload in payloads {
        if payload.eval_res.is_none() {
            deps_graph.remove_lock(&payload.lock_path);
            continue;
        }

        deps_graph.update_lock(
            payload.lock_path,
            RootMeta {
                root_type: payload.root_type,
                target: payload.target,
                files: payload.files.into_iter().collect(),
                dirs: payload.dirs.into_iter().collect(),
            },
        );
    }
}

fn apply_missing(missing: &[PathBuf], deps_graph: &mut DependencyGraph) {
    for lock in missing {
        deps_graph.remove_lock(lock);
    }
}

fn resolve_target_scope(
    target_path: Option<&PathBuf>,
    music_directory: &Path,
) -> (PathBuf, UpdateScope) {
    let scan_root = target_path.map_or_else(
        || music_directory.to_path_buf(),
        |p| p.canonicalize().unwrap_or_else(|_| p.clone()),
    );

    let scope = target_path.map_or_else(
        || UpdateScope::All,
        |p| {
            let mut set = HashSet::new();
            set.insert(p.canonicalize().unwrap_or_else(|_| p.clone()));
            UpdateScope::Paths(set)
        },
    );

    (scan_root, scope)
}

async fn run_delegated_update(
    target_path: Option<PathBuf>,
    force: ForceMode,
    jobs: Option<usize>,
    verbosity: LogVerbosity,
    port: u16,
) -> Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "path": target_path.map(|p| p.to_string_lossy().to_string()),
        "force": force == ForceMode::Force,
        "jobs": jobs,
        "silent": verbosity == LogVerbosity::Silent,
    });

    let mut res = client
        .post(format!(
            "http://127.0.0.1:{port}/api/internal/trigger_update"
        ))
        .json(&body)
        .send()
        .await
        .context("Failed to connect to running server for delegated update")?;

    if !res.status().is_success() {
        let text = res.text().await.unwrap_or_default();
        anyhow::bail!("Server update failed: {text}");
    }

    let mut buffer = String::new();

    while let Ok(Some(chunk)) = res.chunk().await {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim_end().to_string();
            buffer.drain(..=pos);
            if !line.is_empty() && verbosity == LogVerbosity::Verbose {
                log::info!("{line}");
            }
        }
    }

    if !buffer.trim().is_empty() && verbosity == LogVerbosity::Verbose {
        let line = buffer.trim();
        log::info!("{line}");
    }

    Ok(())
}

pub async fn is_server_up(port: u16) -> bool {
    let client = reqwest::Client::new();
    client
        .get(format!(
            "http://127.0.0.1:{port}/api/internal/tracked_albums"
        ))
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}
