use std::io::Cursor;

use image::{DynamicImage, GenericImageView};

#[must_use]
pub fn calculate_chroma(img: &DynamicImage) -> f64 {
    let (width, height) = img.dimensions();
    let total = f64::from(width) * f64::from(height);
    if total == 0.0 {
        return 0.0;
    }

    let mut sum_rg = 0.0;
    let mut sum_yb = 0.0;
    let mut sum_sq_rg = 0.0;
    let mut sum_sq_yb = 0.0;

    for p in img.pixels() {
        let r = f64::from(p.2[0]);
        let g = f64::from(p.2[1]);
        let b = f64::from(p.2[2]);
        let rg = (r - g).abs();
        let yb = (0.5f64.mul_add(r + g, -b)).abs();
        sum_rg += rg;
        sum_yb += yb;
        sum_sq_rg = rg.mul_add(rg, sum_sq_rg);
        sum_sq_yb = yb.mul_add(yb, sum_sq_yb);
    }

    let m_rg = sum_rg / total;
    let m_yb = sum_yb / total;
    let v_rg = m_rg.mul_add(-m_rg, sum_sq_rg / total);
    let v_yb = m_yb.mul_add(-m_yb, sum_sq_yb / total);
    let std_root = (v_rg.max(0.0) + v_yb.max(0.0)).sqrt();
    let mean_root = m_rg.hypot(m_yb);
    0.3f64.mul_add(mean_root, std_root)
}

#[must_use]
pub fn calculate_entropy(img: &DynamicImage) -> usize {
    let gray = img.grayscale();
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    let _ = gray.write_to(&mut cursor, image::ImageFormat::Png);
    buf.len()
}
