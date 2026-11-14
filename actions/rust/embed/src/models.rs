use std::collections::HashMap;
use std::path::PathBuf;

use lofty::tag::ItemKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDeleteMode {
    DeleteOther,
    PreserveOther,
}

impl From<bool> for TagDeleteMode {
    fn from(val: bool) -> Self {
        if val {
            Self::DeleteOther
        } else {
            Self::PreserveOther
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverDeleteMode {
    DeleteOther,
    PreserveOther,
}

impl From<bool> for CoverDeleteMode {
    fn from(val: bool) -> Self {
        if val {
            Self::DeleteOther
        } else {
            Self::PreserveOther
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoMode {
    Yes,
    Prompt,
}

impl From<bool> for AutoMode {
    fn from(val: bool) -> Self {
        if val { Self::Yes } else { Self::Prompt }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverStatus {
    Update,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverDiffDisplay {
    Show,
    Hide,
}

pub struct DiskCover {
    pub path: PathBuf,
    pub hash: String,
}

pub struct TrackTask {
    pub path: PathBuf,
    pub target_tags: HashMap<ItemKey, String>,
    pub diffs: Vec<String>,
}

pub struct CliOptions {
    pub cover: Option<PathBuf>,
    pub delete_other_tags: TagDeleteMode,
    pub delete_other_covers: CoverDeleteMode,
    pub auto_mode: AutoMode,
    pub tasks: Vec<TrackTask>,
}
