const BMP_HEADER_OFFSET: u32 = 54;
const DIB_HEADER_SIZE: u32 = 40;
const BMP_COLOR_PLANES: u16 = 1;
const BMP_BITS_PER_PIXEL: u16 = 24;
const RGB_CHANNELS: usize = 3;

#[must_use]
pub fn convert(img: &image::RgbImage) -> Option<Vec<u8>> {
    let width = img.width();
    let height = img.height();

    let width_usize = usize::try_from(width).ok()?;
    let height_usize = usize::try_from(height).ok()?;
    let width_i32 = i32::try_from(width).ok()?;
    let neg_height_i32 = i32::try_from(height).ok()?.checked_neg()?;

    let row_bytes = width_usize.checked_mul(RGB_CHANNELS)?;
    let padding = (4 - (row_bytes % 4)) % 4;
    let image_size = (row_bytes + padding).checked_mul(height_usize)?;
    let file_size = image_size.checked_add(54)?;

    let file_size_u32 = u32::try_from(file_size).ok()?;
    let image_size_u32 = u32::try_from(image_size).ok()?;

    let mut bmp = Vec::with_capacity(file_size);

    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size_u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&BMP_HEADER_OFFSET.to_le_bytes());

    bmp.extend_from_slice(&DIB_HEADER_SIZE.to_le_bytes());
    bmp.extend_from_slice(&width_i32.to_le_bytes());
    bmp.extend_from_slice(&neg_height_i32.to_le_bytes());
    bmp.extend_from_slice(&BMP_COLOR_PLANES.to_le_bytes());
    bmp.extend_from_slice(&BMP_BITS_PER_PIXEL.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&image_size_u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 16]);

    let pad_bytes = [0u8; 3];
    let raw = img.as_raw();
    for row in raw.chunks_exact(row_bytes) {
        let (pixels, _) = row.as_chunks::<RGB_CHANNELS>();
        for &[r, g, b] in pixels {
            bmp.push(b);
            bmp.push(g);
            bmp.push(r);
        }
        if padding > 0 {
            bmp.extend_from_slice(&pad_bytes[..padding]);
        }
    }

    Some(bmp)
}
