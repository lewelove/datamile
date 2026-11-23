use std::path::PathBuf;
use std::sync::Arc;

use datamile_lua_domain::{GrouperFormatContext, LogicManifest};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::engine::{EntityRecord, ImageLookupItem, LogicEngine, PlayableLookupItem};
use crate::sort::{SortKey, SortOrder, value_to_sort_key};
use crate::text::tag_text;

fn manifest() -> LogicManifest {
    let mut manifest: LogicManifest = serde_json::from_value(json!({
        "filters": { "owned": { "label": "Owned" } },
        "groupers": { "genre": { "label": "Genre" } },
        "orders": { "alpha": { "label": "A-Z" } },
        "libraries": { "all": { "label": "All" } },
        "shelves": { "recent": { "label": "Recent" } },
        "cabinets": {}
    }))
    .expect("manifest fixture parses");
    manifest.normalize();
    manifest
}

fn record(id: &str, path: &str, title: &str, subtitle: &str) -> EntityRecord {
    let lock = json!({ "album": { "id": id, "tags": { "album": title } } });
    let dict = json!({ "id": id, "tags": {}, "objects": {}, "images": {} });

    EntityRecord {
        media_type: "album".to_string(),
        id: id.to_string(),
        path: PathBuf::from(path),
        title: title.to_string(),
        subtitle: subtitle.to_string(),
        main_image_path: None,
        raw_json: Arc::from(lock.to_string()),
        dict_raw: RawValue::from_string(dict.to_string()).expect("raw value"),
        total_discs: 1,
        total_tracks: 2,
        duration_ms: 1000,
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
    PlayableLookupItem {
        media_type: "album".to_string(),
        media_id: media_id.to_string(),
        node_index: index,
        track_no: u32::try_from(index + 1).unwrap_or(1),
        disc_no: 1,
        title: "Track".to_string(),
        artist: "Artist".to_string(),
        duration: "1:00".to_string(),
        duration_ms: 60_000,
        path: uri.to_string(),
    }
}

fn image(_address: &str) -> ImageLookupItem {
    ImageLookupItem {
        entity_dir: PathBuf::from("/library/a"),
        path: "cover.jpg".to_string(),
        fit: "inside".to_string(),
        filter: "catmullrom".to_string(),
        target_width: 1080,
        target_height: 1080,
    }
}

fn engine_with_two_entities() -> LogicEngine {
    let mut engine = LogicEngine::new(PathBuf::from("/config/init.lua"), manifest());

    engine
        .upsert_entity(
            record("a", "/library/a", "Beta", "Zed"),
            vec![playable("a", 0, "a/01.flac")],
            vec![("a-img".to_string(), image("a-img"))],
            eval("all", "owned", "recent", "Rock", "Beta"),
        )
        .expect("first entity ingests");
    engine
        .upsert_entity(
            record("b", "/library/b", "Alpha", "Aaa"),
            vec![playable("b", 0, "b/01.flac")],
            vec![("b-img".to_string(), image("b-img"))],
            eval("all", "owned", "recent", "Jazz", "Alpha"),
        )
        .expect("second entity ingests");
    engine.build_cache(&crate::engine::plain_grouper_formatter);

    engine
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

    assert!(engine.entity_record("a").is_some());
    assert_eq!(
        engine.entity_id_by_path(&PathBuf::from("/library/a")),
        Some("a")
    );
    assert!(engine.playable_lookup.contains_key("a/01.flac"));
    assert!(engine.image_lookup.contains_key("a-img"));

    let dict = engine.entity_dict("a").expect("dict exists");
    let parsed: Value = serde_json::from_str(dict.get()).expect("dict parses");
    assert_eq!(parsed["id"], json!("a"));
}

/// A view is the intersection of library, filter and search, then ordered.
#[test]
fn request_view_filters_sorts_and_reverses() {
    let engine = engine_with_two_entities();

    assert_eq!(
        engine.request_view("all", None, "alpha", None, None, SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_view("all", None, "alpha", None, None, SortOrder::Reverse),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        engine.request_view(
            "all",
            None,
            "alpha",
            Some("search"),
            Some("zed"),
            SortOrder::Forward
        ),
        vec!["a".to_string()]
    );
    assert_eq!(
        engine.request_view(
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

/// Shelves keep their native order unless a named order is applied.
#[test]
fn request_shelf_view_honours_named_order() {
    let engine = engine_with_two_entities();

    assert_eq!(
        engine.request_shelf_view("recent", None, SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_shelf_view("recent", Some("alpha"), SortOrder::Forward),
        vec!["b".to_string(), "a".to_string()]
    );
    assert_eq!(
        engine.request_shelf_view("recent", None, SortOrder::Reverse),
        vec!["a".to_string(), "b".to_string()]
    );
}

/// The grouper formatter is caller-supplied, so a custom label and parent flow
/// straight into the tree.
#[test]
fn request_group_uses_custom_formatter() {
    let engine = engine_with_two_entities();

    let formatter = |_: &str, ctx: &GrouperFormatContext<'_>| {
        datamile_lua_domain::FormattedGrouperResult {
            label: format!("{}!", ctx.value),
            sublabel: None,
            sort: json!(ctx.value),
            parent: None,
        }
    };

    let result = engine.request_group("all", None, "genre", &formatter);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0]["label"], json!("Jazz!"));
    assert_eq!(result[1]["label"], json!("Rock!"));
}

/// Removing by path or id also drops the playable and image lookups.
#[test]
fn removal_clears_lookups() {
    let mut engine = engine_with_two_entities();

    assert_eq!(
        engine.remove_entity_by_path(&PathBuf::from("/library/a")),
        Some("a".to_string())
    );
    assert!(!engine.playable_lookup.contains_key("a/01.flac"));
    assert!(!engine.image_lookup.contains_key("a-img"));

    engine.remove_entity_by_id("b");
    assert!(engine.entity_record("b").is_none());
    assert!(!engine.playable_lookup.contains_key("b/01.flac"));
}
