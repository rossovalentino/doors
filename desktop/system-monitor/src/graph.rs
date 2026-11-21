//! Time-based, anti-aliased resource graphs rasterized at native density.
//!
//! Samples are placed by their timer tick, not by index, so an irregular or
//! paused interval leaves a real gap and the grid scrolls with time exactly
//! as the data does. The result is an opaque BGRA image for `View::color_image`.
use alloc::{vec, vec::Vec};

/// RGB colors; the caller resolves theme.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub background: [u8; 3],
    pub grid: Option<[u8; 3]>,
    pub line: [u8; 3],
    /// Fill opacity under the curve (0 disables the fill).
    pub fill: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct Plot<'a> {
    /// (tick, value); `None` marks an interval without a valid measurement.
    pub points: &'a [(u64, Option<u32>)],
    pub max: u32,
    /// Right edge time and visible span, in 100 Hz ticks.
    pub now: u64,
    pub span: u64,
}
/// Vertical grid lines every three seconds (20 columns over a minute).
pub const GRID_TICKS: u64 = 300;

fn blend(dst: &mut [u8], color: [u8; 3], alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    for (i, c) in [color[2], color[1], color[0]].into_iter().enumerate() {
        dst[i] = (dst[i] as f32 + (c as f32 - dst[i] as f32) * a + 0.5) as u8;
    }
}
fn floor(v: f32) -> i32 {
    let i = v as i32;
    if (i as f32) > v { i - 1 } else { i }
}
/// Physical x of a tick: the right edge is `now`, the left edge `now - span`.
pub fn x_of(tick: u64, now: u64, span: u64, width: usize) -> f32 {
    let age = now.saturating_sub(tick) as f32;
    width as f32 - age * width as f32 / span.max(1) as f32
}
#[cfg(test)]
pub fn render(
    width: usize,
    height: usize,
    scale: usize,
    plot: &Plot<'_>,
    style: &Style,
) -> Vec<u8> {
    render_physical(width * scale, height * scale, scale as f32, plot, style)
}
/// `render` at physical resolution `w` x `h` with a stroke `stroke` physical
/// pixels wide (the display scale), so fractional scales keep 1 px grid
/// lines snapped instead of resampling a whole-density image.
pub fn render_physical(w: usize, h: usize, stroke: f32, plot: &Plot<'_>, style: &Style) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return out;
    }
    for p in out.chunks_exact_mut(4) {
        p.copy_from_slice(&[
            style.background[2],
            style.background[1],
            style.background[0],
            255,
        ]);
    }
    if let Some(grid) = style.grid {
        for k in 1..10 {
            let y = k * h / 10;
            for x in 0..w {
                blend(&mut out[(y * w + x) * 4..][..4], grid, 1.0);
            }
        }
        let first = plot.now.saturating_sub(plot.span) / GRID_TICKS * GRID_TICKS;
        let mut t = first;
        while t <= plot.now {
            let x = floor(x_of(t, plot.now, plot.span, w));
            if (0..w as i32).contains(&x) {
                for y in 0..h {
                    blend(&mut out[(y * w + x as usize) * 4..][..4], grid, 1.0);
                }
            }
            t += GRID_TICKS;
        }
    }
    let top = 0.5 * stroke;
    let usable = h as f32 - stroke;
    let max = plot.max.max(1) as f32;
    let y_of = |v: u32| top + usable * (1.0 - v.min(plot.max) as f32 / max);
    // Contiguous measured runs; a missing sample breaks the line.
    let mut segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for pair in plot.points.windows(2) {
        if let ((t0, Some(v0)), (t1, Some(v1))) = (pair[0], pair[1]) {
            if t1 <= t0 {
                continue;
            }
            let a = (x_of(t0, plot.now, plot.span, w), y_of(v0));
            let b = (x_of(t1, plot.now, plot.span, w), y_of(v1));
            if b.0 < 0.0 || a.0 > w as f32 {
                continue;
            }
            segments.push((a, b));
        }
    }
    let at = |x: f32| -> Option<f32> {
        segments
            .iter()
            .find(|(a, b)| x >= a.0 && x <= b.0)
            .map(|(a, b)| a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0).max(1e-3))
    };
    if style.fill != 0 {
        let alpha = style.fill as f32 / 255.0;
        for x in 0..w {
            let Some(y) = at(x as f32 + 0.5) else {
                continue;
            };
            let first = floor(y).max(0) as usize;
            for row in first..h {
                let coverage = if row == first {
                    1.0 - (y - first as f32)
                } else {
                    1.0
                };
                blend(
                    &mut out[(row * w + x) * 4..][..4],
                    style.line,
                    alpha * coverage,
                );
            }
        }
    }
    // Stroke: per covered column, the exact vertical extent the segment sweeps,
    // widened by the stroke; coverage is the pixel's overlap with that span.
    let half = 0.5 * stroke + 0.25;
    let mut coverage = vec![0u8; w * h];
    for (a, b) in &segments {
        let x0 = a.0.max(0.0);
        let x1 = b.0.min(w as f32);
        if x1 < x0 {
            continue;
        }
        for column in floor(x0)..=floor(x1).min(w as i32 - 1) {
            if column < 0 {
                continue;
            }
            let l = (column as f32).max(a.0);
            let r = (column as f32 + 1.0).min(b.0);
            if r < l {
                continue;
            }
            let yl = a.1 + (b.1 - a.1) * (l - a.0) / (b.0 - a.0).max(1e-3);
            let yr = a.1 + (b.1 - a.1) * (r - a.0) / (b.0 - a.0).max(1e-3);
            let (lo, hi) = (yl.min(yr) - half, yl.max(yr) + half);
            for row in floor(lo).max(0)..=floor(hi).min(h as i32 - 1) {
                let overlap = (hi.min(row as f32 + 1.0) - lo.max(row as f32)).clamp(0.0, 1.0);
                let cell = &mut coverage[row as usize * w + column as usize];
                *cell = (*cell).max((overlap * 255.0 + 0.5) as u8);
            }
        }
    }
    for (i, c) in coverage.iter().enumerate() {
        if *c != 0 {
            blend(&mut out[i * 4..][..4], style.line, *c as f32 / 255.0);
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    const STYLE: Style = Style {
        background: [255, 255, 255],
        grid: None,
        line: [0, 0, 255],
        fill: 0,
    };
    fn ink(img: &[u8], w: usize, x: usize, y: usize) -> u8 {
        255 - img[(y * w + x) * 4 + 1]
    }
    #[test]
    fn steep_and_flat_segments_leave_no_gaps_and_follow_time() {
        let points = [
            (0, Some(0)),
            (100, Some(0)),
            (101, Some(100)),
            (200, Some(0)),
            (300, Some(50)),
        ];
        let plot = Plot {
            points: &points,
            max: 100,
            now: 300,
            span: 300,
        };
        for scale in 1..=3 {
            let (w, h) = (60, 40);
            let img = render(w, h, scale, &plot, &STYLE);
            let (pw, ph) = (w * scale, h * scale);
            // Every column inside the measured range carries stroke ink.
            for x in 1..pw - 1 {
                assert!(
                    (0..ph).any(|y| ink(&img, pw, x, y) > 100),
                    "gap at column {x} scale {scale}"
                );
            }
            // The near-vertical rise at one third of the width is continuous.
            let x = pw / 3;
            let painted = (0..ph).filter(|&y| ink(&img, pw, x, y) > 0).count();
            assert!(painted > ph * 3 / 4, "rise {painted}/{ph} at scale {scale}");
            assert!(img.chunks_exact(4).all(|p| p[3] == 255));
        }
    }
    #[test]
    fn missing_samples_break_the_line_and_old_points_scroll_out() {
        let points = [(0, Some(50)), (100, None), (200, Some(50)), (300, Some(50))];
        let plot = Plot {
            points: &points,
            max: 100,
            now: 300,
            span: 300,
        };
        let img = render(30, 20, 1, &plot, &STYLE);
        let column = |x: usize| (0..20).any(|y| ink(&img, 30, x, y) > 60);
        assert!(!column(5) && !column(15));
        assert!(column(25));
        // Time-based placement: the same data one span later is entirely gone.
        let later = Plot { now: 700, ..plot };
        let img = render(30, 20, 1, &later, &STYLE);
        assert!(img.chunks_exact(4).all(|p| p[..3] == [255, 255, 255]));
    }
    #[test]
    fn grid_columns_move_with_time() {
        let style = Style {
            grid: Some([0, 0, 0]),
            ..STYLE
        };
        let a = render(
            60,
            10,
            1,
            &Plot {
                points: &[],
                max: 1,
                now: 6000,
                span: 6000,
            },
            &style,
        );
        let b = render(
            60,
            10,
            1,
            &Plot {
                points: &[],
                max: 1,
                now: 6100,
                span: 6000,
            },
            &style,
        );
        assert_ne!(a, b);
    }
}
