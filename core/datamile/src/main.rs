mod interface;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use datamile_album_legacy::{AlbumLegacyBackend, compile, update, x};
use datamile_fs::expand_path;

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
            Self::Compile(cmd) => cmd.run().await,
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
        let backend = Arc::new(AlbumLegacyBackend::new());
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
            update::ForceMode::Force
        } else {
            update::ForceMode::Preserve
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
