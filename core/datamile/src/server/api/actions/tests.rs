use serde_json::json;

use super::ActionBody;

/// A single `id` and the `ids` array collapse into one deduplicated list.
#[test]
fn action_body_merges_ids() {
    let body: ActionBody = serde_json::from_value(json!({
        "type": "album",
        "id": "a",
        "ids": ["b", "a"]
    }))
    .expect("body");

    assert_eq!(body.requested_ids(), vec!["b".to_string(), "a".to_string()]);
}

/// Body-less actions are valid; targets simply stay empty.
#[test]
fn action_body_defaults_to_empty() {
    let body: ActionBody = serde_json::from_value(json!({})).expect("body");
    assert!(body.requested_ids().is_empty());
    assert_eq!(body.options, None);
}

/// Options carry trailing CLI-style arguments untouched.
#[test]
fn action_body_reads_options() {
    let body: ActionBody =
        serde_json::from_value(json!({ "options": "--force" })).expect("body");
    assert_eq!(body.options.as_deref(), Some("--force"));
}
