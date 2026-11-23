use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use datamile_fs::ResolvedFileLocation;
use datamile_images::{
    FitMode, ImageError, ImageSpec as RenderSpec, MAX_IMAGE_DIMENSION, parse_filter,
    parse_fit_mode, render_master_from_file,
};
use serde_json::{Value, json};

use crate::error::DatamileError;

struct ImageRule {
    render: RenderSpec,
    sources: Vec<String>,
}

fn parse_fit_tuple(
    spec_obj: &serde_json::Map<String, Value>,
    album_root: &Path,
    namespace: &str,
) -> Result<(FitMode, u32, u32), DatamileError> {
    let fit_arr = spec_obj
        .get("fit")
        .and_then(Value::as_array)
        .ok_or_else(|| DatamileError::InvalidImageFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: "Missing or invalid 'fit' tuple, expected { mode, width, height }"
                .to_string(),
        })?;

    if fit_arr.len() != 3 {
        return Err(DatamileError::InvalidImageFit {
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
        .ok_or_else(|| DatamileError::InvalidImageFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: "First element of 'fit' tuple must be a string ('crop' or 'inside')"
                .to_string(),
        })?
        .to_string();

    let fit_mode =
        parse_fit_mode(&fit_mode_str).ok_or_else(|| DatamileError::InvalidImageFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: format!(
                "Unknown fit mode '{fit_mode_str}', expected 'crop' or 'inside'"
            ),
        })?;

    let target_width = fit_arr[1]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&v| (1..=MAX_IMAGE_DIMENSION).contains(&v))
        .ok_or_else(|| DatamileError::InvalidImageFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: format!(
                "Second element of 'fit' tuple must be an integer between 1 and {MAX_IMAGE_DIMENSION}"
            ),
        })?;

    let target_height = fit_arr[2]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .filter(|&v| (1..=MAX_IMAGE_DIMENSION).contains(&v))
        .ok_or_else(|| DatamileError::InvalidImageFit {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            message: format!(
                "Third element of 'fit' tuple must be an integer between 1 and {MAX_IMAGE_DIMENSION}"
            ),
        })?;

    Ok((fit_mode, target_width, target_height))
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

fn parse_image_spec(
    namespace: &str,
    spec_val: &Value,
    album_root: &Path,
) -> Result<ImageRule, DatamileError> {
    let spec_obj =
        spec_val
            .as_object()
            .ok_or_else(|| DatamileError::InvalidImageFit {
                path: album_root.to_path_buf(),
                namespace: namespace.to_string(),
                message: "Image specification must be a table".to_string(),
            })?;

    let (fit_mode, target_width, target_height) =
        parse_fit_tuple(spec_obj, album_root, namespace)?;

    let filter_str = spec_obj
        .get("filter")
        .and_then(Value::as_str)
        .ok_or_else(|| DatamileError::InvalidImageFilter {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            found: "missing".to_string(),
        })?
        .to_string();

    let filter_type =
        parse_filter(&filter_str).ok_or_else(|| DatamileError::InvalidImageFilter {
            path: album_root.to_path_buf(),
            namespace: namespace.to_string(),
            found: filter_str.clone(),
        })?;

    let sources = parse_sources(spec_obj, album_root, namespace)?;

    Ok(ImageRule {
        render: RenderSpec {
            fit_mode,
            target_width,
            target_height,
            filter_type,
        },
        sources,
    })
}

fn resolve_source_target(
    src_str: &str,
    album_root: &Path,
    album_root_canon: &Path,
    dependencies: &mut HashSet<PathBuf>,
) -> Option<(PathBuf, String)> {
    match datamile_fs::resolve_file_location(src_str, album_root, album_root_canon)? {
        ResolvedFileLocation::InsideBase {
            canonical,
            rel_path,
        } => Some((canonical, rel_path)),
        ResolvedFileLocation::OutsideBase {
            canonical,
            abs_path,
        } => {
            dependencies.insert(canonical.clone());
            Some((canonical, abs_path))
        }
    }
}

pub(crate) fn process_images(
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
        let rule = parse_image_spec(namespace, spec_val, album_root)?;
        let mut entries = Vec::new();

        for src_str in &rule.sources {
            let Some((canon_path, path_for_json)) = resolve_source_target(
                src_str,
                album_root,
                &album_root_canon,
                dependencies,
            ) else {
                continue;
            };

            // Unreadable or undecodable sources are skipped, but a metadata
            // failure must abort the album like it always has.
            let master =
                match render_master_from_file(&canon_path, &rule.render, cache_root) {
                    Ok(master) => master,
                    Err(ImageError::Decode { .. } | ImageError::SourceRead { .. }) => {
                        continue;
                    }
                    Err(error) => {
                        return Err(DatamileError::ManifestIoError(
                            std::io::Error::other(error),
                        ));
                    }
                };

            entries.push(json!({
                "address": master.address,
                "width": master.rendered_width,
                "height": master.rendered_height,
                "fit": master.fit,
                "filter": master.filter,
                "source": {
                    "file": {
                        "path": path_for_json,
                        "hash": master.source_hash,
                        "mtime": master.source_mtime,
                        "byte_size": master.source_byte_size
                    },
                    "info": {
                        "width": master.source_width,
                        "height": master.source_height,
                        "format": master.source_format
                    }
                }
            }));
        }

        result_map.insert(namespace.clone(), Value::Array(entries));
    }

    Ok(result_map)
}
