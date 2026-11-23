use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::codec::{
    FilterType, ImageWireFormat, decode_qoi_to_rgb, encode_rgb_image, filter_type_to_str,
    read_qoi_dimensions, resize_image_max_edge,
};
use crate::error::ImageError;

const QOI_HEADER_BYTES: usize = 14;

/// Largest edge of the master, used to resolve derivative sizes.
pub fn read_master_dimension(master_path: &Path) -> Result<u32, ImageError> {
    let mut header = [0u8; QOI_HEADER_BYTES];
    let mut file = File::open(master_path)?;

    if file.read_exact(&mut header).is_err() {
        return Err(ImageError::QoiFormat {
            path: master_path.to_path_buf(),
        });
    }

    let Some((width, height)) = read_qoi_dimensions(&header) else {
        return Err(ImageError::QoiFormat {
            path: master_path.to_path_buf(),
        });
    };

    Ok(std::cmp::max(width, height))
}

/// Derivative cache name: `s{size}[_{filter}][_q{quality}].{ext}`.
#[must_use]
pub fn derivative_filename(
    resolved_size: u32,
    master_dim: u32,
    filter: FilterType,
    quality: u8,
    wire_format: ImageWireFormat,
) -> String {
    let filter_suffix = if resolved_size == master_dim {
        String::new()
    } else {
        format!("_{}", filter_type_to_str(filter))
    };

    let quality_suffix = if wire_format == ImageWireFormat::Jpg {
        format!("_q{quality}")
    } else {
        String::new()
    };

    format!(
        "s{resolved_size}{filter_suffix}{quality_suffix}.{}",
        wire_format.extension()
    )
}

/// Reads a master QOI and renders the requested derivative from it.
pub fn render_derivative_from_master(
    master_path: &Path,
    size: u32,
    master_dim: u32,
    filter: FilterType,
    wire_format: ImageWireFormat,
    quality: u8,
) -> Result<Vec<u8>, ImageError> {
    let master_bytes = std::fs::read(master_path)?;
    let mut img = decode_qoi_to_rgb(&master_bytes).ok_or(ImageError::QoiFormat {
        path: master_path.to_path_buf(),
    })?;

    if size != master_dim {
        img = resize_image_max_edge(&img, size, filter).ok_or(ImageError::Resize {
            width: size,
            height: size,
        })?;
    }

    encode_rgb_image(&img, wire_format, quality).ok_or(ImageError::Encode {
        format: wire_format.extension(),
    })
}
