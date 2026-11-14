use image::DynamicImage;
use palette::{FromColor, Oklab, Oklch, Srgb};

const RATIO_SCALE: u64 = 65_535;
const RATIO_SCALE_F32: f32 = 65_535.0;

#[must_use]
pub fn oklab_dist(c1: &Oklab, c2: &Oklab) -> f32 {
    (c1.b - c2.b)
        .mul_add(
            c1.b - c2.b,
            (c1.a - c2.a).mul_add(c1.a - c2.a, (c1.l - c2.l).powi(2)),
        )
        .sqrt()
}

#[must_use]
pub fn hue_dist(h1: f32, h2: f32) -> f32 {
    let diff = (h1 - h2).abs() % 360.0;
    if diff > 180.0 { 360.0 - diff } else { diff }
}

#[must_use]
pub fn calculate_palette_ratios(
    img_to_process: &DynamicImage,
    candidate_colors: Vec<Srgb>,
    threshold_val: f32,
) -> Vec<(Srgb, f32)> {
    let oklab_centers: Vec<Oklab> = candidate_colors
        .iter()
        .map(|&c| Oklab::from_color(c))
        .collect();
    let mut counts = vec![0usize; oklab_centers.len()];

    for p in img_to_process.to_rgb8().pixels() {
        let pixel_oklab = Oklab::from_color(Srgb::new(
            f32::from(p[0]) / 255.0,
            f32::from(p[1]) / 255.0,
            f32::from(p[2]) / 255.0,
        ));
        let mut best_idx = 0;
        let mut min_dist_sq = f32::MAX;
        for (i, center) in oklab_centers.iter().enumerate() {
            let dist_sq = (pixel_oklab.b - center.b).mul_add(
                pixel_oklab.b - center.b,
                (pixel_oklab.a - center.a).mul_add(
                    pixel_oklab.a - center.a,
                    (pixel_oklab.l - center.l).powi(2),
                ),
            );
            if dist_sq < min_dist_sq {
                min_dist_sq = dist_sq;
                best_idx = i;
            }
        }
        counts[best_idx] += 1;
    }

    let total_pixels = counts.iter().sum::<usize>();
    let total_u64 = u64::try_from(total_pixels).unwrap_or(0);

    let mut palette: Vec<(Srgb, f32)> = candidate_colors
        .into_iter()
        .zip(counts)
        .filter_map(|(color, count)| {
            let count_u64 = u64::try_from(count).unwrap_or(0);
            let ratio = count_u64
                .checked_mul(RATIO_SCALE)
                .and_then(|scaled| scaled.checked_div(total_u64))
                .and_then(|scaled| u16::try_from(scaled).ok())
                .map_or(0.0, |scaled_u16| f32::from(scaled_u16) / RATIO_SCALE_F32);

            if ratio > 0.0 {
                Some((color, ratio))
            } else {
                None
            }
        })
        .collect();

    palette.retain(|&(_, ratio)| ratio >= threshold_val);

    let final_total: f32 = palette.iter().map(|(_, r)| r).sum();
    if final_total > 0.0 {
        for item in &mut palette {
            item.1 /= final_total;
        }
    }
    palette
}

pub fn sort_palette(palette: &mut Vec<(Srgb, f32)>, sort_type: &str) {
    match sort_type {
        "L" => palette.sort_by(|a, b| {
            let l_a = Oklch::from_color(a.0).l;
            let l_b = Oklch::from_color(b.0).l;
            l_b.partial_cmp(&l_a).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "C" => palette.sort_by(|a, b| {
            let c_a = Oklch::from_color(a.0).chroma;
            let c_b = Oklch::from_color(b.0).chroma;
            c_b.partial_cmp(&c_a).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "H" => palette.sort_by(|a, b| {
            let h_a = Oklch::from_color(a.0).hue.into_raw_degrees();
            let h_b = Oklch::from_color(b.0).hue.into_raw_degrees();
            h_a.partial_cmp(&h_b).unwrap_or(std::cmp::Ordering::Equal)
        }),
        "LC" => palette.sort_by(|a, b| {
            let oklch_a = Oklch::from_color(a.0);
            let oklch_b = Oklch::from_color(b.0);
            let val_a = oklch_a.l * oklch_a.chroma;
            let val_b = oklch_b.l * oklch_b.chroma;
            val_b
                .partial_cmp(&val_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "gradient" => sort_palette_gradient(palette),
        _ => palette
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)),
    }
}

pub fn sort_palette_gradient(palette: &mut Vec<(Srgb, f32)>) {
    if palette.is_empty() {
        return;
    }
    let mut pool: Vec<(Oklab, Srgb, f32)> = palette
        .iter()
        .map(|&(srgb, ratio)| (Oklab::from_color(srgb), srgb, ratio))
        .collect();

    let mut sorted = Vec::with_capacity(pool.len());

    let start_idx = pool
        .iter()
        .enumerate()
        .max_by(|(_, (ok_a, _, _)), (_, (ok_b, _, _))| {
            ok_a.l
                .partial_cmp(&ok_b.l)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map_or(0, |(i, _)| i);

    let first = pool.remove(start_idx);
    let mut current_ok = first.0;
    sorted.push((first.1, first.2));

    let end_node_idx = if pool.is_empty() {
        None
    } else {
        pool.iter()
            .enumerate()
            .min_by(|(_, (ok_a, _, _)), (_, (ok_b, _, _))| {
                ok_a.l
                    .partial_cmp(&ok_b.l)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
    };

    let end_node = end_node_idx.map(|idx| pool.remove(idx));

    while !pool.is_empty() {
        let next_idx = pool
            .iter()
            .enumerate()
            .min_by(|(_, (ok_a, _, _)), (_, (ok_b, _, _))| {
                let dist_a = (ok_a.b - current_ok.b).mul_add(
                    ok_a.b - current_ok.b,
                    (ok_a.a - current_ok.a)
                        .mul_add(ok_a.a - current_ok.a, (ok_a.l - current_ok.l).powi(2)),
                );
                let dist_b = (ok_b.b - current_ok.b).mul_add(
                    ok_b.b - current_ok.b,
                    (ok_b.a - current_ok.a)
                        .mul_add(ok_b.a - current_ok.a, (ok_b.l - current_ok.l).powi(2)),
                );
                dist_a
                    .partial_cmp(&dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(0, |(i, _)| i);

        let next = pool.remove(next_idx);
        current_ok = next.0;
        sorted.push((next.1, next.2));
    }

    if let Some(node) = end_node {
        sorted.push((node.1, node.2));
    }

    *palette = sorted;
}
