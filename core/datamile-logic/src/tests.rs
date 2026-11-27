use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use datamile_cache::EntityKey;
use datamile_lua_domain::LogicManifest;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::engine::{
    CachedImageLookup, EntityRecord, ImageSpecMeta, LogicEngine, PlayableLookupItem,
};
use crate::sort::{SortKey, SortOrder, value_to_sort_key};
use crate::text::tag_text;

const ALBUM: &str = "album";
const BOOK: &str = "book";

fn manifest() -> LogicManifest {
    let mut manifest: LogicManifest = serde_json::from_value(json!({
        "filters": { "owned": { "media": "album", "label": "Owned" } },
        "groupers": { "genre": { "media": "album", "label": "Genre" } },
        "orders": { "alpha": { "media": "album", "label": "A-Z" } },
        "libraries": { "all": { "media": "album", "label": "All" } },
        "shelves": { "recent": { "media": "album", "label": "Recent" } },
        "cabinets": {}
    }))
    .expect("manifest fixture parses");
    manifest.normalize();
    manifest
}

fn record(media_type: &str, id: &str, path: &str) -> EntityRecord {
    let lock = json!({ media_type: { "id": id } });
    let dict = json!({ "id": id, "tags": {}, "objects": {}, "images": {} });

    EntityRecord {
        media_type: media_type.to_string(),
        id: id.to_string(),
        path: PathBuf::from(path),
        raw_json: Arc::from(lock.to_string()),
        dict_raw: RawValue::from_string(dict.to_string()).expect("raw value"),
        search_texts: vec![id.to_lowercase()],
        playable_uris: vec![format!("{id}/01.flac")],
        image_namespaces: vec![("main".to_string(), format!("{id}-img"))],
    }
}

fn eval(library: &str, filter: &str, shelf: &str, genre: &str, title: &str) -> Value {
    json!({
        "libraries": { library: true },
        "filters": { filter: true },
        "shelves": { shelf: true },
        "groupers": { "genre": [genre] },
        "orders": { "alpha": title },
        "shelf_sorts": { shelf: title }
    })
}

fn playable(media_id: &str, index: usize, uri: &str) -> PlayableLookupItem {
    playable_for(ALBUM, media_id, index, uri)
}

fn playable_for(
    media_type: &str,
    media_id: &str,
    index: usize,
    uri: &str,
) -> PlayableLookupItem {
    PlayableLookupItem {
        media_type: media_type.to_string(),
        media_id: media_id.to_string(),
        node_index: index,
        node_path: vec![("track".to_string(), index)],
        path: uri.to_string(),
        meta: json!({ "title": "Track" }),
    }
}

/// A record whose own image namespaces point at one content address.
fn record_with_image(
    media_type: &str,
    id: &str,
    path: &str,
    address: &str,
) -> EntityRecord {
    EntityRecord {
        image_namespaces: vec![("main".to_string(), address.to_string())],
        ..record(media_type, id, path)
    }
}

fn spec() -> ImageSpecMeta {
    ImageSpecMeta {
        fit: "inside".to_string(),
        filter: "catmullrom".to_string(),
        target_width: 1080,
        target_height: 1080,
    }
}

/// One image upsert: address, rendering spec, and the absolute source path.
fn image(address: &str, source: &str) -> (String, ImageSpecMeta, PathBuf) {
    (address.to_string(), spec(), PathBuf::from(source))
}

fn engine_with_two_entities() -> LogicEngine {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());

    engine
        .upsert_entity(
            record(ALBUM, "a", "/library/a"),
            vec![playable("a", 0, "a/01.flac")],
            vec![image("a-img", "/library/a/cover.jpg")],
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("first entity ingests");
    engine
        .upsert_entity(
            record(ALBUM, "b", "/library/b"),
            vec![playable("b", 0, "b/01.flac")],
            vec![image("b-img", "/library/b/cover.jpg")],
            eval("all", "owned", "recent", "Jazz", "Alpha"),
        )
        .expect("second entity ingests");
    engine.build_cache();

    engine
}

/// A hot reload must re-evaluate the same payload the compile pass evaluated:
/// the inner `<media_type>` entity, not the `{ "<media_type>": ... }` lock
/// wrapper. The wrapper has no `tags`, so every match function misses and the
/// library view comes back empty.
#[test]
fn reload_manifest_evaluates_the_inner_entity() {
    let mut engine = engine_with_two_entities();
    let seen = RefCell::new(Vec::new());

    engine.reload_manifest(
        &PathBuf::from("/config/init.lua"),
        manifest(),
        &|_media_type, entity| {
            seen.borrow_mut().push(entity.clone());

            // Mimic a config library match: it reads the entity payload.
            if entity.get("id").is_none() {
                return Ok(json!({}));
            }
            Ok(eval("all", "owned", "recent", "Rock", "Beta"))
        },
    );

    assert_eq!(seen.borrow().len(), 2, "every entity is re-evaluated");
    for entity in seen.borrow().iter() {
        assert!(entity.get("id").is_some(), "lock wrapper leaked: {entity}");
        assert!(entity.get(ALBUM).is_none(), "lock wrapper leaked: {entity}");
    }

    assert_eq!(
        engine
            .request_view(ALBUM, "all", None, "alpha", None, None, SortOrder::Forward)
            .len(),
        2,
        "the library view must survive a reload"
    );
}

/// Tags keep their JSON type, so every consumer that renders one as text goes
/// through `tag_text` instead of asking for a string and falling back.
#[test]
fn scalar_tags_become_text() {
    assert_eq!(
        tag_text(Some(&json!("Ambient"))),
        Some("Ambient".to_string())
    );
    assert_eq!(tag_text(Some(&json!(1984))), Some("1984".to_string()));
    assert_eq!(tag_text(Some(&json!(true))), Some("true".to_string()));
}

#[test]
fn scalar_lists_are_joined() {
    assert_eq!(
        tag_text(Some(&json!(["Ambient", "Drone", 1994]))),
        Some("Ambient, Drone, 1994".to_string())
    );
}

/// Empty and non-scalar tags have no text form; callers supply their own fallback.
#[test]
fn tags_without_a_text_form_are_none() {
    for value in [
        None,
        Some(&json!("")),
        Some(&json!([])),
        Some(&json!({ "a": 1 })),
        Some(&json!(null)),
    ] {
        assert_eq!(tag_text(value), None, "value {value:?} produced text");
    }
}

/// Numbers sort numerically, strings alphanumerically, and mixed types order by
/// kind instead of panicking.
#[test]
fn sort_keys_order_by_kind_and_value() {
    assert!(value_to_sort_key(&json!(2)) < value_to_sort_key(&json!(10)));
    assert!(value_to_sort_key(&json!("track 2")) < value_to_sort_key(&json!("track 10")));
    assert!(value_to_sort_key(&json!(1)) < value_to_sort_key(&json!("a")));
    assert_eq!(
        value_to_sort_key(&json!(["a", 2])),
        SortKey::Tuple(vec![SortKey::String("a".to_string()), SortKey::Number(2)])
    );
}

/// Ingestion registers playable URIs and image addresses so lookups resolve.
#[test]
fn upsert_entity_registers_lookups() {
    let engine = engine_with_two_entities();

    assert!(engine.entity_record(ALBUM, "a").is_some());
    assert_eq!(
        engine.entity_id_by_path(ALBUM, &PathBuf::from("/library/a")),
        Some("a")
    );
    assert!(engine.playable_lookup.contains_key("a/01.flac"));
    assert!(engine.image_lookup.contains_key("a-img"));

    let dict = engine.entity_dict(ALBUM, "a").expect("dict exists");
    let parsed: Value = serde_json::from_str(dict.get()).expect("dict parses");
    assert_eq!(parsed["id"], json!("a"));
}

/// A view is the intersection of library, filter and search, then ordered.
#[test]
fn request_view_filters_sorts_and_reverses() {
    let engine = engine_with_two_entities();

    assert_eq!(
        engine.request_view(ALBUM, "all", None, "alpha", None, None, SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_view(ALBUM, "all", None, "alpha", None, None, SortOrder::Reverse),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        engine.request_view(
            ALBUM,
            "all",
            None,
            "alpha",
            Some("search"),
            Some("a"),
            SortOrder::Forward
        ),
        vec!["a".to_string()]
    );
    assert_eq!(
        engine.request_view(
            ALBUM,
            "all",
            None,
            "alpha",
            Some("genre"),
            Some("Jazz"),
            SortOrder::Forward
        ),
        vec!["b".to_string()]
    );
}

/// Search matches any indexed text, not only the id.
#[test]
fn search_uses_indexed_texts() {
    let mut engine = engine_with_two_entities();
    if let Some(index) = engine.types.get_mut(ALBUM)
        && let Some(uid) = index.id_to_uid.get("a").copied()
        && let Some(record) = index.entities.get_mut(&uid)
    {
        record.search_texts = vec!["beta artist".to_string()];
    }

    assert_eq!(
        engine.request_view(
            ALBUM,
            "all",
            None,
            "alpha",
            Some("search"),
            Some("artist"),
            SortOrder::Forward
        ),
        vec!["a".to_string()]
    );
}

/// Shelves keep their native order unless a named order is applied.
#[test]
fn request_shelf_view_honours_named_order() {
    let engine = engine_with_two_entities();

    assert_eq!(
        engine.request_shelf_view(ALBUM, "recent", None, SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_shelf_view(ALBUM, "recent", Some("alpha"), SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_shelf_view(ALBUM, "recent", None, SortOrder::Reverse),
        vec!["a".to_string(), "b".to_string()]
    );
}

/// Groupers are flat buckets: `{ value, label, count, children: [] }`, sorted
/// by the group key and reversed on demand.
#[test]
fn request_group_is_flat_and_sorted() {
    let engine = engine_with_two_entities();

    let result = engine.request_group(ALBUM, "all", None, "genre");

    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["value"], json!("Jazz"));
    assert_eq!(result[0]["label"], json!("Jazz"));
    assert_eq!(result[0]["count"], json!(1));
    assert_eq!(result[0]["children"], json!([]));
    assert!(result[0].get("sublabel").is_none());

    let mut reversed_manifest = manifest();
    reversed_manifest
        .groupers
        .get_mut("genre")
        .expect("grouper")
        .reverse = true;
    let mut reversed = engine_with_two_entities();
    reversed.manifest = reversed_manifest;
    reversed.build_cache();

    let reversed_result = reversed.request_group(ALBUM, "all", None, "genre");
    assert_eq!(reversed_result[0]["value"], json!("Rock"));
}

/// Removing by type and path also drops the playable and image lookups.
#[test]
fn removal_clears_lookups() {
    let mut engine = engine_with_two_entities();

    assert_eq!(
        engine.remove_entity_by_type_and_path(ALBUM, &PathBuf::from("/library/a")),
        Some("a".to_string())
    );
    assert!(!engine.playable_lookup.contains_key("a/01.flac"));
    assert!(!engine.image_lookup.contains_key("a-img"));

    engine.remove_entity_by_id(ALBUM, "b");
    assert!(engine.entity_record(ALBUM, "b").is_none());
    assert!(!engine.playable_lookup.contains_key("b/01.flac"));
}

/// Two media types may compile the same directory; removing one leaves the
/// other indexed.
#[test]
fn removal_by_type_and_path_keeps_the_sibling_media_type() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());

    engine
        .upsert_entity(
            record(ALBUM, "shared", "/library/shared"),
            Vec::new(),
            Vec::new(),
            eval("all", "owned", "recent", "Rock", "Shared"),
        )
        .expect("album ingests");
    engine
        .upsert_entity(
            record(BOOK, "shared", "/library/shared"),
            Vec::new(),
            Vec::new(),
            json!({ "libraries": { "all": true } }),
        )
        .expect("book ingests");

    assert_eq!(
        engine.remove_entity_by_type_and_path(ALBUM, &PathBuf::from("/library/shared")),
        Some("shared".to_string())
    );
    assert!(engine.entity_record(ALBUM, "shared").is_none());
    assert!(engine.entity_record(BOOK, "shared").is_some());
}

/// One file can be playable under several media types; every owner is kept and
/// only removal of the last owner drops the URI.
#[test]
fn playable_lookup_keeps_every_owning_media_type() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());
    let uri = "shared/01.flac";

    engine
        .upsert_entity(
            record(ALBUM, "shared", "/library/shared"),
            vec![playable_for("album", "shared", 0, uri)],
            Vec::new(),
            eval("all", "owned", "recent", "Rock", "Shared"),
        )
        .expect("album ingests");
    engine
        .upsert_entity(
            record("vinyl", "shared", "/library/shared"),
            vec![playable_for("vinyl", "shared", 0, uri)],
            Vec::new(),
            json!({}),
        )
        .expect("vinyl ingests");

    let owners: Vec<String> = engine.playable_lookup[uri]
        .iter()
        .map(|item| item.media_type.clone())
        .collect();
    assert_eq!(owners, vec!["album".to_string(), "vinyl".to_string()]);

    engine.remove_entity_by_type_and_path(ALBUM, &PathBuf::from("/library/shared"));
    let owners: Vec<String> = engine.playable_lookup[uri]
        .iter()
        .map(|item| item.media_type.clone())
        .collect();
    assert_eq!(owners, vec!["vinyl".to_string()]);

    engine.remove_entity_by_type_and_path("vinyl", &PathBuf::from("/library/shared"));
    assert!(!engine.playable_lookup.contains_key(uri));
}

/// Re-ingesting one entity replaces its own playable entry in place instead of
/// adding a duplicate beside another media type's owner.
#[test]
fn playable_lookup_replaces_one_owner_without_duplicates() {
    let mut engine = engine_with_two_entities();
    let uri = "a/01.flac";

    engine
        .upsert_entity(
            record(ALBUM, "a", "/library/a"),
            vec![playable_for("album", "a", 0, uri)],
            Vec::new(),
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("re-ingest");

    assert_eq!(engine.playable_lookup[uri].len(), 1);
}

/// Every owner of one image address shares a single spec and source list, and
/// a cold-cache read can fall back to any owner that still has the file.
#[test]
fn image_lookup_shares_one_address_between_owners() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());
    let address = "shared-img";

    engine
        .upsert_entity(
            record_with_image(ALBUM, "a", "/library/a", address),
            Vec::new(),
            vec![image(address, "/library/a/cover.jpg")],
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("first owner ingests");
    engine
        .upsert_entity(
            record_with_image(BOOK, "a", "/library/a", address),
            Vec::new(),
            vec![image(address, "/library/a/other.jpg")],
            json!({}),
        )
        .expect("second owner ingests");

    let sources: Vec<EntityKey> = engine.image_lookup[address]
        .sources
        .iter()
        .map(|(key, _)| key.clone())
        .collect();
    assert_eq!(
        sources,
        vec![
            EntityKey {
                media_type: ALBUM.to_string(),
                dir: PathBuf::from("/library/a"),
            },
            EntityKey {
                media_type: BOOK.to_string(),
                dir: PathBuf::from("/library/a"),
            },
        ]
    );

    engine.remove_entity_by_type_and_path(BOOK, &PathBuf::from("/library/a"));
    let surviving: &CachedImageLookup = &engine.image_lookup[address];
    assert_eq!(surviving.sources.len(), 1, "one owner remains");
    assert_eq!(surviving.spec, spec());

    engine.remove_entity_by_type_and_path(ALBUM, &PathBuf::from("/library/a"));
    assert!(!engine.image_lookup.contains_key(address));
}

/// Re-ingesting an owner updates its source path without adding a second
/// entry for the same entity key.
#[test]
fn image_lookup_replaces_one_owner_path_in_place() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());
    let address = "a-img";

    engine
        .upsert_entity(
            record_with_image(ALBUM, "a", "/library/a", address),
            Vec::new(),
            vec![image(address, "/library/a/cover.jpg")],
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("first ingest");
    engine
        .upsert_entity(
            record_with_image(ALBUM, "a", "/library/a", address),
            Vec::new(),
            vec![image(address, "/library/a/scan.jpg")],
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("second ingest");

    let sources = &engine.image_lookup[address].sources;
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].1, PathBuf::from("/library/a/scan.jpg"));
}

/// Two media types may share an id, path lookup and removal stay isolated, and
/// one type's logic never sees the other's entities.
#[test]
fn media_types_are_partitioned() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());

    engine
        .upsert_entity(
            record(ALBUM, "Dune", "/library/Dune"),
            Vec::new(),
            Vec::new(),
            eval("all", "owned", "recent", "Rock", "Dune"),
        )
        .expect("album ingests");
    engine
        .upsert_entity(
            record(BOOK, "Dune", "/books/Dune"),
            Vec::new(),
            Vec::new(),
            json!({ "libraries": { "all": true } }),
        )
        .expect("book ingests");
    engine.build_cache();

    assert_eq!(
        engine.entity_record(ALBUM, "Dune").expect("album").path,
        PathBuf::from("/library/Dune")
    );
    assert_eq!(
        engine.entity_record(BOOK, "Dune").expect("book").path,
        PathBuf::from("/books/Dune")
    );

    assert_eq!(
        engine.entity_id_by_path(BOOK, &PathBuf::from("/books/Dune")),
        Some("Dune")
    );
    assert!(
        engine
            .request_view(BOOK, "all", None, "alpha", None, None, SortOrder::Forward)
            .is_empty()
    );

    engine.remove_entity_by_id(BOOK, "Dune");
    assert!(engine.entity_record(BOOK, "Dune").is_none());
    assert!(engine.entity_record(ALBUM, "Dune").is_some());
}

/// Library caches only accept entities of the library's own media type.
#[test]
fn libraries_do_not_leak_across_types() {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());

    engine
        .upsert_entity(
            record(BOOK, "b1", "/books/b1"),
            Vec::new(),
            Vec::new(),
            json!({ "libraries": { "all": true }, "filters": { "owned": true } }),
        )
        .expect("book ingests");
    engine.build_cache();

    assert!(
        engine
            .request_view(BOOK, "all", None, "alpha", None, None, SortOrder::Forward)
            .is_empty(),
        "an album library must not match a book"
    );
    assert_eq!(
        engine.request_view(
            BOOK,
            "library",
            None,
            "alpha",
            None,
            None,
            SortOrder::Forward
        ),
        vec!["b1".to_string()],
        "the fallback library only contains the requested type"
    );
}

/// `find_ids` is scoped by media type as well.
#[test]
fn find_ids_is_scoped() {
    let mut engine = engine_with_two_entities();
    engine
        .upsert_entity(
            record(BOOK, "a", "/books/a"),
            Vec::new(),
            Vec::new(),
            json!({}),
        )
        .expect("book ingests");

    assert_eq!(engine.find_ids(ALBUM, "a"), vec!["a".to_string()]);
    assert_eq!(engine.find_ids(BOOK, "a"), vec!["a".to_string()]);
    assert_eq!(
        engine.media_types(),
        vec![(ALBUM.to_string(), 2), (BOOK.to_string(), 1)]
    );
}
