use crate::error::DatamileError;
use crate::harvest::is_audio_file;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn find_target_albums(path: &Path) -> Result<Vec<PathBuf>, DatamileError> {
    let markers = ["album.toml"];
    let mut results = std::collections::HashSet::new();

    for marker in markers {
        if path.join(marker).exists() {
            results.insert(path.to_path_buf());
        }
    }

    if results.is_empty() {
        for entry in WalkDir::new(path).follow_links(true) {
            match entry {
                Ok(e) => {
                    if markers.contains(&e.file_name().to_string_lossy().as_ref())
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
