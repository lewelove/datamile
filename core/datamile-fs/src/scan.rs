use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// Case-insensitive membership test for a bare extension (`"flac"`).
#[must_use]
pub fn is_matching_extension<S: AsRef<str>>(ext: &str, candidates: &[S]) -> bool {
    candidates
        .iter()
        .any(|candidate| candidate.as_ref().eq_ignore_ascii_case(ext))
}

/// Whether the path's extension is one of `extensions`; paths without an
/// extension never match.
#[must_use]
pub fn has_extension<S: AsRef<str>>(path: &Path, extensions: &[S]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_matching_extension(ext, extensions))
}

/// Every regular file below `root` with a matching extension, in natural
/// alphanumeric order (`2.flac` before `10.flac`).
#[must_use]
pub fn scan_files_by_extension<S: AsRef<str>>(
    root: &Path,
    extensions: &[S],
) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .filter(|path| has_extension(path, extensions))
        .collect();

    files.sort_by(|a, b| alphanumeric_sort::compare_path(a, b));
    files
}
