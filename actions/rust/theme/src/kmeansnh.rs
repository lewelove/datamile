use image::DynamicImage;
use kmeans_colors::get_kmeans_hamerly;
use libactions::color::hue_dist;
use palette::{FromColor, Lab, Oklch, Srgb};

pub fn extract(img: &DynamicImage, args: &str) -> Vec<Srgb> {
    let k = args
        .split(',')
        .find(|s| s.trim().starts_with("k="))
        .and_then(|s| s.trim().strip_prefix("k="))
        .and_then(|val| val.parse::<usize>().ok())
        .unwrap_or(24)
        .clamp(1, 64);

    let n = args
        .split(',')
        .find(|s| s.trim().starts_with("n="))
        .and_then(|s| s.trim().strip_prefix("n="))
        .and_then(|val| val.parse::<usize>().ok())
        .unwrap_or(8)
        .clamp(2, 24);

    let h_min = args
        .split(',')
        .find(|s| s.trim().starts_with("h="))
        .and_then(|s| s.trim().strip_prefix("h="))
        .and_then(|val| val.parse::<f32>().ok())
        .unwrap_or(30.0)
        .clamp(0.0, 180.0);

    let conv = args
        .split(',')
        .find(|s| s.trim().starts_with("conv="))
        .and_then(|s| s.trim().strip_prefix("conv="))
        .and_then(|val| val.parse::<f32>().ok())
        .unwrap_or(0.001);

    let pixels: Vec<Lab> = img
        .to_rgb8()
        .pixels()
        .map(|p| {
            Lab::from_color(Srgb::new(
                f32::from(p[0]) / 255.0,
                f32::from(p[1]) / 255.0,
                f32::from(p[2]) / 255.0,
            ))
        })
        .collect();

    let result = get_kmeans_hamerly(k, 20, conv, false, &pixels, 42);
    let pool: Vec<Srgb> = result.centroids.into_iter().map(Srgb::from_color).collect();

    if pool.len() <= n {
        return pool;
    }

    let mut lch_pool: Vec<(Srgb, Oklch)> = pool
        .into_iter()
        .map(|s| (s, Oklch::from_color(s)))
        .collect();

    lch_pool.sort_by(|a, b| {
        a.1.l
            .partial_cmp(&b.1.l)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let darkest = lch_pool.remove(0);
    let lightest = lch_pool
        .pop()
        .expect("palette must hold at least two colors");

    lch_pool.sort_by(|a, b| {
        b.1.chroma
            .partial_cmp(&a.1.chroma)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut selected = vec![lightest, darkest];

    for candidate in &lch_pool {
        if selected.len() >= n {
            break;
        }

        let mut too_close = false;
        for existing in &selected {
            if hue_dist(
                candidate.1.hue.into_raw_degrees(),
                existing.1.hue.into_raw_degrees(),
            ) < h_min
            {
                too_close = true;
                break;
            }
        }

        if !too_close {
            selected.push(*candidate);
        }
    }

    if selected.len() < n {
        for candidate in lch_pool {
            if selected.len() >= n {
                break;
            }
            if !selected.iter().any(|s| s.0 == candidate.0) {
                selected.push(candidate);
            }
        }
    }

    selected.into_iter().map(|(s, _)| s).collect()
}
