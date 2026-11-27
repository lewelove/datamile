mod interface;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use datamile_fs::expand_path;
use datamile_logic::LogicEngine;
use datamile_lua_domain::{ActionCtx, DomainLuaEngine, LogicManifest};
use datamile_media::state::{COMPILE_ROOT_LUA, MEDIA_LUA};
use datamile_media::{
    ForceMode, LogSink, MediaEngineBackend, MediaState, open_engine, preview_root,
};
use datamile_server::instance::InstanceError;
use datamile_server::{Identity, InstanceLock, MediaBackend};
use serde::Deserialize;
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
            let base_url = daemon_base_url(&state.host, state.port);
            check_daemon(&base_url, &state.config_path).await?;
            datamile_media::update::delegate(
                &base_url,
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
    /// Canonical action name; `-` and `_` are the same name.
    #[arg(value_name = "NAME", required = true)]
    name: String,
    #[arg(long = "dir", value_name = "ENTITY_DIR")]
    dirs: Vec<String>,
    #[arg(long = "media", value_name = "MEDIA_TYPE")]
    media: Vec<String>,
    /// Everything after `--` reaches the action unchanged as `ctx.options`.
    #[arg(last = true, value_name = "OPTION")]
    options: Vec<String>,
}

/// One resolved target returned by `POST /api/x`.
#[derive(Deserialize)]
struct XRun {
    media: Option<String>,
    lock: Option<Value>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct XResponse {
    runs: Vec<XRun>,
}

impl XArgs {
    /// The daemon resolves targets because it owns the entity index; the run
    /// itself happens here so interactive actions keep the caller's terminal
    /// and working directory.
    async fn run(self) -> Result<()> {
        let name = self.name.replace('-', "_");
        let config_path = datamile_lua_domain::resolve_config_path()
            .context("Failed to resolve init.lua")?;

        // A Lua VM is not `Send`, so it cannot live across the awaits below:
        // one VM builds the request, a fresh one runs the action.
        let (base_url, dirs) = action_plan(&name, &config_path, &self.dirs)?;
        require_daemon(&base_url, &config_path).await?;
        let runs =
            fetch_runs(&base_url, &name, &dirs, &self.media, &self.options).await?;

        let engine = action_engine()?;
        engine.evaluate_config(&config_path)?;
        run_locally(&engine, &name, &runs, &self.options);

        Ok(())
    }
}

/// Evaluates `init.lua` only as far as the request needs it: where the daemon
/// answers, and the directories to resolve.
fn action_plan(
    name: &str,
    config_path: &Path,
    raw_dirs: &[String],
) -> Result<(String, Vec<String>)> {
    let engine = action_engine()?;
    let evaluated = engine.evaluate_config(config_path)?;
    let server = &evaluated.config.server;
    let base_url = daemon_base_url(&server.host, server.port);
    let declared = evaluated.actions.get(name).map(Vec::as_slice);

    Ok((base_url, resolve_action_dirs(raw_dirs, declared)))
}

/// The daemon binds `server.host`, so the CLI must call it there.
fn daemon_base_url(host: &str, port: u16) -> String {
    format!("http://{}", datamile_server::client_authority(host, port))
}

/// The same lightweight loader `datamile interface` uses: the media DSL and
/// native helpers, then `init.lua`, with no discovery and no library scan.
fn action_engine() -> Result<DomainLuaEngine> {
    let engine = DomainLuaEngine::new()?;
    engine.load_extension("media.lua", MEDIA_LUA)?;
    engine.load_extension("compile_root.lua", COMPILE_ROOT_LUA)?;
    datamile_media::lua_bridge::register_native_helpers(&engine.runtime)?;

    Ok(engine)
}

/// There is no offline resolver: without a live daemon the targets cannot be
/// resolved, so refuse before reading any directory.
///
/// Example: `datamile x open_terminal` with no daemon running exits non-zero
/// naming `datamile server`.
async fn require_daemon(base_url: &str, config_path: &Path) -> Result<()> {
    let holder = InstanceLock::holder().map_err(|e| anyhow::anyhow!("{e}"))?;
    if holder.is_none() {
        bail!("datamile daemon is not running; start it with 'datamile server'");
    }

    check_daemon(base_url, config_path).await
}

/// The daemon only accepts expanded absolute paths, so the CLI expands `~` and
/// resolves `.` here. A scoped action with no `--dir` targets the working
/// directory; a generic action runs targetless.
fn resolve_action_dirs(
    raw_dirs: &[String],
    declared_media: Option<&[String]>,
) -> Vec<String> {
    if !raw_dirs.is_empty() {
        return raw_dirs.iter().map(|dir| canonical_dir(dir)).collect();
    }

    let scoped = declared_media.is_some_and(|media| !media.is_empty());
    if !scoped {
        return Vec::new();
    }

    vec![canonical_dir(".")]
}

async fn fetch_runs(
    base_url: &str,
    name: &str,
    dirs: &[String],
    media: &[String],
    options: &[String],
) -> Result<Vec<XRun>> {
    let response = local_client()
        .post(format!("{base_url}/api/x"))
        .json(&json!({
            "name": name,
            "dirs": dirs,
            "media": media,
            "options": options,
            "execute": false,
        }))
        .send()
        .await
        .context("Failed to reach the running daemon")?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Action request failed: {text}");
    }

    let response: XResponse = serde_json::from_str(&text)?;

    Ok(response.runs)
}

/// Runs every resolved target in this process and lets the remaining targets
/// proceed when one fails.
fn run_locally(engine: &DomainLuaEngine, name: &str, runs: &[XRun], options: &[String]) {
    let isatty = std::io::stdin().is_terminal();
    let mut failed = false;

    for run in runs {
        if let Some(error) = &run.error {
            eprintln!("error: {error}");
            failed = true;
            continue;
        }

        let ctx = ActionCtx {
            lock: run.lock.clone(),
            media: run.media.clone(),
            options: options.to_vec(),
            isatty,
        };

        if let Err(error) = engine.execute_action(name, &ctx) {
            eprintln!("error: {error}");
            failed = true;
        }
    }

    if failed {
        std::process::exit(1);
    }
}

/// The daemon only accepts expanded absolute paths, so the CLI expands `~` and
/// resolves `.` against the shell's working directory first.
fn canonical_dir(raw: &str) -> String {
    let expanded = expand_path(raw);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&expanded))
            .unwrap_or(expanded)
    };

    absolute
        .canonicalize()
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned()
}

/// The identity match rule: same canonical config, same version, no reload
/// error. A mismatch is fatal; there is no local fallback.
async fn check_daemon(base_url: &str, config_path: &Path) -> Result<()> {
    let identity = probe(base_url).await.with_context(|| {
        format!(
            "The instance lock is held but nothing answered at {base_url}. The holder \
             is another CLI, a starting daemon, or a hung daemon."
        )
    })?;

    identity
        .check(config_path, CLI_VERSION)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

async fn probe(base_url: &str) -> Option<Identity> {
    let response = local_client()
        .get(format!("{base_url}/api/identity"))
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
