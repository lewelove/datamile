use std::path::PathBuf;

use clap::ValueEnum;

const UUID_LENGTH: usize = 36;
const HYPHEN_INDICES: [usize; 4] = [8, 13, 18, 23];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityType {
    Release,
    ReleaseGroup,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    #[value(name = "release")]
    Release,
    #[value(name = "release-group")]
    ReleaseGroup,
    #[value(name = "all-releases")]
    AllReleases,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteMode {
    Force,
    Preserve,
}

impl From<bool> for OverwriteMode {
    fn from(force: bool) -> Self {
        if force { Self::Force } else { Self::Preserve }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTarget {
    pub mbid: String,
    pub kind: TargetKind,
}

#[derive(Debug, Clone)]
pub struct FetchPlan {
    pub target: FetchTarget,
    pub output_path: Option<PathBuf>,
    pub overwrite: OverwriteMode,
    pub retry_count: usize,
}

#[must_use]
pub fn is_valid_uuid(s: &str) -> bool {
    if s.len() != UUID_LENGTH {
        return false;
    }

    for (idx, byte) in s.bytes().enumerate() {
        if HYPHEN_INDICES.contains(&idx) {
            if byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }

    true
}

#[must_use]
pub fn extract_mbid(raw: &str) -> Option<String> {
    let clean = raw
        .split('/')
        .next()
        .unwrap_or(raw)
        .split('?')
        .next()
        .unwrap_or(raw)
        .split('#')
        .next()
        .unwrap_or(raw)
        .trim();

    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

#[must_use]
pub fn parse_url(input: &str) -> Option<(String, EntityType)> {
    let trimmed = input.trim();

    if let Some(id_str) = trimmed.split("musicbrainz.org/release-group/").nth(1) {
        let id = extract_mbid(id_str)?;
        return Some((id, EntityType::ReleaseGroup));
    }

    if let Some(id_str) = trimmed.split("musicbrainz.org/release/").nth(1) {
        let id = extract_mbid(id_str)?;
        return Some((id, EntityType::Release));
    }

    None
}
