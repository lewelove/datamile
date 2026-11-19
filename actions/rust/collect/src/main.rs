mod discogs;
mod fs;
mod models;
mod musicbrainz;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use models::{AlbumData, FormattingConfig};

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Scrape metadata from Discogs/MusicBrainz and create album structure"
)]
struct Args {
    #[arg(long, required = true)]
    url: String,

    #[arg(long, required = true)]
    root: PathBuf,

    #[arg(long, default_value = "{albumartist} - {album}")]
    format_album: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let target_url = args.url.trim();
    let mut album_data = AlbumData::default();

    if let Some(mb_target) = musicbrainz::parse_musicbrainz_url(target_url) {
        musicbrainz::execute_musicbrainz(mb_target, &mut album_data).await?;
    } else if let Some(discogs_target) = discogs::parse_discogs_url(target_url) {
        discogs::execute_discogs(discogs_target, &mut album_data).await?;
    } else {
        anyhow::bail!("Invalid or unsupported URL provided: '{target_url}'");
    }

    if album_data.albumartist.is_empty() || album_data.album.is_empty() {
        anyhow::bail!("Failed to fetch sufficient album data");
    }

    let formatting = FormattingConfig {
        album: args.format_album,
    };

    let root_expanded = if args.root.starts_with("~") {
        libactions::paths::expand_path(&args.root.to_string_lossy())
    } else {
        args.root
    };

    fs::create_album_directory(&album_data, &formatting, &root_expanded)?;

    Ok(())
}
