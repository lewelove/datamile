use super::{EntityIngest, LogicEngine};
use serde_json::json;
use std::fs;

fn ingest(engine: &mut LogicEngine, root_type: &str, dir: &std::path::Path) {
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

    engine
        .ingest_entity(EntityIngest {
            root_type,
            target: dir,
            lock_path: &lock_path,
            id: "Dune",
            lock_json: &lock_json,
            eval_res: json!({}),
            media_directory: dir,
        })
        .expect("ingest");
}

/// Ids are globally unique across root types (Law 2).
#[test]
fn logic_rejects_duplicate_id() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = tmp.path().join("init.lua");
    fs::write(&config, "datamile.config({})").expect("config");
    libdatamile::lua::set_config_path_override(config);

    let film = tmp.path().join("film");
    let audiobook = tmp.path().join("audiobook");
    fs::create_dir_all(&film).expect("film dir");
    fs::create_dir_all(&audiobook).expect("audiobook dir");

    let mut engine = LogicEngine::new().expect("engine");
    ingest(&mut engine, "film", &film);

    let lock_path = audiobook.join("entity.lock.json");
    fs::write(&lock_path, b"{}").expect("lock");
    let lock_json = json!({
        "audiobook": { "id": "Dune", "tags": {}, "objects": {}, "images": {} }
    })
    .to_string();

    let err = engine
        .ingest_entity(EntityIngest {
            root_type: "audiobook",
            target: &audiobook,
            lock_path: &lock_path,
            id: "Dune",
            lock_json: &lock_json,
            eval_res: json!({}),
            media_directory: tmp.path(),
        })
        .expect_err("duplicate id");

    assert!(err.to_string().contains("Duplicate Media ID"));
}
