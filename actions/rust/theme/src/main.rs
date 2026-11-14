mod kmeans;
mod kmeansn;
mod kmeansnd;
mod kmeansnh;
mod kmeansnv;
mod mean_shift;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use image::DynamicImage;
use image::imageops::FilterType;
use libactions::color::{calculate_palette_ratios, sort_palette};
use palette::Srgb;

const COVER_CANDIDATES: &[&str] = &[
    "cover.jpg",
    "cover.png",
    "folder.jpg",
    "front.jpg",
    "cover.jpeg",
    "front.png",
];

#[derive(Parser)]
#[command(author, version, about = "Extract color palettes from album artwork")]
struct Cli {
    #[arg(long)]
    cover: Option<PathBuf>,

    #[arg(long)]
    path: Option<PathBuf>,

    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(long, default_value = "kmeansnv")]
    algo: String,

    #[arg(long, default_value = "gradient")]
    sort: String,

    #[arg(long, default_value = "")]
    args: String,

    #[arg(long, default_value = "0.001")]
    threshold: f32,

    #[arg(long)]
    open_with: Option<String>,

    #[arg(long, short = 'f')]
    force: bool,
}

fn main() -> Result<()> {
    let args = Cli::parse();

    let cover_path = if let Some(cp) = args.cover {
        cp
    } else if let Some(ref p) = args.path {
        resolve_from_lock(p)
            .or_else(|| resolve_cover_candidate(p))
            .context("Could not find cover image in album directory")?
    } else {
        anyhow::bail!("Must specify either --cover or --path");
    };

    let out_path = if let Some(op) = args.output {
        op
    } else if let Some(ref p) = args.path {
        p.join("theme.toml")
    } else {
        cover_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("theme.toml")
    };

    if out_path.exists() && !args.force {
        return Ok(());
    }

    let img = image::open(&cover_path)
        .context(format!("Failed to open image at {}", cover_path.display()))?;

    let palette = process_image_to_palette(
        &img,
        &args.algo,
        &args.sort,
        &args.args,
        args.threshold,
    )
    .context("Failed to extract color palette")?;

    let hex_colors: Vec<String> = palette
        .into_iter()
        .map(|(srgb, _)| {
            let srgb_u8: Srgb<u8> = srgb.into_format();
            format!(
                "#{:02X}{:02X}{:02X}",
                srgb_u8.red, srgb_u8.green, srgb_u8.blue
            )
        })
        .collect();

    let formatted_bg = if hex_colors.is_empty() {
        String::new()
    } else {
        format!(
            "\n{}\n",
            hex_colors
                .iter()
                .map(|c| format!("  \"{c}\","))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let toml_content = format!("[album.colors]\n\nbackground = [{formatted_bg}]\n");
    std::fs::write(&out_path, toml_content)?;

    let abs_path = out_path.canonicalize().unwrap_or(out_path);
    println!("Created theme.toml at: {}", abs_path.display());

    if let Some(prog) = args.open_with {
        let cmd_str = format!("{prog} \"{}\"", abs_path.display());
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd_str)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    Ok(())
}

fn resolve_from_lock(album_dir: &Path) -> Option<PathBuf> {
    let lock_path = album_dir.join("album.lock.json");
    let content = std::fs::read_to_string(lock_path).ok()?;
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

fn resolve_cover_candidate(album_dir: &Path) -> Option<PathBuf> {
    for candidate in COVER_CANDIDATES {
        let p = album_dir.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn process_image_to_palette(
    img: &DynamicImage,
    algo_type: &str,
    sort_type: &str,
    args: &str,
    threshold_val: f32,
) -> Option<Vec<(Srgb, f32)>> {
    let sample_dim = args
        .split(',')
        .find(|s| s.trim().starts_with("dim="))
        .and_then(|s| s.trim().strip_prefix("dim="))
        .and_then(|val| val.parse::<u32>().ok())
        .unwrap_or(512);

    let img_to_process = if sample_dim == 0 {
        img.clone()
    } else {
        img.resize_exact(sample_dim, sample_dim, FilterType::Nearest)
    };

    let candidate_colors = match algo_type {
        "msc" => mean_shift::extract(&img_to_process, args),
        "kmeansn" => kmeansn::extract(&img_to_process, args),
        "kmeansnh" => kmeansnh::extract(&img_to_process, args),
        "kmeansnd" => kmeansnd::extract(&img_to_process, args),
        "kmeansnv" => kmeansnv::extract(&img_to_process, args),
        _ => kmeans::extract(&img_to_process, args),
    };

    if candidate_colors.is_empty() {
        return None;
    }

    let mut palette =
        calculate_palette_ratios(&img_to_process, candidate_colors, threshold_val);
    sort_palette(&mut palette, sort_type);

    Some(palette)
}
