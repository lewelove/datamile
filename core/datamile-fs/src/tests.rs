use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{
    HashMode, ResolvedFileLocation, blake3_string, expand_path, file_meta,
    format_blake3_string, has_extension, is_matching_extension, json_to_toml,
    read_file_info, rel_path, resolve_file_location, resolve_path,
    scan_files_by_extension, toml_to_json, write_atomic,
};

/// `~` expands to the home directory, `~/x` appends to it, and everything else
/// is taken verbatim.
#[test]
fn expand_path_handles_tilde_and_plain_paths() {
    if let Some(home) = dirs::home_dir() {
        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/sub"), home.join("sub"));
    }

    assert_eq!(expand_path("/abs/path"), PathBuf::from("/abs/path"));
    assert_eq!(expand_path("rel/path"), PathBuf::from("rel/path"));
}

/// Absolute paths win, an existing direct child wins over a subdirectory, and a
/// dangling relative path falls back to the direct child.
#[test]
fn resolve_path_prefers_existing_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path();

    let absolute = config_dir.join("abs.toml");
    assert_eq!(
        resolve_path(absolute.to_str().unwrap(), config_dir),
        absolute
    );

    fs::write(config_dir.join("direct.toml"), "").unwrap();
    assert_eq!(
        resolve_path("direct.toml", config_dir),
        config_dir.join("direct.toml")
    );

    fs::create_dir(config_dir.join("nested")).unwrap();
    fs::write(config_dir.join("nested/deep.toml"), "").unwrap();
    assert_eq!(
        resolve_path("deep.toml", config_dir),
        config_dir.join("nested/deep.toml")
    );

    assert_eq!(
        resolve_path("missing.toml", config_dir),
        config_dir.join("missing.toml")
    );
}

/// A matching base is stripped; a foreign target is returned as its own string.
#[test]
fn rel_path_strips_matching_base_and_falls_back() {
    let base = Path::new("/music/library");

    assert_eq!(rel_path(Path::new("/music/library/album"), base), "album");
    assert_eq!(
        rel_path(Path::new("/elsewhere/album"), base),
        "/elsewhere/album"
    );
}

/// Missing parents are created and an existing file is replaced without
/// leaving stale bytes behind.
#[test]
fn write_atomic_creates_parents_and_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a/b/c.bin");

    write_atomic(&path, b"\x00\x01binary").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"\x00\x01binary");

    write_atomic(&path, b"shorter").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"shorter");
}

/// `HashMode::Skip` reports metadata only; `HashMode::Compute` adds the full
/// base64 digest and its 16 character URL-safe address.
#[test]
fn read_file_info_hashes_only_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, b"hello").unwrap();

    let skipped = read_file_info(&path, "a.txt", HashMode::Skip).unwrap();
    assert_eq!(skipped["path"], json!("a.txt"));
    assert_eq!(skipped["hash"], json!(null));
    assert_eq!(skipped["byte_size"], json!(5));

    let computed = read_file_info(&path, "a.txt", HashMode::Compute).unwrap();
    let hash = &computed["hash"];
    assert!(hash["string"].as_str().unwrap().starts_with("blake3-"));
    assert_eq!(hash["address"].as_str().unwrap().chars().count(), 16);
}

/// Digests are tagged and standard-base64 encoded, so the same bytes always
/// produce the same string.
#[test]
fn blake3_string_is_tagged_base64() {
    let digest = blake3_string(b"hello");

    assert!(digest.starts_with("blake3-"));
    // 32 raw bytes encode to 44 standard-base64 characters.
    assert_eq!(digest.len(), "blake3-".len() + 44);
    assert_eq!(
        digest,
        format_blake3_string(blake3::hash(b"hello").as_bytes())
    );
    assert_ne!(digest, blake3_string(b"other"));
}

/// Lock metadata is the byte size plus whole seconds since the epoch.
#[test]
fn file_meta_reports_size_and_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, b"hello").unwrap();

    let meta = fs::metadata(&path).unwrap();
    let info = file_meta(&meta).unwrap();

    assert_eq!(info.byte_size, 5);
    assert!(info.mtime > 0);
}

/// Extensions compare case-insensitively and a missing extension never matches.
#[test]
fn extension_matching_is_case_insensitive() {
    assert!(is_matching_extension("FLAC", &["flac", "mp3"]));
    assert!(!is_matching_extension("ogg", &["flac", "mp3"]));

    assert!(has_extension(Path::new("song.FLAC"), &["flac"]));
    assert!(!has_extension(Path::new("song.ogg"), &["flac"]));
    assert!(!has_extension(Path::new("no_extension"), &["flac"]));
}

/// Scanning walks subdirectories, filters by extension and orders files
/// naturally (`2.flac` before `10.flac`).
#[test]
fn scan_files_by_extension_is_recursive_and_natural() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("2.flac"), b"").unwrap();
    fs::write(dir.path().join("10.flac"), b"").unwrap();
    fs::write(dir.path().join("notes.txt"), b"").unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/3.FLAC"), b"").unwrap();

    let files = scan_files_by_extension(dir.path(), &["flac"]);
    let names: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    assert_eq!(names.len(), 3);
    assert!(!names.contains(&"notes.txt".to_string()));
    assert!(names.contains(&"sub/3.FLAC".to_string()));

    let two = names.iter().position(|n| n == "2.flac").unwrap();
    let ten = names.iter().position(|n| n == "10.flac").unwrap();
    assert!(two < ten, "{names:?} is not naturally ordered");
}

/// Files under the base report a relative path, anything else an absolute one,
/// and unusable references resolve to nothing.
#[test]
fn resolve_file_location_reports_containment() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("album");
    fs::create_dir_all(base.join("sub")).unwrap();
    fs::write(base.join("cover.jpg"), b"img").unwrap();
    let base_canon = base.canonicalize().unwrap();

    let inside = resolve_file_location("cover.jpg", &base, &base_canon).unwrap();
    assert_eq!(
        inside,
        ResolvedFileLocation::InsideBase {
            canonical: base_canon.join("cover.jpg"),
            rel_path: "cover.jpg".to_string(),
        }
    );

    let absolute = base.join("cover.jpg");
    assert!(matches!(
        resolve_file_location(absolute.to_str().unwrap(), &base, &base_canon),
        Some(ResolvedFileLocation::InsideBase { .. })
    ));

    let outside_dir = tempfile::tempdir().unwrap();
    let outside_path = outside_dir.path().join("cover.jpg");
    fs::write(&outside_path, b"img").unwrap();
    match resolve_file_location(outside_path.to_str().unwrap(), &base, &base_canon) {
        Some(ResolvedFileLocation::OutsideBase {
            canonical,
            abs_path,
        }) => {
            assert_eq!(canonical, outside_path.canonicalize().unwrap());
            assert_eq!(abs_path, canonical.to_string_lossy());
        }
        other => panic!("outside file resolved as {other:?}"),
    }

    assert!(resolve_file_location("missing.jpg", &base, &base_canon).is_none());
    assert!(resolve_file_location("sub", &base, &base_canon).is_none());
}

/// Converting TOML to JSON and back preserves scalars, nesting and arrays.
#[test]
fn toml_and_json_round_trip() {
    let source = "name = \"x\"\ncount = 3\nnested = { flag = true }\nitems = [1, 2]\n";
    let toml_val = toml::from_str::<toml::Value>(source).unwrap();

    let json_val = toml_to_json(toml_val);
    assert_eq!(json_val["name"], json!("x"));
    assert_eq!(json_val["nested"]["flag"], json!(true));

    assert_eq!(
        json_to_toml(json_val),
        toml::from_str::<toml::Value>(source).unwrap()
    );
}
