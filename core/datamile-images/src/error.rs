use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("Image IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to read image source {path}: {source}")]
    SourceRead {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to decode image {path}: {source}")]
    Decode {
        path: PathBuf,
        source: image::ImageError,
    },

    #[error("Invalid QOI data in {path}")]
    QoiFormat { path: PathBuf },

    #[error("Failed to resize image to {width}x{height}")]
    Resize { width: u32, height: u32 },

    #[error("Failed to encode image as '{format}'")]
    Encode { format: &'static str },
}
