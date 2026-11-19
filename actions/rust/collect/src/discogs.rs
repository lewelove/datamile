use anyhow::Result;
pub use libactions::discogs::{TargetUrl, parse_discogs_url};
use libactions::discogs::{
    fetch_discogs_master, fetch_discogs_release, format_artist_credits,
    parse_discogs_position,
};

use crate::models::{AlbumData, Track};

pub async fn execute_discogs(target: TargetUrl, data: &mut AlbumData) -> Result<()> {
    match target {
        TargetUrl::DiscogsMaster(id) => {
            fetch_and_fill_discogs_master(id, data).await?;
        }
        TargetUrl::DiscogsRelease(id) => {
            let release = fetch_discogs_release(id).await?;
            if let Some(master_id) = release.master_id {
                fetch_and_fill_discogs_master(master_id, data).await?;
            } else {
                fill_from_discogs_release(release, data);
            }
        }
    }
    Ok(())
}

async fn fetch_and_fill_discogs_master(id: u64, data: &mut AlbumData) -> Result<()> {
    let master = fetch_discogs_master(id).await?;
    data.album = master.title;
    data.date = master.year.map_or_else(String::new, |y| y.to_string());

    if let Some(artists) = master.artists {
        data.albumartist = format_artist_credits(&artists);
    }

    let mut tracks = Vec::new();
    if let Some(tracklist) = master.tracklist {
        let mut d_counter = 1;
        let mut t_counter = 0;
        for t in tracklist {
            if let Some(pos) = t.position {
                if pos.is_empty() {
                    continue;
                }
                let (disc, track) =
                    parse_discogs_position(&pos, &mut d_counter, &mut t_counter);
                let mut track_artist = None;
                if let Some(artists_val) = t.extra.get("artists")
                    && let Ok(artists) = serde_json::from_value::<
                        Vec<discogs_rs::ArtistCredit>,
                    >(artists_val.clone())
                {
                    let parsed_art = format_artist_credits(&artists);
                    if !parsed_art.is_empty() {
                        track_artist = Some(parsed_art);
                    }
                }
                tracks.push(Track {
                    discnumber: disc,
                    tracknumber: track,
                    title: t.title,
                    artist: track_artist,
                });
            }
        }
    }
    data.tracks = tracks;

    Ok(())
}

fn fill_from_discogs_release(release: discogs_rs::Release, data: &mut AlbumData) {
    data.album = release.title;
    data.date = release.year.map_or_else(String::new, |y| y.to_string());

    if let Some(artists) = release.artists {
        data.albumartist = format_artist_credits(&artists);
    }

    let mut tracks = Vec::new();
    if let Some(tracklist) = release.tracklist {
        let mut d_counter = 1;
        let mut t_counter = 0;
        for t in tracklist {
            if let Some(pos) = t.position {
                if pos.is_empty() {
                    continue;
                }
                let (disc, track) =
                    parse_discogs_position(&pos, &mut d_counter, &mut t_counter);
                let mut track_artist = None;
                if let Some(artists_val) = t.extra.get("artists")
                    && let Ok(artists) = serde_json::from_value::<
                        Vec<discogs_rs::ArtistCredit>,
                    >(artists_val.clone())
                {
                    let parsed_art = format_artist_credits(&artists);
                    if !parsed_art.is_empty() {
                        track_artist = Some(parsed_art);
                    }
                }
                tracks.push(Track {
                    discnumber: disc,
                    tracknumber: track,
                    title: t.title,
                    artist: track_artist,
                });
            }
        }
    }
    data.tracks = tracks;
}
