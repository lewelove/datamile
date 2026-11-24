mod interface;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use datamile_fs::expand_path;
use datamile_logic::LogicEngine;
use datamile_lua_domain::LogicManifest;
use datamile_media::{
    ForceMode, LogSink, MediaEngineBackend, MediaState, open_engine, preview_root,
};
use datamile_server::instance::InstanceError;
use datamile_server::{ActionTargetQuery, Identity, InstanceLock, MediaBackend};
use serde_json::{Value, json};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Config directory containing init.lua, which is also the root for
    /// `require` and relative storage paths. Default: ~/.config/datamile
    #[arg(long, global = true, value_name = "DIR")]
    config: Option<PathBuf>,
    /// Raise the log filter to debug, which makes `Unchanged` visible.
    #[arg(long, short = 'v', global = true)]
    verbose: bool,
    /// Print nothing.
    #[arg(long, global = true)]
    silent: bool,
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

/// One daemon per user: the instance lock is taken before anything binds.
#[derive(Args)]
struct ServerArgs {}

impl ServerArgs {
    async fn run(self) -> Result<()> {
        let _lock = InstanceLock::acquire().map_err(|e| anyhow::anyhow!("{e}"))?;
        let backend = Arc::new(MediaEngineBackend::new());
        datamile_server::run(backend).await
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
/// roots and manifest as the daemon.
fn load_state() -> Result<(MediaEngineBackend, MediaState)> {
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

    Ok((backend, state))
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
    pretty: bool,
}

impl CompileArgs {
    /// Renders locks and prints them. Nothing is written in the roots.
    fn run(self) -> Result<()> {
        let (_backend, state) = load_state()?;
        let path = expand_path(&self.path)
            .canonicalize()
            .unwrap_or_else(|_| expand_path(&self.path));
        let engine = open_engine(&state)?;
        let roots = roots_under(&state, &path, self.media_type.as_deref());

        if roots.is_empty() {
            bail!("no media root matches {}", path.display());
        }

        for root in roots {
            let lock =
                preview_root(&engine, &root.media_type, &root.dir, &state.cache_root)?;
            if self.pretty {
                println!("{lock}");
            } else {
                let value: Value = serde_json::from_str(&lock)?;
                println!("{}", serde_json::to_string(&value)?);
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
}

impl UpdateArgs {
    async fn run(self) -> Result<()> {
        let (backend, state) = load_state()?;
        let path = self.path.map(|p| {
            let expanded = expand_path(&p);
            expanded.canonicalize().unwrap_or(expanded)
        });
        let force = ForceMode::from(self.force);

        // A free lock means no daemon: take it for this pass so a daemon cannot
        // start mid-pass. A busy one means we ask the configured port.
        let lock = match InstanceLock::acquire() {
            Ok(lock) => Some(lock),
            Err(InstanceError::Busy { .. }) => None,
            Err(e) => return Err(e.into()),
        };

        let result = if lock.is_some() {
            datamile_media::update::run_local(
                &backend.shared(),
                path,
                self.media_type.as_deref(),
                force,
                self.jobs,
                &LogSink,
            )?
        } else {
            check_daemon(&state).await?;
            datamile_media::update::delegate(
                state.port,
                path,
                self.media_type,
                force,
                self.jobs,
                &LogSink,
            )
            .await?
        };

        drop(lock);
        finish(result)
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
        let (backend, state) = load_state()?;
        let _ = self.debug;

        let query = ActionTargetQuery {
            media_type: self.media_type,
            playing: self.playing,
            id: self.id,
            query: self.query,
            directory: self.directory,
            recursive: self.recursive,
            library: self.library,
            options: self
                .args
                .iter()
                .map(|arg| (arg.clone(), String::new()))
                .collect(),
        };

        // Read-only: the selection never takes the instance lock.
        let items = match InstanceLock::holder().map_err(|e| anyhow::anyhow!("{e}"))? {
            None => {
                let mut logic =
                    LogicEngine::new(PathBuf::new(), LogicManifest::default());
                backend.reload_config_and_logic(&mut logic)?;
                backend.scan_library(&mut logic);

                let playing = if query.playing {
                    playing_file(&state).await?
                } else {
                    None
                };
                backend.resolve_action_targets(&query, &logic, playing.as_deref())
            }
            Some(_) => select_from_daemon(&state, &query).await?,
        };

        let payload = json!({
            "items": items,
            "options": query.options_string(),
            "isatty": std::io::stdin().is_terminal()
        });

        if self.name == "intermediary" {
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }

        let name = self.name.replace('-', "_");
        backend.execute_action(&name, &payload)
    }
}

/// Asks the live daemon for the items, after checking it is ours to ask.
async fn select_from_daemon(
    state: &MediaState,
    query: &ActionTargetQuery,
) -> Result<Vec<Value>> {
    check_daemon(state).await?;

    let response = local_client()
        .post(format!("http://127.0.0.1:{}/api/select", state.port))
        .json(query)
        .send()
        .await
        .context("Failed to reach the running daemon")?;

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("Daemon selection failed: {text}");
    }

    Ok(response.json().await?)
}

/// The identity match rule: same canonical config, same version, no reload
/// error. A mismatch is fatal; there is no local fallback.
async fn check_daemon(state: &MediaState) -> Result<()> {
    let identity = probe(state.port).await.with_context(|| {
        format!(
            "The instance lock is held but nothing answered on port {}. The holder is \
             another CLI, a starting daemon, or a hung daemon.",
            state.port
        )
    })?;

    identity
        .check(&state.config_path, CLI_VERSION)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

async fn probe(port: u16) -> Option<Identity> {
    let response = local_client()
        .get(format!("http://127.0.0.1:{port}/api/identity"))
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    response.json::<Identity>().await.ok()
}

/// Any failure in a pass exits non-zero, including under `--silent`.
fn finish(result: datamile_media::PassResult) -> Result<()> {
    if result.ok() {
        return Ok(());
    }

    std::process::exit(1)
}

/// The daemon first, one-shot MPD query as the fallback.
async fn playing_file(state: &MediaState) -> Result<Option<String>> {
    if let Ok(response) = local_client()
        .get(format!(
            "http://127.0.0.1:{}/api/playback/current",
            state.port
        ))
        .timeout(PROBE_TIMEOUT)
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

/// Every call here is loopback, so an `HTTP_PROXY` in the environment must not
/// route it.
fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_default()
}

fn init_logger(level: log::LevelFilter) {
    simple_logger::SimpleLogger::new()
        .with_level(level)
        .with_module_level("mpd_protocol", log::LevelFilter::Warn)
        .with_module_level("mpd_client", log::LevelFilter::Warn)
        .with_module_level("tracing", log::LevelFilter::Warn)
        .env()
        .init()
        .ok();
}

#[tokio::main]
async fn main() -> Result<()> {
    // The filter is chosen after parsing, because `-v` and `--silent` are flags.
    let cli = Cli::parse();
    let level = if cli.silent {
        log::LevelFilter::Off
    } else if cli.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    init_logger(level);

    if let Some(dir) = cli.config {
        datamile_lua_domain::set_config_dir(dir);
    }

    cli.command.execute().await
}
