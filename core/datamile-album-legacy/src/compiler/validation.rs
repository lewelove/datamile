use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use crate::compiler::manifest::extract_strict_u32;
use crate::error::DatamileError;

pub fn validate_track_indices(
    entries: &[Value],
    root: &Path,
) -> Result<(), DatamileError> {
    let mut seen_ids = HashSet::new();
    for (idx, entry) in entries.iter().enumerate() {
        let t = extract_strict_u32(entry.get("tracknumber"), "tracknumber", None)
            .map_err(|_| DatamileError::MissingTrackIdentity {
                manifest: "album.toml".to_string(),
                path: root.to_path_buf(),
                index: idx + 1,
            })?;
        let d = extract_strict_u32(entry.get("discnumber"), "discnumber", Some(1))?;

        if !seen_ids.insert((d, t)) {
            return Err(DatamileError::DuplicateTrackIdentity {
                manifest: "album.toml".to_string(),
                path: root.to_path_buf(),
                disc: d,
                track: t,
            });
        }
    }
    Ok(())
}
