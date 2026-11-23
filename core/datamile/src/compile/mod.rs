pub mod album;
pub mod build;
pub mod context;
pub mod images;
pub mod stream;
pub mod tracks;
pub mod utils;

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
    let config = crate::config::load().context("Config failed")?;

    if !options.flags.contains(&"default".to_string()) {
        options.flags.push("default".to_string());
    }

    let effective_jobs = options.jobs.or(config.app.compiler.jobs);

    let albums = if let Some(l) = options.specific_albums {
        l
    } else {
        libdatamile::scanner::find_target_albums(&options.target_path)?
    };

    if albums.is_empty() {
        return Ok(0);
    }

    if options.compile_flags.mode == CompileMode::Intermediary {
        let engine =
            libdatamile::lua::LuaEngine::new().context("Failed to create Lua engine")?;
        engine
            .evaluate_config(&config.path)
            .context("Failed to evaluate config in Lua engine")?;
        for root in &albums {
            let m = build::build(root, &config, &engine)?;
            if options.compile_flags.pretty {
                println!("{}", serde_json::to_string_pretty(&m.lock_json)?);
            } else {
                println!("{}", serde_json::to_string(&m.lock_json)?);
            }
        }
        return Ok(0);
    }

    let ctx = stream::StreamContext {
        albums: albums.clone(),
        config: Arc::new(config),
        target: options.compile_flags.target,
        jobs: effective_jobs,
        ingest_tx: options.ingest_tx,
        active_writes: options.active_writes,
        verbosity: options.verbosity,
    };

    stream::run(ctx).await
}
