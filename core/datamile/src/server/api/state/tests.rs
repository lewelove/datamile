use std::path::Path;

use serde_json::json;

use super::{merge, namespace_file, read_namespace};

/// Namespaces become files under `<state_root>/state`.
#[test]
fn namespace_maps_to_state_file() {
    let path = namespace_file(Path::new("/state"), "web-app").expect("valid");
    assert_eq!(path, Path::new("/state/state/web-app.json"));
}

/// Path traversal and separators are rejected.
#[test]
fn namespace_rejects_unsafe_names() {
    for name in ["", "../escape", "a/b", "a.b", "a b"] {
        assert!(
            namespace_file(Path::new("/state"), name).is_none(),
            "{name}"
        );
    }
}

/// A missing namespace file reads as an empty object.
#[test]
fn missing_namespace_reads_empty() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let value = read_namespace(&tmp.path().join("missing.json"));
    assert_eq!(value, json!({}));
}

/// Patches merge object keys and replace non-object roots.
#[test]
fn merge_combines_objects() {
    let mut current = json!({ "activeTab": "home", "nested": { "a": 1 } });

    merge(&mut current, json!({ "activeTab": "queue", "extra": true }));
    assert_eq!(current["activeTab"], "queue");
    assert_eq!(current["extra"], true);
    assert_eq!(current["nested"], json!({ "a": 1 }));

    merge(&mut current, json!([]));
    assert_eq!(current, json!([]));
}
