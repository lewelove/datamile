use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::DatamileError;

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
    datamile_fs::scan_files_by_extension(root, extensions)
}
