use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::models::{AlbumData, Track};

pub enum MbTargetUrl {
    Release(String),
    ReleaseGroup(String),
}

#[must_use]
pub fn parse_musicbrainz_url(opts: &str) -> Option<MbTargetUrl> {
    let url = opts.trim();
    if let Some(id_str) = url.split("musicbrainz.org/release-group/").nth(1) {
        let id = extract_mbid(id_str)?;
        return Some(MbTargetUrl::ReleaseGroup(id));
    }
    if let Some(id_str) = url.split("musicbrainz.org/release/").nth(1) {
        let id = extract_mbid(id_str)?;
        return Some(MbTargetUrl::Release(id));
    }
    None
}

fn extract_mbid(s: &str) -> Option<String> {
    let clean = s
        .split('/')
        .next()
        .unwrap_or(s)
        .split('?')
        .next()
        .unwrap_or(s)
        .split('#')
        .next()
        .unwrap_or(s)
        .trim();
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

pub async fn execute_musicbrainz(
    target: MbTargetUrl,
    data: &mut AlbumData,
) -> Result<()> {
    let client = Client::builder()
        .user_agent("Datamile/0.1.0 https://github.com/lewelove/datamile")
        .build()
        .context("Failed to build HTTP client")?;

    match target {
        MbTargetUrl::Release(release_mbid) => {
            fetch_and_fill_release(&client, &release_mbid, data).await?;
        }
        MbTargetUrl::ReleaseGroup(rg_mbid) => {
            fetch_and_fill_release_group(&client, &rg_mbid, data).await?;
        }
    }

    Ok(())
}

async fn fetch_and_fill_release(
    client: &Client,
    release_mbid: &str,
    data: &mut AlbumData,
) -> Result<()> {
    let release_url = format!(
        "https://musicbrainz.org/ws/2/release/{release_mbid}?inc=recordings+artist-credits+labels+discids+isrcs+media+release-groups+genres+tags+ratings+aliases+annotation+url-rels&fmt=json"
    );
    let release_val = fetch_json(client, &release_url).await?;

    fill_from_mb_release(&release_val, data);

    if data.date.is_empty()
        && let Some(rg_id) = release_val
            .get("release-group")
            .and_then(|rg| rg.get("id"))
            .and_then(Value::as_str)
    {
        tokio::time::sleep(Duration::from_millis(1050)).await;

        let rg_url = format!(
            "https://musicbrainz.org/ws/2/release-group/{rg_id}?inc=artists+ratings+genres+tags+aliases+annotation+url-rels&fmt=json"
        );
        if let Ok(rg_val) = fetch_json(client, &rg_url).await
            && let Some(first_date) =
                rg_val.get("first-release-date").and_then(Value::as_str)
        {
            data.date = first_date.to_string();
        }
    }

    Ok(())
}

async fn fetch_and_fill_release_group(
    client: &Client,
    rg_mbid: &str,
    data: &mut AlbumData,
) -> Result<()> {
    let rg_url = format!(
        "https://musicbrainz.org/ws/2/release-group/{rg_mbid}?inc=artists+ratings+genres+tags+aliases+annotation+url-rels&fmt=json"
    );
    let rg_val = fetch_json(client, &rg_url).await?;

    data.album = rg_val
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    data.albumartist = format_mb_artist_credits(rg_val.get("artist-credit"));
    data.date = rg_val
        .get("first-release-date")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    tokio::time::sleep(Duration::from_millis(1050)).await;

    let all_releases = browse_all_releases(client, rg_mbid).await?;

    if let Some(releases_arr) = all_releases.as_array()
        && let Some(best_release) = select_best_release(releases_arr)
    {
        data.tracks =
            extract_tracks_from_media(best_release.get("media"), &data.albumartist);
        if data.date.is_empty()
            && let Some(d) = best_release.get("date").and_then(Value::as_str)
        {
            data.date = d.to_string();
        }
    }

    Ok(())
}

async fn browse_all_releases(client: &Client, rg_mbid: &str) -> Result<Value> {
    let mut all_releases = Vec::new();
    let mut offset = 0;
    let limit = 100;

    loop {
        let browse_url = format!(
            "https://musicbrainz.org/ws/2/release?release-group={rg_mbid}&inc=media+recordings+artist-credits+labels+discids+isrcs+release-groups+genres+tags+ratings+aliases&limit={limit}&offset={offset}&fmt=json"
        );

        let page_val = fetch_json(client, &browse_url).await?;
        let total_count = page_val
            .get("release-count")
            .and_then(Value::as_u64)
            .and_then(|c| usize::try_from(c).ok())
            .unwrap_or(0);

        let Some(releases) = page_val.get("releases").and_then(Value::as_array) else {
            break;
        };

        if releases.is_empty() {
            break;
        }

        let page_len = releases.len();
        all_releases.extend(releases.clone());
        offset += page_len;

        if offset >= total_count {
            break;
        }

        tokio::time::sleep(Duration::from_millis(1050)).await;
    }

    Ok(Value::Array(all_releases))
}

fn select_best_release(releases: &[Value]) -> Option<&Value> {
    let with_tracks: Vec<&Value> = releases
        .iter()
        .filter(|r| {
            r.get("media").and_then(Value::as_array).is_some_and(|m| {
                m.iter().any(|medium| {
                    medium
                        .get("tracks")
                        .and_then(Value::as_array)
                        .is_some_and(|t| !t.is_empty())
                })
            })
        })
        .collect();

    with_tracks
        .iter()
        .copied()
        .find(|r| r.get("status").and_then(Value::as_str) == Some("Official"))
        .or_else(|| with_tracks.into_iter().next())
        .or_else(|| releases.first())
}

fn fill_from_mb_release(release_val: &Value, data: &mut AlbumData) {
    data.album = release_val
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    data.albumartist = format_mb_artist_credits(release_val.get("artist-credit"));

    data.date = release_val
        .get("date")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    data.tracks = extract_tracks_from_media(release_val.get("media"), &data.albumartist);
}

fn extract_tracks_from_media(media_val: Option<&Value>, albumartist: &str) -> Vec<Track> {
    let mut tracks = Vec::new();
    let Some(media_arr) = media_val.and_then(Value::as_array) else {
        return tracks;
    };

    for (disc_idx, medium) in media_arr.iter().enumerate() {
        let fallback_disc = u32::try_from(disc_idx.saturating_add(1)).unwrap_or(u32::MAX);
        let disc_no = medium
            .get("position")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .unwrap_or(fallback_disc);

        let Some(track_arr) = medium.get("tracks").and_then(Value::as_array) else {
            continue;
        };

        for (track_idx, track) in track_arr.iter().enumerate() {
            let fallback_track =
                u32::try_from(track_idx.saturating_add(1)).unwrap_or(u32::MAX);
            let track_no = track
                .get("position")
                .and_then(Value::as_u64)
                .and_then(|p| u32::try_from(p).ok())
                .unwrap_or(fallback_track);

            let title = track
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            let artist_credit = format_mb_artist_credits(track.get("artist-credit"));
            let track_artist =
                if !artist_credit.is_empty() && artist_credit != albumartist {
                    Some(artist_credit)
                } else {
                    None
                };

            tracks.push(Track {
                discnumber: disc_no,
                tracknumber: track_no,
                title,
                artist: track_artist,
            });
        }
    }

    tracks
}

#[must_use]
pub fn format_mb_artist_credits(val: Option<&Value>) -> String {
    let Some(arr) = val.and_then(Value::as_array) else {
        return String::new();
    };

    let mut out = String::new();
    for item in arr {
        let name = item
            .get("name")
            .or_else(|| item.get("artist").and_then(|a| a.get("name")))
            .and_then(Value::as_str)
            .unwrap_or("");
        let join = item.get("joinphrase").and_then(Value::as_str).unwrap_or("");
        out.push_str(name);
        out.push_str(join);
    }
    out.trim().to_string()
}

async fn fetch_json(client: &Client, url: &str) -> Result<Value> {
    let resp = client
        .get(url)
        .send()
        .await
        .context(format!("Failed request to {url}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        anyhow::bail!("MusicBrainz request to {url} returned status {status}");
    }

    let val = resp.json().await.context("Failed to parse JSON response")?;
    Ok(val)
}
