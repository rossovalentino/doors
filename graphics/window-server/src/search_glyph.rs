//! The Windows 11 26H2 taskbar Search glyph: a ring filled by a sweep
//! gradient (Windows blue round the bottom and left, cyan near 12 o'clock,
//! green at 1-2 o'clock, teal down the right side) over a frosted disc, with
//! a solid blue handle to the lower right. Measured from the owner's 26H2
//! screenshot at 150 % (a 34 px glyph: a 30 px ring with a 4 px stroke and
//! a 4 px handle), i.e. a 24 DIP design box. It is rasterized here as an
//! antialiased vector shape at the canvas scale, so fractional scales stay
//! crisp. Contrast themes keep the monochrome Segoe Fluent mask in the
//! system text colour.
use crate::paint::{self, Palette};
use door_graphics_core::Canvas;

/// Design box (DIP) the measurements are expressed in.
pub const DESIGN: u32 = 24;
/// Fixed point: 1/256 of a design DIP (or of a physical pixel).
const ONE: i64 = 256;
/// Ring centre (10.5, 10.5), outer radius 10 (30 px at 150 %) and stroke
/// 8/3 DIP (4 px at 150 %), in 1/256 design DIP.
const CENTER: i64 = 2688;
const OUTER: i64 = 2560;
const STROKE: i64 = 683;
/// Ring centre line radius and half stroke.
pub const MID: i64 = OUTER - STROKE / 2;
const HALF: i64 = STROKE / 2;
/// Handle: a round-capped bar 8/3 DIP thick (4 px at 150 %) from the ring's
/// lower right (45°) to (22, 22) near the bottom-right corner.
const HANDLE_END: i64 = 22 * ONE;
const HANDLE_HALF: i64 = 341;
/// Handle colour (#0077D4, sRGB).
pub const HANDLE: [u8; 3] = [0x00, 0x77, 0xD4];
/// Frosted inner disc: white at about 16 % (#504840-ish over #2E2013).
pub const DISC_ALPHA: u8 = 42;
/// Pressed feedback: the glyph shrinks to 87.5 % while its button is held.
pub const PRESSED_PERMILLE: u32 = 875;
/// Ring sweep stops: degrees clockwise from 12 o'clock and sRGB colour.
pub const STOPS: [(u32, [u8; 3]); 14] = [
    (0, [0x2E, 0xAB, 0xA7]),
    (30, [0x50, 0xD2, 0x86]),
    (60, [0x5E, 0xE1, 0x78]),
    (74, [0x49, 0xCA, 0x8C]),
    (88, [0x3E, 0xBE, 0x97]),
    (102, [0x33, 0xB2, 0xA1]),
    (116, [0x29, 0xA5, 0xAB]),
    (128, [0x1E, 0x9A, 0xB6]),
    (140, [0x07, 0x81, 0xCC]),
    (150, [0x00, 0x78, 0xD3]),
    (300, [0x00, 0x78, 0xD3]),
    (320, [0x0B, 0x84, 0xC8]),
    (340, [0x16, 0x91, 0xBD]),
    (360, [0x2E, 0xAB, 0xA7]),
];

/// The ring colour at `degrees` (clockwise from 12 o'clock, x256).
pub fn sweep(degrees: i64) -> [u8; 3] {
    let d = degrees.rem_euclid(360 * ONE);
    for pair in STOPS.windows(2) {
        let (a, ca) = pair[0];
        let (b, cb) = pair[1];
        let (a, b) = (a as i64 * ONE, b as i64 * ONE);
        if d >= a && d <= b {
            let t = d - a;
            let span = (b - a).max(1);
            let mut out = [0u8; 3];
            for (o, (x, y)) in out.iter_mut().zip(ca.iter().zip(cb.iter())) {
                *o = ((*x as i64 * (span - t) + *y as i64 * t + span / 2) / span) as u8;
            }
            return out;
        }
    }
    STOPS[0].1
}

/// atan(t / 65536) in degrees x256 for t in 0..=65536 (max error ~0.1°).
fn atan_unit(t: i64) -> i64 {
    let k = 3589 + 973 * t / 65536;
    11520 * t / 65536 - (t * (t - 65536) / 65536) * k / 65536
}

/// Angle of (`dx`, `dy`) (y down) clockwise from 12 o'clock, degrees x256.
pub fn clock_degrees(dx: i64, dy: i64) -> i64 {
    let (ux, uy) = (dx, -dy);
    let (ax, ay) = (ux.abs(), uy.abs());
    if ax == 0 && ay == 0 {
        return 0;
    }
    let base = if ax <= ay {
        atan_unit(ax * 65536 / ay)
    } else {
        90 * ONE - atan_unit(ay * 65536 / ax)
    };
    match (ux >= 0, uy >= 0) {
        (true, true) => base,
        (true, false) => 180 * ONE - base,
        (false, false) => 180 * ONE + base,
        (false, true) => (360 * ONE - base) % (360 * ONE),
    }
}

fn isqrt(n: i64) -> i64 {
    if n <= 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Coverage 0..=255 of a shape whose signed distance from the pixel centre
/// is `d` physical pixels x256 (inside negative): a one-pixel ramp.
fn coverage(d: i64) -> u8 {
    ((ONE / 2 - d).clamp(0, ONE) * 255 / ONE) as u8
}

/// Distance (design x256) from `p` to the segment `a`..`b`.
fn segment_distance(p: (i64, i64), a: (i64, i64), b: (i64, i64)) -> i64 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = (vx * vx + vy * vy).max(1);
    let t = ((wx * vx + wy * vy) * 4096 / len2).clamp(0, 4096);
    let (cx, cy) = (a.0 + vx * t / 4096, a.1 + vy * t / 4096);
    isqrt((p.0 - cx).pow(2) + (p.1 - cy).pow(2))
}

/// Draw the glyph in a `size` DIP box at logical (`x`, `y`), scaled about
/// the box centre by `permille` (the taskbar's pressed feedback).
pub fn draw(c: &mut Canvas<'_>, x: i32, y: i32, size: u32, palette: &Palette, permille: u32) {
    if palette.contrast {
        // Forced colours: the monochrome system glyph, centred.
        let (mask, s) = if size >= 24 {
            (crate::shell_mask!("shell-taskbar-search", 24), 24)
        } else {
            (crate::shell_mask!("shell-search", 16), 16)
        };
        let o = (size as i32 - s as i32) / 2;
        paint::glyph(c, x + o, y + o, s, mask, palette.text);
        return;
    }
    let scale = c.scale();
    let Some(clip) = c.clip_rect() else {
        return;
    };
    // Physical pixels per design DIP = num / den.
    let num = size as i64 * permille.clamp(1, 1000) as i64 * scale.percent() as i64;
    let den = DESIGN as i64 * 1000 * 100;
    let box_px = scale.len(size) as i64;
    let centre = (
        scale.px(x) as i64 * ONE + box_px * ONE / 2,
        scale.px(y) as i64 * ONE + box_px * ONE / 2,
    );
    let half = (DESIGN as i64 * ONE / 2) * num / den / ONE + 2;
    let mid_design = DESIGN as i64 * ONE / 2;
    let handle_start = (CENTER + MID * 181 / 256, CENTER + MID * 181 / 256);
    let handle_end = (HANDLE_END, HANDLE_END);
    let to_px = |d: i64| d * num / den;
    let disc_color = paint::white(DISC_ALPHA);
    let handle_color = paint::rgb(HANDLE[0], HANDLE[1], HANDLE[2]);
    let (cx_px, cy_px) = (centre.0 / ONE, centre.1 / ONE);
    for py in cy_px - half..=cy_px + half {
        for px in cx_px - half..=cx_px + half {
            if !clip.contains(px as i32, py as i32) {
                continue;
            }
            // Pixel centre in design space (x256).
            let vx = mid_design + ((px * ONE + ONE / 2) - centre.0) * den / num;
            let vy = mid_design + ((py * ONE + ONE / 2) - centre.1) * den / num;
            let (dx, dy) = (vx - CENTER, vy - CENTER);
            let r = isqrt(dx * dx + dy * dy);
            let disc = coverage(to_px(r - MID));
            if disc > 0 {
                let _ = c.pixel(px as i32, py as i32, paint::alpha(disc_color, disc));
            }
            let handle = coverage(to_px(
                segment_distance((vx, vy), handle_start, handle_end) - HANDLE_HALF,
            ));
            if handle > 0 {
                let _ = c.pixel(px as i32, py as i32, paint::alpha(handle_color, handle));
            }
            let ring = coverage(to_px((r - MID).abs() - HALF));
            if ring > 0 {
                let [r8, g8, b8] = sweep(clock_degrees(dx, dy));
                let _ = c.pixel(
                    px as i32,
                    py as i32,
                    paint::alpha(paint::rgb(r8, g8, b8), ring),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_angles_and_sweep_stops() {
        assert_eq!(clock_degrees(0, -10), 0);
        assert!((clock_degrees(10, 0) - 90 * ONE).abs() < 32);
        assert!((clock_degrees(0, 10) - 180 * ONE).abs() < 32);
        assert!((clock_degrees(-10, 0) - 270 * ONE).abs() < 32);
        assert!((clock_degrees(10, -10) - 45 * ONE).abs() < 32);
        assert!((clock_degrees(-10, 10) - 225 * ONE).abs() < 32);
        assert_eq!(sweep(0), [0x2E, 0xAB, 0xA7]);
        assert_eq!(sweep(200 * ONE), [0x00, 0x78, 0xD3]);
        assert_eq!(sweep(60 * ONE), [0x5E, 0xE1, 0x78]);
        assert_eq!(sweep(359 * ONE + 255)[2], 0xA7);
    }
}
