use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use serde_json::json;

use crate::{CacheError, read_object_cached, read_or_compute_cached};

/// Stand-in compute step: the file contents, or the IO error.
fn read_text(path: &Path) -> std::io::Result<String> {
    fs::read_to_string(path)
}

/// Reading an object writes exactly one blob under `<cache_root>/objects`.
fn cached_object_path(cache_root: &Path) -> PathBuf {
    let mut entries: Vec<PathBuf> = fs::read_dir(cache_root.join("objects"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1, "expected a single cached object");
    entries.remove(0)
}

/// A computed value is stored under the namespace and served without calling
/// the compute step again.
#[test]
fn read_or_compute_cached_reuses_the_stored_value() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.txt");
    fs::write(&path, "first").unwrap();

    let calls = std::cell::Cell::new(0);
    let compute = |path: &Path| {
        calls.set(calls.get() + 1);
        read_text(path)
    };

    let value = read_or_compute_cached(&path, &cache_root, "docs", compute).unwrap();
    assert_eq!(value, "first");

    let again = read_or_compute_cached(&path, &cache_root, "docs", compute).unwrap();
    assert_eq!(again, "first");
    assert_eq!(calls.get(), 1, "second read must not recompute");
    assert_eq!(fs::read_dir(cache_root.join("docs")).unwrap().count(), 1);
}

/// Editing the file changes the key, so the value is recomputed.
#[test]
fn read_or_compute_cached_recomputes_after_modification() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.txt");
    fs::write(&path, "first").unwrap();

    read_or_compute_cached(&path, &cache_root, "docs", read_text).unwrap();

    sleep(Duration::from_millis(20));
    fs::write(&path, "second").unwrap();
    let value = read_or_compute_cached(&path, &cache_root, "docs", read_text).unwrap();

    assert_eq!(value, "second");
}

/// A blob that no longer deserializes is dropped and rebuilt.
#[test]
fn read_or_compute_cached_rebuilds_corrupt_blobs() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.txt");
    fs::write(&path, "payload").unwrap();

    read_or_compute_cached(&path, &cache_root, "docs", read_text).unwrap();

    let cache_file = fs::read_dir(cache_root.join("docs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(&cache_file, "{corrupt").unwrap();

    let value = read_or_compute_cached(&path, &cache_root, "docs", read_text).unwrap();
    assert_eq!(value, "payload");
    assert_eq!(fs::read_to_string(&cache_file).unwrap(), "\"payload\"");
}

/// JSON and TOML documents are parsed once and then served from their cache
/// entry without rewriting it.
#[test]
fn valid_documents_are_cached_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let json_path = dir.path().join("doc.json");
    fs::write(&json_path, r#"{"a": 1}"#).unwrap();

    assert_eq!(
        read_object_cached(&json_path, &cache_root).unwrap(),
        json!({ "a": 1 })
    );
    let cache_file = cached_object_path(&cache_root);
    let first_mtime = fs::metadata(&cache_file).unwrap().modified().unwrap();

    sleep(Duration::from_millis(20));
    assert_eq!(
        read_object_cached(&json_path, &cache_root).unwrap(),
        json!({ "a": 1 })
    );
    assert_eq!(
        fs::metadata(&cache_file).unwrap().modified().unwrap(),
        first_mtime,
        "cache hit must not rewrite the blob"
    );

    let toml_path = dir.path().join("doc.toml");
    fs::write(&toml_path, "a = 2\n").unwrap();
    assert_eq!(
        read_object_cached(&toml_path, &cache_root).unwrap(),
        json!({ "a": 2 })
    );
    assert_eq!(fs::read_dir(cache_root.join("objects")).unwrap().count(), 2);
}

/// Size and mtime are part of the cache key, so an edited file is re-parsed
/// under a fresh key.
#[test]
fn changed_size_or_mtime_invalidates_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.json");

    fs::write(&path, r#"{"a": 1}"#).unwrap();
    read_object_cached(&path, &cache_root).unwrap();

    sleep(Duration::from_millis(20));
    fs::write(&path, r#"{"a": 1, "b": 2}"#).unwrap();
    assert_eq!(
        read_object_cached(&path, &cache_root).unwrap(),
        json!({ "a": 1, "b": 2 })
    );
    assert_eq!(fs::read_dir(cache_root.join("objects")).unwrap().count(), 2);
}

/// An unreadable cache blob is dropped and rebuilt from the source document.
#[test]
fn corrupt_cache_entries_are_reparsed() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.json");
    fs::write(&path, r#"{"a": 1}"#).unwrap();

    read_object_cached(&path, &cache_root).unwrap();
    let cache_file = cached_object_path(&cache_root);
    fs::write(&cache_file, "not json").unwrap();

    assert_eq!(
        read_object_cached(&path, &cache_root).unwrap(),
        json!({ "a": 1 })
    );
    let repaired: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cache_file).unwrap()).unwrap();
    assert_eq!(repaired, json!({ "a": 1 }));
}

/// Whitespace-only documents carry no data and read as JSON null.
#[test]
fn empty_documents_read_as_null() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("empty.json");
    fs::write(&path, "  \n").unwrap();

    assert_eq!(read_object_cached(&path, &cache_root).unwrap(), json!(null));
    assert_eq!(read_object_cached(&path, &cache_root).unwrap(), json!(null));
}

/// Only JSON and TOML are cacheable documents.
#[test]
fn unsupported_extensions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let path = dir.path().join("doc.yaml");
    fs::write(&path, "a: 1").unwrap();

    let err = read_object_cached(&path, &cache_root).unwrap_err();
    assert!(matches!(
        err,
        CacheError::InvalidFileExtension { expected, .. } if expected == "json or toml"
    ));
}

/// A missing source file surfaces the underlying not-found IO error.
#[test]
fn missing_files_surface_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");

    let err =
        read_object_cached(&dir.path().join("missing.json"), &cache_root).unwrap_err();
    assert!(
        matches!(err, CacheError::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound)
    );
}

/// Malformed documents report which parser failed.
#[test]
fn malformed_documents_report_the_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");

    let json_path = dir.path().join("bad.json");
    fs::write(&json_path, "{oops").unwrap();
    assert!(matches!(
        read_object_cached(&json_path, &cache_root).unwrap_err(),
        CacheError::JsonParse { .. }
    ));

    let toml_path = dir.path().join("bad.toml");
    fs::write(&toml_path, "= nope").unwrap();
    assert!(matches!(
        read_object_cached(&toml_path, &cache_root).unwrap_err(),
        CacheError::TomlParse { .. }
    ));
}
