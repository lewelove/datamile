use crate::harvest::is_audio_file;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
