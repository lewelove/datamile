use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use datamile_logic::{EntityRecord, LogicEngine};
use datamile_lua_domain::{FormattedGrouperResult, GrouperFormatContext, LogicManifest};
use datamile_mpd::MpdEngine;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::backend::{ActionTargetQuery, MediaBackend, PlaybackSlice, UpdateRequest};
use crate::state::{ImageClaim, ImageTaskRegistry, ServerConfig, ServerState};

fn lock_json(id: &str, tracks: &[(&str, Value)]) -> Value {
    let track_nodes: Vec<Value> = tracks
        .iter()
        .map(|(path, extra)| {
            let mut track = json!({
                "location": { "kind": "file", "path": path, "mtime": 1, "byte_size": 2 },
                "tags": {}
            });
            if let Some(extra) = extra.as_object() {
                for (k, v) in extra {
                    track[k] = v.clone();
                }
            }
            track
        })
        .collect();

    json!({
        "album": {
            "id": id,
            "location": { "kind": "dir", "path": ".", "mtime": 1 },
            "tags": {},
            "objects": {},
            "images": {},
            "nodes": { "track": track_nodes }
        }
    })
}

fn record(id: &str, lock: &Value) -> EntityRecord {
    EntityRecord {
        media_type: "album".to_string(),
        id: id.to_string(),
        path: PathBuf::from("/library").join(id),
        title: id.to_string(),
        subtitle: String::new(),
        main_image_path: None,
        raw_json: Arc::from(lock.to_string()),
        dict_raw: RawValue::from_string(
            json!({ "id": id, "tags": {}, "objects": {}, "images": {} }).to_string(),
        )
        .expect("raw dict"),
        total_discs: 1,
        total_tracks: 1,
        duration_ms: 0,
        playable_uris: Vec::new(),
        image_namespaces: Vec::new(),
    }
}

fn engine_with(ids: &[(&str, Value)]) -> LogicEngine {
    let mut engine =
        LogicEngine::new(PathBuf::from("/config/init.lua"), LogicManifest::default());
    for (id, lock) in ids {
        engine
            .upsert_entity(record(id, lock), Vec::new(), Vec::new(), json!({}))
            .expect("entity ingests");
    }
    engine
}

struct MockBackend;

impl MediaBackend for MockBackend {
    fn scan_library(&self, _logic: &mut LogicEngine) {}

    fn reload_config_and_logic(
        &self,
        _logic: &mut LogicEngine,
    ) -> anyhow::Result<ServerConfig> {
        Ok(ServerConfig {
            music_directory: PathBuf::from("/library"),
            cache_root: PathBuf::from("/cache"),
            state_root: PathBuf::from("/state"),
            resolved_dependencies: Vec::new(),
            interfaces: HashMap::new(),
            actions: HashMap::from([(
                "open_folder".to_string(),
                datamile_lua_domain::ActionDef {
                    label: "Open folder".to_string(),
                    description: None,
                },
            )]),
            config_dir: PathBuf::from("/config"),
            config_path: PathBuf::from("/config/init.lua"),
            compiler_jobs: None,
        })
    }

    fn resolve_playback_uris(
        &self,
        _media_type: &str,
        _id: &str,
        _slice: PlaybackSlice,
        _logic: &LogicEngine,
    ) -> Vec<String> {
        Vec::new()
    }

    fn resolve_action_targets(
        &self,
        _query: &ActionTargetQuery,
        _logic: &LogicEngine,
        _playing_file: Option<&str>,
    ) -> Vec<Value> {
        Vec::new()
    }

    fn execute_action(&self, _name: &str, _payload: &Value) -> anyhow::Result<()> {
        Ok(())
    }

    fn format_grouper(
        &self,
        _grouper: &str,
        ctx: &GrouperFormatContext<'_>,
    ) -> FormattedGrouperResult {
        FormattedGrouperResult {
            label: ctx.value.to_string(),
            sublabel: None,
            sort: json!(ctx.value),
            parent: None,
        }
    }

    fn handle_fs_events<'a>(
        &'a self,
        _paths: &'a [PathBuf],
        _state: &'a Arc<ServerState>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn extra_watch_dirs(&self, _state: &ServerState) -> (Vec<PathBuf>, Vec<PathBuf>) {
        (Vec::new(), Vec::new())
    }

    fn trigger_update(
        &self,
        _state: Arc<ServerState>,
        _request: UpdateRequest,
        _log_tx: Option<tokio::sync::mpsc::Sender<String>>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn force_update_entity(
        &self,
        _state: Arc<ServerState>,
        _media_type: &str,
        _id: &str,
    ) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn tracked_changes(&self) -> (bool, Vec<String>) {
        (false, Vec::new())
    }
}

async fn test_state(logic: LogicEngine) -> Arc<ServerState> {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(1);

    Arc::new(ServerState {
        logic: Arc::new(tokio::sync::RwLock::new(logic)),
        ui_state: tokio::sync::RwLock::new(json!({})),
        tx: broadcast_tx,
        config: tokio::sync::RwLock::new(
            MockBackend
                .reload_config_and_logic(&mut LogicEngine::new(
                    PathBuf::new(),
                    LogicManifest::default(),
                ))
                .expect("config"),
        ),
        mpd_engine: MpdEngine::new(tx),
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: tokio::sync::RwLock::new(String::new()),
        backend: Arc::new(MockBackend),
    })
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

/// `None` keeps the file verbatim.
#[test]
fn depth_none_keeps_the_full_tree() {
    let mut lock = lock_json("a", &[("01.flac", json!({}))]);
    let before = lock.clone();

    crate::api::tree::apply_depth(&mut lock, "album", None);

    assert_eq!(lock, before);
}

/// `depth=0` drops the root's nodes.
#[test]
fn depth_zero_drops_root_nodes() {
    let mut lock = lock_json("a", &[("01.flac", json!({}))]);

    crate::api::tree::apply_depth(&mut lock, "album", Some(0));

    assert!(lock["album"].get("nodes").is_none());
    assert_eq!(lock["album"]["id"], json!("a"));
}

/// `depth=1` keeps `nodes.track` and drops their children.
#[test]
fn depth_one_keeps_track_nodes() {
    let mut lock = lock_json("a", &[("01.flac", json!({}))]);

    crate::api::tree::apply_depth(&mut lock, "album", Some(1));

    assert_eq!(
        lock["album"]["nodes"]["track"][0]["location"]["path"],
        json!("01.flac")
    );
    assert!(lock["album"]["nodes"]["track"][0].get("nodes").is_none());
}

/// `depth=2` keeps one more level of children.
#[test]
fn depth_two_keeps_grandchildren() {
    let mut lock = lock_json(
        "a",
        &[(
            "01.flac",
            json!({ "nodes": { "chapter": [{ "tags": {} }] } }),
        )],
    );

    crate::api::tree::apply_depth(&mut lock, "album", Some(2));

    assert!(
        lock["album"]["nodes"]["track"][0]["nodes"]["chapter"][0]
            .get("nodes")
            .is_none()
    );
}

/// A lock without `nodes` is left untouched at every depth.
#[test]
fn missing_nodes_is_a_no_op() {
    let mut lock = json!({ "album": { "id": "a" } });

    crate::api::tree::apply_depth(&mut lock, "album", Some(3));

    assert_eq!(lock, json!({ "album": { "id": "a" } }));
}

/// A lock without the requested media key is left untouched.
#[test]
fn missing_media_key_is_a_no_op() {
    let mut lock = json!({ "other": { "nodes": {} } });
    let before = lock.clone();

    crate::api::tree::apply_depth(&mut lock, "album", Some(0));

    assert_eq!(lock, before);
}

/// One owner renders a path; the next waiter is told when the owner finishes.
#[test]
fn image_registry_singleflights_and_notifies() {
    let registry = Arc::new(ImageTaskRegistry::default());
    let path = PathBuf::from("/cache/master.qoi");

    let Some(ImageClaim::Claimed(guard)) = registry.claim(path.clone()) else {
        panic!("first claim must own the path");
    };

    let Some(ImageClaim::InFlight(mut rx)) = registry.claim(path.clone()) else {
        panic!("second claim must wait");
    };

    drop(guard);

    assert!(*rx.borrow_and_update());
    assert!(matches!(registry.claim(path), Some(ImageClaim::Claimed(_))));
}

#[tokio::test]
async fn router_lists_media_types_and_items() {
    let state = test_state(engine_with(&[(
        "a",
        lock_json("a", &[("01.flac", json!({}))]),
    )]))
    .await;
    let app = crate::api::router(state);

    let response = app
        .clone()
        .oneshot(Request::get("/api/media").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["types"]["album"], json!(1));

    let response = app
        .oneshot(
            Request::get("/api/media/album?q=a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await[0]["id"], json!("a"));
}

#[tokio::test]
async fn router_returns_the_literal_lock_with_depth() {
    let state = test_state(engine_with(&[(
        "a",
        lock_json("a", &[("01.flac", json!({}))]),
    )]))
    .await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::get("/api/media/album/a?depth=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert!(body["album"].get("nodes").is_none());
    assert_eq!(body["album"]["location"]["kind"], json!("dir"));
}

#[tokio::test]
async fn router_rejects_an_empty_image_cache_request() {
    let state = test_state(engine_with(&[])).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::post("/api/images/cache")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"format":"bmp","size":200,"filter":"lanczos"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn router_returns_404_for_an_unknown_action() {
    let state = test_state(engine_with(&[])).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::post("/api/actions/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
