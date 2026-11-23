use std::collections::HashMap;

use serde_json::json;

use super::handler::filter_reingested_ids;

/// Verify that an album ID present in both removal and update sets is purged from removals.
///
/// Moving or renaming a directory generates an inotify removal for the old path and an
/// addition for the new path in the same batch. Since metadata IDs are identical, the ID
/// must not be broadcast as removed.
#[test]
fn test_filter_purges_reingested_id() {
    let album_id = "Artist - 2024 - Album".to_string();

    // Stale removal and fresh re-ingestion for the same album ID.
    let mut removed_ids = vec![album_id.clone()];
    let updated_entries = HashMap::from([(album_id.clone(), json!({ "id": album_id }))]);

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    // The re-ingested ID must be filtered out of the removal list.
    assert!(removed_ids.is_empty());
}

/// Verify that genuinely removed album IDs absent from the update set are retained.
#[test]
fn test_filter_retains_unmatched_id() {
    let removed_id = "Artist - 2020 - Deleted".to_string();
    let reingested_id = "Artist - 2024 - Renamed".to_string();

    let mut removed_ids = vec![removed_id.clone(), reingested_id.clone()];
    let updated_entries =
        HashMap::from([(reingested_id.clone(), json!({ "id": reingested_id }))]);

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    // Only the genuinely removed ID must remain.
    assert_eq!(removed_ids, vec![removed_id]);
}

/// Verify that empty input sets return early without modification.
#[test]
fn test_filter_noop_on_empty_inputs() {
    let mut removed_ids = Vec::new();
    let updated_entries: HashMap<String, serde_json::Value> = HashMap::new();

    filter_reingested_ids(&mut removed_ids, &updated_entries);

    assert!(removed_ids.is_empty());
}
