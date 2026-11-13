use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use libdatamile::error::DatamileError;
use libdatamile::images::{
    FitMode, calculate_rendered_dimensions, parse_filter, parse_fit_mode,
    resize_image_fit,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

struct CoverSpec {
    fit_mode: FitMode,
    fit_mode_str: String,
    target_width: u32,
    target_height: u32,
    filter_type: libdatamile::images::FilterType,
    filter_str: String,
    sources: Vec<String>,
}

fn parse_fit_tuple(
    spec_obj: &serde_json::Map<String, Value>,
    album_root: &Path,
    namespace: &str,
) -> Result<(FitMode, String, u32, u32), DatamileError> {
    let fit_arr = spec_obj
        .get("fit")
        .and_then(Value::as_array)
        .ok_or_else(|| DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: "Missing or invalid 'fit' tuple, expected { mode, width, height }"
                .to_string(),
        })?;

    if fit_arr.len() != 3 {
        return Err(DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: format!(
                "'fit' tuple must contain exactly 3 elements, found {}",
                fit_arr.len()
            ),
        });
    }

    let fit_mode_str = fit_arr[0]
        .as_str()
        .ok_or_else(|| DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: "First element of 'fit' tuple must be a string ('crop' or 'inside')"
                .to_string(),
        })?
        .to_string();

    let fit_mode =
        parse_fit_mode(&fit_mode_str).ok_or_else(|| DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: format!(
                "Unknown fit mode '{fit_mode_str}', expected 'crop' or 'inside'"
            ),
        })?;

    let target_width = fit_arr[1]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&v| (1..=4096).contains(&v))
        .ok_or_else(|| DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message:
                "Second element of 'fit' tuple must be an integer between 1 and 4096"
                    .to_string(),
        })?;

    let target_height = fit_arr[2]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&v| (1..=4096).contains(&v))
        .ok_or_else(|| DatamileError::InvalidCoverFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: "Third element of 'fit' tuple must be an integer between 1 and 4096"
                .to_string(),
        })?;

    Ok((fit_mode, fit_mode_str, target_width, target_height))
}

fn parse_sources(
    spec_obj: &serde_json::Map<String, Value>,
    album_root: &Path,
    namespace: &str,
) -> Result<Vec<String>, DatamileError> {
    let mut sources = Vec::new();
    let Some(src_val) = spec_obj.get("sources") else {
        return Ok(sources);
    };

    match src_val {
        Value::String(s) => {
            if !s.is_empty() {
                sources.push(s.clone());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str()
                    && !s.is_empty()
                {
                    sources.push(s.to_string());
                }
            }
        }
        Value::Null => {}
        _ => {
            return Err(DatamileError::TypeMismatch {
                path: album_root.to_path_buf(),
                key: format!("images.{namespace}.sources"),
                expected_type: "string, array of strings, or nil".to_string(),
                found_val: src_val.to_string(),
            });
        }
    }

    Ok(sources)
}

fn parse_cover_spec(
    namespace: &str,
    spec_val: &Value,
    album_root: &Path,
) -> Result<CoverSpec, DatamileError> {
    let spec_obj =
        spec_val
            .as_object()
            .ok_or_else(|| DatamileError::InvalidCoverFit {
                path: album_root.to_path_buf(),
                namespace: namespace.to_string(),
                message: "Cover specification must be a table".to_string(),
            })?;

    let (fit_mode, fit_mode_str, target_width, target_height) =
        parse_fit_tuple(spec_obj, album_root, namespace)?;

    let filter_str = spec_obj
        .get("filter")
        .and_then(Value::as_str)
        .ok_or_else(|| DatamileError::InvalidCoverFilter {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            found: "missing".to_string(),
        })?
        .to_string();

    let filter_type =
        parse_filter(&filter_str).ok_or_else(|| DatamileError::InvalidCoverFilter {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            found: filter_str.clone(),
        })?;

    let sources = parse_sources(spec_obj, album_root, namespace)?;

    Ok(CoverSpec {
        fit_mode,
        fit_mode_str,
        target_width,
        target_height,
        filter_type,
        filter_str,
        sources,
    })
}

fn compute_cover_address(
    fit_mode: &str,
    width: u32,
    height: u32,
    filter: &str,
    source_hash_bytes: &[u8],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(fit_mode.as_bytes());
    hasher.update(&[0]);
    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    hasher.update(filter.as_bytes());
    hasher.update(&[0]);
    hasher.update(source_hash_bytes);
    let hash_result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash_result.as_bytes())[..16].to_string()
}

fn resolve_source_target(
    src_str: &str,
    album_root: &Path,
    album_root_canon: &Path,
    dependencies: &mut HashSet<PathBuf>,
) -> Option<(PathBuf, String)> {
    let raw_path = Path::new(src_str);
    let target = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        album_root.join(raw_path)
    };

    let canon = target.canonicalize().ok()?;
    if !canon.is_file() {
        return None;
    }

    if let Ok(rel) = canon.strip_prefix(album_root_canon) {
        let rel_str = rel.to_string_lossy().to_string();
        Some((canon, rel_str))
    } else {
        dependencies.insert(canon.clone());
        let abs_str = canon.to_string_lossy().to_string();
        Some((canon, abs_str))
    }
}

pub fn process_images(
    album_root: &Path,
    cache_root: &Path,
    images_val: Option<&Value>,
    dependencies: &mut HashSet<PathBuf>,
) -> Result<BTreeMap<String, Value>, DatamileError> {
    let mut result_map = BTreeMap::new();
    let Some(images_obj) = images_val.and_then(Value::as_object) else {
        return Ok(result_map);
    };

    let album_root_canon = album_root
        .canonicalize()
        .unwrap_or_else(|_| album_root.to_path_buf());

    for (namespace, spec_val) in images_obj {
        let spec = parse_cover_spec(namespace, spec_val, album_root)?;
        let mut entries = Vec::new();

        for src_str in &spec.sources {
            let Some((canon_path, path_for_json)) = resolve_source_target(
                src_str,
                album_root,
                &album_root_canon,
                dependencies,
            ) else {
                continue;
            };

            let Ok(bytes) = fs::read(&canon_path) else {
                continue;
            };

            let raw_hash = blake3::hash(&bytes);
            let hash_str = format!("blake3-{}", STANDARD.encode(raw_hash.as_bytes()));

            let meta =
                fs::metadata(&canon_path).map_err(DatamileError::ManifestIoError)?;
            let mtime = meta
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let byte_size = meta.len();

            let Ok(img) = image::load_from_memory(&bytes) else {
                continue;
            };

            let src_w = img.width();
            let src_h = img.height();
            let format_str = image::guess_format(&bytes).map_or_else(
                |_| "unknown".to_string(),
                |f| format!("{f:?}").to_lowercase(),
            );

            let address = compute_cover_address(
                &spec.fit_mode_str,
                spec.target_width,
                spec.target_height,
                &spec.filter_str,
                raw_hash.as_bytes(),
            );

            let (rendered_w, rendered_h) = calculate_rendered_dimensions(
                src_w,
                src_h,
                spec.target_width,
                spec.target_height,
                spec.fit_mode,
            );

            let master_dir = cache_root.join("images").join(&address);
            let master_qoi_path = master_dir.join("master.qoi");

            if !master_qoi_path.exists() {
                let img_rgb = img.into_rgb8();
                if let Some(resized) = resize_image_fit(
                    &img_rgb,
                    spec.target_width,
                    spec.target_height,
                    spec.fit_mode,
                    spec.filter_type,
                ) {
                    let _ = fs::create_dir_all(&master_dir);
                    let _ = resized
                        .save_with_format(&master_qoi_path, image::ImageFormat::Qoi);
                }
            }

            entries.push(json!({
                "address": address,
                "width": rendered_w,
                "height": rendered_h,
                "fit": spec.fit_mode_str,
                "filter": spec.filter_str,
                "source": {
                    "file": {
                        "path": path_for_json,
                        "hash": hash_str,
                        "mtime": mtime,
                        "byte_size": byte_size
                    },
                    "info": {
                        "width": src_w,
                        "height": src_h,
                        "format": format_str
                    }
                }
            }));
        }

        result_map.insert(namespace.clone(), Value::Array(entries));
    }

    Ok(result_map)
}
