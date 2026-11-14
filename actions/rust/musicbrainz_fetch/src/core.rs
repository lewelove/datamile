use anyhow::Result;

use crate::client::MusicBrainzClient;
use crate::models::{FetchPlan, OverwriteMode, TargetKind};
use crate::writer::write_output;

pub async fn execute(plan: FetchPlan) -> Result<()> {
    if let Some(ref path) = plan.output_path
        && plan.overwrite == OverwriteMode::Preserve
        && path.exists()
    {
        return Ok(());
    }

    let client = MusicBrainzClient::new()?;

    let json_val = match plan.target.kind {
        TargetKind::Release => {
            client
                .fetch_release(&plan.target.mbid, plan.retry_count)
                .await?
        }
        TargetKind::ReleaseGroup => {
            client
                .fetch_release_group(&plan.target.mbid, plan.retry_count)
                .await?
        }
        TargetKind::AllReleases => {
            client
                .browse_all_releases(&plan.target.mbid, plan.retry_count)
                .await?
        }
    };

    write_output(plan.output_path.as_deref(), &json_val)?;

    Ok(())
}
