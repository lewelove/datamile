pub mod images;
pub mod media;
pub mod stream;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    Standard,
    Intermediary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportTarget {
    File,
    Stdout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogVerbosity {
    Verbose,
    Silent,
}

pub struct CompileFlags {
    pub mode: CompileMode,
    pub target: ExportTarget,
    pub pretty: bool,
}

pub struct CompileOptions {
    pub target_path: PathBuf,
    pub specific_paths: Option<Vec<PathBuf>>,
    pub jobs: Option<usize>,
    pub compile_flags: CompileFlags,
    pub ingest_tx: Option<
        tokio::sync::mpsc::Sender<crate::server::api::system::EntityIngestPayload>,
    >,
    pub active_writes: Option<Arc<Mutex<HashSet<PathBuf>>>>,
    pub log_txs: Vec<crate::logging::LogTx>,
    pub verbosity: LogVerbosity,
}

/// Outcome of a compile pass. A per-album failure does not abort the batch:
/// callers persist state for the albums that succeeded, then surface `error`.
pub struct CompileReport {
    pub written: usize,
    pub error: Option<anyhow::Error>,
}

impl CompileReport {
    pub fn into_result(self) -> Result<usize> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(self.written),
        }
    }
}

pub async fn run(options: CompileOptions) -> Result<CompileReport> {
    let config = libdatamile::lua::ResolvedConfig::load().context("Config failed")?;

    let effective_jobs = options.jobs.or(config.app.compiler.jobs);
    let cache_root = libdatamile::utils::expand_path(&config.app.storage.cache);

    let (roots, intermediary_outputs) = {
        let engine =
            libdatamile::lua::LuaEngine::new().context("Failed to create Lua engine")?;
        engine
            .evaluate_config(&config.path)
            .context("Failed to evaluate config in Lua engine")?;

        let roots = media::discover_targets(
            &engine,
            &options.target_path,
            options.specific_paths.as_deref(),
        )?;

        let mut outputs = Vec::new();
        if options.compile_flags.mode == CompileMode::Intermediary {
            for root in &roots {
                let out = media::compile_root(&engine, root, &cache_root)?;
                outputs.push(out.lock_json);
            }
        }

        (roots, outputs)
    };

    if roots.is_empty() {
        return Ok(CompileReport {
            written: 0,
            error: None,
        });
    }

    if options.compile_flags.mode == CompileMode::Intermediary {
        for out in intermediary_outputs {
            if options.compile_flags.pretty {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{}", serde_json::to_string(&out)?);
            }
        }
        return Ok(CompileReport {
            written: 0,
            error: None,
        });
    }

    let ctx = stream::StreamContext {
        roots,
        config: Arc::new(config),
        cache_root,
        target: options.compile_flags.target,
        jobs: effective_jobs,
        ingest_tx: options.ingest_tx,
        active_writes: options.active_writes,
        log_txs: options.log_txs,
        verbosity: options.verbosity,
    };

    stream::run(ctx).await
}
