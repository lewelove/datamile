use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context, Result};
use lofty::picture::PictureType;
use lofty::tag::ItemKey;

use crate::mapping::item_key_to_flag;
use crate::models::{CoverDeleteMode, CoverDiffDisplay, TagDeleteMode, TrackTask};
use crate::tag::{extract_tag_map, read_file_tag};

const TTY_PATH: &str = "/dev/tty";

#[must_use]
pub fn format_display_path(path: &Path) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(rel) = path.strip_prefix(&cwd) {
            return rel.display().to_string();
        }

        if let (Ok(cwd_canon), Ok(path_canon)) = (cwd.canonicalize(), path.canonicalize())
            && let Ok(rel) = path_canon.strip_prefix(&cwd_canon)
        {
            return rel.display().to_string();
        }
    }

    path.display().to_string()
}

pub fn evaluate_track_diffs(
    task: &mut TrackTask,
    delete_tags: TagDeleteMode,
    delete_covers: CoverDeleteMode,
) -> Result<bool> {
    let (tag, _) = read_file_tag(&task.path)?;
    let audio_tags = extract_tag_map(&tag);
    let target_keys: HashSet<ItemKey> = task.target_tags.keys().copied().collect();
    let mut has_changes = false;
    let mut diffs = Vec::new();

    if delete_tags == TagDeleteMode::DeleteOther {
        let mut old_keys: Vec<ItemKey> = audio_tags.keys().copied().collect();
        old_keys.sort_by_key(|k| item_key_to_flag(*k));

        for old_key in old_keys {
            if old_key == ItemKey::EncoderSoftware || old_key == ItemKey::EncodedBy {
                continue;
            }

            if !target_keys.contains(&old_key) {
                has_changes = true;
                let old_val = &audio_tags[&old_key];
                let flag_name = item_key_to_flag(old_key);
                diffs.push(format!("\x1b[31m- {flag_name}: \x1b[90m{old_val}\x1b[0m"));
            }
        }
    }

    let mut target_keys_sorted: Vec<ItemKey> = task.target_tags.keys().copied().collect();
    target_keys_sorted.sort_by_key(|k| item_key_to_flag(*k));

    for key in target_keys_sorted {
        let new_val = &task.target_tags[&key];
        let old_val = audio_tags.get(&key).map_or("", String::as_str);
        if old_val == new_val {
            continue;
        }

        has_changes = true;
        let flag_name = item_key_to_flag(key);
        if old_val.is_empty() {
            diffs.push(format!("\x1b[32m+ {flag_name}: \x1b[90m{new_val}\x1b[0m"));
        } else {
            diffs.push(format!(
                "\x1b[34m~ {flag_name}: \x1b[90m{old_val} -> {new_val}\x1b[0m"
            ));
        }
    }

    if delete_covers == CoverDeleteMode::DeleteOther {
        let has_other = tag
            .pictures()
            .iter()
            .any(|p| p.pic_type() != PictureType::CoverFront);
        if has_other {
            has_changes = true;
            diffs.push("\x1b[31m- Other Embedded Covers\x1b[0m".to_string());
        }
    }

    task.diffs = diffs;
    Ok(has_changes)
}

#[must_use]
pub fn resolve_header(tasks: &[TrackTask]) -> String {
    for task in tasks {
        let artist = task
            .target_tags
            .get(&ItemKey::AlbumArtist)
            .or_else(|| task.target_tags.get(&ItemKey::TrackArtist));
        let album = task.target_tags.get(&ItemKey::AlbumTitle);

        if let (Some(art), Some(alb)) = (artist, album)
            && !art.is_empty()
            && !alb.is_empty()
        {
            return format!("{art} - {alb}");
        }
    }

    for task in tasks {
        if let Some(alb) = task.target_tags.get(&ItemKey::AlbumTitle)
            && !alb.is_empty()
        {
            return alb.clone();
        }
    }

    if let Some(first_track) = tasks.first()
        && let Some(parent) = first_track.path.parent()
        && let Some(name) = parent.file_name()
    {
        return name.to_string_lossy().to_string();
    }

    "Album".to_string()
}

pub fn dedup_common_diffs(tasks: &mut [TrackTask]) -> Vec<String> {
    let mut common = Vec::new();
    let active: Vec<&TrackTask> = tasks.iter().filter(|t| !t.diffs.is_empty()).collect();
    if active.len() <= 1 {
        return common;
    }

    for d in &active[0].diffs {
        if active.iter().all(|t| t.diffs.contains(d)) {
            common.push(d.clone());
        }
    }

    for t in tasks {
        t.diffs.retain(|d| !common.contains(d));
    }

    common
}

pub fn print_diffs(
    header: &str,
    tasks: &[TrackTask],
    common_diffs: &[String],
    show_cover: CoverDiffDisplay,
) {
    println!();
    println!("\x1b[1;36m{header}\x1b[0m");

    if show_cover == CoverDiffDisplay::Show {
        println!("\n\x1b[33m🖼️  Cover update required\x1b[0m");
    }

    if !common_diffs.is_empty() {
        println!("\n\x1b[1;34m💿 Album Diff\x1b[0m");
        for d in common_diffs {
            println!("   {d}");
        }
    }

    for task in tasks {
        if !task.diffs.is_empty() {
            let path_disp = format_display_path(&task.path);
            println!("\n\x1b[1m🎵 {path_disp}\x1b[0m");
            for d in &task.diffs {
                println!("   {d}");
            }
        }
    }
}

pub fn prompt_confirm() -> Result<bool> {
    print!("\n\x1b[1;35mApply changes? [y/N]: \x1b[0m");
    std::io::stdout().flush()?;

    let tty = File::open(TTY_PATH).context("Failed to open /dev/tty for user input")?;
    let mut reader = BufReader::new(tty);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let trimmed = line.trim().to_lowercase();
    Ok(trimmed == "y" || trimmed == "yes")
}
