use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use datamile_fs::write_atomic;
use datamile_images::{
    FilterType, ImageError, ImageSpec, ImageWireFormat, MAX_IMAGE_DIMENSION,
    derivative_filename, master_dir, master_path, parse_filter, parse_wire_format,
    read_master_dimension, render_derivative_from_master, write_master_qoi,
};
use serde::Deserialize;
use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

use crate::server::state::{AppState, ImageClaim};

#[derive(Deserialize)]
pub struct ImageQuery {
    pub size: Option<u32>,
    pub filter: Option<String>,
    pub quality: Option<u8>,
}

#[derive(Deserialize)]
pub struct CacheImagesPayload {
    #[serde(default, deserialize_with = "deserialize_names_or_addresses")]
    pub names: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_names_or_addresses")]
    pub addresses: Option<Vec<String>>,
    pub format: String,
    pub size: u32,
    pub filter: String,
    pub quality: Option<u8>,
}

fn deserialize_names_or_addresses<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Single(String),
        Multiple(Vec<String>),
    }

    match Option::<StringOrVec>::deserialize(deserializer)? {
        Some(StringOrVec::Single(s)) => Ok(Some(vec![s])),
        Some(StringOrVec::Multiple(v)) => Ok(Some(v)),
        None => Ok(None),
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn ensure_master_image(
    state: &Arc<AppState>,
    cache_root: &StdPath,
    address: &str,
) -> Result<PathBuf, (StatusCode, &'static str)> {
    let not_found = (StatusCode::NOT_FOUND, "Image not found");
    let master_qoi = master_path(cache_root, address);

    if master_qoi.exists() {
        return Ok(master_qoi);
    }

    // A prewarm worker or a parallel request may already render this address.
    let guard = match state.image_tasks.claim(master_qoi.clone()) {
        Some(ImageClaim::Claimed(guard)) => guard,
        Some(ImageClaim::InFlight(mut rx)) => {
            let _ = rx.wait_for(|done| *done).await;
            return master_qoi.exists().then_some(master_qoi).ok_or(not_found);
        }
        None => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Image task registry unavailable",
            ));
        }
    };

    // The owner may have finished between our `exists()` check and the claim.
    if master_qoi.exists() {
        return Ok(master_qoi);
    }

    let lookup_item = {
        let logic = state.logic.read().await;
        logic.image_lookup.get(address).cloned()
    };

    let Some(item) = lookup_item else {
        return Err(not_found);
    };

    let raw_path = std::path::Path::new(&item.path);
    let original_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        item.album_dir.join(raw_path)
    };

    let blob_path_clone = master_qoi.clone();

    // The guard travels with the blocking work: a dropped request must not
    // release the claim while the image is still being rendered.
    let gen_result = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let spec = ImageSpec {
            fit_mode: item.fit_mode,
            target_width: item.target_width,
            target_height: item.target_height,
            filter_type: item.filter,
        };

        write_master_qoi(&original_path, &blob_path_clone, &spec).ok()?;
        Some(())
    })
    .await;

    match gen_result {
        Ok(Some(())) => Ok(master_qoi),
        Ok(None) => Err(not_found),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Image render task failed",
        )),
    }
}

fn parse_target_request(
    target: &str,
) -> Result<(&str, ImageWireFormat), (StatusCode, &'static str)> {
    let Some((address, ext)) = target.rsplit_once('.') else {
        return Err((StatusCode::BAD_REQUEST, "Missing format extension"));
    };

    if address.is_empty() || address.len() > 64 {
        return Err((StatusCode::BAD_REQUEST, "Invalid address"));
    }

    let Some(wire_format) = parse_wire_format(ext) else {
        return Err((StatusCode::BAD_REQUEST, "Unsupported image format"));
    };

    Ok((address, wire_format))
}

async fn master_dimension(
    master_qoi: PathBuf,
) -> Result<u32, (StatusCode, &'static str)> {
    tokio::task::spawn_blocking(move || read_master_dimension(&master_qoi))
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Image metadata task failed",
            )
        })?
        .map_err(|error| match error {
            ImageError::Io(_) => (StatusCode::NOT_FOUND, "Failed to read master image"),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid master image header",
            ),
        })
}

fn resolve_image_params(
    m_dim: u32,
    query: &ImageQuery,
    wire_format: ImageWireFormat,
) -> Result<(u32, FilterType, u8), (StatusCode, &'static str)> {
    let resolved_size = match query.size {
        None => m_dim,
        Some(s) if (1..=MAX_IMAGE_DIMENSION).contains(&s) => s,
        Some(_) => {
            return Err((
                StatusCode::BAD_REQUEST,
                "size must be an integer between 1 and 4096",
            ));
        }
    };

    let filter_type = if resolved_size == m_dim {
        FilterType::CatmullRom
    } else {
        match query.filter.as_deref() {
            None => FilterType::CatmullRom,
            Some(f_str) => match parse_filter(f_str) {
                Some(f) => f,
                None => return Err((StatusCode::BAD_REQUEST, "Unknown filter")),
            },
        }
    };

    let quality = match wire_format {
        ImageWireFormat::Jpg => match query.quality {
            None => 100,
            Some(q) if (1..=100).contains(&q) => q,
            Some(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "quality must be an integer between 1 and 100",
                ));
            }
        },
        _ => 100,
    };

    Ok((resolved_size, filter_type, quality))
}

async fn render_in_memory(
    master_qoi: PathBuf,
    resolved_size: u32,
    m_dim: u32,
    filter: FilterType,
    wire_format: ImageWireFormat,
    quality: u8,
) -> Result<Vec<u8>, StatusCode> {
    tokio::task::spawn_blocking(move || {
        render_derivative_from_master(
            &master_qoi,
            resolved_size,
            m_dim,
            filter,
            wire_format,
            quality,
        )
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    .and_then(|result| result.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR))
}

pub async fn image(
    Path(target): Path<String>,
    Query(query): Query<ImageQuery>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Response {
    let (address, wire_format) = match parse_target_request(&target) {
        Ok(res) => res,
        Err((status, msg)) => return error_response(status, msg),
    };

    let cache_root = {
        let guard = state.config.read().await;
        guard.cache_root.clone()
    };

    let master_qoi = match ensure_master_image(&state, &cache_root, address).await {
        Ok(path) => path,
        Err((status, msg)) => return error_response(status, msg),
    };

    let m_dimension = match master_dimension(master_qoi.clone()).await {
        Ok(dim) => dim,
        Err((status, msg)) => return error_response(status, msg),
    };

    let (resolved_size, filter_type, quality) =
        match resolve_image_params(m_dimension, &query, wire_format) {
            Ok(params) => params,
            Err((status, msg)) => return error_response(status, msg),
        };

    let cache_filename = derivative_filename(
        resolved_size,
        m_dimension,
        filter_type,
        quality,
        wire_format,
    );
    let etag = format!("\"{cache_filename}\"");

    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && if_none_match == etag
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }

    let cached_file_path = master_dir(&cache_root, address).join(&cache_filename);

    let bytes = if let Ok(data) = tokio::fs::read(&cached_file_path).await {
        data
    } else {
        match render_in_memory(
            master_qoi,
            resolved_size,
            m_dimension,
            filter_type,
            wire_format,
            quality,
        )
        .await
        {
            Ok(data) => data,
            Err(status) => return error_response(status, "Image processing error"),
        }
    };

    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(wire_format.mime_type()),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
            (
                header::ETAG,
                HeaderValue::try_from(etag.as_str())
                    .unwrap_or_else(|_| HeaderValue::from_static("\"\"")),
            ),
            (
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            ),
        ],
        bytes,
    )
        .into_response()
}

fn collect_matching_addresses(
    logic: &crate::server::logic::LogicEngine,
    names: Option<&[String]>,
    addresses: Option<&[String]>,
) -> Vec<String> {
    let mut matched = HashSet::new();

    // An empty list means "no filter", not "match nothing".
    let addr_filter: Option<HashSet<&str>> = addresses
        .filter(|arr| !arr.is_empty())
        .map(|arr| arr.iter().map(String::as_str).collect());
    let name_filter: Option<HashSet<&str>> = names
        .filter(|arr| !arr.is_empty())
        .map(|arr| arr.iter().map(String::as_str).collect());

    for record in logic.albums.values() {
        for (ns, addr) in &record.image_namespaces {
            if let Some(ref nf) = name_filter
                && !nf.contains(ns.as_str())
            {
                continue;
            }

            if let Some(ref af) = addr_filter
                && !af.contains(addr.as_str())
            {
                continue;
            }

            matched.insert(addr.clone());
        }
    }

    matched.into_iter().collect()
}

async fn process_cache_candidate(
    state: &Arc<AppState>,
    master_qoi: PathBuf,
    target_path: PathBuf,
    m_dim: u32,
    params: (u32, FilterType, u8, ImageWireFormat),
) {
    let Some(ImageClaim::Claimed(guard)) = state.image_tasks.claim(target_path.clone())
    else {
        return;
    };

    let (size, filter, quality, wire_format) = params;

    let _ = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let encoded = render_derivative_from_master(
            &master_qoi,
            size,
            m_dim,
            filter,
            wire_format,
            quality,
        )
        .ok()?;
        write_atomic(&target_path, &encoded).ok()?;
        Some(())
    })
    .await;
}

async fn run_cache_images_batch(
    state: Arc<AppState>,
    names: Option<Vec<String>>,
    addresses: Option<Vec<String>>,
    wire_format: ImageWireFormat,
    size: u32,
    filter: FilterType,
    quality: u8,
) {
    let target_addresses = {
        let logic = state.logic.read().await;
        collect_matching_addresses(&logic, names.as_deref(), addresses.as_deref())
    };

    if target_addresses.is_empty() {
        return;
    }

    let (cache_root, concurrency) = {
        let guard = state.config.read().await;
        let c = guard
            .app
            .compiler
            .jobs
            .unwrap_or_else(|| {
                std::thread::available_parallelism().map_or(4, std::num::NonZero::get)
            })
            .max(1);
        (guard.cache_root.clone(), c)
    };

    let semaphore = Arc::new(Semaphore::new(concurrency));

    for address in target_addresses {
        let sem_clone = Arc::clone(&semaphore);
        let state_clone = Arc::clone(&state);
        let cache_root = cache_root.clone();

        tokio::spawn(async move {
            let Ok(_permit) = sem_clone.acquire().await else {
                return;
            };

            let Ok(master_qoi) =
                ensure_master_image(&state_clone, &cache_root, &address).await
            else {
                return;
            };

            let Ok(m_dimension) = master_dimension(master_qoi.clone()).await else {
                return;
            };

            let cache_filename =
                derivative_filename(size, m_dimension, filter, quality, wire_format);
            let target_path = master_dir(&cache_root, &address).join(&cache_filename);

            if target_path.exists() {
                return;
            }

            process_cache_candidate(
                &state_clone,
                master_qoi,
                target_path,
                m_dimension,
                (size, filter, quality, wire_format),
            )
            .await;
        });
    }
}

pub async fn trigger_cache_images(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CacheImagesPayload>,
) -> Response {
    let has_names = payload.names.as_ref().is_some_and(|n| !n.is_empty());
    let has_addrs = payload.addresses.as_ref().is_some_and(|a| !a.is_empty());

    if !has_names && !has_addrs {
        return error_response(
            StatusCode::BAD_REQUEST,
            "At least one of 'names' or 'addresses' must be provided",
        );
    }

    let Some(wire_format) = parse_wire_format(&payload.format) else {
        return error_response(StatusCode::BAD_REQUEST, "Unsupported image format");
    };

    if payload.size == 0 || payload.size > MAX_IMAGE_DIMENSION {
        return error_response(
            StatusCode::BAD_REQUEST,
            "size must be an integer between 1 and 4096",
        );
    }

    let Some(filter_type) = parse_filter(&payload.filter) else {
        return error_response(StatusCode::BAD_REQUEST, "Unknown filter");
    };

    let quality = match wire_format {
        ImageWireFormat::Jpg => match payload.quality {
            None => 100,
            Some(q) if (1..=100).contains(&q) => q,
            Some(_) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "quality must be an integer between 1 and 100",
                );
            }
        },
        _ => 100,
    };

    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        run_cache_images_batch(
            state_clone,
            payload.names,
            payload.addresses,
            wire_format,
            payload.size,
            filter_type,
            quality,
        )
        .await;
    });

    StatusCode::ACCEPTED.into_response()
}

pub async fn album_metadata(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let json_opt = {
        let logic = state.logic.read().await;
        logic.album_json(&id)
    };
    if let Some(data) = json_opt {
        return (
            [(header::CONTENT_TYPE, "application/json")],
            data.to_string(),
        )
            .into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

pub async fn album_image(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let path_opt = {
        let logic = state.logic.read().await;
        logic.album_record(&id).and_then(|record| {
            let ip = record.main_image_path.as_deref()?;
            Some(if std::path::Path::new(ip).is_absolute() {
                PathBuf::from(ip)
            } else {
                record.path.join(ip)
            })
        })
    };

    if let Some(path) = path_opt {
        return serve_image(path).await;
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn serve_image(path: PathBuf) -> Response {
    if let Ok(mut file) = File::open(&path).await {
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).await.is_ok() {
            let mime = if path.extension().is_some_and(|e| e == "png") {
                "image/png"
            } else {
                "image/jpeg"
            };

            let cache_header = HeaderValue::from_static("public, max-age=86400");

            return (
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
                    (header::CACHE_CONTROL, cache_header),
                    (
                        header::ACCESS_CONTROL_ALLOW_ORIGIN,
                        HeaderValue::from_static("*"),
                    ),
                ],
                buf,
            )
                .into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}
