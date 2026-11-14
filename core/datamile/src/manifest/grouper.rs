use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use libdatamile::harvest::is_audio_file;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorValidation {
    Validate,
    Skip,
}

pub type GroupedTracks =
    HashMap<Vec<String>, Vec<(PathBuf, serde_json::Map<String, Value>)>>;

pub fn normalize_tag(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(arr)) => serde_json::to_string(arr).unwrap_or_default(),
        Some(v) => v.to_string().replace('"', "").trim().to_string(),
        None => String::new(),
    }
}

fn parse_sort_int(value: Option<&Value>) -> u32 {
    let s = normalize_tag(value);
    let s = s.split('/').next().unwrap_or("0").trim();
    s.parse().unwrap_or(0)
}

pub fn group_tracks(
    tracks: Vec<(PathBuf, serde_json::Map<String, Value>)>,
    keys: &[String],
) -> GroupedTracks {
    let mut buckets: GroupedTracks = HashMap::new();

    for (path, mut track) in tracks {
        let group_key: Vec<String> =
            keys.iter().map(|k| normalize_tag(track.get(k))).collect();
        track.insert(
            "track_path_absolute".to_string(),
            Value::String(path.to_string_lossy().to_string()),
        );
        buckets.entry(group_key).or_default().push((path, track));
    }

    buckets
}

pub fn sort_album_tracks(tracks: &mut [(PathBuf, serde_json::Map<String, Value>)]) {
    tracks.sort_by(|(p_a, t_a), (p_b, t_b)| {
        let disc_a = parse_sort_int(t_a.get("discnumber"));
        let disc_b = parse_sort_int(t_b.get("discnumber"));
        if disc_a != disc_b {
            return disc_a.cmp(&disc_b);
        }

        let trk_a = parse_sort_int(t_a.get("tracknumber"));
        let trk_b = parse_sort_int(t_b.get("tracknumber"));
        if trk_a != trk_b {
            return trk_a.cmp(&trk_b);
        }

        alphanumeric_sort::compare_path(p_a, p_b)
    });
}

pub fn resolve_anchor<S: AsRef<str>>(
    tracks: &[(PathBuf, serde_json::Map<String, Value>)],
    validation: AnchorValidation,
    supported_exts: &[S],
) -> (Option<PathBuf>, bool) {
    if tracks.is_empty() {
        return (None, false);
    }

    let mut paths = Vec::new();
    let mut group_paths_set = HashSet::new();

    for (p, _) in tracks {
        paths.push(p.clone());
        group_paths_set.insert(p.clone());
    }

    let mut anchor = paths[0].parent().unwrap_or(&paths[0]).to_path_buf();
    for p in &paths[1..] {
        while !p.starts_with(&anchor) {
            if let Some(parent) = anchor.parent() {
                anchor = parent.to_path_buf();
            } else {
                break;
            }
        }
    }

    if validation == AnchorValidation::Skip {
        return (Some(anchor), true);
    }

    let mut valid = true;
    for entry in walkdir::WalkDir::new(&anchor)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path().to_path_buf();
        if is_audio_file(&p, supported_exts) && !group_paths_set.contains(&p) {
            log::warn!(
                "Exclusivity Violation: {}\nCollision from: {}",
                anchor.display(),
                p.display()
            );
            valid = false;
            break;
        }
    }

    (Some(anchor), valid)
}
