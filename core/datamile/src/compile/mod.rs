pub mod images;
pub mod media;
pub mod stream;

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
    pub flags: Vec<String>,
    pub specific_albums: Option<Vec<PathBuf>>,
    pub jobs: Option<usize>,
    pub compile_flags: CompileFlags,
    pub ingest_tx:
        Option<tokio::sync::mpsc::Sender<crate::server::api::system::AlbumIngestPayload>>,
    pub active_writes: Option<Arc<Mutex<HashSet<PathBuf>>>>,
    pub verbosity: LogVerbosity,
}

pub async fn run(mut options: CompileOptions) -> Result<usize> {
    let config = libdatamile::lua::ResolvedConfig::load().context("Config failed")?;
    if !options.flags.contains(&"default".to_string()) {
        options.flags.push("default".to_string());
    }

    let effective_jobs = options.jobs.or(config.app.compiler.jobs);
    let cache_root = libdatamile::utils::expand_path(&config.app.storage.cache);

    let (roots, intermediary_outputs) = {
        let engine =
            libdatamile::lua::LuaEngine::new().context("Failed to create Lua engine")?;
        engine
            .evaluate_config(&config.path)
            .context("Failed to evaluate config in Lua engine")?;

        let roots = media::discover_roots(
            &engine,
            &options.target_path,
            options.specific_albums.as_deref(),
        )?;

        let mut outputs = Vec::new();
        if options.compile_flags.mode == CompileMode::Intermediary {
            for (root_type, source) in &roots {
                let out = media::compile_root(&engine, root_type, source, &cache_root)?;
                outputs.push(out.lock_json);
            }
        }

        (roots, outputs)
    };

    if roots.is_empty() {
        return Ok(0);
    }

    if options.compile_flags.mode == CompileMode::Intermediary {
        for out in intermediary_outputs {
            if options.compile_flags.pretty {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{}", serde_json::to_string(&out)?);
            }
        }
        return Ok(0);
    }

    let ctx = stream::StreamContext {
        roots,
        config: Arc::new(config),
        cache_root,
        target: options.compile_flags.target,
        jobs: effective_jobs,
        ingest_tx: options.ingest_tx,
        active_writes: options.active_writes,
        verbosity: options.verbosity,
    };

    stream::run(ctx).await
}
