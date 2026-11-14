use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::{Value, json};

use super::{EntityIngest, LogicEngine, SortOrder};

/// One process-lifetime config keeps parallel tests off each other's temp dirs.
fn install_test_config() {
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();

    let path = CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join("datamile-logic-tests");
        fs::create_dir_all(&dir).expect("config dir");
        let path = dir.join("init.lua");
        fs::write(&path, "datamile.config({})").expect("config");
        libdatamile::lua::set_config_path_override(path.clone());

        path
    });

    assert!(path.exists());
}

fn ingest(
    engine: &mut LogicEngine,
    root_type: &str,
    dir: &std::path::Path,
    eval_res: Value,
) {
    let lock_path = dir.join("entity.lock.json");
    fs::write(&lock_path, b"{}").expect("lock");
    let lock_json = json!({
        root_type: {
            "id": "Dune",
            "tags": {},
            "objects": {},
            "images": {},
        }
    })
    .to_string();

    let media_roots = vec![dir.to_path_buf()];
    engine
        .ingest_entity(EntityIngest {
            root_type,
            target: dir,
            lock_path: &lock_path,
            id: "Dune",
            lock_json: &lock_json,
            eval_res,
            media_roots: &media_roots,
        })
        .expect("ingest");
}

/// Ids are unique within a root type, not across root types.
#[test]
fn logic_scopes_ids_by_root_type() {
    install_test_config();

    let tmp = tempfile::tempdir().expect("tempdir");
    let film = tmp.path().join("film");
    let audiobook = tmp.path().join("audiobook");
    let other_film = tmp.path().join("other-film");
    for dir in [&film, &audiobook, &other_film] {
        fs::create_dir_all(dir).expect("dir");
    }

    let mut engine = LogicEngine::new().expect("engine");
    ingest(&mut engine, "film", &film, json!({}));
    ingest(&mut engine, "audiobook", &audiobook, json!({}));
    assert_eq!(engine.albums.len(), 2);

    let film_record = engine.entity("film", "Dune").expect("film");
    assert_eq!(film_record.path, film.canonicalize().expect("canon"));
    assert!(engine.entity("album", "Dune").is_none());

    let ids = vec!["Dune".to_string()];
    let sections = vec!["tags".to_string()];
    let items = engine.view_entries("film", &ids, Some(&sections));
    assert_eq!(items[0]["film"]["id"], "Dune");
    assert!(items[0]["film"].get("objects").is_none());
    assert!(engine.view_entries("album", &ids, None).is_empty());

    let lock_path = other_film.join("entity.lock.json");
    fs::write(&lock_path, b"{}").expect("lock");
    let lock_json = json!({
        "film": { "id": "Dune", "tags": {}, "objects": {}, "images": {} }
    })
    .to_string();
    let media_roots = vec![tmp.path().to_path_buf()];

    let err = engine
        .ingest_entity(EntityIngest {
            root_type: "film",
            target: &other_film,
            lock_path: &lock_path,
            id: "Dune",
            lock_json: &lock_json,
            eval_res: json!({}),
            media_roots: &media_roots,
        })
        .expect_err("duplicate film id");

    assert!(err.to_string().contains("Duplicate Media ID"));
}

/// Re-ingesting an unchanged entity replaces it; views must not duplicate it.
#[test]
fn logic_replaces_reingested_entity() {
    install_test_config();

    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("album");
    fs::create_dir_all(&dir).expect("album dir");

    let eval = json!({ "shelves": { "recent": true } });
    let mut engine = LogicEngine::new().expect("engine");
    ingest(&mut engine, "album", &dir, eval.clone());
    ingest(&mut engine, "album", &dir, eval);

    engine.build_cache();

    let ids = engine.request_shelf_view("album", "recent", None, SortOrder::Forward);
    assert_eq!(ids, vec!["Dune".to_string()]);
}
