mod compile;
mod harvest;
mod interface;
mod manifest;
mod query;
mod server;
mod update;
mod x;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use libdatamile::utils::expand_path;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    config: Option<String>,
    #[arg(
        long = "media-directory",
        alias = "media_directory",
        global = true,
        value_name = "DIR"
    )]
    media_directory: Option<String>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Harvest(HarvestArgs),
    Server(ServerArgs),
    Interface(InterfaceArgs),
    Compile(CompileArgs),
    Update(UpdateArgs),
    Manifest(ManifestArgs),
    X(XArgs),
    Query(QueryArgs),
}

impl Commands {
    async fn execute(self) -> Result<()> {
        match self {
            Self::Harvest(cmd) => cmd.run(),
            Self::Server(cmd) => cmd.run().await,
            Self::Interface(cmd) => cmd.run(),
            Self::Compile(cmd) => cmd.run().await,
            Self::Update(cmd) => cmd.run().await,
            Self::Manifest(cmd) => cmd.run(),
            Self::X(cmd) => cmd.run().await,
            Self::Query(cmd) => cmd.run().await,
        }
    }
}

#[derive(Args)]
struct HarvestArgs {
    #[arg(value_name = "PATHS", required = true, num_args = 1..)]
    paths: Vec<String>,
    #[arg(long)]
    pretty: bool,
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
}

impl HarvestArgs {
    fn run(self) -> Result<()> {
        if let Some(j) = self.jobs {
            rayon::ThreadPoolBuilder::new()
                .num_threads(j)
                .build_global()
                .context("Failed to build thread pool")?;
        }

        let mut targets = Vec::with_capacity(self.paths.len());
        for p in self.paths {
            let expanded = expand_path(&p);
            if let Ok(canon) = expanded.canonicalize() {
                targets.push(canon);
            } else {
                targets.push(expanded);
            }
        }

        let format = if self.pretty {
            harvest::FormatMode::Pretty
        } else {
            harvest::FormatMode::Compact
        };

        harvest::run(targets, format);
        Ok(())
    }
}

#[derive(Args)]
struct ServerArgs {
    #[arg(long, default_value = "8000")]
    port: u16,
}

impl ServerArgs {
    async fn run(self) -> Result<()> {
        server::run(self.port).await
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

#[derive(Args)]
struct CompileArgs {
    #[arg(value_name = "PATH", required = true)]
    path: String,
    #[arg(long)]
    stdout: bool,
    #[arg(long)]
    intermediary: bool,
    #[arg(long)]
    pretty: bool,
    #[arg(long, value_delimiter = ',')]
    flags: Vec<String>,
}

impl CompileArgs {
    async fn run(self) -> Result<()> {
        let expanded = expand_path(&self.path);
        let mode = if self.intermediary {
            compile::CompileMode::Intermediary
        } else {
            compile::CompileMode::Standard
        };
        let target = if self.stdout {
            compile::ExportTarget::Stdout
        } else {
            compile::ExportTarget::File
        };

        let options = compile::CompileOptions {
            target_path: expanded,
            flags: self.flags,
            specific_albums: None,
            jobs: None,
            compile_flags: compile::CompileFlags {
                mode,
                target,
                pretty: self.pretty,
            },
            ingest_tx: None,
            active_writes: None,
            verbosity: compile::LogVerbosity::Verbose,
        };
        let _ = compile::run(options).await?;
        Ok(())
    }
}

#[derive(Args)]
struct UpdateArgs {
    #[arg(value_name = "PATH")]
    path: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long, short = 'j')]
    jobs: Option<usize>,
    #[arg(long)]
    silent: bool,
}

impl UpdateArgs {
    async fn run(self) -> Result<()> {
        let expanded = self.path.map(|p| expand_path(&p));
        let force_mode = if self.force {
            update::client::ForceMode::Force
        } else {
            update::client::ForceMode::Preserve
        };
        let verbosity = if self.silent {
            compile::LogVerbosity::Silent
        } else {
            compile::LogVerbosity::Verbose
        };
        update::run(expanded, force_mode, self.jobs, verbosity).await
    }
}

#[derive(Args)]
struct ManifestArgs {
    #[arg(value_name = "PATH")]
    path: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long, required_unless_present = "library", conflicts_with = "library")]
    album: bool,
    #[arg(long, required_unless_present = "album", conflicts_with = "album")]
    library: bool,
    #[arg(long)]
    stdout: bool,
}

impl ManifestArgs {
    fn run(self) -> Result<()> {
        let expanded = self.path.map(|p| expand_path(&p));
        let mode = if self.album {
            manifest::ManifestMode::Album
        } else {
            manifest::ManifestMode::Library
        };
        let options = manifest::ManifestOptions {
            mode,
            force: self.force,
            stdout: self.stdout,
        };
        manifest::run(expanded.as_deref(), &options)
    }
}

#[derive(Args)]
struct XArgs {
    #[arg(value_name = "NAME", required = true)]
    name: String,
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
        let target = x::TargetFlags {
            playing: self.playing,
            id: self.id,
            query: self.query,
            directory: self.directory,
            recursive: self.recursive,
            library: self.library,
        };
        let debug = if self.debug {
            x::DebugMode::Enabled
        } else {
            x::DebugMode::Disabled
        };
        x::execute(self.name, target, debug, self.args).await
    }
}

#[derive(Args)]
struct QueryArgs {
    #[arg(value_name = "QUERY")]
    query_str: Option<String>,
    #[arg(long)]
    playing: bool,
    #[arg(long)]
    lock: bool,
    #[arg(long)]
    id: bool,
    #[arg(long)]
    json: bool,
}

impl QueryArgs {
    async fn run(self) -> Result<()> {
        let flags = query::QueryFlags {
            playing: self.playing,
            lock: self.lock,
            id: self.id,
            json: self.json,
        };
        query::run(self.query_str, flags).await
    }
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

    if let Some(config) = cli.config {
        let path = expand_path(&config);
        let init = if path.is_dir() {
            path.join("init.lua")
        } else {
            path
        };

        if !init.is_file() {
            return Err(anyhow::anyhow!(
                "config init.lua not found: {}",
                init.display()
            ));
        }

        libdatamile::lua::set_config_path_override(init);
    }

    if let Some(media_directory) = cli.media_directory {
        libdatamile::lua::set_media_directory_override(expand_path(&media_directory));
    }

    cli.command.execute().await
}
