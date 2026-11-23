use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::codec::{
    FilterType, FitMode, calculate_rendered_dimensions, filter_type_to_str,
    resize_image_fit, save_qoi_atomic,
};
use crate::error::ImageError;

/// Address length in base64 characters, keeping cache paths short.
const ADDRESS_CHARS: usize = 16;

/// Byte separating the unscoped fields in the address digest.
const ADDRESS_SEPARATOR: u8 = 0;

/// Content-addressed location: `<cache_root>/images/<address>/master.qoi`.
const MASTER_DIR: &str = "images";
const MASTER_FILE: &str = "master.qoi";

#[derive(Debug, Clone, Copy)]
pub struct ImageSpec {
    pub fit_mode: FitMode,
    pub target_width: u32,
    pub target_height: u32,
    pub filter_type: FilterType,
}

#[derive(Debug, Clone)]
pub struct RenderedMaster {
    pub address: String,
    pub rendered_width: u32,
    pub rendered_height: u32,
    pub fit: &'static str,
    pub filter: &'static str,
    pub source_width: u32,
    pub source_height: u32,
    pub source_format: String,
    pub source_hash: String,
    pub source_mtime: u64,
    pub source_byte_size: u64,
}

/// Single source of truth for where master and derivative images live on disk.
#[must_use]
pub fn master_dir(cache_root: &Path, address: &str) -> PathBuf {
    cache_root.join(MASTER_DIR).join(address)
}

#[must_use]
pub fn master_path(cache_root: &Path, address: &str) -> PathBuf {
    master_dir(cache_root, address).join(MASTER_FILE)
}

/// Blake3 over the canonical spec and source bytes, truncated to a short path
/// segment. Canonical enum spellings keep `Lanczos` and `lanczos` from
/// producing two addresses for the same bytes.
#[must_use]
pub fn compute_image_address(
    fit_mode: FitMode,
    width: u32,
    height: u32,
    filter: FilterType,
    source_hash_bytes: &[u8],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(fit_mode.as_str().as_bytes());
    hasher.update(&[ADDRESS_SEPARATOR]);
    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    hasher.update(filter_type_to_str(filter).as_bytes());
    hasher.update(&[ADDRESS_SEPARATOR]);
    hasher.update(source_hash_bytes);

    let digest = hasher.finalize();
    URL_SAFE_NO_PAD.encode(digest.as_bytes())[..ADDRESS_CHARS].to_string()
}

/// Renders the QOI master for `source_path` unless it already exists, and
/// describes the result for the caller's lock entry.
pub fn render_master_from_file(
    source_path: &Path,
    spec: &ImageSpec,
    cache_root: &Path,
) -> Result<RenderedMaster, ImageError> {
    let bytes = fs::read(source_path).map_err(|source| ImageError::SourceRead {
        path: source_path.to_path_buf(),
        source,
    })?;
    let meta = fs::metadata(source_path)?;
    let file_meta = datamile_fs::file_meta(&meta)?;

    let raw_hash = blake3::hash(&bytes);
    let source_hash = datamile_fs::format_blake3_string(raw_hash.as_bytes());

    let img = image::load_from_memory(&bytes).map_err(|source| ImageError::Decode {
        path: source_path.to_path_buf(),
        source,
    })?;

    let source_width = img.width();
    let source_height = img.height();
    let source_format = image::guess_format(&bytes).map_or_else(
        |_| "unknown".to_string(),
        |format| format!("{format:?}").to_lowercase(),
    );

    let address = compute_image_address(
        spec.fit_mode,
        spec.target_width,
        spec.target_height,
        spec.filter_type,
        raw_hash.as_bytes(),
    );
    let (rendered_width, rendered_height) = calculate_rendered_dimensions(
        source_width,
        source_height,
        spec.target_width,
        spec.target_height,
        spec.fit_mode,
    );

    let target = master_path(cache_root, &address);
    if !target.exists() {
        let source_rgb = img.into_rgb8();
        if let Some(resized) = resize_image_fit(
            &source_rgb,
            spec.target_width,
            spec.target_height,
            spec.fit_mode,
            spec.filter_type,
        ) && let Err(error) = save_qoi_atomic(&target, &resized)
        {
            log::warn!("Failed to write master image {}: {error}", target.display());
        }
    }

    Ok(RenderedMaster {
        address,
        rendered_width,
        rendered_height,
        fit: spec.fit_mode.as_str(),
        filter: filter_type_to_str(spec.filter_type),
        source_width,
        source_height,
        source_format,
        source_hash,
        source_mtime: file_meta.mtime,
        source_byte_size: file_meta.byte_size,
    })
}

/// Lazy fallback used by the server when a lock entry exists but its master
/// QOI is missing.
pub fn write_master_qoi(
    source_path: &Path,
    master_qoi_path: &Path,
    spec: &ImageSpec,
) -> Result<(), ImageError> {
    let img = image::open(source_path).map_err(|source| ImageError::Decode {
        path: source_path.to_path_buf(),
        source,
    })?;

    let resized = resize_image_fit(
        &img.into_rgb8(),
        spec.target_width,
        spec.target_height,
        spec.fit_mode,
        spec.filter_type,
    )
    .ok_or(ImageError::Resize {
        width: spec.target_width,
        height: spec.target_height,
    })?;

    save_qoi_atomic(master_qoi_path, &resized)?;
    Ok(())
}
