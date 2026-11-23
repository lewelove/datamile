use std::path::{Path, PathBuf};

use datamile_audio::{
    AudioError, SUPPORTED_AUDIO_EXTENSIONS, scan_audio_files, validate_extensions,
};
use datamile_fs::expand_path;

use crate::error::DatamileError;
use crate::lua::ResolvedConfig;

pub(crate) struct PreparedContext {
    pub audio_files: Vec<PathBuf>,
    pub music_directory: PathBuf,
}

/// Applies the checks that used to live inside `ResolvedConfig::load`, so the
/// Lua layer stays free of compiler and media knowledge while every command
/// still starts from a validated config.
pub(crate) fn validate_config(config: &mut ResolvedConfig) -> Result<(), DatamileError> {
    if let Some(ref manifests) = config.app.compiler.manifests {
        let validated =
            crate::compiler::manifest::validate_and_filter_manifest_names(manifests)?;
        config.app.compiler.manifests = Some(validated);
    }

    validate_audio_config(config)
}

/// Rejects unsupported `audio_extensions` from the compiler config section.
fn validate_audio_config(config: &ResolvedConfig) -> Result<(), DatamileError> {
    validate_section(
        config.app.compiler.audio_extensions.as_deref(),
        "compiler.audio_extensions",
    )
}

fn validate_section(exts: Option<&[String]>, section: &str) -> Result<(), DatamileError> {
    let Some(exts) = exts else {
        return Ok(());
    };

    match validate_extensions(exts) {
        Ok(_) => Ok(()),
        Err(AudioError::UnsupportedExtension { extension }) => {
            Err(DatamileError::UnsupportedAudioExtension {
                extension,
                section: section.to_string(),
            })
        }
    }
}

pub(crate) fn prepare_build_context(
    config: &ResolvedConfig,
    album_root: &Path,
) -> PreparedContext {
    let audio_files = config.app.compiler.audio_extensions.as_ref().map_or_else(
        || scan_audio_files(album_root, SUPPORTED_AUDIO_EXTENSIONS),
        |exts| scan_audio_files(album_root, exts),
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
