use std::collections::HashSet;

use serde_json::Value;

pub fn compress(
    mut raw_tracks: Vec<serde_json::Map<String, Value>>,
) -> (
    serde_json::Map<String, Value>,
    Vec<serde_json::Map<String, Value>>,
) {
    if raw_tracks.is_empty() {
        return (serde_json::Map::new(), Vec::new());
    }

    let first_track = &raw_tracks[0];
    let mut candidate_keys: HashSet<String> = first_track.keys().cloned().collect();

    for track in raw_tracks.iter().skip(1) {
        candidate_keys.retain(|k| track.contains_key(k));
    }

    let mut album_pool = serde_json::Map::new();
    let mut keys_to_promote = Vec::new();

    let forced_track_keys: HashSet<&str> =
        ["tracknumber", "discnumber", "title"].into_iter().collect();

    for key in candidate_keys {
        let is_identical = raw_tracks
            .iter()
            .all(|t| t.get(&key) == first_track.get(&key));

        if is_identical {
            if forced_track_keys.contains(key.as_str()) {
                continue;
            }
            keys_to_promote.push(key.clone());
            album_pool.insert(
                key.clone(),
                first_track
                    .get(&key)
                    .expect("key is read from the first track")
                    .clone(),
            );
        }
    }

    for track in &mut raw_tracks {
        for k in &keys_to_promote {
            track.remove(k);
        }
    }

    (album_pool, raw_tracks)
}
