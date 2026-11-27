use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use datamile_cache::stat::LOCK_SUFFIX;
use datamile_logic::LogicEngine;
use datamile_lua_domain::{ActionCtx, ActionEntry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::ServerState;

const UPDATE_HINT: &str = "run 'datamile update'";

#[derive(Debug, Deserialize)]
pub struct XRequest {
    pub name: String,
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub media: Vec<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default = "default_true")]
    pub execute: bool,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct XRun {
    pub media: Option<String>,
    pub dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl XRun {
    /// A generic action with no target: `lock` and `media` stay absent.
    const fn targetless() -> Self {
        Self {
            media: None,
            dir: None,
            lock: None,
            error: None,
        }
    }

    fn target(dir: PathBuf, media: String) -> Self {
        Self {
            dir: Some(dir),
            media: Some(media),
            ..Self::targetless()
        }
    }

    const fn failed(dir: Option<PathBuf>, media: Option<String>, error: String) -> Self {
        Self {
            dir,
            media,
            lock: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct XResponse {
    pub runs: Vec<XRun>,
}

/// `POST /api/x`: resolve the requested targets against the daemon's index and
/// either run the action here (`execute: true`) or hand the resolved locks back
/// to the CLI (`execute: false`).
pub async fn run(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    payload: Result<Json<XRequest>, JsonRejection>,
) -> Response {
    let request = match payload {
        Ok(Json(request)) => request,
        Err(rejection) => {
            return error_response(json_error_status(&rejection), &rejection.body_text());
        }
    };

    let name = request.name.replace('-', "_");
    let (action, media_types) = {
        let config = state.config.read().await;
        (
            config.actions.get(&name).cloned(),
            config.media_types.clone(),
        )
    };

    let Some(action) = action else {
        return error_response(
            StatusCode::NOT_FOUND,
            &format!("Action '{name}' is not registered."),
        );
    };

    if !request.execute && is_browser(&headers) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "execute=false is restricted to CLI callers",
        );
    }

    if let Some(unknown) = request
        .media
        .iter()
        .find(|media_type| !media_types.contains(media_type))
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!("Media type '{unknown}' is not registered."),
        );
    }

    if let Some(raw) = request
        .dirs
        .iter()
        .find(|dir| !Path::new(dir).is_absolute())
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "'{raw}' is not an absolute path; expand '~' and '.' before sending."
            ),
        );
    }

    let mut runs = if request.dirs.is_empty() {
        if !request.media.is_empty() {
            return error_response(StatusCode::BAD_REQUEST, "media requires dirs");
        }
        if action.media.is_some() {
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Action '{name}' is scoped to a media type; pass dirs."),
            );
        }

        vec![XRun::targetless()]
    } else {
        let logic = state.logic.read().await;
        resolve_targets(&logic, &action, &request)
    };

    for run in &mut runs {
        attach_lock(run);
    }

    if request.execute {
        execute_runs(&state, &name, &request.options, &mut runs).await;
    }

    Json(XResponse { runs }).into_response()
}

/// A non-JSON body is `415`; anything else the extractor rejects is `422`.
const fn json_error_status(rejection: &JsonRejection) -> StatusCode {
    if matches!(rejection, JsonRejection::MissingJsonContentType(_)) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }

    StatusCode::UNPROCESSABLE_ENTITY
}

/// Resolve every `(dir, media)` pair in argument order. A failing pair becomes
/// a run with an `error` and does not stop the remaining pairs.
fn resolve_targets(
    logic: &LogicEngine,
    action: &ActionEntry,
    request: &XRequest,
) -> Vec<XRun> {
    let mut runs = Vec::new();

    for raw in &request.dirs {
        let Ok(dir) = std::fs::canonicalize(raw) else {
            runs.extend(dir_errors(raw, &request.media));
            continue;
        };

        if request.media.is_empty() {
            runs.push(resolve_default_media(logic, action, &dir));
            continue;
        }

        for media_type in &request.media {
            runs.push(entity_run(logic, &dir, media_type));
        }
    }

    runs
}

/// One missing directory yields one error per requested media type, or a single
/// error when the media type is left to the resolver.
fn dir_errors(raw: &str, media: &[String]) -> Vec<XRun> {
    let message = format!("target directory '{raw}' does not exist; {UPDATE_HINT}");
    let dir = Some(PathBuf::from(raw));

    if media.is_empty() {
        return vec![XRun::failed(dir, None, message)];
    }

    media
        .iter()
        .map(|media_type| {
            XRun::failed(dir.clone(), Some(media_type.clone()), message.clone())
        })
        .collect()
}

fn resolve_default_media(logic: &LogicEngine, action: &ActionEntry, dir: &Path) -> XRun {
    let indexed = indexed_types(logic, dir);

    let Some(declared) = &action.media else {
        return match indexed.as_slice() {
            [] => no_indexed(dir),
            [only] => entity_run(logic, dir, only),
            _ => ambiguous(dir, &indexed),
        };
    };

    let matches: Vec<String> = indexed
        .iter()
        .filter(|media_type| declared.contains_key(*media_type))
        .cloned()
        .collect();

    match matches.as_slice() {
        [] if indexed.is_empty() => no_indexed(dir),
        [] => no_matching(dir, &indexed),
        [only] => entity_run(logic, dir, only),
        _ => ambiguous(dir, &matches),
    }
}

fn indexed_types(logic: &LogicEngine, dir: &Path) -> Vec<String> {
    logic
        .media_types()
        .into_iter()
        .filter(|(media_type, _)| logic.entity_id_by_path(media_type, dir).is_some())
        .map(|(media_type, _)| media_type)
        .collect()
}

fn entity_run(logic: &LogicEngine, dir: &Path, media_type: &str) -> XRun {
    if logic.entity_id_by_path(media_type, dir).is_some() {
        return XRun::target(dir.to_path_buf(), media_type.to_string());
    }

    XRun::failed(
        Some(dir.to_path_buf()),
        Some(media_type.to_string()),
        format!(
            "entity ('{media_type}', '{}') is not in the entity index; {UPDATE_HINT}",
            dir.display()
        ),
    )
}

fn no_indexed(dir: &Path) -> XRun {
    XRun::failed(
        Some(dir.to_path_buf()),
        None,
        format!("no indexed entity in '{}'; {UPDATE_HINT}", dir.display()),
    )
}

fn ambiguous(dir: &Path, types: &[String]) -> XRun {
    XRun::failed(
        Some(dir.to_path_buf()),
        None,
        format!(
            "ambiguous target at '{}' ({}); pass --media",
            dir.display(),
            types.join(", ")
        ),
    )
}

fn no_matching(dir: &Path, indexed: &[String]) -> XRun {
    XRun::failed(
        Some(dir.to_path_buf()),
        None,
        format!(
            "no matching entity at '{}' (indexed: {}); pass --media to select explicitly",
            dir.display(),
            indexed.join(", ")
        ),
    )
}

/// Reads `<dir>/<media>.lock.json` into the run. A missing or broken lock is a
/// run error, so the caller sees one failing target instead of a failed request.
fn attach_lock(run: &mut XRun) {
    if run.error.is_some() {
        return;
    }

    let (Some(dir), Some(media_type)) = (&run.dir, &run.media) else {
        return;
    };
    let path = dir.join(format!("{media_type}{LOCK_SUFFIX}"));

    match std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        Some(lock) => run.lock = Some(lock),
        None => {
            run.error = Some(format!(
                "lock file '{}' is missing or unreadable; {UPDATE_HINT}",
                path.display()
            ));
        }
    }
}

async fn execute_runs(
    state: &Arc<ServerState>,
    name: &str,
    options: &[String],
    runs: &mut [XRun],
) {
    for run in runs {
        if run.error.is_some() {
            continue;
        }

        let ctx = ActionCtx {
            // `execute: true` omits the lock from the response.
            lock: run.lock.take(),
            media: run.media.clone(),
            options: options.to_vec(),
            isatty: false,
        };

        if let Err(error) = execute(state, name, &ctx).await {
            log::error!("Action '{name}' failed: {error}");
            run.error = Some(error.to_string());
        }
    }
}

async fn execute(
    state: &Arc<ServerState>,
    name: &str,
    ctx: &ActionCtx,
) -> anyhow::Result<()> {
    let backend = Arc::clone(&state.backend);
    let name = name.to_string();
    let ctx = ctx.clone();

    tokio::task::spawn_blocking(move || backend.execute_action(&name, &ctx))
        .await
        .context("action task panicked")?
}

/// A browser always sends `Origin` or `Sec-Fetch-Site`; `datamile x` sends
/// neither.
fn is_browser(headers: &HeaderMap) -> bool {
    headers.contains_key(header::ORIGIN) || headers.contains_key("sec-fetch-site")
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}
