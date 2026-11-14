mod metrics;
mod models;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use clap::Parser;
use models::{CoverFileInfo, CoverMetricsDoc};
use tempfile::NamedTempFile;

const COVER_CANDIDATES: &[&str] = &[
    "cover.jpg",
    "cover.png",
    "folder.jpg",
    "front.jpg",
    "cover.jpeg",
    "front.png",
];

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Calculate cover image entropy and chroma metrics"
)]
struct Args {
    #[arg(long, required = true)]
    path: PathBuf,

    #[arg(long)]
    cover: Option<PathBuf>,

    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(long, short = 'f')]
    force: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let album_dir = args.path.canonicalize().context("Invalid album path")?;

    let info_dir = album_dir.join("Info");
    let metrics_file = args
        .output
        .unwrap_or_else(|| info_dir.join("cover_metrics.json"));

    if !args.force && verify_existing_metrics(&metrics_file, &info_dir) {
        return Ok(());
    }

    let cover_path = args
        .cover
        .or_else(|| resolve_from_lock(&album_dir))
        .or_else(|| resolve_cover_candidate(&album_dir))
        .context("No cover image found for album")?;

    let cover_meta = fs::metadata(&cover_path)?;
    let current_mtime = cover_meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let current_size = cover_meta.len();

    let bytes = fs::read(&cover_path)?;
    let raw_hash = blake3::hash(&bytes);
    let hash_str = format!("blake3-{}", STANDARD.encode(raw_hash.as_bytes()));

    let img = image::load_from_memory(&bytes).context("Failed to decode cover image")?;

    let chroma = metrics::calculate_chroma(&img);
    let entropy = metrics::calculate_entropy(&img);

    let rel_path_for_doc = relative_path_from(&cover_path, &info_dir);

    let doc = CoverMetricsDoc {
        cover: CoverFileInfo {
            path: rel_path_for_doc,
            mtime: current_mtime,
            byte_size: current_size,
            hash: hash_str,
        },
        chroma,
        entropy,
    };

    let json_str = serde_json::to_string_pretty(&doc)?;

    if let Some(parent) = metrics_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let parent_dir = metrics_file.parent().unwrap_or(&info_dir);
    let mut temp = NamedTempFile::new_in(parent_dir)?;
    temp.write_all(json_str.as_bytes())?;
    temp.write_all(b"\n")?;
    temp.persist(&metrics_file)?;

    println!(
        "\x1b[32m✔\x1b[0m Calculated cover metrics for: {}",
        album_dir.display()
    );

    Ok(())
}

fn resolve_from_lock(album_dir: &Path) -> Option<PathBuf> {
    let lock_path = album_dir.join("album.lock.json");
    let content = fs::read_to_string(lock_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let path_str = val
        .pointer("/album/images/main/0/source/file/path")
        .and_then(serde_json::Value::as_str)?;
    let target = if Path::new(path_str).is_absolute() {
        PathBuf::from(path_str)
    } else {
        album_dir.join(path_str)
    };
    if target.is_file() { Some(target) } else { None }
}

fn verify_existing_metrics(metrics_file: &Path, info_dir: &Path) -> bool {
    if !metrics_file.exists() {
        return false;
    }

    let Ok(content) = fs::read_to_string(metrics_file) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<CoverMetricsDoc>(&content) else {
        return false;
    };

    let resolved_cover = info_dir.join(&doc.cover.path);
    let Ok(meta) = fs::metadata(&resolved_cover) else {
        return false;
    };

    let mtime = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let size = meta.len();

    doc.cover.mtime == mtime && doc.cover.byte_size == size
}

fn relative_path_from(target: &Path, base_dir: &Path) -> String {
    if let Ok(rel) = target.strip_prefix(base_dir) {
        return rel.to_string_lossy().to_string();
    }
    if let Some(parent) = base_dir.parent()
        && let Ok(rel) = target.strip_prefix(parent)
    {
        return format!("../{}", rel.to_string_lossy());
    }
    target.to_string_lossy().to_string()
}

fn resolve_cover_candidate(album_dir: &Path) -> Option<PathBuf> {
    for candidate in COVER_CANDIDATES {
        let p = album_dir.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}
