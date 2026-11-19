mod client;
mod core;
mod models;
mod writer;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use models::{
    EntityType, FetchPlan, FetchTarget, OverwriteMode, TargetKind, extract_mbid,
    is_valid_uuid, parse_url,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Fetch raw JSON responses from MusicBrainz")]
struct Cli {
    #[arg(long = "url", group = "input", value_name = "URL")]
    url: Option<String>,

    #[arg(long = "release-id", group = "input", value_name = "MBID")]
    release_id: Option<String>,

    #[arg(long = "release-group-id", group = "input", value_name = "MBID")]
    release_group_id: Option<String>,

    #[arg(short = 't', long = "target", value_enum)]
    target: Option<TargetKind>,

    #[arg(short = 'o', long = "output", value_name = "PATH")]
    output: Option<PathBuf>,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(long = "retry", default_value_t = 0, value_name = "COUNT")]
    retry: usize,
}

fn resolve_entity(cli: &Cli) -> Result<(String, EntityType)> {
    if let Some(url_str) = &cli.url {
        return parse_url(url_str).context("Invalid or unsupported MusicBrainz URL");
    }

    if let Some(rel_id) = &cli.release_id {
        let id = extract_mbid(rel_id).context("Invalid release MBID")?;
        return Ok((id, EntityType::Release));
    }

    if let Some(rg_id) = &cli.release_group_id {
        let id = extract_mbid(rg_id).context("Invalid release-group MBID")?;
        return Ok((id, EntityType::ReleaseGroup));
    }

    anyhow::bail!(
        "Missing required input: specify --url, --release-id, or --release-group-id"
    )
}

fn resolve_target(cli: &Cli) -> Result<FetchTarget> {
    let (mbid, entity) = resolve_entity(cli)?;

    if !is_valid_uuid(&mbid) {
        anyhow::bail!("MBID '{mbid}' is not a valid UUID");
    }

    let kind = match (entity, cli.target) {
        (EntityType::Release, None | Some(TargetKind::Release)) => TargetKind::Release,
        (EntityType::Release, Some(other)) => {
            anyhow::bail!("Target '{other:?}' is invalid for a release")
        }
        (EntityType::ReleaseGroup, None | Some(TargetKind::ReleaseGroup)) => {
            TargetKind::ReleaseGroup
        }
        (EntityType::ReleaseGroup, Some(TargetKind::AllReleases)) => {
            TargetKind::AllReleases
        }
        (EntityType::ReleaseGroup, Some(TargetKind::Release)) => {
            anyhow::bail!("Target 'release' is invalid for a release-group")
        }
    };

    Ok(FetchTarget { mbid, kind })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let target = resolve_target(&cli)?;

    let output_path = cli.output.map(|p| {
        let p_str = p.to_string_lossy();
        if p_str.starts_with('~') {
            libactions::paths::expand_path(&p_str)
        } else {
            p
        }
    });

    let plan = FetchPlan {
        target,
        output_path,
        overwrite: OverwriteMode::from(cli.force),
        retry_count: cli.retry,
    };

    core::execute(plan).await?;

    Ok(())
}
