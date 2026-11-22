use serde_json::json;

use super::tag_text;

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
