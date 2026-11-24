mod interface;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use datamile_fs::expand_path;
use datamile_logic::LogicEngine;
use datamile_lua_domain::LogicManifest;
use datamile_media::{
    ForceMode, MediaEngineBackend, MediaState, Verbosity, compile_root, open_engine,
    preview_root,
};
use datamile_server::MediaBackend;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Config directory containing init.lua, which is also the root for
    /// `require` and relative storage paths. Default: ~/.config/datamile
    #[arg(long, global = true, value_name = "DIR")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server(ServerArgs),
    Interface(InterfaceArgs),
    Compile(CompileArgs),
    Update(UpdateArgs),
    X(XArgs),
}

impl Commands {
    async fn execute(self) -> Result<()> {
        match self {
            Self::Server(cmd) => cmd.run().await,
            Self::Interface(cmd) => cmd.run(),
            Self::Compile(cmd) => cmd.run(),
            Self::Update(cmd) => cmd.run().await,
            Self::X(cmd) => cmd.run().await,
        }
    }
}

#[derive(Args)]
struct ServerArgs {
    #[arg(long, default_value = "8000")]
    port: u16,
}

impl ServerArgs {
    async fn run(self) -> Result<()> {
        let backend = Arc::new(MediaEngineBackend::new());
        datamile_server::run(backend, self.port).await
    }
}

#[derive(Args)]
struct InterfaceArgs {
    #[arg(value_name = "NAME")]
    name: Option<String>,
}

impl InterfaceArgs {
    fn run(self) -> Result<()> {
        interface::execute(self.name)
    }
}

/// Loads the config through the media backend so CLI commands see the same
/// roots and manifest as the server.
fn load_state() -> Result<(MediaEngineBackend, MediaState, LogicManifest)> {
    let backend = MediaEngineBackend::new();
    let mut logic = LogicEngine::new(PathBuf::new(), LogicManifest::default());
    backend.reload_config_and_logic(&mut logic)?;

    let state = backend
        .shared()
        .state
        .read()
        .map_err(|_| anyhow::anyhow!("media state lock is poisoned"))?
        .clone()
        .context("Media state is not loaded")?;

    Ok((backend, state, logic.manifest))
}

fn roots_under<'a>(
    state: &'a MediaState,
    path: &Path,
    media_type: Option<&str>,
) -> Vec<&'a datamile_media::DiscoveredRoot> {
    state
        .roots
        .iter()
        .filter(|root| media_type.is_none_or(|mt| root.media_type == mt))
        .filter(|root| {
            root.dir == path || root.dir.starts_with(path) || path.starts_with(&root.dir)
        })
        .collect()
}

#[derive(Args)]
struct CompileArgs {
    #[arg(value_name = "PATH", required = true)]
    path: String,
    #[arg(long = "type", value_name = "MEDIA_TYPE")]
    media_type: Option<String>,
    #[arg(long)]
    stdout: bool,
    #[arg(long)]
    pretty: bool,
    #[arg(long)]
    force: bool,
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
    #[arg(long)]
    silent: bool,
}

impl CompileArgs {
    fn run(self) -> Result<()> {
        let (_backend, state, _manifest) = load_state()?;
        let path = expand_path(&self.path)
            .canonicalize()
            .unwrap_or_else(|_| expand_path(&self.path));
        let engine = open_engine(&state)?;
        let roots = roots_under(&state, &path, self.media_type.as_deref());

        if roots.is_empty() {
            anyhow::bail!("no media root matches {}", path.display());
        }

        let _ = (self.force, self.jobs);

        for root in roots {
            if self.stdout {
                let lock = preview_root(
                    &engine,
                    &root.media_type,
                    &root.dir,
                    &state.cache_root,
                )?;
                if self.pretty {
                    println!("{lock}");
                } else {
                    let value: Value = serde_json::from_str(&lock)?;
                    println!("{}", serde_json::to_string(&value)?);
                }
            } else {
                let compiled = compile_root(
                    &engine,
                    &root.media_type,
                    &root.dir,
                    &state.cache_root,
                    None,
                )?;
                if !self.silent {
                    log::info!(
                        "Compiled {}/{} ({})",
                        compiled.media_type,
                        compiled.id,
                        if compiled.modified {
                            "updated"
                        } else {
                            "unchanged"
                        }
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Args)]
struct UpdateArgs {
    #[arg(value_name = "PATH")]
    path: Option<String>,
    #[arg(long = "type", value_name = "MEDIA_TYPE")]
    media_type: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
    #[arg(long)]
    silent: bool,
}

impl UpdateArgs {
    async fn run(self) -> Result<()> {
        let path = self.path.map(|p| expand_path(&p));
        let verbosity = if self.silent {
            Verbosity::Silent
        } else {
            Verbosity::Verbose
        };
        datamile_media::update::run(
            path,
            self.media_type,
            ForceMode::from(self.force),
            self.jobs,
            verbosity,
        )
        .await
    }
}

#[derive(Args)]
struct XArgs {
    #[arg(value_name = "NAME", required = true)]
    name: String,
    #[arg(long = "type", value_name = "MEDIA_TYPE")]
    media_type: Option<String>,
    #[arg(long, short = 'p', group = "target")]
    playing: bool,
    #[arg(long, group = "target")]
    id: Option<String>,
    #[arg(long, short = 'q', group = "target")]
    query: Option<String>,
    #[arg(long, short = 'd', group = "target")]
    directory: Option<String>,
    #[arg(long, short = 'r', group = "target")]
    recursive: Option<String>,
    #[arg(long, short = 'l', group = "target")]
    library: bool,
    #[arg(long)]
    debug: bool,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

impl XArgs {
    async fn run(self) -> Result<()> {
        let (backend, state, _manifest) = load_state()?;
        let mut logic = LogicEngine::new(PathBuf::new(), LogicManifest::default());
        backend.reload_config_and_logic(&mut logic)?;
        backend.scan_library(&mut logic);

        let media_types: Vec<String> = self.media_type.map_or_else(
            || state.media.roots_order.clone(),
            |media_type| vec![media_type],
        );

        let mut items: Vec<Value> = Vec::new();

        if self.playing {
            let file = playing_file(&state).await?;
            if let Some(file) = file {
                let clean = file.trim_start_matches('/');
                if let Some(item) = logic
                    .playable_lookup
                    .get(clean)
                    .or_else(|| logic.playable_lookup.get(&file))
                {
                    items.extend(item_for(&logic, &item.media_type, &item.media_id));
                }
            }
        } else if let Some(query) = &self.query {
            for media_type in &media_types {
                for id in logic.find_ids(media_type, query) {
                    items.extend(item_for(&logic, media_type, &id));
                }
            }
        } else if let Some(id) = &self.id {
            for media_type in &media_types {
                items.extend(item_for(&logic, media_type, id));
            }
        } else if self.library || self.recursive.is_some() {
            let root = self.recursive.as_ref().map_or_else(
                || state.init_dirs().first().cloned().unwrap_or_default(),
                |dir| expand_path(dir),
            );
            let root = root.canonicalize().unwrap_or(root);
            for record in logic.all_entities() {
                if media_types.contains(&record.media_type)
                    && record.path.starts_with(&root)
                {
                    items.extend(item_for(&logic, &record.media_type, &record.id));
                }
            }
        } else {
            let dir = self.directory.as_deref().unwrap_or(".");
            let path = expand_path(dir);
            let path = path.canonicalize().unwrap_or(path);
            for media_type in &media_types {
                if let Some(id) = logic.entity_id_by_path(media_type, &path) {
                    items.extend(item_for(&logic, media_type, id));
                }
            }
        }

        let payload = json!({
            "items": items,
            "options": self.args.join(" "),
            "isatty": std::io::stdin().is_terminal()
        });

        if self.name == "intermediary" {
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }

        let name = self.name.replace('-', "_");
        let _ = self.debug;
        backend.execute_action(&name, &payload)
    }
}

fn item_for(logic: &LogicEngine, media_type: &str, id: &str) -> Option<Value> {
    let record = logic.entity_record(media_type, id)?;
    let lock = serde_json::from_str::<Value>(&record.raw_json).ok()?;
    Some(json!({
        "type": record.media_type,
        "id": record.id,
        "path": record.path.to_string_lossy(),
        "lock": lock
    }))
}

/// Server first, one-shot MPD query as the fallback.
async fn playing_file(state: &MediaState) -> Result<Option<String>> {
    let client = reqwest::Client::new();
    if let Ok(response) = client
        .get("http://127.0.0.1:8000/api/playback/current")
        .timeout(std::time::Duration::from_millis(300))
        .send()
        .await
        && let Ok(value) = response.json::<Value>().await
        && let Some(file) = value.get("file").and_then(Value::as_str)
        && !file.is_empty()
    {
        return Ok(Some(file.to_string()));
    }

    datamile_mpd::current_playing_file(&state.mpd.addr())
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn init_logger() {
    simple_logger::SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .with_module_level("mpd_protocol", log::LevelFilter::Warn)
        .with_module_level("mpd_client", log::LevelFilter::Warn)
        .with_module_level("tracing", log::LevelFilter::Warn)
        .env()
        .init()
        .ok();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_logger();
    let cli = Cli::parse();

    if let Some(dir) = cli.config {
        datamile_lua_domain::set_config_dir(dir);
    }

    cli.command.execute().await
}
