use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::mapping::{canonical_key, flag_to_item_key};
use crate::models::{AutoMode, CliOptions, CoverDeleteMode, TagDeleteMode, TrackTask};

pub fn parse_cli_args() -> Result<CliOptions> {
    let mut cover: Option<PathBuf> = None;
    let mut delete_other_tags = TagDeleteMode::PreserveOther;
    let mut delete_other_covers = CoverDeleteMode::PreserveOther;
    let mut auto_mode = AutoMode::Prompt;
    let mut tasks: Vec<TrackTask> = Vec::new();
    let mut current_idx: Option<usize> = None;

    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;

    while i < raw_args.len() {
        let arg = &raw_args[i];

        if arg == "-h" || arg == "--help" {
            println!("Usage: embed [OPTIONS] --track <FILE> [TAG_OPTIONS]...");
            std::process::exit(0);
        } else if arg == "-V" || arg == "--version" {
            println!("embed 0.1.0");
            std::process::exit(0);
        } else if arg == "-c" || arg == "--cover" {
            i += 1;
            if i < raw_args.len() {
                cover = Some(PathBuf::from(&raw_args[i]));
            }
        } else if let Some(path_str) = arg.strip_prefix("--cover=") {
            cover = Some(PathBuf::from(path_str));
        } else if let Some(path_str) = arg.strip_prefix("-c=") {
            cover = Some(PathBuf::from(path_str));
        } else if arg == "--delete-other-tags" {
            delete_other_tags = TagDeleteMode::DeleteOther;
        } else if arg == "--delete-other-covers" {
            delete_other_covers = CoverDeleteMode::DeleteOther;
        } else if arg == "-y" || arg == "--yes" {
            auto_mode = AutoMode::Yes;
        } else if arg == "--track" {
            i += 1;
            if i < raw_args.len() {
                tasks.push(TrackTask {
                    path: PathBuf::from(&raw_args[i]),
                    target_tags: HashMap::new(),
                    diffs: Vec::new(),
                });
                current_idx = Some(tasks.len().saturating_sub(1));
            }
        } else if let Some(path_str) = arg.strip_prefix("--track=") {
            tasks.push(TrackTask {
                path: PathBuf::from(path_str),
                target_tags: HashMap::new(),
                diffs: Vec::new(),
            });
            current_idx = Some(tasks.len().saturating_sub(1));
        } else if arg.starts_with("--") {
            let (flag_name, value) = if let Some((k, v)) = arg.split_once('=') {
                (k, v.to_string())
            } else {
                i += 1;
                if i < raw_args.len() {
                    (arg.as_str(), raw_args[i].clone())
                } else {
                    anyhow::bail!("Missing argument value for '{arg}'");
                }
            };

            let Some(item_key) = flag_to_item_key(flag_name) else {
                anyhow::bail!("Unknown option: '{flag_name}'");
            };

            let Some(idx) = current_idx else {
                anyhow::bail!("Option '{flag_name}' must follow a '--track' declaration");
            };

            tasks[idx]
                .target_tags
                .insert(canonical_key(item_key), value);
        } else {
            anyhow::bail!("Unexpected argument: '{arg}'");
        }

        i += 1;
    }

    if tasks.is_empty() {
        anyhow::bail!("No tracks specified. Use '--track <FILE>' to add track files.");
    }

    for task in &tasks {
        if !task.path.is_file() {
            anyhow::bail!("Track file not found: {}", task.path.display());
        }
    }

    Ok(CliOptions {
        cover,
        delete_other_tags,
        delete_other_covers,
        auto_mode,
        tasks,
    })
}
