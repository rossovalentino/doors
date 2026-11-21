//! Icons and cursors as user32 keeps them: premultiplied `0xAARRGGBB`
//! pixels, top row first. Building one from CreateIconIndirect's colour and
//! AND-mask bitmaps, the masks GetIconInfo hands back, resampling to the
//! size a caller asks for, and the system icons (IDI_* / OIC_*) drawn in
//! the Windows 11 style at any size.
//!
//! Inputs are untrusted sizes: every function bounds its work by
//! `MAX_EDGE` per side.
use alloc::vec;
use alloc::vec::Vec;

/// The largest icon edge accepted (Windows' own limit is 256 for ICO
/// entries; LoadImage may stretch further).
pub const MAX_EDGE: u32 = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Premultiplied `0xAARRGGBB`, `width * height`, top-down.
    pub px: Vec<u32>,
}
impl Image {
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
            return None;
        }
        Some(Self {
            width,
            height,
            px: vec![0; width as usize * height as usize],
        })
    }
    /// Premultiplied BGRA bytes (door-pe-icon's frames, asset pixels).
    pub fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Option<Self> {
        let mut image = Self::new(width, height)?;
        if bgra.len() != image.px.len() * 4 {
            return None;
        }
        for (p, b) in image.px.iter_mut().zip(bgra.chunks_exact(4)) {
            *p = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        Some(image)
    }
    /// Straight (unpremultiplied) `0xAARRGGBB`: the colour bitmap
    /// GetIconInfo returns for a 32-bit icon.
    pub fn straight(&self) -> Vec<u32> {
        self.px
            .iter()
            .map(|&p| {
                let a = p >> 24;
                if a == 0 {
                    return 0;
                }
                let un = |c: u32| ((c & 255) * 255 + a / 2) / a;
                a << 24 | un(p >> 16).min(255) << 16 | un(p >> 8).min(255) << 8 | un(p).min(255)
            })
            .collect()
    }
    /// The AND mask as 1-bpp rows padded to 16 bits (CreateBitmap's
    /// layout): set where the icon is transparent.
    pub fn and_mask(&self) -> Vec<u8> {
        let stride = (self.width as usize).div_ceil(16) * 2;
        let mut bits = vec![0u8; stride * self.height as usize];
        for y in 0..self.height as usize {
            for x in 0..self.width as usize {
                if self.px[y * self.width as usize + x] >> 24 == 0 {
                    bits[y * stride + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        bits
    }
    /// Area-averaged resample to `width` x `height` (shrinking) or
    /// bilinear (growing), in premultiplied space.
    pub fn resample(&self, width: u32, height: u32) -> Option<Self> {
        if (width, height) == (self.width, self.height) {
            return Some(self.clone());
        }
        let mut out = Self::new(width, height)?;
        let (sw, sh) = (self.width as usize, self.height as usize);
        let channel = |p: u32, i: u32| (p >> (8 * i)) & 255;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let mut sum = [0u32; 4];
                let mut n = 0u32;
                // Source footprint of the destination pixel.
                let x0 = x * sw / width as usize;
                let x1 = ((x + 1) * sw).div_ceil(width as usize).max(x0 + 1).min(sw);
                let y0 = y * sh / height as usize;
                let y1 = ((y + 1) * sh).div_ceil(height as usize).max(y0 + 1).min(sh);
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let p = self.px[sy * sw + sx];
                        for (i, s) in sum.iter_mut().enumerate() {
                            *s += channel(p, i as u32);
                        }
                        n += 1;
                    }
                }
                let n = n.max(1);
                out.px[y * width as usize + x] = sum
                    .iter()
                    .enumerate()
                    .fold(0, |acc, (i, s)| acc | ((s + n / 2) / n) << (8 * i));
            }
        }
        Some(out)
    }
}

/// CreateIconIndirect: a colour bitmap (`0xAARRGGBB`, straight alpha when
/// any pixel has some, else opaque) with its AND mask (`mask` white =
/// transparent), or a monochrome mask of twice the height (AND above XOR).
pub fn from_bitmaps(color: Option<(u32, u32, &[u32])>, mask: (u32, u32, &[u32])) -> Option<Image> {
    let (mw, mh, mpx) = mask;
    if mpx.len() != mw as usize * mh as usize {
        return None;
    }
    let white = |p: u32| p & 0x00ff_ffff == 0x00ff_ffff;
    match color {
        Some((w, h, px)) => {
            if px.len() != w as usize * h as usize {
                return None;
            }
            let mut image = Image::new(w, h)?;
            let alpha = px.iter().any(|p| p >> 24 != 0);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let p = px[y * w as usize + x];
                    let a = if alpha {
                        p >> 24
                    } else {
                        let hidden =
                            (x as u32) < mw && (y as u32) < mh && white(mpx[y * mw as usize + x]);
                        if hidden { 0 } else { 255 }
                    };
                    let m = |c: u32| ((c & 255) * a + 127) / 255;
                    image.px[y * w as usize + x] =
                        a << 24 | m(p >> 16) << 16 | m(p >> 8) << 8 | m(p);
                }
            }
            Some(image)
        }
        None => {
            // AND above XOR: AND 1 XOR 0 transparent, 0/0 black, 0/1
            // white; 1/1 inverts the screen (kept transparent here).
            if mh % 2 != 0 {
                return None;
            }
            let h = mh / 2;
            let mut image = Image::new(mw, h)?;
            for y in 0..h as usize {
                for x in 0..mw as usize {
                    let and = white(mpx[y * mw as usize + x]);
                    let xor = white(mpx[(y + h as usize) * mw as usize + x]);
                    image.px[y * mw as usize + x] = match (and, xor) {
                        (false, false) => 0xff00_0000,
                        (false, true) => 0xffff_ffff,
                        _ => 0,
                    };
                }
            }
            Some(image)
        }
    }
}

/// The system icons LoadIcon(NULL, IDI_*) and LoadImage(OIC_*) name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Application,
    Error,
    Question,
    Warning,
    Information,
    WinLogo,
    Shield,
}
impl System {
    /// IDI_* / OIC_* ids (32512..=32518).
    pub fn from_id(id: u64) -> Option<Self> {
        Some(match id {
            32512 => Self::Application,
            32513 => Self::Error,
            32514 => Self::Question,
            32515 => Self::Warning,
            32516 => Self::Information,
            32517 => Self::WinLogo,
            32518 => Self::Shield,
            _ => return None,
        })
    }
}

// Windows 11 palette (sRGB).
const RED: u32 = 0xc4_2b1c;
const BLUE: u32 = 0x00_5fb8;
const YELLOW: u32 = 0xff_c83d;
const SHIELD_YELLOW: u32 = 0xff_b900;
const WHITE: u32 = 0xff_ffff;
const INK: u32 = 0x1a_1a1a;
const FRAME: u32 = 0x86_8686;

/// A filled shape: coverage at a point of the unit square (0..1, y down).
type Shape = fn(f32, f32) -> bool;

fn circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> bool {
    (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r
}
/// Distance from (x, y) to the segment a-b is at most `r`.
fn capsule(x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (px, py) = (a.0 + t * dx - x, a.1 + t * dy - y);
    px * px + py * py <= r * r
}
fn disc(x: f32, y: f32) -> bool {
    circle(x, y, 0.5, 0.5, 0.46)
}
fn cross(x: f32, y: f32) -> bool {
    capsule(x, y, (0.33, 0.33), (0.67, 0.67), 0.045)
        || capsule(x, y, (0.67, 0.33), (0.33, 0.67), 0.045)
}
fn info_glyph(x: f32, y: f32) -> bool {
    circle(x, y, 0.5, 0.3, 0.06) || capsule(x, y, (0.5, 0.45), (0.5, 0.72), 0.048)
}
fn question_glyph(x: f32, y: f32) -> bool {
    // The hook: an arc from the left over the top to the right, then a
    // stroke down to the stem; the dot below.
    let (cx, cy, r) = (0.5, 0.38, 0.13);
    let (dx, dy) = (x - cx, y - cy);
    let d = libm_sqrt(dx * dx + dy * dy);
    let angle = libm_atan2(dy, dx);
    let on_arc = (d - r).abs() <= 0.045 && !(0.35..=2.6).contains(&angle);
    on_arc
        || capsule(x, y, (cx + r * 0.94, cy + r * 0.34), (0.5, 0.56), 0.045)
        || capsule(x, y, (0.5, 0.56), (0.5, 0.6), 0.045)
        || circle(x, y, 0.5, 0.72, 0.055)
}
fn warning_triangle(x: f32, y: f32) -> bool {
    // Apex (0.5, 0.1), base from (0.06, 0.88) to (0.94, 0.88), rounded by
    // testing a slightly smaller triangle with a margin.
    let inside = |x: f32, y: f32| (0.12..=0.86).contains(&y) && (x - 0.5).abs() <= (y - 0.1) * 0.56;
    inside(x, y)
        || capsule(x, y, (0.5, 0.12), (0.08, 0.86), 0.03)
        || capsule(x, y, (0.5, 0.12), (0.92, 0.86), 0.03)
        || capsule(x, y, (0.08, 0.86), (0.92, 0.86), 0.03)
}
fn warning_glyph(x: f32, y: f32) -> bool {
    capsule(x, y, (0.5, 0.36), (0.5, 0.6), 0.045) || circle(x, y, 0.5, 0.72, 0.05)
}
fn shield(x: f32, y: f32) -> bool {
    // A flat top with rounded shoulders down to a point.
    let top = 0.12;
    if y < top || !(0.14..=0.86).contains(&x) {
        return false;
    }
    if y <= 0.55 {
        return true;
    }
    // The lower edges curve in to the point at (0.5, 0.92).
    let t = (y - 0.55) / 0.37;
    (x - 0.5).abs() <= 0.36 * (1.0 - t * t)
}
fn shield_left(x: f32, y: f32) -> bool {
    shield(x, y) && x < 0.5
}
fn window_frame(x: f32, y: f32) -> bool {
    (0.1..=0.9).contains(&x) && (0.16..=0.84).contains(&y)
}
fn window_client(x: f32, y: f32) -> bool {
    (0.14..=0.86).contains(&x) && (0.3..=0.8).contains(&y)
}
fn window_title(x: f32, y: f32) -> bool {
    (0.14..=0.86).contains(&x) && (0.2..0.3).contains(&y)
}
fn logo(x: f32, y: f32) -> bool {
    let pane = |x: f32| (0.14..=0.48).contains(&x) || (0.52..=0.86).contains(&x);
    pane(x) && pane(y)
}

/// sqrt and atan2 without libm (no_std): Newton steps and a rational
/// approximation, accurate well past a pixel at icon sizes.
fn libm_sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut r = if v > 1.0 { v } else { 1.0 };
    for _ in 0..8 {
        r = 0.5 * (r + v / r);
    }
    r
}
fn libm_atan2(y: f32, x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let atan = |z: f32| z * (PI / 4.0) + 0.273 * z * (1.0 - z.abs());
    let (ax, ay) = (x.abs(), y.abs());
    let a = if ax >= ay {
        atan(ay / ax)
    } else {
        PI / 2.0 - atan(ax / ay)
    };
    match (x < 0.0, y < 0.0) {
        (false, false) => a,
        (true, false) => PI - a,
        (true, true) => -(PI - a),
        (false, true) => -a,
    }
}

/// Paint `layers` (shape, sRGB colour, opacity 0-255), bottom first, at
/// `size` px with 4x4 supersampling.
fn render(size: u32, layers: &[(Shape, u32, u32)]) -> Option<Image> {
    let mut image = Image::new(size, size)?;
    let s = size as f32;
    for y in 0..size {
        for x in 0..size {
            // Premultiplied accumulation in 0..=255 per channel.
            let mut acc = [0f32; 4];
            for (shape, rgb, opacity) in layers {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let u = (x as f32 + (sx as f32 + 0.5) / 4.0) / s;
                        let v = (y as f32 + (sy as f32 + 0.5) / 4.0) / s;
                        if shape(u, v) {
                            hits += 1;
                        }
                    }
                }
                if hits == 0 {
                    continue;
                }
                let a = hits as f32 / 16.0 * *opacity as f32 / 255.0;
                let c = [rgb & 255, rgb >> 8 & 255, rgb >> 16 & 255];
                for i in 0..3 {
                    acc[i] = c[i] as f32 * a + acc[i] * (1.0 - a);
                }
                acc[3] = 255.0 * a + acc[3] * (1.0 - a);
            }
            let ch = |v: f32| (v + 0.5).clamp(0.0, 255.0) as u32;
            image.px[(y * size + x) as usize] =
                ch(acc[3]) << 24 | ch(acc[2]) << 16 | ch(acc[1]) << 8 | ch(acc[0]);
        }
    }
    Some(image)
}

/// A system icon at `size` px.
pub fn system(which: System, size: u32) -> Option<Image> {
    match which {
        System::Error => render(size, &[(disc, RED, 255), (cross, WHITE, 255)]),
        System::Information => render(size, &[(disc, BLUE, 255), (info_glyph, WHITE, 255)]),
        System::Question => render(size, &[(disc, BLUE, 255), (question_glyph, WHITE, 255)]),
        System::Warning => render(
            size,
            &[(warning_triangle, YELLOW, 255), (warning_glyph, INK, 240)],
        ),
        System::Shield => render(
            size,
            &[(shield, SHIELD_YELLOW, 255), (shield_left, BLUE, 255)],
        ),
        System::Application => render(
            size,
            &[
                (window_frame, FRAME, 255),
                (window_title, BLUE, 255),
                (window_client, WHITE, 255),
            ],
        ),
        System::WinLogo => render(size, &[(logo, 0x00_78d4, 255)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_icons_draw_their_shapes_at_any_size() {
        for which in [
            System::Application,
            System::Error,
            System::Question,
            System::Warning,
            System::Information,
            System::WinLogo,
            System::Shield,
        ] {
            for size in [16, 32, 48, 256] {
                let icon = system(which, size).unwrap();
                assert_eq!(icon.px.len(), (size * size) as usize);
                // Corners transparent, the middle opaque.
                assert_eq!(icon.px[0] >> 24, 0, "{which:?} corner at {size}");
                let opaque = icon.px.iter().filter(|p| *p >> 24 == 255).count();
                assert!(opaque >= (size * size / 4) as usize, "{which:?} at {size}");
            }
        }
        // Error: red disc, white cross through the centre.
        let e = system(System::Error, 32).unwrap();
        assert_eq!(e.px[16 * 32 + 16] & 0xffffff, 0xffffff);
        assert_eq!(e.px[16 * 32 + 5] & 0xffffff, RED);
        assert!(System::from_id(32519).is_none());
        assert_eq!(System::from_id(32516), Some(System::Information));
    }
    #[test]
    fn masks_and_colour_bitmaps_round_trip() {
        // 2x1: opaque red, transparent.
        let color = [0x00ff_0000u32, 0x0012_3456];
        let mask = [0x0000_0000u32, 0x00ff_ffff];
        let icon = from_bitmaps(Some((2, 1, &color)), (2, 1, &mask)).unwrap();
        assert_eq!(icon.px, [0xffff_0000, 0]);
        assert_eq!(icon.and_mask(), [0x40, 0]);
        assert_eq!(icon.straight(), [0xffff_0000, 0]);
        // Per-pixel alpha wins over the mask and is premultiplied.
        let color = [0x80ff_ffffu32, 0xff00_00ff];
        let icon = from_bitmaps(Some((2, 1, &color)), (2, 1, &mask)).unwrap();
        assert_eq!(icon.px[0], 0x8080_8080);
        assert_eq!(icon.px[1], 0xff00_00ff);
        assert_eq!(icon.straight()[0], 0x80ff_ffff);
        // Monochrome: AND above XOR.
        let mono = [0x00ff_ffffu32, 0, 0, 0x00ff_ffff, 0, 0x00ff_ffff];
        let icon = from_bitmaps(None, (3, 2, &mono)).unwrap();
        assert_eq!(icon.px, [0, 0xff00_0000, 0xffff_ffff]);
        assert!(from_bitmaps(None, (3, 1, &mono[..3])).is_none());
    }
    #[test]
    fn resampling_averages_down_and_keeps_alpha() {
        let mut big = Image::new(4, 4).unwrap();
        big.px[..2].copy_from_slice(&[0xffff_ffff, 0xffff_ffff]);
        big.px[4..6].copy_from_slice(&[0xffff_ffff, 0xffff_ffff]);
        let small = big.resample(2, 2).unwrap();
        assert_eq!(small.px, [0xffff_ffff, 0, 0, 0]);
        let grown = small.resample(4, 4).unwrap();
        assert_eq!(grown.px[0], 0xffff_ffff);
        assert_eq!(grown.px[15], 0);
        assert!(big.resample(0, 4).is_none());
        assert!(big.resample(MAX_EDGE + 1, 4).is_none());
    }
}
#[cfg(test)]
mod dump {
    extern crate std;
    use super::*;
    /// DOOR_ICON_DUMP=<file.bmp>: the system icons at 48 px side by side
    /// over white, for a visual check. `--ignored`.
    #[test]
    #[ignore]
    fn dump_system_icons() {
        let Ok(path) = std::env::var("DOOR_ICON_DUMP") else {
            return;
        };
        let all = [
            System::Application,
            System::Error,
            System::Question,
            System::Warning,
            System::Information,
            System::WinLogo,
            System::Shield,
        ];
        let (s, w, h) = (48usize, 48 * 7 + 8 * 8, 64usize);
        let mut px = std::vec![0xffu8; w * h * 3];
        for (i, which) in all.iter().enumerate() {
            let icon = system(*which, s as u32).unwrap();
            for y in 0..s {
                for x in 0..s {
                    let p = icon.px[y * s + x];
                    let a = p >> 24;
                    let (ox, oy) = (8 + i * (s + 8) + x, 8 + y);
                    let at = ((h - 1 - oy) * w + ox) * 3;
                    for c in 0..3 {
                        let v = (p >> (8 * c)) & 255;
                        px[at + c] = (v + 255 * (255 - a) / 255) as u8;
                    }
                }
            }
        }
        let mut f = std::vec::Vec::new();
        let size = 54 + px.len() as u32;
        f.extend_from_slice(b"BM");
        f.extend_from_slice(&size.to_le_bytes());
        f.extend_from_slice(&[0; 4]);
        f.extend_from_slice(&54u32.to_le_bytes());
        f.extend_from_slice(&40u32.to_le_bytes());
        f.extend_from_slice(&(w as u32).to_le_bytes());
        f.extend_from_slice(&(h as u32).to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&24u16.to_le_bytes());
        f.extend_from_slice(&[0; 24]);
        f.extend_from_slice(&px);
        std::fs::write(path, f).unwrap();
    }
}
