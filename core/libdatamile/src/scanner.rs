use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::DatamileError;
use crate::harvest::is_audio_file;

pub fn find_target_albums(path: &Path) -> Result<Vec<PathBuf>, DatamileError> {
    let mut results = std::collections::HashSet::new();
    if path.join("album.toml").exists() {
        results.insert(path.to_path_buf());
    } else {
        for entry in WalkDir::new(path).follow_links(true) {
            match entry {
                Ok(e) => {
                    if e.file_name() == "album.toml"
                        && let Some(parent) = e.path().parent()
                    {
                        results.insert(parent.to_path_buf());
                    }
                }
                Err(e) => {
                    log::debug!("Skipping path during scan due to error: {e}");
                }
            }
        }
    }
    let mut vec_results: Vec<PathBuf> = results.into_iter().collect();
    vec_results.sort();
    Ok(vec_results)
}

pub fn scan_audio_files<S: AsRef<str>>(root: &Path, extensions: &[S]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().to_path_buf())
        .filter(|p| is_audio_file(p, extensions))
        .collect();
    files.sort_by(|a, b| alphanumeric_sort::compare_path(a, b));
    files
}
