use image::DynamicImage;
use kmeans_colors::get_kmeans_hamerly;
use libactions::color::oklab_dist;
use palette::{FromColor, Lab, Oklab, Srgb};

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
        .unwrap_or(6)
        .clamp(2, 24);

    let d_min = args
        .split(',')
        .find(|s| s.trim().starts_with("d="))
        .and_then(|s| s.trim().strip_prefix("d="))
        .and_then(|val| val.parse::<f32>().ok())
        .unwrap_or(0.1)
        .clamp(0.0, 1.0);

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

    let mut data: Vec<(Srgb, f32, f32, Oklab)> = pool
        .into_iter()
        .map(|s| {
            let ok = Oklab::from_color(s);
            let rgb_vividness =
                s.red.max(s.green).max(s.blue) - s.red.min(s.green).min(s.blue);
            (s, ok.l, rgb_vividness, ok)
        })
        .collect();

    data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let darkest = data.remove(0);
    let lightest = data.pop().expect("palette must hold at least two colors");

    data.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected = vec![lightest, darkest];

    for candidate in &data {
        if selected.len() >= n {
            break;
        }

        let mut too_close = false;
        for existing in &selected {
            if oklab_dist(&candidate.3, &existing.3) < d_min {
                too_close = true;
                break;
            }
        }

        if !too_close {
            selected.push(*candidate);
        }
    }

    if selected.len() < n {
        for candidate in data {
            if selected.len() >= n {
                break;
            }
            if !selected.iter().any(|s| s.0 == candidate.0) {
                selected.push(candidate);
            }
        }
    }

    selected.into_iter().map(|(s, _, _, _)| s).collect()
}
