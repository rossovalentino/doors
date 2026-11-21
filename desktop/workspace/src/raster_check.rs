//! Host checks for painted BGRA frames at fractional display scales (tests
//! of every application share them). A 1 px line drawn at a fractional
//! physical offset shows up as two adjacent partial rows (or columns) on one
//! background: that is a "blurred" line. Snapped painting never produces it.

/// Minimum straight run, in physical pixels, that counts as a line.
pub const RUN: usize = 24;

fn pixel(p: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let at = (y * width + x) * 4;
    p[at..at + 4].try_into().unwrap()
}
/// Both colours are displaced from `g` in the same direction in every
/// channel: two partial-coverage samples of one ink over `g`.
fn same_side(g: [u8; 4], a: [u8; 4], b: [u8; 4]) -> bool {
    (0..3).all(|c| {
        let (da, db) = (
            i16::from(a[c]) - i16::from(g[c]),
            i16::from(b[c]) - i16::from(g[c]),
        );
        da.signum() == db.signum()
    })
}
/// Straight lines in one orientation: `(blurred, crisp)` counts.
fn scan(p: &[u8], width: usize, height: usize, transpose: bool) -> (usize, usize) {
    let (major, minor) = if transpose {
        (width, height)
    } else {
        (height, width)
    };
    let at = |m: usize, n: usize| {
        if transpose {
            pixel(p, width, m, n)
        } else {
            pixel(p, width, n, m)
        }
    };
    let (mut blurred, mut crisp) = (0, 0);
    for m in 1..major.saturating_sub(2) {
        let column = |n: usize| [at(m - 1, n), at(m, n), at(m + 1, n), at(m + 2, n)];
        let mut n = 0;
        while n < minor {
            let first = column(n);
            let mut end = n + 1;
            while end < minor && column(end) == first {
                end += 1;
            }
            if end - n >= RUN {
                let [g, a, b, g2] = first;
                if g == g2 && a != g && b != g && a != b && same_side(g, a, b) {
                    blurred += 1;
                } else if a != g && (b == g || a == b && g2 == g) {
                    crisp += 1;
                }
            }
            n = end;
        }
    }
    (blurred, crisp)
}
/// Blurred straight lines (horizontal plus vertical runs of at least `RUN`).
pub fn blurred_lines(pixels: &[u8], width: usize, height: usize) -> usize {
    scan(pixels, width, height, false).0 + scan(pixels, width, height, true).0
}
/// Crisp 1 px lines found: one row (column) of ink with the same uniform
/// colour on both sides, the evidence that a frame actually has lines.
pub fn crisp_lines(pixels: &[u8], width: usize, height: usize) -> usize {
    scan(pixels, width, height, false).1 + scan(pixels, width, height, true).1
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame(rows: &[[u8; 4]], width: usize) -> alloc::vec::Vec<u8> {
        rows.iter()
            .flat_map(|c| core::iter::repeat_n(*c, width).flatten())
            .collect()
    }
    #[test]
    fn half_alpha_rows_are_blurred_and_snapped_rows_are_crisp() {
        let (g, ink) = ([255, 255, 255, 255], [0, 0, 0, 255]);
        let crisp = frame(&[g, g, ink, g, g, g], 40);
        assert_eq!(
            (blurred_lines(&crisp, 40, 6), crisp_lines(&crisp, 40, 6)),
            (0, 1)
        );
        let soft = frame(
            &[g, g, [128, 128, 128, 255], [200, 200, 200, 255], g, g],
            40,
        );
        assert_eq!(blurred_lines(&soft, 40, 6), 1);
    }
}
