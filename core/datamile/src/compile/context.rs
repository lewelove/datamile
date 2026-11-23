use std::path::{Path, PathBuf};

use datamile_fs::expand_path;
use libdatamile::harvest::SUPPORTED_AUDIO_EXTENSIONS;

pub struct PreparedContext {
    pub audio_files: Vec<PathBuf>,
    pub music_directory: PathBuf,
}

pub fn prepare_build_context(
    config: &libdatamile::lua::ResolvedConfig,
    album_root: &Path,
) -> PreparedContext {
    let audio_files = config.app.compiler.audio_extensions.as_ref().map_or_else(
        || libdatamile::scanner::scan_audio_files(album_root, SUPPORTED_AUDIO_EXTENSIONS),
        |exts| libdatamile::scanner::scan_audio_files(album_root, exts),
    );

    let music_dir_raw = &config.app.storage.music_directory;
    let music_directory = expand_path(music_dir_raw)
        .canonicalize()
        .unwrap_or_else(|_| expand_path(music_dir_raw));

    PreparedContext {
        audio_files,
        music_directory,
    }
}
