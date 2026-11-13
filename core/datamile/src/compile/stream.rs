use crate::compile::{CompileReport, ExportTarget, LogVerbosity, media};
use anyhow::Result;
use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub struct CompiledItem {
    pub root_type: String,
    pub target: PathBuf,
    pub id: String,
    pub lock_path: PathBuf,
    pub lock_val: Value,
    pub eval_res: Option<Value>,
    pub files: HashSet<PathBuf>,
    pub dirs: HashSet<PathBuf>,
}

pub struct StreamContext {
    pub roots: Vec<media::RootTarget>,
    pub config: Arc<libdatamile::lua::ResolvedConfig>,
    pub cache_root: PathBuf,
    pub target: ExportTarget,
    pub jobs: Option<usize>,
    pub ingest_tx: Option<
        tokio::sync::mpsc::Sender<crate::server::api::system::EntityIngestPayload>,
    >,
    pub active_writes: Option<Arc<Mutex<HashSet<PathBuf>>>>,
    pub verbosity: LogVerbosity,
}

pub async fn run(ctx: StreamContext) -> Result<CompileReport> {
    let (dtx, mut drx) = tokio::sync::mpsc::channel::<CompiledItem>(512);
    let written_count = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(Mutex::new(None));

    let build_handle = spawn_builders(&ctx, dtx, Arc::clone(&errors));

    let target = ctx.target;
    let written_ref = Arc::clone(&written_count);
    let ingest_tx = ctx.ingest_tx;
    let active_writes = ctx.active_writes.clone();
    let verbosity = ctx.verbosity;
    let direct_errors = Arc::clone(&errors);

    let direct_handle = tokio::spawn(async move {
        while let Some(item) = drx.recv().await {
            let written_inner = Arc::clone(&written_ref);
            let active_ref = active_writes.clone();
            let lock_path = item.lock_path.clone();
            let res = tokio::task::spawn_blocking(move || {
                finalize(item, target, active_ref.as_ref(), verbosity)
            })
            .await;

            match res {
                Ok(Ok((written, payload))) => {
                    if written {
                        written_inner.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(p) = payload
                        && let Some(ref tx) = ingest_tx
                    {
                        let _ = tx.send(p).await;
                    }
                }
                Ok(Err(e)) => record_error(
                    &direct_errors,
                    e.context(format!("Failed to finalize {}", lock_path.display())),
                ),
                Err(e) => record_error(
                    &direct_errors,
                    anyhow::Error::new(e).context(format!(
                        "Finalize task failed for {}",
                        lock_path.display()
                    )),
                ),
            }
        }
    });

    let _ = build_handle.await;
    let _ = direct_handle.await;

    let error = errors.lock().ok().and_then(|mut slot| slot.take());

    Ok(CompileReport {
        written: written_count.load(Ordering::Relaxed),
        error,
    })
}

fn record_error(errors: &Arc<Mutex<Option<anyhow::Error>>>, err: anyhow::Error) {
    log::error!("{err:#}");
    let Ok(mut slot) = errors.lock() else {
        return;
    };
    if slot.is_none() {
        *slot = Some(err);
    }
}

fn spawn_builders(
    ctx: &StreamContext,
    dtx: tokio::sync::mpsc::Sender<CompiledItem>,
    errors: Arc<Mutex<Option<anyhow::Error>>>,
) -> tokio::task::JoinHandle<()> {
    let roots = ctx.roots.clone();
    let cfg = Arc::clone(&ctx.config);
    let cache_root = ctx.cache_root.clone();
    let jobs = ctx.jobs;

    tokio::task::spawn_blocking(move || {
        let default_jobs =
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.unwrap_or(default_jobs))
            .build()
            .unwrap();
        pool.install(|| {
            roots.par_iter().for_each_init(
                || match libdatamile::lua::LuaEngine::new() {
                    Ok(engine) => match engine.evaluate_config(&cfg.path) {
                        Ok(_) => Some(engine),
                        Err(e) => {
                            record_error(
                                &errors,
                                e.context("Failed to evaluate config for worker thread"),
                            );
                            None
                        }
                    },
                    Err(e) => {
                        record_error(
                            &errors,
                            e.context("Failed to initialize Lua engine for worker thread"),
                        );
                        None
                    }
                },
                |engine_opt, root| {
                    let display = root
                        .source
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("<virtual>");
                    let Some(engine) = engine_opt.as_ref() else {
                        record_error(
                            &errors,
                            anyhow::anyhow!(
                                "Skipping compilation of {display} due to worker Lua engine initialization failure"
                            ),
                        );
                        return;
                    };

                    match media::compile_root(engine, root, &cache_root) {
                        Ok(out) => match engine.evaluate_entity_logic(&out.root_type, &out.lock_json) {
                            Ok(eval_res) => {
                                let _ = dtx.blocking_send(CompiledItem {
                                    root_type: out.root_type,
                                    target: out.target,
                                    id: out.id,
                                    lock_path: out.lock_path,
                                    lock_val: out.lock_json,
                                    eval_res: Some(eval_res),
                                    files: out.files,
                                    dirs: out.dirs,
                                });
                            }
                            Err(e) => {
                                record_error(
                                    &errors,
                                    e.context(format!("Logic evaluation failed for {display}")),
                                );
                            }
                        },
                        Err(e) => {
                            record_error(
                                &errors,
                                e.context(format!("Compilation failed for {display}")),
                            );
                        }
                    }
                },
            );
        });
    })
}

fn finalize(
    item: CompiledItem,
    target: ExportTarget,
    active_writes: Option<&Arc<Mutex<HashSet<PathBuf>>>>,
    verbosity: LogVerbosity,
) -> Result<(
    bool,
    Option<crate::server::api::system::EntityIngestPayload>,
)> {
    let content = serde_json::to_string_pretty(&item.lock_val)?;

    if target == ExportTarget::Stdout {
        println!("{content}");
        return Ok((false, None));
    }

    let should_write = std::fs::read_to_string(&item.lock_path)
        .map_or(true, |existing| existing != content);

    let payload = crate::server::api::system::EntityIngestPayload {
        root_type: item.root_type.clone(),
        target: item.target,
        lock_path: item.lock_path.clone(),
        id: item.id,
        lock_json: content.clone(),
        eval_res: item.eval_res,
        files: item.files.into_iter().collect(),
        dirs: item.dirs.into_iter().collect(),
        modified: should_write,
    };

    if should_write {
        if let Some(aw) = active_writes
            && let Ok(mut active) = aw.lock()
        {
            active.insert(item.lock_path.clone());
        } else if verbosity == LogVerbosity::Verbose {
            log::info!(
                "Updated {} lock: {}",
                item.root_type,
                item.lock_path.display()
            );
        }
        if let Some(parent) = item.lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&item.lock_path, content)?;
        return Ok((true, Some(payload)));
    }

    Ok((false, Some(payload)))
}
