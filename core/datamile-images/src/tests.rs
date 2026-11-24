use std::path::{Path, PathBuf};

use image::RgbImage;

use crate::{
    FilterType, FitMode, ImageSpec, ImageWireFormat, calculate_rendered_dimensions,
    compute_image_address, derivative_filename, master_path, read_master_dimension,
    render_derivative_from_master, render_master_cached, render_master_from_file,
};

/// 64x32 gradient source: a 2:1 aspect ratio that makes `Inside` scaling
/// visible.
fn write_source(dir: &Path, name: &str) -> PathBuf {
    let img = RgbImage::from_fn(64, 32, |x, y| {
        image::Rgb([u8::try_from(x).unwrap(), u8::try_from(y).unwrap(), 0x40])
    });
    let path = dir.join(name);
    img.save_with_format(&path, image::ImageFormat::Png)
        .unwrap();
    path
}

fn inside_spec() -> ImageSpec {
    ImageSpec {
        fit_mode: FitMode::Inside,
        target_width: 32,
        target_height: 32,
        filter_type: FilterType::CatmullRom,
    }
}

/// The address is stable for identical input and changes with every scoped
/// field.
#[test]
fn compute_image_address_is_deterministic_and_field_sensitive() {
    let hash = [7u8; 32];
    let base = compute_image_address(FitMode::Crop, 100, 50, FilterType::Lanczos3, &hash);

    assert_eq!(base.len(), 16);
    assert_eq!(
        base,
        compute_image_address(FitMode::Crop, 100, 50, FilterType::Lanczos3, &hash)
    );

    assert_ne!(
        base,
        compute_image_address(FitMode::Inside, 100, 50, FilterType::Lanczos3, &hash)
    );
    assert_ne!(
        base,
        compute_image_address(FitMode::Crop, 101, 50, FilterType::Lanczos3, &hash)
    );
    assert_ne!(
        base,
        compute_image_address(FitMode::Crop, 100, 50, FilterType::Box, &hash)
    );
    assert_ne!(
        base,
        compute_image_address(FitMode::Crop, 100, 50, FilterType::Lanczos3, &[8u8; 32])
    );
}

/// `Crop` fills the target box; `Inside` keeps the source aspect ratio.
#[test]
fn calculate_rendered_dimensions_follows_the_fit_mode() {
    assert_eq!(
        calculate_rendered_dimensions(1600, 900, 400, 400, FitMode::Crop),
        (400, 400)
    );
    assert_eq!(
        calculate_rendered_dimensions(1600, 900, 400, 400, FitMode::Inside),
        (400, 225)
    );
}

/// A master rendered from disk can be probed and encoded into every wire
/// format, with filenames matching the cache contract.
#[test]
fn master_and_derivative_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let source = write_source(dir.path(), "source.png");
    let cache_root = dir.path().join("cache");

    let master = render_master_from_file(&source, &inside_spec(), &cache_root).unwrap();

    assert_eq!(master.rendered_width, 32);
    assert_eq!(master.rendered_height, 16);
    assert_eq!(master.fit, "inside");
    assert_eq!(master.filter, "catmullrom");
    assert_eq!(master.source_width, 64);
    assert_eq!(master.source_height, 32);
    assert_eq!(master.source_format, "png");

    let master_file = master_path(&cache_root, &master.address);
    assert!(master_file.exists());
    assert_eq!(read_master_dimension(&master_file).unwrap(), 32);

    assert_eq!(
        derivative_filename(16, 32, FilterType::CatmullRom, 80, ImageWireFormat::Png),
        "s16_catmullrom.png"
    );
    assert_eq!(
        derivative_filename(32, 32, FilterType::CatmullRom, 80, ImageWireFormat::Jpg),
        "s32_q80.jpg"
    );

    let expected_magic: [(ImageWireFormat, &[u8]); 4] = [
        (ImageWireFormat::Qoi, b"qoif"),
        (ImageWireFormat::Bmp, b"BM"),
        (ImageWireFormat::Png, b"\x89PNG"),
        (ImageWireFormat::Jpg, b"\xff\xd8"),
    ];

    for (wire_format, magic) in expected_magic {
        let bytes = render_derivative_from_master(
            &master_file,
            16,
            32,
            FilterType::CatmullRom,
            wire_format,
            80,
        )
        .unwrap();

        assert!(
            bytes.starts_with(magic),
            "{wire_format:?} derivative has the wrong signature"
        );
    }
}

/// Re-rendering the same source and spec reuses the existing master.
#[test]
fn render_master_from_file_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let source = write_source(dir.path(), "source.png");
    let cache_root = dir.path().join("cache");
    let spec = inside_spec();

    let first = render_master_from_file(&source, &spec, &cache_root).unwrap();
    let second = render_master_from_file(&source, &spec, &cache_root).unwrap();

    assert_eq!(first.address, second.address);
}

/// A warm probe plus an existing master means the address is stable, and a
/// missing master is rebuilt from the same source without changing it.
#[test]
fn cached_render_reuses_probe_and_rebuilds_missing_master() {
    let dir = tempfile::tempdir().unwrap();
    let source = write_source(dir.path(), "cover.png");
    let spec = inside_spec();

    let first = render_master_cached(&source, &spec, dir.path()).unwrap();
    let master = master_path(dir.path(), &first.address);
    assert!(master.exists(), "first render writes the master");

    let second = render_master_cached(&source, &spec, dir.path()).unwrap();
    assert_eq!(first.address, second.address);
    assert_eq!(first.source_width, second.source_width);

    std::fs::remove_file(&master).unwrap();
    let third = render_master_cached(&source, &spec, dir.path()).unwrap();
    assert_eq!(first.address, third.address);
    assert!(master.exists(), "missing master is rebuilt");
}
