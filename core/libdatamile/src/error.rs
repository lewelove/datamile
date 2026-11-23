use std::path::PathBuf;

use datamile_cache::CacheError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatamileError {
    #[error("Missing primary manifest: album.toml not found in {path}")]
    MissingPrimaryManifest { path: PathBuf },

    #[error("IO Error: {0}")]
    ManifestIoError(#[from] std::io::Error),

    #[error("Parse Error: Failed to parse TOML in {path}: {source}")]
    ManifestParseError {
        path: PathBuf,
        source: toml::de::Error,
    },

    #[error("Parse Error: Failed to parse JSON in {path}: {source}")]
    JsonParseError {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[error("Invalid file extension: Expected '{expected}' in {path}")]
    InvalidFileExtension { path: PathBuf, expected: String },

    #[error("Invalid manifest name '{name}': {reason}")]
    InvalidManifestName { name: String, reason: String },

    #[error("Duplicate manifest name '{name}' declared in configuration")]
    DuplicateManifestName { name: String },

    #[error(
        "Unsupported audio extension '{extension}' declared in '{section}' configuration"
    )]
    UnsupportedAudioExtension { extension: String, section: String },

    #[error("Serialization Error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Schema Error: [[tracks]] array not found in album.toml for {path}")]
    MissingTracksBlock { path: PathBuf },

    #[error(
        "Manifest Structure Violation: Auxiliary manifest '{manifest}' in {path} defines {aux_count} tracks, but album.toml defines {primary_count} tracks. Every manifest must describe the exact same track count."
    )]
    TrackCountMismatch {
        manifest: String,
        path: PathBuf,
        primary_count: usize,
        aux_count: usize,
    },

    #[error(
        "Inventory Mismatch in {path}: Found {files_count} audio files on disk but album.toml defines {tracks_count} tracks. Datamile requires an explicit 1:1 mapping between disk files and metadata entries."
    )]
    PhysicalInventoryMismatch {
        path: PathBuf,
        files_count: usize,
        tracks_count: usize,
    },

    #[error(
        "Identity Error: Manifest '{manifest}' in {path} is missing or has an invalid TRACKNUMBER field at track index {index}"
    )]
    MissingTrackIdentity {
        manifest: String,
        path: PathBuf,
        index: usize,
    },

    #[error("{message} for field '{field}'")]
    InvalidIdentityFormat { field: String, message: String },

    #[error(
        "Identity Collision: Manifest '{manifest}' in {path} defines Disc {disc}, Track {track} more than once."
    )]
    DuplicateTrackIdentity {
        manifest: String,
        path: PathBuf,
        disc: u32,
        track: u32,
    },

    #[error("Harvest Error: Failed to harvest metadata from audio file {path}: {source}")]
    HarvestError {
        path: PathBuf,
        source: anyhow::Error,
    },

    #[error(
        "Type Mismatch in {path}: Field '{key}' is expected to be a '{expected_type}', but found value: {found_val}"
    )]
    TypeMismatch {
        path: PathBuf,
        key: String,
        expected_type: String,
        found_val: String,
    },

    #[error("Duplicate Album ID '{id}': defined in both {path_a} and {path_b}")]
    DuplicateAlbumId {
        id: String,
        path_a: PathBuf,
        path_b: PathBuf,
    },

    #[error("Invalid Image Fit in {path} for '{namespace}': {message}")]
    InvalidImageFit {
        path: PathBuf,
        namespace: String,
        message: String,
    },

    #[error(
        "Invalid Image Filter in {path} for '{namespace}': '{found}' is not a valid filter"
    )]
    InvalidImageFilter {
        path: PathBuf,
        namespace: String,
        found: String,
    },
}

/// Keeps manifest error text identical to the cache crate's wording.
impl From<CacheError> for DatamileError {
    fn from(err: CacheError) -> Self {
        match err {
            CacheError::InvalidFileExtension { path, expected } => {
                Self::InvalidFileExtension { path, expected }
            }
            CacheError::Io(err) => Self::ManifestIoError(err),
            CacheError::JsonParse { path, source } => {
                Self::JsonParseError { path, source }
            }
            CacheError::TomlParse { path, source } => {
                Self::ManifestParseError { path, source }
            }
            CacheError::JsonSerialize(err) => Self::JsonError(err),
        }
    }
}
