//! Exact area (box) filtering of coverage masks and images to another size,
//! for fractional display scales where only 1x/2x/3x exports exist. Each
//! target pixel averages the source area it covers with rational weights.

/// Coverage of target pixel (`x`, `y`) when a `source`x`source` mask is
/// resampled to `target`x`target`.
pub fn area_sample(mask: &[u8], source: usize, target: usize, x: usize, y: usize) -> u8 {
    sample(mask, 1, 0, (source, source), (target, target), x, y)
}
/// Area-filter a square premultiplied BGRA image (`source`x`source`) to
/// `target`x`target`. Averaging keeps every channel at or below its alpha.
pub fn area_resample_bgra(pixels: &[u8], source: usize, target: usize) -> alloc::vec::Vec<u8> {
    area_resample(pixels, 4, (source, source), (target, target))
}
/// Area-filter an interleaved image of `channels` bytes per pixel from
/// `source` (width, height) to `target` (width, height). A malformed source
/// yields a zeroed image.
pub fn area_resample(
    pixels: &[u8],
    channels: usize,
    source: (usize, usize),
    target: (usize, usize),
) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec![0u8; target.0 * target.1 * channels];
    if channels == 0
        || source.0 == 0
        || source.1 == 0
        || pixels.len() != source.0 * source.1 * channels
    {
        return out;
    }
    for (i, p) in out.chunks_exact_mut(channels).enumerate() {
        for (channel, value) in p.iter_mut().enumerate() {
            *value = sample(
                pixels,
                channels,
                channel,
                source,
                target,
                i % target.0,
                i / target.0,
            );
        }
    }
    out
}
fn sample(
    data: &[u8],
    channels: usize,
    channel: usize,
    (sw, sh): (usize, usize),
    (tw, th): (usize, usize),
    x: usize,
    y: usize,
) -> u8 {
    let at = |sx: usize, sy: usize| data[(sy * sw + sx) * channels + channel];
    if (sw, sh) == (tw, th) {
        return at(x, y);
    }
    // Along an axis, target pixel t spans [t*source, (t+1)*source) in units
    // of 1/target of a source pixel; source pixel i spans
    // [i*target, (i+1)*target).
    let span = |t: usize, source: usize, target: usize| {
        let start = t * source;
        let end = start + source;
        (start / target, end.div_ceil(target).min(source), start, end)
    };
    let (x0, x1, xs, xe) = span(x, sw, tw);
    let (y0, y1, ys, ye) = span(y, sh, th);
    let mut sum = 0u64;
    for sy in y0..y1 {
        let wy = (((sy + 1) * th).min(ye) - (sy * th).max(ys)) as u64;
        for sx in x0..x1 {
            let wx = (((sx + 1) * tw).min(xe) - (sx * tw).max(xs)) as u64;
            sum += u64::from(at(sx, sy)) * wx * wy;
        }
    }
    let total = (sw * sh) as u64;
    ((sum + total / 2) / total) as u8
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn area_filter_preserves_mass_and_identity() {
        let source = 24;
        let mask: std::vec::Vec<u8> = (0..source * source)
            .map(|i| ((i * 37) % 256) as u8)
            .collect();
        for y in 0..source {
            for x in 0..source {
                assert_eq!(
                    area_sample(&mask, source, source, x, y),
                    mask[y * source + x]
                );
            }
        }
        // Exact 2:1 reduction is the mean of each 2x2 block.
        let (x, y) = (3, 5);
        let block: u32 = [(6, 10), (7, 10), (6, 11), (7, 11)]
            .iter()
            .map(|(sx, sy)| u32::from(mask[sy * source + sx]))
            .sum();
        assert_eq!(
            u32::from(area_sample(&mask, source, 12, x, y)),
            (block + 2) / 4
        );
        // Total ink is preserved (within rounding) at every fractional size.
        for target in [15, 18, 21] {
            let mass: u64 = (0..target)
                .flat_map(|y| (0..target).map(move |x| (x, y)))
                .map(|(x, y)| u64::from(area_sample(&mask, source, target, x, y)))
                .sum();
            let expected: u64 = mask.iter().map(|v| u64::from(*v)).sum::<u64>()
                * (target * target) as u64
                / (source * source) as u64;
            assert!(mass.abs_diff(expected) <= (target * target) as u64 / 2 + 1);
        }
        // A full mask stays full.
        let full = std::vec![255u8; 36 * 36];
        assert_eq!(area_sample(&full, 36, 27, 13, 26), 255);
    }
}
