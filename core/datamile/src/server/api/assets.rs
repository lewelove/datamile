use std::collections::HashSet;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use libdatamile::images::{
    FilterType, ImageWireFormat, decode_qoi_to_rgb, encode_rgb_image, filter_type_to_str,
    parse_filter, parse_wire_format, read_qoi_dimensions, resize_image_fit,
    resize_image_max_edge,
};
use serde::Deserialize;
use serde_json::json;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

use crate::server::state::AppState;

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
    master_path: PathBuf,
    address: &str,
) -> Result<(), StatusCode> {
    if master_path.exists() {
        return Ok(());
    }

    let lookup_item = {
        let logic = state.logic.read().await;
        logic.cover_lookup.get(address).cloned()
    };

    let Some(item) = lookup_item else {
        return Err(StatusCode::NOT_FOUND);
    };

    let raw_path = std::path::Path::new(&item.path);
    let original_path = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        item.album_dir.join(raw_path)
    };

    let blob_path_clone = master_path.clone();

    let gen_result = tokio::task::spawn_blocking(move || {
        let img = image::open(&original_path).ok()?.into_rgb8();
        let resized = resize_image_fit(
            &img,
            item.target_width,
            item.target_height,
            item.fit_mode,
            item.filter,
        )?;

        if let Some(parent) = blob_path_clone.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }

        resized
            .save_with_format(&blob_path_clone, image::ImageFormat::Qoi)
            .ok()?;
        Some(())
    })
    .await;

    if !matches!(gen_result, Ok(Some(()))) {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(())
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

async fn read_master_dimension(
    master_path: &StdPath,
) -> Result<u32, (StatusCode, &'static str)> {
    let mut header_buf = [0u8; 14];
    let Ok(mut master_file) = File::open(master_path).await else {
        return Err((StatusCode::NOT_FOUND, "Failed to read master cover"));
    };

    if master_file.read_exact(&mut header_buf).await.is_err() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Invalid master cover header",
        ));
    }

    let Some((master_w, master_h)) = read_qoi_dimensions(&header_buf) else {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, "Invalid QOI dimensions"));
    };

    Ok(std::cmp::max(master_w, master_h))
}

fn resolve_image_params(
    m_dim: u32,
    query: &ImageQuery,
    wire_format: ImageWireFormat,
) -> Result<(u32, FilterType, u8), (StatusCode, &'static str)> {
    let resolved_size = match query.size {
        None => m_dim,
        Some(s) if (1..=4096).contains(&s) => s,
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

fn build_cache_filename(
    resolved_size: u32,
    m_dim: u32,
    filter: FilterType,
    quality: u8,
    wire_format: ImageWireFormat,
) -> String {
    let filter_suffix = if resolved_size == m_dim {
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

async fn render_in_memory(
    master_path: PathBuf,
    resolved_size: u32,
    m_dim: u32,
    filter: FilterType,
    wire_format: ImageWireFormat,
    quality: u8,
) -> Result<Vec<u8>, StatusCode> {
    tokio::task::spawn_blocking(move || {
        let master_bytes = std::fs::read(&master_path).ok()?;
        let mut img = decode_qoi_to_rgb(&master_bytes)?;

        if resolved_size != m_dim {
            img = resize_image_max_edge(&img, resolved_size, filter)?;
        }

        encode_rgb_image(&img, wire_format, quality)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    .and_then(|opt| opt.ok_or(StatusCode::INTERNAL_SERVER_ERROR))
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

    let master_dir = cache_root.join("images").join(address);
    let master_path = master_dir.join("master.qoi");

    if let Err(status) = ensure_master_image(&state, master_path.clone(), address).await {
        return error_response(status, "Cover not found");
    }

    let m_dimension = match read_master_dimension(&master_path).await {
        Ok(dim) => dim,
        Err((status, msg)) => return error_response(status, msg),
    };

    let (resolved_size, filter_type, quality) =
        match resolve_image_params(m_dimension, &query, wire_format) {
            Ok(params) => params,
            Err((status, msg)) => return error_response(status, msg),
        };

    let cache_filename = build_cache_filename(
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

    let cached_file_path = master_dir.join(&cache_filename);

    let bytes = if let Ok(data) = tokio::fs::read(&cached_file_path).await {
        data
    } else {
        match render_in_memory(
            master_path,
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

    let addr_filter: Option<HashSet<&str>> =
        addresses.map(|arr| arr.iter().map(String::as_str).collect());
    let name_filter: Option<HashSet<&str>> =
        names.map(|arr| arr.iter().map(String::as_str).collect());

    for record in logic.albums.values() {
        for (ns, addr) in &record.cover_namespaces {
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
    master_dir: PathBuf,
    master_path: PathBuf,
    target_path: PathBuf,
    params: (u32, FilterType, u8, ImageWireFormat),
) {
    let Some(guard) = state.cover_tasks.try_claim(target_path.clone()) else {
        return;
    };

    let (size, filter, quality, wire_format) = params;
    let target_path_clone = target_path.clone();

    let _ = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let master_bytes = std::fs::read(&master_path).ok()?;
        let (master_w, master_h) = read_qoi_dimensions(&master_bytes)?;
        let m_dim = std::cmp::max(master_w, master_h);

        let mut img = decode_qoi_to_rgb(&master_bytes)?;
        if size != m_dim {
            img = resize_image_max_edge(&img, size, filter)?;
        }

        let encoded = encode_rgb_image(&img, wire_format, quality)?;
        let _ = std::fs::create_dir_all(&master_dir);
        let _ = std::fs::write(&target_path_clone, encoded);
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
        let c = guard.app.compiler.jobs.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(4, std::num::NonZero::get)
        });
        (guard.cache_root.clone(), c)
    };

    let semaphore = Arc::new(Semaphore::new(concurrency));

    for address in target_addresses {
        let master_dir = cache_root.join("images").join(&address);
        let master_path = master_dir.join("master.qoi");

        if ensure_master_image(&state, master_path.clone(), &address)
            .await
            .is_err()
        {
            continue;
        }

        let Ok(m_dimension) = read_master_dimension(&master_path).await else {
            continue;
        };

        let cache_filename =
            build_cache_filename(size, m_dimension, filter, quality, wire_format);
        let target_path = master_dir.join(&cache_filename);

        if target_path.exists() {
            continue;
        }

        let sem_clone = Arc::clone(&semaphore);
        let state_clone = Arc::clone(&state);

        tokio::spawn(async move {
            let Ok(_permit) = sem_clone.acquire().await else {
                return;
            };
            process_cache_candidate(
                &state_clone,
                master_dir,
                master_path,
                target_path,
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

    if payload.size == 0 || payload.size > 4096 {
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

pub async fn entity_metadata(
    Path((root_type, id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let json_opt = {
        let logic = state.logic.read().await;
        logic.entity_json(&root_type, &id)
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
