use image::DynamicImage;
use kmeans_colors::get_kmeans_hamerly;
use palette::{FromColor, Lab, Srgb};
use rand::SeedableRng;
use rand::distr::{Distribution, Uniform};
use rand::rngs::StdRng;

pub fn extract(img: &DynamicImage, args: &str) -> Vec<Srgb> {
    let k = args
        .split(',')
        .find(|s| s.trim().starts_with("k="))
        .and_then(|s| s.trim().strip_prefix("k="))
        .and_then(|val| val.parse::<usize>().ok())
        .unwrap_or(10)
        .clamp(1, 24);

    let noise = args
        .split(',')
        .find(|s| s.trim().starts_with("noise="))
        .and_then(|s| s.trim().strip_prefix("noise="))
        .and_then(|val| val.parse::<f32>().ok())
        .unwrap_or(0.0);

    let conv = args
        .split(',')
        .find(|s| s.trim().starts_with("conv="))
        .and_then(|s| s.trim().strip_prefix("conv="))
        .and_then(|val| val.parse::<f32>().ok())
        .unwrap_or(0.000);

    let mut pixels: Vec<Lab> = img
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

    if noise > 0.0
        && let Ok(dist) = Uniform::new_inclusive(-noise, noise)
    {
        let mut rng = StdRng::seed_from_u64(42);
        for pixel in &mut pixels {
            let offset = dist.sample(&mut rng);
            pixel.l = (pixel.l + offset).clamp(0.0, 100.0);
        }
    }

    let result = get_kmeans_hamerly(k, 20, conv, false, &pixels, 42);

    result.centroids.into_iter().map(Srgb::from_color).collect()
}
