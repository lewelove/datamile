use std::path::Path;

use anyhow::Result;
use discogs_rs::{ArtistCredit, Auth, DiscogsClient, Master, Release};

pub enum TargetUrl {
    DiscogsMaster(u64),
    DiscogsRelease(u64),
}

#[must_use]
pub fn parse_discogs_url(opts: &str) -> Option<TargetUrl> {
    let url = opts.trim();
    if let Some(id_str) = url.split("discogs.com/master/").nth(1) {
        let id = extract_discogs_id(id_str)?;
        return Some(TargetUrl::DiscogsMaster(id));
    }
    if let Some(id_str) = url.split("discogs.com/release/").nth(1) {
        let id = extract_discogs_id(id_str)?;
        return Some(TargetUrl::DiscogsRelease(id));
    }
    None
}

fn extract_discogs_id(s: &str) -> Option<u64> {
    let clean = s
        .split('/')
        .next()
        .unwrap_or(s)
        .split('?')
        .next()
        .unwrap_or(s)
        .split('-')
        .next()
        .unwrap_or(s);
    clean.parse::<u64>().ok()
}

pub fn build_client() -> Result<DiscogsClient> {
    let token = std::env::var("DISCOGS_TOKEN").unwrap_or_default();
    let mut builder = DiscogsClient::with_default_user_agent();
    if !token.is_empty() {
        builder = builder.auth(Auth::UserToken { token });
    }
    Ok(builder.build()?)
}

pub async fn fetch_discogs_master(id: u64) -> Result<Master> {
    let client = build_client()?;
    let res = client.database().get_master(id).await?;
    Ok(res.data)
}

pub async fn fetch_discogs_release(id: u64) -> Result<Release> {
    let client = build_client()?;
    let res = client.database().get_release(id, None).await?;
    Ok(res.data)
}

pub async fn download_discogs_cover(url: &str, dest: &Path) -> Result<()> {
    let token = std::env::var("DISCOGS_TOKEN").unwrap_or_default();
    let client = reqwest::Client::builder()
        .user_agent("Datamile/0.1.0")
        .build()?;
    let mut req = client.get(url);
    if !token.is_empty() {
        req = req.header("Authorization", format!("Discogs token={token}"));
    }
    let res = req.send().await?;
    if !res.status().is_success() {
        anyhow::bail!("Failed to download cover: {}", res.status());
    }
    let bytes = res.bytes().await?;
    std::fs::write(dest, bytes)?;
    Ok(())
}

#[must_use]
pub fn format_artist_credits(artists: &[ArtistCredit]) -> String {
    let mut out = String::new();
    for artist in artists {
        let name = clean_artist_name(&artist.name);
        let join = artist.join.as_deref().unwrap_or("");
        out.push_str(&name);
        if join.is_empty() {
            out.push_str(", ");
        } else {
            out.push_str(join);
        }
    }
    out.trim_end_matches(", ").to_string()
}

fn clean_artist_name(name: &str) -> String {
    let re = regex::Regex::new(r" \(\d+\)$").expect("artist suffix regex is valid");
    re.replace(name, "").to_string()
}

pub fn parse_discogs_position(
    pos: &str,
    disc_counter: &mut u32,
    track_counter: &mut u32,
) -> (u32, u32) {
    if pos.contains('-') {
        let parts: Vec<&str> = pos.split('-').collect();
        let d = parts[0].parse().unwrap_or(1);
        let t = parts[1].parse().unwrap_or(1);
        *disc_counter = d;
        *track_counter = t;
        (d, t)
    } else if let Ok(t) = pos.parse::<u32>() {
        *track_counter = t;
        (*disc_counter, t)
    } else {
        *track_counter += 1;
        (*disc_counter, *track_counter)
    }
}
