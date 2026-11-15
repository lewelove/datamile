use super::write_atomic_cache_file;

/// Binary payloads (encoded images) must round-trip unchanged, not just UTF-8 text.
#[test]
fn writes_binary_payload_atomically() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("nested").join("cover.bmp");
    let payload: Vec<u8> = vec![0x42, 0x4d, 0x00, 0xff, 0x80];

    write_atomic_cache_file(&target, &payload).expect("write");

    assert_eq!(std::fs::read(&target).expect("read"), payload);
}

/// Leaves no half-written temp file behind once the target exists.
#[test]
fn leaves_only_target_in_directory() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("cover.bmp");

    write_atomic_cache_file(&target, b"first").expect("write");
    write_atomic_cache_file(&target, b"second").expect("overwrite");

    assert_eq!(std::fs::read(&target).expect("read"), b"second");
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 1);
}
