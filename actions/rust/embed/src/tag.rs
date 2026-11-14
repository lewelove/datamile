use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lofty::config::{ParseOptions, WriteOptions};
use lofty::picture::{Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, Tag, TagType};
use xxhash_rust::xxh64::xxh64;

use crate::mapping::canonical_key;
use crate::models::{CoverDeleteMode, CoverStatus, DiskCover, TagDeleteMode, TrackTask};

const HASH_SEED: u64 = 0;

#[must_use]
pub fn hash(data: &[u8]) -> String {
    let h = xxh64(data, HASH_SEED).to_be_bytes();
    URL_SAFE_NO_PAD.encode(h)
}

pub fn read_file_tag(path: &Path) -> Result<(Tag, Option<String>)> {
    let tagged_file = Probe::open(path)
        .with_context(|| format!("Failed to open file: {}", path.display()))?
        .options(ParseOptions::new().read_cover_art(true))
        .guess_file_type()
        .with_context(|| format!("Failed to guess file type: {}", path.display()))?
        .read()
        .with_context(|| format!("Failed to read tags: {}", path.display()))?;

    let primary_type = tagged_file.primary_tag_type();
    let tag = tagged_file.primary_tag().cloned().unwrap_or_else(|| {
        let mut new_tag = Tag::new(primary_type);
        if let Some(first) = tagged_file.first_tag() {
            for item in first.items() {
                new_tag.insert(item.clone());
            }
            for pic in first.pictures() {
                new_tag.push_picture(pic.clone());
            }
        }
        new_tag
    });

    let cover_hash = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|p| hash(p.data()));

    Ok((tag, cover_hash))
}

pub fn extract_tag_map(tag: &Tag) -> HashMap<ItemKey, String> {
    let mut map = HashMap::new();

    for item in tag.items() {
        let key = canonical_key(item.key());
        let value = match item.value() {
            ItemValue::Text(text) | ItemValue::Locator(text) => text.trim(),
            ItemValue::Binary(_) => {
                continue;
            }
        };

        if !value.is_empty() {
            map.entry(key)
                .and_modify(|existing: &mut String| {
                    existing.push_str("; ");
                    existing.push_str(value);
                })
                .or_insert_with(|| value.to_string());
        }
    }

    map
}

pub fn read_disk_cover(cover_path: Option<&Path>) -> Result<Option<DiskCover>> {
    let Some(path) = cover_path else {
        return Ok(None);
    };
    if !path.is_file() {
        anyhow::bail!("Cover image not found: {}", path.display());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read cover image: {}", path.display()))?;
    let hash = hash(&bytes);
    Ok(Some(DiskCover {
        path: path.to_path_buf(),
        hash,
    }))
}

pub fn resolve_cover_status(
    tasks: &[TrackTask],
    disk_cover: Option<&DiskCover>,
) -> Result<CoverStatus> {
    let Some(disk) = disk_cover else {
        return Ok(CoverStatus::Preserve);
    };

    for track in tasks {
        let (tag, embedded_hash) = read_file_tag(&track.path)?;
        if embedded_hash.as_deref() != Some(&disk.hash) {
            return Ok(CoverStatus::Update);
        }
        let has_front = tag
            .pictures()
            .iter()
            .any(|p| p.pic_type() == PictureType::CoverFront);
        if !has_front {
            return Ok(CoverStatus::Update);
        }
    }

    Ok(CoverStatus::Preserve)
}

fn apply_tags_and_cover(
    path: &Path,
    mut tag: Tag,
    target_tags: &HashMap<ItemKey, String>,
    new_picture: Option<&Picture>,
    delete_tags: TagDeleteMode,
    delete_covers: CoverDeleteMode,
) -> Result<()> {
    if delete_tags == TagDeleteMode::DeleteOther {
        let target_keys: HashSet<ItemKey> = target_tags.keys().copied().collect();
        tag.retain(|item| {
            let key = canonical_key(item.key());
            key == ItemKey::EncoderSoftware
                || key == ItemKey::EncodedBy
                || target_keys.contains(&key)
        });
    }

    for (k, v) in target_tags {
        let write_key = match (*k, tag.tag_type()) {
            (ItemKey::Lyrics, TagType::Id3v2) => ItemKey::UnsyncLyrics,
            (key, _) => key,
        };
        tag.insert_text(write_key, v.clone());
    }

    if let Some(pic) = new_picture {
        if delete_covers == CoverDeleteMode::DeleteOther {
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
        } else {
            tag.remove_picture_type(PictureType::CoverFront);
        }
        tag.push_picture(pic.clone());
    } else if delete_covers == CoverDeleteMode::DeleteOther {
        let mut i = 0;
        while i < tag.pictures().len() {
            if tag.pictures()[i].pic_type() == PictureType::CoverFront {
                i += 1;
            } else {
                tag.remove_picture(i);
            }
        }
    }

    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

pub fn write_tasks(
    tasks: &[TrackTask],
    cover_status: CoverStatus,
    disk_cover: Option<&DiskCover>,
    delete_tags: TagDeleteMode,
    delete_covers: CoverDeleteMode,
) -> Result<()> {
    let new_picture = if cover_status == CoverStatus::Update
        && let Some(cover) = disk_cover
    {
        let mut pic = Picture::from_reader(&mut File::open(&cover.path)?)?;
        pic.set_pic_type(PictureType::CoverFront);
        Some(pic)
    } else {
        None
    };

    for task in tasks {
        let (tag, _) = read_file_tag(&task.path)?;
        apply_tags_and_cover(
            &task.path,
            tag,
            &task.target_tags,
            new_picture.as_ref(),
            delete_tags,
            delete_covers,
        )?;
    }

    Ok(())
}
