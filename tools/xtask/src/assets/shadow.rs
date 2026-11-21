//! Original finite Gaussian shadow coverage, generated once at each native DPI.
use super::*;

fn image(scale: usize) -> (usize, Vec<f64>, Vec<f64>) {
    let side = 64 * scale + 1;
    let margin = 12 * scale;
    let radius = 8 * scale;
    let last = 52 * scale + 1;
    let mut mask = vec![0.0; side * side];
    for y in margin..last {
        for x in margin..last {
            let mut inside = 0;
            for dy in [2, 6, 10, 14] {
                for dx in [2, 6, 10, 14] {
                    let sx = x * 16 + dx;
                    let sy = y * 16 + dy;
                    let cx = sx.clamp((margin + radius) * 16, (last - radius) * 16);
                    let cy = sy.clamp((margin + radius) * 16, (last - radius) * 16);
                    let distance = sx.abs_diff(cx).pow(2) + sy.abs_diff(cy).pow(2);
                    inside += usize::from(distance <= (radius * 16).pow(2));
                }
            }
            mask[y * side + x] = inside as f64 / 16.0;
        }
    }
    let sigma = (4 * scale) as f64;
    let mut kernel: Vec<_> = (0..=margin * 2)
        .map(|i| {
            let x = i as f64 - margin as f64;
            (-x * x / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = kernel.iter().sum();
    for n in &mut kernel {
        *n /= sum;
    }
    (side, mask, kernel)
}

fn filtered(scale: usize) -> Vec<f64> {
    let (side, mask, kernel) = image(scale);
    let radius = 12 * scale;
    let mut horizontal = vec![0.0; mask.len()];
    let mut output = vec![0.0; mask.len()];
    for y in 0..side {
        for x in 0..side {
            for (k, weight) in kernel.iter().enumerate() {
                if let Some(sx) = (x + k).checked_sub(radius).filter(|v| *v < side) {
                    horizontal[y * side + x] += mask[y * side + sx] * weight;
                }
            }
        }
    }
    for y in 0..side {
        for x in 0..side {
            for (k, weight) in kernel.iter().enumerate() {
                if let Some(sy) = (y + k).checked_sub(radius).filter(|v| *v < side) {
                    output[y * side + x] += horizontal[sy * side + x] * weight;
                }
            }
        }
    }
    output
}

pub(super) fn export(root: &Path, check: bool) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(14_723);
    for scale in 1..=3 {
        let image = filtered(scale);
        let side = 64 * scale + 1;
        // A mirrored quadrant, including its center edge. Finite12-DIP support
        // plus8-DIP corner radius reaches32DIP from the shadow's outer corner.
        for y in 0..=32 * scale {
            for x in 0..=32 * scale {
                bytes.push((image[y * side + x] * 255.0).round() as u8);
            }
        }
    }
    if bytes.len() != 14_723 {
        return Err("shadow coverage extent".into());
    }
    let path = root.join("assets/raster/soft-shadow.a8");
    asset_output(&path, &bytes, check)?;
    asset_output(
        &root.join("assets/raster/gaussian-cdf.u16"),
        &gaussian_cdf(),
        check,
    )
}

/// Standard normal CDF at t = -4 + k/64 (k = 0..=512) as little-endian u16
/// (0..=65535), with exact 0 and 65535 at the ends so the window shadow's
/// table lookup (`door_graphics_core` render.rs) saturates beyond 4 sigma.
/// The guest uses no floating point: it interpolates this table.
fn gaussian_cdf() -> Vec<u8> {
    // erf by its Maclaurin series; |x| <= 4 / sqrt(2) converges well within
    // 120 terms at f64 precision.
    fn erf(x: f64) -> f64 {
        let mut sum = 0.0;
        let mut term = x;
        for n in 0..120 {
            sum += term / (2 * n + 1) as f64;
            term *= -x * x / (n + 1) as f64;
        }
        sum * 2.0 / std::f64::consts::PI.sqrt()
    }
    let mut bytes = Vec::with_capacity(513 * 2);
    for k in 0..=512 {
        let t = -4.0 + f64::from(k) / 64.0;
        let value = match k {
            0 => 0,
            512 => 65535,
            _ => ((0.5 * (1.0 + erf(t / std::f64::consts::SQRT_2))) * 65535.0).round() as u16,
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gaussian_cdf_table_is_monotonic_symmetric_and_saturates() {
        let table: Vec<u16> = gaussian_cdf()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(table.len(), 513);
        assert_eq!((table[0], table[256], table[512]), (0, 32768, 65535));
        assert!(table.windows(2).all(|w| w[0] <= w[1]));
        for k in 1..512 {
            assert!((i32::from(table[k]) + i32::from(table[512 - k]) - 65535).abs() <= 1);
        }
        // Phi(1) = 0.841344746.
        assert_eq!(
            table[256 + 64],
            (0.841_344_746_f64 * 65535.0).round() as u16
        );
        assert_eq!(
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets/raster/gaussian-cdf.u16")
            )
            .unwrap(),
            gaussian_cdf(),
            "regenerate with cargo xtask assets"
        );
    }
    #[test]
    fn separable_filter_matches_direct_two_dimensional_convolution() {
        for scale in 1..=3 {
            let (side, mask, kernel) = image(scale);
            let output = filtered(scale);
            let radius = 12 * scale;
            for (x, y) in [
                (0, 0),
                (12 * scale, 12 * scale),
                (16 * scale, 10 * scale),
                (32 * scale, 12 * scale),
                (32 * scale, 32 * scale),
            ] {
                let mut expected = 0.0;
                for (ky, wy) in kernel.iter().enumerate() {
                    for (kx, wx) in kernel.iter().enumerate() {
                        if let (Some(sx), Some(sy)) = (
                            (x + kx).checked_sub(radius).filter(|v| *v < side),
                            (y + ky).checked_sub(radius).filter(|v| *v < side),
                        ) {
                            expected += mask[sy * side + sx] * wx * wy;
                        }
                    }
                }
                assert!((expected - output[y * side + x]).abs() < 1e-12);
                assert_eq!(
                    (expected * 255.0).round(),
                    (output[y * side + x] * 255.0).round()
                );
            }
            assert!(output.iter().all(|v| (0.0..=1.000000000001).contains(v)));
        }
    }
}
