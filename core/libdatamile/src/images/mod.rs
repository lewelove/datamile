pub mod fast_rgb_to_bmp;

use std::io::Cursor;

pub use fast_image_resize::FilterType;
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::ImageEncoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Crop,
    Inside,
}

impl FitMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Crop => "crop",
            Self::Inside => "inside",
        }
    }
}

#[must_use]
pub fn parse_fit_mode(mode: &str) -> Option<FitMode> {
    match mode.to_lowercase().as_str() {
        "crop" => Some(FitMode::Crop),
        "inside" => Some(FitMode::Inside),
        _ => None,
    }
}

#[must_use]
pub fn parse_filter(algo: &str) -> Option<FilterType> {
    match algo.to_lowercase().as_str() {
        "box" => Some(FilterType::Box),
        "bilinear" => Some(FilterType::Bilinear),
        "catmullrom" => Some(FilterType::CatmullRom),
        "gaussian" => Some(FilterType::Gaussian),
        "hamming" => Some(FilterType::Hamming),
        "lanczos" => Some(FilterType::Lanczos3),
        "mitchell" => Some(FilterType::Mitchell),
        _ => None,
    }
}

#[must_use]
pub const fn filter_type_to_str(filter: FilterType) -> &'static str {
    match filter {
        FilterType::Box => "box",
        FilterType::Bilinear => "bilinear",
        FilterType::Gaussian => "gaussian",
        FilterType::Hamming => "hamming",
        FilterType::Lanczos3 => "lanczos",
        FilterType::Mitchell => "mitchell",
        _ => "catmullrom",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageWireFormat {
    Qoi,
    Bmp,
    Png,
    Jpg,
}

impl ImageWireFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Qoi => "qoi",
            Self::Bmp => "bmp",
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }

    #[must_use]
    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Qoi => "image/x-qoi",
            Self::Bmp => "image/bmp",
            Self::Png => "image/png",
            Self::Jpg => "image/jpeg",
        }
    }
}

#[must_use]
pub fn parse_wire_format(s: &str) -> Option<ImageWireFormat> {
    match s.to_lowercase().as_str() {
        "qoi" => Some(ImageWireFormat::Qoi),
        "bmp" => Some(ImageWireFormat::Bmp),
        "png" => Some(ImageWireFormat::Png),
        "jpg" | "jpeg" => Some(ImageWireFormat::Jpg),
        _ => None,
    }
}

#[must_use]
pub fn read_qoi_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 14 || &bytes[0..4] != b"qoif" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let height = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    Some((width, height))
}

#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded values are clamped to 1..=4096 before the cast"
)]
pub fn calculate_rendered_dimensions(
    src_w: u32,
    src_h: u32,
    target_w: u32,
    target_h: u32,
    mode: FitMode,
) -> (u32, u32) {
    match mode {
        FitMode::Crop => (target_w, target_h),
        FitMode::Inside => {
            let scale = f64::min(
                f64::from(target_w) / f64::from(src_w),
                f64::from(target_h) / f64::from(src_h),
            );
            let rendered_w = (f64::from(src_w) * scale).round() as u32;
            let rendered_h = (f64::from(src_h) * scale).round() as u32;
            (rendered_w.clamp(1, 4096), rendered_h.clamp(1, 4096))
        }
    }
}

#[must_use]
pub fn resize_image_fit(
    src: &image::RgbImage,
    target_width: u32,
    target_height: u32,
    mode: FitMode,
    filter: FilterType,
) -> Option<image::RgbImage> {
    let src_width = src.width();
    let src_height = src.height();

    let src_image = Image::from_vec_u8(
        src_width,
        src_height,
        src.clone().into_raw(),
        PixelType::U8x3,
    )
    .ok()?;

    let mut resizer = Resizer::new();

    match mode {
        FitMode::Crop => {
            let target_ratio = f64::from(target_width) / f64::from(target_height);
            let src_ratio = f64::from(src_width) / f64::from(src_height);

            let (crop_w, crop_h, crop_x, crop_y) = if src_ratio > target_ratio {
                let h = f64::from(src_height);
                let w = h * target_ratio;
                let x = (f64::from(src_width) - w) / 2.0;
                (w, h, x, 0.0)
            } else {
                let w = f64::from(src_width);
                let h = w / target_ratio;
                let y = (f64::from(src_height) - h) / 2.0;
                (w, h, 0.0, y)
            };

            let mut dst_image = Image::new(target_width, target_height, PixelType::U8x3);
            let options = ResizeOptions::new()
                .crop(crop_x, crop_y, crop_w, crop_h)
                .resize_alg(ResizeAlg::Convolution(filter));

            resizer.resize(&src_image, &mut dst_image, &options).ok()?;
            image::RgbImage::from_raw(target_width, target_height, dst_image.into_vec())
        }
        FitMode::Inside => {
            let (dst_w, dst_h) = calculate_rendered_dimensions(
                src_width,
                src_height,
                target_width,
                target_height,
                FitMode::Inside,
            );

            let mut dst_image = Image::new(dst_w, dst_h, PixelType::U8x3);
            let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));

            resizer.resize(&src_image, &mut dst_image, &options).ok()?;
            image::RgbImage::from_raw(dst_w, dst_h, dst_image.into_vec())
        }
    }
}

#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "rounded values are clamped to 1..=4096 before the cast"
)]
pub fn resize_image_max_edge(
    src: &image::RgbImage,
    max_edge: u32,
    filter: FilterType,
) -> Option<image::RgbImage> {
    let src_width = src.width();
    let src_height = src.height();
    let current_max = std::cmp::max(src_width, src_height);
    if current_max == 0 {
        return None;
    }

    let scale = f64::from(max_edge) / f64::from(current_max);
    let dst_w = (f64::from(src_width) * scale).round() as u32;
    let dst_h = (f64::from(src_height) * scale).round() as u32;

    let src_image = Image::from_vec_u8(
        src_width,
        src_height,
        src.clone().into_raw(),
        PixelType::U8x3,
    )
    .ok()?;

    let mut dst_image =
        Image::new(dst_w.clamp(1, 4096), dst_h.clamp(1, 4096), PixelType::U8x3);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter));

    let mut resizer = Resizer::new();
    resizer.resize(&src_image, &mut dst_image, &options).ok()?;
    image::RgbImage::from_raw(dst_image.width(), dst_image.height(), dst_image.into_vec())
}

#[must_use]
pub fn decode_qoi_to_rgb(bytes: &[u8]) -> Option<image::RgbImage> {
    let (header, decoded) = qoi::decode_to_vec(bytes).ok()?;
    let width = header.width;
    let height = header.height;

    match header.channels {
        qoi::Channels::Rgb => image::RgbImage::from_raw(width, height, decoded),
        qoi::Channels::Rgba => {
            let img_rgba = image::RgbaImage::from_raw(width, height, decoded)?;
            Some(image::DynamicImage::ImageRgba8(img_rgba).into_rgb8())
        }
    }
}

#[must_use]
pub fn encode_rgb_image(
    img: &image::RgbImage,
    format: ImageWireFormat,
    quality: u8,
) -> Option<Vec<u8>> {
    match format {
        ImageWireFormat::Qoi => {
            qoi::encode_to_vec(img.as_raw(), img.width(), img.height()).ok()
        }
        ImageWireFormat::Bmp => fast_rgb_to_bmp::convert(img),
        ImageWireFormat::Png => {
            let mut buf = Vec::new();
            let mut cursor = Cursor::new(&mut buf);
            img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
            Some(buf)
        }
        ImageWireFormat::Jpg => {
            let mut buf = Vec::new();
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
            encoder
                .write_image(
                    img.as_raw(),
                    img.width(),
                    img.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .ok()?;
            Some(buf)
        }
    }
}
