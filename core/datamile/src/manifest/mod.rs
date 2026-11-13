pub mod compressor;
pub mod engine;
pub mod grouper;

use anyhow::{Context, Result};
use grouper::AnchorValidation;
use libdatamile::harvest::{SUPPORTED_AUDIO_EXTENSIONS, harvest_file, is_audio_file};
use libdatamile::utils::expand_path;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteMode {
    Force,
    Preserve,
}

pub enum ManifestMode {
    Album,
    Library,
}

pub struct ManifestOptions {
    pub mode: ManifestMode,
    pub force: bool,
    pub stdout: bool,
}

pub fn run(target_path: Option<&Path>, options: &ManifestOptions) -> Result<()> {
    let config =
        libdatamile::lua::ResolvedConfig::load().context("Failed to load config")?;
    let music_dir = expand_path(&config.app.storage.music_directory);

    config.app.manifest.audio_extensions.as_ref().map_or_else(
        || {
            execute_manifest_run(
                target_path,
                options,
                &music_dir,
                SUPPORTED_AUDIO_EXTENSIONS,
            )
        },
        |exts| execute_manifest_run(target_path, options, &music_dir, exts),
    )
}

fn execute_manifest_run<S: AsRef<str> + Sync>(
    target_path: Option<&Path>,
    options: &ManifestOptions,
    music_dir: &Path,
    supported_exts: &[S],
) -> Result<()> {
    let grouping_keys = vec!["albumartist".to_string(), "album".to_string()];

    let scan_root = match options.mode {
        ManifestMode::Library => {
            target_path.map_or_else(|| music_dir.to_path_buf(), Path::to_path_buf)
        }
        ManifestMode::Album => target_path.map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            Path::to_path_buf,
        ),
    };

    let overwrite = if options.force {
        OverwriteMode::Force
    } else {
        OverwriteMode::Preserve
    };

    let dirs_to_harvest = match options.mode {
        ManifestMode::Album => vec![scan_root],
        ManifestMode::Library => {
            find_harvestable_directories(&scan_root, overwrite, supported_exts)
        }
    };

    if dirs_to_harvest.is_empty() {
        if !options.stdout {
            log::info!("No new audio directories found.");
        }
        return Ok(());
    }

    let harvested = harvest_audio_files(dirs_to_harvest, supported_exts, options);
    if harvested.is_empty() {
        return Ok(());
    }

    let buckets = grouper::group_tracks(harvested, &grouping_keys);

    for (_group_id, tracks) in buckets {
        process_album_group(tracks, supported_exts, options)?;
    }

    Ok(())
}

fn process_album_group<S: AsRef<str>>(
    mut tracks: Vec<(PathBuf, serde_json::Map<String, serde_json::Value>)>,
    supported_exts: &[S],
    options: &ManifestOptions,
) -> Result<()> {
    let validation = match options.mode {
        ManifestMode::Album => AnchorValidation::Skip,
        ManifestMode::Library => AnchorValidation::Validate,
    };

    let (anchor_opt, is_valid) =
        grouper::resolve_anchor(&tracks, validation, supported_exts);
    if !is_valid {
        return Ok(());
    }

    let Some(anchor) = anchor_opt else {
        return Ok(());
    };

    grouper::sort_album_tracks(&mut tracks);
    let clean_tracks: Vec<_> = tracks
        .into_iter()
        .map(|(_, mut t)| {
            t.remove("track_path_absolute");
            t
        })
        .collect();

    let (album_pool, track_pools) = compressor::compress(clean_tracks);

    let toml_content = serialize_manifest(&album_pool, &track_pools);
    if toml_content.trim().is_empty() {
        return Ok(());
    }

    if options.stdout {
        println!("{toml_content}");
    } else {
        let out_path = anchor.join("album.toml");
        if !options.force && out_path.exists() {
            log::warn!(
                "Existing album.toml detected in {}. Use --force to overwrite.",
                anchor.display()
            );
            return Ok(());
        }

        fs::write(&out_path, &toml_content)?;
        log::info!("Generated album manifest: {}", out_path.display());
    }

    Ok(())
}

fn find_harvestable_directories<S: AsRef<str>>(
    scan_root: &Path,
    overwrite: OverwriteMode,
    supported_exts: &[S],
) -> Vec<PathBuf> {
    let mut dirs_to_harvest = Vec::new();
    let mut it = WalkDir::new(scan_root).follow_links(true).into_iter();

    while let Some(Ok(entry)) = it.next() {
        if entry.file_type().is_dir() {
            let path = entry.path();
            if overwrite == OverwriteMode::Preserve && path.join("album.toml").exists() {
                it.skip_current_dir();
                continue;
            }

            let has_audio = fs::read_dir(path).is_ok_and(|mut d| {
                d.any(|e| {
                    if let Ok(f) = e
                        && f.path().is_file()
                    {
                        return is_audio_file(&f.path(), supported_exts);
                    }
                    false
                })
            });

            if has_audio {
                dirs_to_harvest.push(path.to_path_buf());
            }
        }
    }
    dirs_to_harvest
}

fn harvest_audio_files<S: AsRef<str> + Sync>(
    dirs_to_harvest: Vec<PathBuf>,
    supported_exts: &[S],
    options: &ManifestOptions,
) -> Vec<(PathBuf, serde_json::Map<String, serde_json::Value>)> {
    let mut audio_files = Vec::new();
    for dir in dirs_to_harvest {
        for entry in WalkDir::new(&dir)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_file() && is_audio_file(path, supported_exts) {
                audio_files.push(path.to_path_buf());
            }
        }
    }

    if !options.stdout {
        log::info!("Harvesting {} new audio files...", audio_files.len());
    }

    audio_files
        .into_par_iter()
        .filter_map(|path| match harvest_file(&path) {
            Ok(data) => {
                let mut map = serde_json::Map::new();
                for (k, v) in data.tags {
                    map.insert(k, v);
                }
                Some((path, map))
            }
            Err(e) => {
                if !options.stdout {
                    log::warn!("Failed to harvest {}: {e}", path.display());
                }
                None
            }
        })
        .collect()
}

fn serialize_manifest(
    album_pool: &serde_json::Map<String, serde_json::Value>,
    track_pools: &[serde_json::Map<String, serde_json::Value>],
) -> String {
    let mut toml_content = String::new();
    let album_lines = engine::render_toml_block(album_pool);
    if !album_lines.is_empty() {
        toml_content.push_str("[album]\n");
        toml_content.push_str(&album_lines.join("\n"));
        toml_content.push_str("\n\n");
    }

    for tp in track_pools {
        let track_lines = engine::render_toml_block(tp);
        if !track_lines.is_empty() {
            toml_content.push_str("[[tracks]]\n");
            toml_content.push_str(&track_lines.join("\n"));
            toml_content.push_str("\n\n");
        }
    }

    toml_content
}
