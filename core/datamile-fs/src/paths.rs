use std::path::{Path, PathBuf};

#[must_use]
pub fn expand_path(path_str: &str) -> PathBuf {
    if path_str.starts_with('~')
        && let Some(home) = dirs::home_dir()
    {
        if path_str == "~" {
            return home;
        }
        if let Some(stripped) = path_str.strip_prefix("~/") {
            return home.join(stripped);
        }
    }
    PathBuf::from(path_str)
}

#[must_use]
pub fn resolve_path(path_str: &str, config_dir: &Path) -> PathBuf {
    let expanded = expand_path(path_str);
    if expanded.is_absolute() {
        return expanded;
    }

    let direct = config_dir.join(&expanded);
    if direct.exists() {
        return direct;
    }

    if let Ok(entries) = std::fs::read_dir(config_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                let sub_candidate = entry.path().join(&expanded);
                if sub_candidate.exists() {
                    return sub_candidate;
                }
            }
        }
    }

    direct
}

/// Where a referenced file ended up, relative to the base it was resolved
/// against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedFileLocation {
    InsideBase {
        canonical: PathBuf,
        rel_path: String,
    },
    OutsideBase {
        canonical: PathBuf,
        abs_path: String,
    },
}

/// Resolves a possibly relative file reference against `base` and reports
/// whether it stayed inside `base_canon`.
///
/// Returns `None` when the target cannot be canonicalized or is not a file, so
/// callers can skip broken references instead of failing the whole pass.
#[must_use]
pub fn resolve_file_location(
    raw_path: &str,
    base: &Path,
    base_canon: &Path,
) -> Option<ResolvedFileLocation> {
    let raw = Path::new(raw_path);
    let target = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    };

    let canonical = target.canonicalize().ok()?;
    if !canonical.is_file() {
        return None;
    }

    Some(match canonical.strip_prefix(base_canon) {
        Ok(rel) => ResolvedFileLocation::InsideBase {
            rel_path: rel.to_string_lossy().to_string(),
            canonical,
        },
        Err(_) => ResolvedFileLocation::OutsideBase {
            abs_path: canonical.to_string_lossy().to_string(),
            canonical,
        },
    })
}

#[must_use]
pub fn rel_path(target: &Path, base: &Path) -> String {
    target.strip_prefix(base).map_or_else(
        |_| target.to_string_lossy().to_string(),
        |p| p.to_string_lossy().to_string(),
    )
}
