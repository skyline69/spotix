//! Extract a color palette from album artwork for Spotify-styled lyrics.
//!
//! Uses a simplified k-means clustering on downsampled pixel data to find
//! dominant colors, then derives a background gradient and adaptive text
//! colors with sufficient contrast.

use druid::{Color, ImageBuf};

/// A palette derived from album artwork.
#[derive(Clone, Debug)]
pub struct AlbumPalette {
    /// Primary dominant color (used for gradient top / main background).
    pub dominant: Color,
    /// Secondary color (used for gradient bottom).
    pub secondary: Color,
    /// Text color chosen for legibility against the background.
    pub text: Color,
    /// Highlighted/active lyric text color.
    pub highlight: Color,
    /// Dimmed/past lyric text color.
    pub past: Color,
}

impl Default for AlbumPalette {
    fn default() -> Self {
        Self {
            dominant: Color::rgb8(25, 20, 20),
            secondary: Color::rgb8(15, 12, 12),
            text: Color::rgb8(255, 255, 255),
            highlight: Color::rgb8(255, 255, 255),
            past: Color::rgba8(255, 255, 255, 100),
        }
    }
}

/// Extract a palette from an `ImageBuf`.
pub fn extract_palette(image: &ImageBuf) -> AlbumPalette {
    let pixels = sample_pixels(image);
    if pixels.is_empty() {
        return AlbumPalette::default();
    }

    let clusters = kmeans(&pixels, 5, 10);
    if clusters.is_empty() {
        return AlbumPalette::default();
    }

    // Sort clusters by population (most dominant first)
    let mut scored: Vec<(usize, [f64; 3])> = clusters;
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    // Pick dominant and secondary colors
    let dominant_rgb = scored[0].1;
    let secondary_rgb = if scored.len() > 1 {
        // Pick the most visually distinct secondary
        scored[1..]
            .iter()
            .max_by(|a, b| {
                let da = color_distance(&dominant_rgb, &a.1);
                let db = color_distance(&dominant_rgb, &b.1);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|c| c.1)
            .unwrap_or(dominant_rgb)
    } else {
        dominant_rgb
    };

    // Darken colors for background use (lyrics need dark backgrounds)
    let dominant = darken(dominant_rgb, 0.35);
    let secondary = darken(secondary_rgb, 0.25);

    // Compute text colors for contrast
    let bg_lum = luminance(&dominant);
    let (text, highlight, past) = if bg_lum > 0.3 {
        // Light background: use dark text
        (
            Color::rgb8(20, 20, 20),
            Color::rgb8(0, 0, 0),
            Color::rgba8(20, 20, 20, 140),
        )
    } else {
        // Dark background: use light text
        (
            Color::rgb8(230, 230, 230),
            Color::rgb8(255, 255, 255),
            Color::rgba8(200, 200, 200, 140),
        )
    };

    AlbumPalette {
        dominant: to_color(&dominant),
        secondary: to_color(&secondary),
        text,
        highlight,
        past,
    }
}

/// Sample pixels from the image, downsampling to keep it fast.
fn sample_pixels(image: &ImageBuf) -> Vec<[f64; 3]> {
    let size = image.size();
    let w = size.width as usize;
    let h = size.height as usize;
    if w == 0 || h == 0 {
        return vec![];
    }

    let raw = image.raw_pixels();
    let pixel_size = if raw.len() >= w * h * 4 {
        4
    } else if raw.len() >= w * h * 3 {
        3
    } else {
        return vec![];
    };

    // Sample every Nth pixel to keep computation fast (~500 samples)
    let total = w * h;
    let step = (total / 500).max(1);
    let mut pixels = Vec::with_capacity(total / step + 1);

    for i in (0..total).step_by(step) {
        let offset = i * pixel_size;
        if offset + 2 < raw.len() {
            let r = raw[offset] as f64;
            let g = raw[offset + 1] as f64;
            let b = raw[offset + 2] as f64;
            // Skip very dark or very bright pixels (borders, pure white)
            let lum = (r * 0.299 + g * 0.587 + b * 0.114) / 255.0;
            if lum > 0.05 && lum < 0.95 {
                pixels.push([r, g, b]);
            }
        }
    }
    pixels
}

/// Simple k-means clustering. Returns (population, centroid) pairs.
fn kmeans(pixels: &[[f64; 3]], k: usize, iterations: usize) -> Vec<(usize, [f64; 3])> {
    if pixels.is_empty() || k == 0 {
        return vec![];
    }
    let k = k.min(pixels.len());

    // Initialize centroids by evenly sampling the pixel list
    let mut centroids: Vec<[f64; 3]> = (0..k).map(|i| pixels[i * pixels.len() / k]).collect();

    let mut assignments = vec![0usize; pixels.len()];

    for _ in 0..iterations {
        // Assign each pixel to nearest centroid
        for (i, pixel) in pixels.iter().enumerate() {
            let mut best = 0;
            let mut best_dist = f64::MAX;
            for (j, centroid) in centroids.iter().enumerate() {
                let d = color_distance(pixel, centroid);
                if d < best_dist {
                    best_dist = d;
                    best = j;
                }
            }
            assignments[i] = best;
        }

        // Recompute centroids
        let mut sums = vec![[0.0f64; 3]; k];
        let mut counts = vec![0usize; k];
        for (i, pixel) in pixels.iter().enumerate() {
            let c = assignments[i];
            sums[c][0] += pixel[0];
            sums[c][1] += pixel[1];
            sums[c][2] += pixel[2];
            counts[c] += 1;
        }
        for j in 0..k {
            if counts[j] > 0 {
                centroids[j] = [
                    sums[j][0] / counts[j] as f64,
                    sums[j][1] / counts[j] as f64,
                    sums[j][2] / counts[j] as f64,
                ];
            }
        }
    }

    // Collect results
    let mut counts = vec![0usize; k];
    for &a in &assignments {
        counts[a] += 1;
    }
    centroids
        .into_iter()
        .zip(counts)
        .map(|(c, n)| (n, c))
        .collect()
}

fn color_distance(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    let dr = a[0] - b[0];
    let dg = a[1] - b[1];
    let db = a[2] - b[2];
    dr * dr + dg * dg + db * db
}

fn darken(rgb: [f64; 3], factor: f64) -> [f64; 3] {
    [
        (rgb[0] * factor).clamp(0.0, 255.0),
        (rgb[1] * factor).clamp(0.0, 255.0),
        (rgb[2] * factor).clamp(0.0, 255.0),
    ]
}

fn luminance(rgb: &[f64; 3]) -> f64 {
    (rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114) / 255.0
}

fn to_color(rgb: &[f64; 3]) -> Color {
    Color::rgb8(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8)
}
