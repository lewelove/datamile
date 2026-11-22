use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use libdatamile::utils::tag_text;
use rayon::prelude::*;
use serde_json::Value;

use crate::compile::{ExportTarget, LogVerbosity, build};

pub struct CompiledItem {
    pub album_dir: PathBuf,
    pub album_id: String,
    pub lock_val: Value,
    pub eval_res: Option<Value>,
    pub deps: HashSet<PathBuf>,
}

pub struct StreamContext {
    pub albums: Vec<PathBuf>,
    pub config: Arc<libdatamile::lua::ResolvedConfig>,
    pub target: ExportTarget,
    pub jobs: Option<usize>,
    pub ingest_tx:
        Option<tokio::sync::mpsc::Sender<crate::server::api::system::AlbumIngestPayload>>,
    pub active_writes: Option<Arc<Mutex<HashSet<PathBuf>>>>,
    pub verbosity: LogVerbosity,
}

pub async fn run(ctx: StreamContext) -> Result<usize> {
    let (dtx, mut drx) = tokio::sync::mpsc::channel::<CompiledItem>(512);
    let written_count = Arc::new(AtomicUsize::new(0));

    let build_handle = spawn_builders(&ctx, dtx);

    let target = ctx.target;
    let written_ref = Arc::clone(&written_count);
    let ingest_tx = ctx.ingest_tx;
    let active_writes = ctx.active_writes.clone();
    let verbosity = ctx.verbosity;

    let direct_handle = tokio::spawn(async move {
        while let Some(item) = drx.recv().await {
            let written_inner = Arc::clone(&written_ref);
            let active_ref = active_writes.clone();
            let res = tokio::task::spawn_blocking(move || {
                finalize(item, target, active_ref.as_ref(), verbosity)
            })
            .await;
            if let Ok(Ok((written, payload))) = res {
                if written {
                    written_inner.fetch_add(1, Ordering::Relaxed);
                }
                if let Some(p) = payload
                    && let Some(ref tx) = ingest_tx
                {
                    let _ = tx.send(p).await;
                }
            }
        }
    });

    let _ = build_handle.await;
    let _ = direct_handle.await;

    Ok(written_count.load(Ordering::Relaxed))
}

fn spawn_builders(
    ctx: &StreamContext,
    dtx: tokio::sync::mpsc::Sender<CompiledItem>,
) -> tokio::task::JoinHandle<()> {
    let albums = ctx.albums.clone();
    let cfg = Arc::clone(&ctx.config);
    let jobs = ctx.jobs;

    tokio::task::spawn_blocking(move || {
        let default_jobs =
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        // `--jobs 0` would make the builder fail, so clamp to one worker.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.unwrap_or(default_jobs).max(1))
            .build()
            .expect("rayon pool with at least one worker thread");
        pool.install(|| {
            albums.par_iter().for_each_init(
                || match libdatamile::lua::LuaEngine::new() {
                    Ok(engine) => match engine.evaluate_config(&cfg.path) {
                        Ok(_) => Some(engine),
                        Err(e) => {
                            log::error!("Failed to evaluate config for worker thread: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        log::error!("Failed to initialize Lua engine for worker thread: {e}");
                        None
                    }
                },
                |engine_opt, ar| {
                    let canon = ar.canonicalize().unwrap_or_else(|_| ar.clone());
                    let canon_display = canon.display();
                    let Some(engine) = engine_opt.as_ref() else {
                        log::error!("Skipping compilation of {canon_display} due to worker Lua engine initialization failure");
                        return;
                    };
                    match build::build(ar, &cfg, engine) {
                        Ok(out) => match engine.evaluate_album_logic(&out.lock_json) {
                            Ok(eval_res) => {
                                let _ = dtx.blocking_send(CompiledItem {
                                    album_dir: out.album_dir,
                                    album_id: out.album_id,
                                    lock_val: out.lock_json,
                                    eval_res: Some(eval_res),
                                    deps: out.dependencies,
                                });
                            }
                            Err(e) => {
                                log::error!("Logic evaluation failed for {canon_display}: {e}");
                            }
                        },
                        Err(e) => {
                            log::error!("Compilation failed for {canon_display}: {e}");
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
) -> Result<(bool, Option<crate::server::api::system::AlbumIngestPayload>)> {
    let artist = tag_text(item.lock_val.pointer("/album/tags/albumartist"))
        .unwrap_or_else(|| "Unknown".to_string());
    let album = tag_text(item.lock_val.pointer("/album/tags/album"))
        .unwrap_or_else(|| "Unknown".to_string());

    let content = serde_json::to_string_pretty(&item.lock_val)?;

    if target == ExportTarget::Stdout {
        println!("{content}");
        return Ok((false, None));
    }

    let lock_path = item.album_dir.join("album.lock.json");
    let lock_canon = item.album_dir.join("album.lock.json");
    let should_write =
        std::fs::read_to_string(&lock_path).map_or(true, |existing| existing != content);

    let payload = crate::server::api::system::AlbumIngestPayload {
        album_dir: item.album_dir,
        id: item.album_id,
        artist,
        album,
        lock_json: content.clone(),
        eval_res: item.eval_res,
        dependencies: item.deps.into_iter().collect(),
        modified: should_write,
    };

    if should_write {
        if let Some(aw) = active_writes
            && let Ok(mut active) = aw.lock()
        {
            active.insert(lock_canon);
            active.insert(lock_path.clone());
        } else if verbosity == LogVerbosity::Verbose {
            log::info!("Updated lock: {}", payload.id);
        }
        std::fs::write(&lock_path, content)?;
        return Ok((true, Some(payload)));
    }

    Ok((false, Some(payload)))
}
