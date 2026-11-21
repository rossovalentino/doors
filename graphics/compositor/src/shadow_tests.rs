use super::*;
use alloc::{vec, vec::Vec};

// Independent full-size reference: sample the actual window, then blur vertically
// and horizontally. Production stretches a compact pre-exported quadrant.
fn reference(width: u32, height: u32, scale: u32) -> Vec<u8> {
    let pad = 12 * scale as usize;
    let (w, h) = (width as usize + 2 * pad, height as usize + 2 * pad);
    let shape = Rect::new(pad as i32, pad as i32, width, height).unwrap();
    let mut mask = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut n = 0;
            for dy in [2, 6, 10, 14] {
                for dx in [2, 6, 10, 14] {
                    n += usize::from(sample_inside(
                        shape,
                        WINDOW_RADIUS * scale,
                        (x * 16 + dx) as i64,
                        (y * 16 + dy) as i64,
                    ));
                }
            }
            mask[y * w + x] = n as f64 / 16.0;
        }
    }
    let sigma = (4 * scale) as f64;
    let mut kernel: Vec<_> = (0..=2 * pad)
        .map(|i| {
            let d = i as f64 - pad as f64;
            (-d * d / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = kernel.iter().sum();
    for v in &mut kernel {
        *v /= sum;
    }
    let mut vertical = vec![0.0; mask.len()];
    for y in 0..h {
        for x in 0..w {
            for (i, weight) in kernel.iter().enumerate() {
                if let Some(sy) = (y + i).checked_sub(pad).filter(|v| *v < h) {
                    vertical[y * w + x] += mask[sy * w + x] * weight;
                }
            }
        }
    }
    let mut output = vec![0; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let mut value = 0.0;
            for (i, weight) in kernel.iter().enumerate() {
                if let Some(sx) = (x + i).checked_sub(pad).filter(|v| *v < w) {
                    value += vertical[y * w + sx] * weight;
                }
            }
            output[y * w + x] = (value * 255.0).round() as u8;
        }
    }
    output
}

#[test]
fn nine_slice_matches_full_gaussian_for_clipped_resized_windows_and_padded_targets() {
    for scale in 1..=3 {
        let (w, h) = (100 * scale, 90 * scale);
        let stride = w as usize * 4 + 12;
        let source: Vec<u8> = (0..stride * h as usize + 64)
            .map(|i| if i % 4 == 3 { 255 } else { (i % 251) as u8 })
            .collect();
        for (width, height) in [
            (40 * scale + 1, 40 * scale + 1),
            (45 * scale + 1, 43 * scale + 1),
        ] {
            let blurred = reference(width, height, scale);
            for (left, top, opacity) in [(30, 25, 54), (-7, -3, 36)] {
                let window =
                    Rect::new(left * scale as i32, top * scale as i32, width, height).unwrap();
                let bounds = window.expanded(ELEVATION_MARGIN * scale as i32).unwrap();
                let clip =
                    Rect::new(17 * scale as i32, 7 * scale as i32, 73 * scale, 68 * scale).unwrap();
                let mut actual = source.clone();
                let mut canvas = Canvas::new(&mut actual, w, h, stride).unwrap();
                canvas.set_clip(clip);
                assert!(canvas.soft_shadow(window, Scale::from_factor(scale).unwrap(), opacity));
                let mut expected = source.clone();
                for y in 0..h as i32 {
                    for x in 0..w as i32 {
                        if !clip.contains(x, y)
                            || !bounds.contains(x, y)
                            || rounded_coverage_sampled(window, WINDOW_RADIUS * scale, x, y) == 255
                        {
                            continue;
                        }
                        let i = (y - bounds.y()) as usize * bounds.width() as usize
                            + (x - bounds.x()) as usize;
                        let alpha = ((blurred[i] as u32 * opacity as u32 + 127) / 255) as u8;
                        let at = y as usize * stride + x as usize * 4;
                        let destination = expected[at..at + 4].try_into().unwrap();
                        expected[at..at + 4].copy_from_slice(&blend(destination, [0, 0, 0, alpha]));
                    }
                }
                assert_eq!(actual, expected, "scale={scale} origin={left},{top}");
            }
        }
    }
}

#[test]
fn unsupported_or_tiny_shadow_is_rejected_without_touching_pixels() {
    let mut pixels = vec![213; 64 * 64 * 4];
    let before = pixels.clone();
    let mut canvas = Canvas::new(&mut pixels, 64, 64, 256).unwrap();
    // Every supported scale has a texture; small geometry still declines.
    assert!(!canvas.soft_shadow(
        Rect::new(2, 2, 60, 60).unwrap(),
        Scale::from_factor(2).unwrap(),
        54
    ));
    assert!(!canvas.soft_shadow(Rect::new(2, 2, 40, 40).unwrap(), Scale::ONE, 54));
    assert_eq!(pixels, before);
}
