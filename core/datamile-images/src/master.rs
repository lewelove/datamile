use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedMaster {
    pub address: String,
    pub rendered_width: u32,
    pub rendered_height: u32,
    pub fit: String,
    pub filter: String,
    pub source_width: u32,
    pub source_height: u32,
    pub source_format: String,
    /// Content digest of the source, kept internal so a warm recompile can
    /// reuse the address without decoding the image.
    pub source_hash: String,
    pub source_mtime: u64,
    pub source_byte_size: u64,
}

/// What a source image looks like, cached by `(canon_path, size, mtime)`.
///
/// Computing it is the only step that decodes bytes; a warm render reads this
/// probe, derives the address, and touches `master.qoi` only if it exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSourceProbe {
    pub raw_hash: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub mtime: u64,
    pub byte_size: u64,
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
    render_master_cached(source_path, spec, cache_root)
}

fn probe_source(source_path: &Path) -> Result<ImageSourceProbe, ImageError> {
    let bytes = fs::read(source_path).map_err(|source| ImageError::SourceRead {
        path: source_path.to_path_buf(),
        source,
    })?;
    let meta = fs::metadata(source_path)?;
    let file_meta = datamile_fs::file_meta(&meta)?;

    let img = image::load_from_memory(&bytes).map_err(|source| ImageError::Decode {
        path: source_path.to_path_buf(),
        source,
    })?;
    let format = image::guess_format(&bytes).map_or_else(
        |_| "unknown".to_string(),
        |format| format!("{format:?}").to_lowercase(),
    );

    Ok(ImageSourceProbe {
        raw_hash: blake3::hash(&bytes).as_bytes().to_vec(),
        width: img.width(),
        height: img.height(),
        format,
        mtime: file_meta.mtime,
        byte_size: file_meta.byte_size,
    })
}

/// Renders the master, reusing a cached source probe so an unchanged image is
/// never decoded twice.
///
/// Warm path: probe from `objects/`, address derived from it, and
/// `master.qoi` already on disk means zero image work.
pub fn render_master_cached(
    source_path: &Path,
    spec: &ImageSpec,
    cache_root: &Path,
) -> Result<RenderedMaster, ImageError> {
    let probe = datamile_cache::read_or_compute_cached(
        source_path,
        cache_root,
        "objects",
        probe_source,
    )?;

    let address = compute_image_address(
        spec.fit_mode,
        spec.target_width,
        spec.target_height,
        spec.filter_type,
        &probe.raw_hash,
    );
    let (rendered_width, rendered_height) = calculate_rendered_dimensions(
        probe.width,
        probe.height,
        spec.target_width,
        spec.target_height,
        spec.fit_mode,
    );

    let target = master_path(cache_root, &address);
    if !target.exists() {
        let bytes = fs::read(source_path).map_err(|source| ImageError::SourceRead {
            path: source_path.to_path_buf(),
            source,
        })?;
        let img =
            image::load_from_memory(&bytes).map_err(|source| ImageError::Decode {
                path: source_path.to_path_buf(),
                source,
            })?;
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
        fit: spec.fit_mode.as_str().to_string(),
        filter: filter_type_to_str(spec.filter_type).to_string(),
        source_width: probe.width,
        source_height: probe.height,
        source_format: probe.format,
        source_hash: datamile_fs::format_blake3_string(
            &probe.raw_hash.as_slice().try_into().unwrap_or([0; 32]),
        ),
        source_mtime: probe.mtime,
        source_byte_size: probe.byte_size,
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
