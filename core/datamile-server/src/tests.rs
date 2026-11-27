use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use datamile_logic::{EntityRecord, ImageSpecMeta, LogicEngine, PlayableLookupItem};
use datamile_lua_domain::LogicManifest;
use datamile_mpd::MpdEngine;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::backend::{ActionTargetQuery, MediaBackend, PlaybackSlice, UpdateRequest};
use crate::state::{ImageClaim, ImageTaskRegistry, ServerConfig, ServerState};

const ALBUM: &str = "album";
const BOOK: &str = "book";

fn lock_json(media_type: &str, id: &str, tracks: &[(&str, Value)]) -> Value {
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
        (media_type): {
            "id": id,
            "location": { "kind": "dir", "path": "/lib/a" },
            "tags": {},
            "objects": {},
            "images": {},
            "nodes": { "track": track_nodes }
        }
    })
}

fn record(media_type: &str, id: &str, lock: &Value) -> EntityRecord {
    EntityRecord {
        media_type: media_type.to_string(),
        id: id.to_string(),
        path: PathBuf::from("/library").join(media_type).join(id),
        raw_json: Arc::from(lock.to_string()),
        dict_raw: RawValue::from_string(
            json!({ "id": id, "tags": {}, "objects": {}, "images": {} }).to_string(),
        )
        .expect("raw dict"),
        search_texts: vec![id.to_lowercase()],
        playable_uris: Vec::new(),
        image_namespaces: Vec::new(),
    }
}

fn playable(media_type: &str, media_id: &str, uri: &str) -> PlayableLookupItem {
    PlayableLookupItem {
        media_type: media_type.to_string(),
        media_id: media_id.to_string(),
        node_index: 0,
        node_path: vec![("track".to_string(), 0)],
        path: uri.to_string(),
        meta: json!({ "title": "Track" }),
    }
}

fn image_spec() -> ImageSpecMeta {
    ImageSpecMeta {
        fit: "inside".to_string(),
        filter: "catmullrom".to_string(),
        target_width: 16,
        target_height: 16,
    }
}

/// A tiny PNG so the cold-cache image path has a real source to render.
fn write_png(path: &std::path::Path) {
    let img = image::RgbImage::from_fn(8, 8, |x, y| {
        image::Rgb([
            u8::try_from(x).unwrap_or(0),
            u8::try_from(y).unwrap_or(0),
            0x40,
        ])
    });
    img.save_with_format(path, image::ImageFormat::Png)
        .expect("png fixture");
}

fn manifest() -> LogicManifest {
    let mut manifest: LogicManifest = serde_json::from_value(json!({
        "filters": { "owned": { "media": "album", "label": "Owned" } },
        "groupers": { "genre": { "media": "album", "label": "Genre" } },
        "orders": { "alpha": { "media": "album", "label": "A-Z" } },
        "libraries": {
            "all": { "media": "album", "label": "All" },
            "books": { "media": "book", "label": "Books" }
        },
        "shelves": { "recent": { "media": "album", "label": "Recent" } },
        "cabinets": {}
    }))
    .expect("manifest fixture");
    manifest.normalize();
    manifest
}

fn engine_with(entries: &[(&str, &str, Value)]) -> LogicEngine {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());
    for (media_type, id, eval) in entries {
        engine
            .upsert_entity(
                record(
                    media_type,
                    id,
                    &lock_json(media_type, id, &[("01.flac", json!({}))]),
                ),
                Vec::new(),
                Vec::new(),
                eval.clone(),
            )
            .expect("entity ingests");
    }
    engine.build_cache();
    engine
}

fn album_eval() -> Value {
    json!({
        "libraries": { "all": true },
        "filters": { "owned": true },
        "shelves": { "recent": true },
        "groupers": { "genre": ["Rock"] },
        "orders": { "alpha": "a" },
        "shelf_sorts": { "recent": "a" }
    })
}

struct MockBackend {
    payloads: Arc<Mutex<Vec<Value>>>,
    /// Makes `trigger_update` fail, as a pass that cannot start does.
    update_fails: bool,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            payloads: Arc::new(Mutex::new(Vec::new())),
            update_fails: false,
        }
    }

    fn failing() -> Self {
        Self {
            update_fails: true,
            ..Self::new()
        }
    }
}

impl MediaBackend for MockBackend {
    fn scan_library(&self, _logic: &mut LogicEngine) {}

    fn reload_config_and_logic(
        &self,
        _logic: &mut LogicEngine,
    ) -> anyhow::Result<ServerConfig> {
        Ok(ServerConfig {
            watch_roots: Vec::new(),
            mpd: datamile_mpd::MpdConfig::default(),
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
            port: crate::DEFAULT_PORT,
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
        query: &ActionTargetQuery,
        _logic: &LogicEngine,
        _playing_file: Option<&str>,
    ) -> Vec<Value> {
        let media_type = query
            .media_type
            .clone()
            .unwrap_or_else(|| ALBUM.to_string());
        query.id.as_ref().map_or_else(Vec::new, |id| {
            vec![json!({
                "type": media_type,
                "id": id,
                "path": format!("/library/{media_type}/{id}"),
                "lock": {}
            })]
        })
    }

    fn execute_action(&self, _name: &str, payload: &Value) -> anyhow::Result<()> {
        self.payloads
            .lock()
            .expect("payloads")
            .push(payload.clone());
        Ok(())
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
        _event_txs: Vec<tokio::sync::mpsc::Sender<String>>,
    ) -> anyhow::Result<()> {
        if self.update_fails {
            anyhow::bail!("the compile pool could not start");
        }

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
}

async fn test_state(logic: LogicEngine) -> Arc<ServerState> {
    test_state_with_backend(logic, Arc::new(MockBackend::new())).await
}

#[allow(
    clippy::unused_async,
    reason = "async keeps every test-state helper callable with `.await`, like its callers"
)]
async fn test_state_with_backend(
    logic: LogicEngine,
    backend: Arc<MockBackend>,
) -> Arc<ServerState> {
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let (broadcast_tx, _) = tokio::sync::broadcast::channel(1);

    Arc::new(ServerState {
        logic: Arc::new(tokio::sync::RwLock::new(logic)),
        ui_state: tokio::sync::RwLock::new(json!({})),
        tx: broadcast_tx,
        config: tokio::sync::RwLock::new(
            MockBackend::new()
                .reload_config_and_logic(&mut LogicEngine::new(
                    PathBuf::new(),
                    LogicManifest::default(),
                ))
                .expect("config"),
        ),
        mpd_engine: MpdEngine::new(tx),
        image_tasks: Arc::new(ImageTaskRegistry::default()),
        now_playing: tokio::sync::RwLock::new("Artist/Album/01.flac".to_string()),
        reload_error: tokio::sync::RwLock::new(None),
        backend,
    })
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json body")
}

/// Object keys in the order the payload serialized them.
fn object_keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

/// `None` keeps the file verbatim.
#[test]
fn depth_none_keeps_the_full_tree() {
    let mut lock = lock_json(ALBUM, "a", &[("01.flac", json!({}))]);
    let before = lock.clone();

    crate::api::tree::apply_depth(&mut lock, ALBUM, None);

    assert_eq!(lock, before);
}

/// `depth=0` drops the root's nodes.
#[test]
fn depth_zero_drops_root_nodes() {
    let mut lock = lock_json(ALBUM, "a", &[("01.flac", json!({}))]);

    crate::api::tree::apply_depth(&mut lock, ALBUM, Some(0));

    assert!(lock[ALBUM].get("nodes").is_none());
    assert_eq!(lock[ALBUM]["id"], json!("a"));
}

/// `depth=1` keeps `nodes.track` and drops their children.
#[test]
fn depth_one_keeps_track_nodes() {
    let mut lock = lock_json(ALBUM, "a", &[("01.flac", json!({}))]);

    crate::api::tree::apply_depth(&mut lock, ALBUM, Some(1));

    assert_eq!(
        lock[ALBUM]["nodes"]["track"][0]["location"]["path"],
        json!("01.flac")
    );
    assert!(lock[ALBUM]["nodes"]["track"][0].get("nodes").is_none());
}

/// `depth=2` keeps one more level of children.
#[test]
fn depth_two_keeps_grandchildren() {
    let mut lock = lock_json(
        ALBUM,
        "a",
        &[(
            "01.flac",
            json!({ "nodes": { "chapter": [{ "tags": {} }] } }),
        )],
    );

    crate::api::tree::apply_depth(&mut lock, ALBUM, Some(2));

    assert!(
        lock[ALBUM]["nodes"]["track"][0]["nodes"]["chapter"][0]
            .get("nodes")
            .is_none()
    );
}

/// A lock without `nodes` is left untouched at every depth.
#[test]
fn missing_nodes_is_a_no_op() {
    let mut lock = json!({ "album": { "id": "a" } });

    crate::api::tree::apply_depth(&mut lock, ALBUM, Some(3));

    assert_eq!(lock, json!({ "album": { "id": "a" } }));
}

/// A lock without the requested media key is left untouched.
#[test]
fn missing_media_key_is_a_no_op() {
    let mut lock = json!({ "other": { "nodes": {} } });
    let before = lock.clone();

    crate::api::tree::apply_depth(&mut lock, ALBUM, Some(0));

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
    let state = test_state(engine_with(&[(ALBUM, "a", album_eval())])).await;
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
    let state = test_state(engine_with(&[(ALBUM, "a", album_eval())])).await;
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
    assert!(body[ALBUM].get("nodes").is_none());
    assert_eq!(body[ALBUM]["location"]["kind"], json!("dir"));
}

/// The whole-library snapshot and the type listing sort their keys, so the same
/// state always serializes to the same bytes instead of `HashMap` iteration order.
#[tokio::test]
async fn init_payload_and_type_list_sort_keys() {
    let state = test_state(engine_with(&[
        (ALBUM, "b", album_eval()),
        (ALBUM, "a", album_eval()),
        (BOOK, "c", json!({ "libraries": { "books": true } })),
    ]))
    .await;

    let init = crate::api::websocket::init_payload(&state).await;
    let init: Value = serde_json::from_str(&init).expect("init json");
    assert_eq!(object_keys(&init["media"]), [ALBUM, BOOK]);
    assert_eq!(object_keys(&init["media"][ALBUM]), ["a", "b"]);

    let response = crate::api::media::list_types(axum::extract::State(state)).await;
    let body = body_json(response).await;
    assert_eq!(object_keys(&body["types"]), [ALBUM, BOOK]);
}

/// The same id under two media types resolves to each type's own lock.
#[tokio::test]
async fn router_scopes_media_by_type() {
    let state = test_state(engine_with(&[
        (ALBUM, "Dune", album_eval()),
        (BOOK, "Dune", json!({ "libraries": { "books": true } })),
    ]))
    .await;
    let app = crate::api::router(state);

    let album = app
        .clone()
        .oneshot(
            Request::get("/api/media/album/Dune")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(album.status(), StatusCode::OK);
    assert!(body_json(album).await.get(ALBUM).is_some());

    let book = app
        .oneshot(
            Request::get("/api/media/book/Dune")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(book.status(), StatusCode::OK);
    assert!(body_json(book).await.get(BOOK).is_some());
}

/// The current file is exposed so the CLI never needs an MPD client.
#[tokio::test]
async fn router_reports_current_playback() {
    let state = test_state(engine_with(&[])).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::get("/api/playback/current")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_json(response).await["file"],
        json!("Artist/Album/01.flac")
    );
}

/// Actions receive `{ items, options, isatty }`, scoped by the `type` param.
#[tokio::test]
async fn router_passes_items_and_type_to_actions() {
    let backend = Arc::new(MockBackend::new());
    let payloads = Arc::clone(&backend.payloads);
    let state = test_state_with_backend(
        engine_with(&[(ALBUM, "a", album_eval())]),
        Arc::clone(&backend),
    )
    .await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::post("/api/actions/open_folder?type=album&id=a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let payload = {
        let recorded = payloads.lock().expect("payloads");
        recorded.first().expect("payload recorded").clone()
    };
    assert_eq!(payload["items"][0]["type"], json!(ALBUM));
    assert_eq!(payload["items"][0]["id"], json!("a"));
    assert_eq!(payload["isatty"], json!(false));
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

/// One file playable under several media types reports the `album` owner to
/// MPD until `datamile-mpv` replaces the bridge.
#[test]
fn mpd_bridge_prefers_the_album_owner() {
    let mut engine = engine_with(&[]);
    engine
        .upsert_entity(
            record(
                ALBUM,
                "a",
                &lock_json(ALBUM, "a", &[("01.flac", json!({}))]),
            ),
            vec![playable(ALBUM, "a", "a/01.flac")],
            Vec::new(),
            album_eval(),
        )
        .expect("album ingests");
    engine
        .upsert_entity(
            record(
                "vinyl",
                "a",
                &lock_json("vinyl", "a", &[("01.flac", json!({}))]),
            ),
            vec![playable("vinyl", "a", "a/01.flac")],
            Vec::new(),
            json!({}),
        )
        .expect("vinyl ingests");

    let item = crate::mpd_bridge::track_meta(&engine, "/a/01.flac").expect("track");

    assert_eq!(item.media_type, ALBUM);
}

/// A cold-cache image read falls back to the next owner that still has its
/// source file on disk.
#[tokio::test]
async fn image_cold_cache_recovers_from_a_surviving_owner() {
    let dir = tempfile::tempdir().expect("temp");
    let cache_root = dir.path().join("cache");
    let gone = dir.path().join("gone.png");
    let alive = dir.path().join("alive.png");
    write_png(&alive);

    let address = "shared-address";
    let mut engine = engine_with(&[]);
    engine
        .upsert_entity(
            record(ALBUM, "a", &lock_json(ALBUM, "a", &[])),
            Vec::new(),
            vec![(address.to_string(), image_spec(), gone)],
            json!({}),
        )
        .expect("first owner ingests");
    engine
        .upsert_entity(
            record(BOOK, "a", &lock_json(BOOK, "a", &[])),
            Vec::new(),
            vec![(address.to_string(), image_spec(), alive)],
            json!({}),
        )
        .expect("second owner ingests");

    let state = test_state(engine).await;
    state.config.write().await.cache_root = cache_root.clone();

    let response = crate::api::router(state)
        .oneshot(
            Request::get(format!("/api/images/{address}.png"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        cache_root
            .join("images")
            .join(address)
            .join("master.qoi")
            .exists(),
        "the surviving owner renders the master"
    );
}

/// INIT carries both media types, even when they share an id.
#[tokio::test]
async fn init_payload_groups_media_by_type() {
    let state = test_state(engine_with(&[
        (ALBUM, "Dune", album_eval()),
        (BOOK, "Dune", json!({ "libraries": { "books": true } })),
    ]))
    .await;

    let payload: Value =
        serde_json::from_str(&crate::api::websocket::init_payload(&state).await)
            .expect("init payload");

    assert_eq!(payload["type"], json!("INIT"));
    assert_eq!(payload["media"][ALBUM]["Dune"]["id"], json!("Dune"));
    assert_eq!(payload["media"][BOOK]["Dune"]["id"], json!("Dune"));
    assert_eq!(payload["shelves"]["recent"], json!(["Dune"]));
}

/// View, group and shelf responses are scoped by `media_type` and omit
/// `sublabel`.
#[tokio::test]
async fn websocket_requests_are_scoped_by_media_type() {
    let state = test_state(engine_with(&[
        (ALBUM, "Dune", album_eval()),
        (BOOK, "Dune", json!({ "libraries": { "books": true } })),
    ]))
    .await;

    let album_view = crate::api::websocket::view_response(
        &state,
        &json!({ "type": "VIEW_REQUEST", "media_type": "album", "library": "all" }),
    )
    .await;
    assert_eq!(album_view["media_type"], json!(ALBUM));
    assert_eq!(album_view["ids"], json!(["Dune"]));

    let book_view = crate::api::websocket::view_response(
        &state,
        &json!({ "type": "VIEW_REQUEST", "media_type": "book", "library": "books" }),
    )
    .await;
    assert_eq!(book_view["media_type"], json!(BOOK));
    assert_eq!(book_view["ids"], json!(["Dune"]));

    let group = crate::api::websocket::group_response(
        &state,
        &json!({
            "type": "GROUP_REQUEST",
            "media_type": "album",
            "library": "all",
            "key": "genre"
        }),
    )
    .await;
    assert_eq!(group["media_type"], json!(ALBUM));
    assert_eq!(group["result"][0]["value"], json!("Rock"));
    assert_eq!(group["result"][0]["children"], json!([]));
    assert!(group["result"][0].get("sublabel").is_none());

    let shelf = crate::api::websocket::shelf_response(
        &state,
        &json!({ "type": "SHELF_REQUEST", "media_type": "album", "shelf": "recent" }),
    )
    .await;
    assert_eq!(shelf["media_type"], json!(ALBUM));
    assert_eq!(shelf["ids"], json!(["Dune"]));
}

/// `/api/identity` is what a CLI compares before delegating: the config it
/// serves, its version, and whether its last reload failed.
#[tokio::test]
async fn router_reports_identity() {
    let state = test_state(engine_with(&[])).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(Request::get("/api/identity").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["config_path"], json!("/config/init.lua"));
    assert_eq!(body["version"], json!(crate::VERSION));
    assert_eq!(body["reload_error"], json!(null));
}

/// `/api/select` resolves action targets from the body, using the daemon's
/// already-built index instead of a local rescan.
#[tokio::test]
async fn router_selects_targets_from_the_request_body() {
    let state = test_state(engine_with(&[(ALBUM, "a", album_eval())])).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::post("/api/select")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"media_type":"album","id":"a"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let items = body_json(response).await;
    assert_eq!(items[0]["type"], json!(ALBUM));
    assert_eq!(items[0]["id"], json!("a"));
}

/// A pass that cannot start still ends the stream with `Finished`, so the CLI
/// reports the failure instead of a truncated stream.
#[tokio::test]
async fn update_stream_reports_a_start_failure_as_events() {
    let state =
        test_state_with_backend(engine_with(&[]), Arc::new(MockBackend::failing())).await;
    let app = crate::api::router(state);

    let response = app
        .oneshot(
            Request::post("/api/update")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"force":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8(bytes.to_vec()).expect("utf8");
    let events: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON event per line"))
        .collect();

    assert_eq!(events.len(), 2, "{text}");
    assert_eq!(events[0]["event"], json!("failed"));
    assert_eq!(
        events[0]["error"],
        json!("the compile pool could not start")
    );
    assert_eq!(events[1]["event"], json!("finished"));
    assert_eq!(events[1]["failed"], json!(1));
}
