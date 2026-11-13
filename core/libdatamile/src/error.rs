use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatamileError {
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

    #[error(
        "Invalid Color Format in {path}: Field '{key}' must be a valid hex color string, found '{found}'"
    )]
    InvalidColorFormat {
        path: PathBuf,
        key: String,
        found: String,
    },

    #[error("Duplicate Media ID '{id}': defined in both {path_a} and {path_b}")]
    DuplicateMediaId {
        id: String,
        path_a: PathBuf,
        path_b: PathBuf,
    },

    #[error(
        "Root media type '{root_type}' selected {path}, which is not an existing directory"
    )]
    RootTargetNotDirectory { root_type: String, path: PathBuf },

    #[error("Invalid Cover Fit in {path} for '{namespace}': {message}")]
    InvalidCoverFit {
        path: PathBuf,
        namespace: String,
        message: String,
    },

    #[error(
        "Invalid Cover Filter in {path} for '{namespace}': '{found}' is not a valid filter"
    )]
    InvalidCoverFilter {
        path: PathBuf,
        namespace: String,
        found: String,
    },
}
