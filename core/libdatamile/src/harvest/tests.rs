use super::*;

/// Cached harvest records carry chapter markers to Lua without re-probing files.
#[test]
fn caches_extracted_chapters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("book.m4b");
    fs::write(&path, b"fake audio").expect("write track");

    let metadata = fs::metadata(&path).expect("metadata");
    let canon = path.canonicalize().expect("canonicalize");
    let key = cache_key(&canon, &metadata);
    let chapters = vec![Chapter {
        title: "Intro".to_string(),
        start_ms: 0,
        end_ms: 1_000,
    }];
    let track = TrackJson {
        path: path.clone(),
        tags: HashMap::new(),
        physics: PhysicsData {
            file_size: metadata.len(),
            mtime: 0,
            duration_ms: 1_000,
            sample_rate: 44_100,
            bit_depth: Some(16),
            channels: 2,
            audio_bitrate: 900,
            overall_bitrate: 900,
            format: "flac".to_string(),
        },
        chapters: Some(chapters.clone()),
    };

    let harvest = tmp.path().join("cache/harvest");
    fs::create_dir_all(&harvest).expect("harvest dir");
    fs::write(
        harvest.join(format!("{key}.json")),
        serde_json::to_string(&track).expect("serialize"),
    )
    .expect("write cache");

    let cached =
        harvest_file_cached(&path, &tmp.path().join("cache")).expect("cached track");

    assert_eq!(cached.chapters, Some(chapters));
}
