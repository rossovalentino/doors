//! Visual fidelity harness (master specification sections 111, 112 and
//! 140.6; acceptance VIS-01, :4277).
//!
//! Five independent dimensions — geometry, colour, typography, iconography and
//! motion — each with thresholds frozen in [`FROZEN`] before any qualification
//! run and mirrored in the reference profile ([`PROFILE`]); [`compare`] refuses
//! a profile whose thresholds differ. Every probe is reported on its own with
//! measured and expected values, so a small but salient deviation is never
//! averaged away. Probes measure the guest capture and the reference capture
//! with the same code; the reference values are frozen in the profile and
//! re-derived from the reference bytes by a host test.
//!
//! Probe states: `frozen` (compared; failure fails the suite),
//! `approved_deviation` (compared and reported as an exception),
//! `uncontrolled` (the reference depends on an uncontrolled input such as the
//! owner's wallpaper behind acrylic: reported only) and `not_captured` (no
//! reference yet: unresolved, which fails closed). A dimension passes only if
//! it has at least one passing probe and no failed or unresolved probe; the
//! suite passes only if all five dimensions pass.
use serde_json::{Value, json};
use std::path::Path;

mod vis01;
// Registered in `scenarios::RUNNERS` by the gate owner.
#[allow(unused_imports)]
pub use vis01::vis_01_suite;

/// The frozen Windows 11 26H2 reference profile.
pub const PROFILE: &str = "profiles/windows-reference/reference-26h2-x64.json";

/// Thresholds frozen on 2026-10-26, before any VIS-01 qualification run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Bounds, band heights and run widths, logical px.
    pub geometry_logical: f64,
    /// CIEDE2000 between measured and reference surface colours.
    pub colour_delta_e: f64,
    /// Text ink block height and vertical position, logical px.
    pub typography_logical: f64,
    /// Icon ink box size and optical centre, logical px.
    pub iconography_logical: f64,
    /// Motion track settle duration, ms.
    pub motion_ms: f64,
    /// CIE76 distance separating ink from its background.
    pub ink_delta_e: f64,
    /// CIE76 distance ending a uniform band (taskbar edge).
    pub band_delta_e: f64,
    /// CIE76 distance bounding a uniform run (surface extent).
    pub run_delta_e: f64,
}
pub const FROZEN: Thresholds = Thresholds {
    geometry_logical: 2.0,
    colour_delta_e: 5.0,
    typography_logical: 1.5,
    iconography_logical: 1.5,
    motion_ms: 50.0,
    ink_delta_e: 20.0,
    band_delta_e: 10.0,
    run_delta_e: 6.0,
};
pub const FROZEN_DATE: &str = "2026-10-26";
impl Thresholds {
    pub fn to_json(self) -> Value {
        json!({"frozen":FROZEN_DATE,"geometry_logical":self.geometry_logical,
            "colour_delta_e2000":self.colour_delta_e,"typography_logical":self.typography_logical,
            "iconography_logical":self.iconography_logical,"motion_ms":self.motion_ms,
            "ink_delta_e76":self.ink_delta_e,"band_delta_e76":self.band_delta_e,
            "run_delta_e76":self.run_delta_e})
    }
}

/// Whether a profile's `thresholds` record mirrors [`FROZEN`] exactly
/// (numbers compared as values, so `2` and `2.0` agree).
pub fn thresholds_match(recorded: &Value) -> bool {
    let frozen = FROZEN.to_json();
    let (Some(a), Some(b)) = (recorded.as_object(), frozen.as_object()) else {
        return false;
    };
    a.len() == b.len()
        && b.iter()
            .all(|(key, want)| match (want.as_f64(), a.get(key)) {
                (Some(w), Some(got)) => got.as_f64() == Some(w),
                (None, Some(got)) => got == want,
                _ => false,
            })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Geometry,
    Colour,
    Typography,
    Iconography,
    Motion,
}
impl Dimension {
    pub const ALL: [Self; 5] = [
        Self::Geometry,
        Self::Colour,
        Self::Typography,
        Self::Iconography,
        Self::Motion,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Geometry => "geometry",
            Self::Colour => "colour",
            Self::Typography => "typography",
            Self::Iconography => "iconography",
            Self::Motion => "motion",
        }
    }
    fn parse(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|d| d.name() == name)
            .ok_or_else(|| format!("unknown fidelity dimension {name}"))
    }
    fn tolerance(self, t: &Thresholds) -> f64 {
        match self {
            Self::Geometry => t.geometry_logical,
            Self::Colour => t.colour_delta_e,
            Self::Typography => t.typography_logical,
            Self::Iconography => t.iconography_logical,
            Self::Motion => t.motion_ms,
        }
    }
}

/// An opaque RGB8 capture (QEMU PPM screendump or reference PNG).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}
impl Image {
    pub fn new(width: usize, height: usize, rgb: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 || rgb.len() != width * height * 3 {
            return Err("fidelity image extent".into());
        }
        Ok(Self { width, height, rgb })
    }
    #[cfg(test)]
    pub fn filled(width: usize, height: usize, colour: [u8; 3]) -> Self {
        Self {
            width,
            height,
            rgb: colour.repeat(width * height),
        }
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.starts_with(b"P6") {
            return Self::from_ppm(&bytes);
        }
        let pixmap = resvg::tiny_skia::Pixmap::decode_png(&bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut rgb = Vec::with_capacity(pixmap.width() as usize * pixmap.height() as usize * 3);
        for p in pixmap.pixels() {
            // Captures are opaque; demultiply defensively.
            let c = p.demultiply();
            rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
        }
        Self::new(pixmap.width() as usize, pixmap.height() as usize, rgb)
    }
    fn from_ppm(bytes: &[u8]) -> Result<Self, String> {
        let mut offset = 0;
        let mut tokens = Vec::new();
        while tokens.len() < 4 {
            while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
                offset += 1;
            }
            let begin = offset;
            while bytes.get(offset).is_some_and(|b| !b.is_ascii_whitespace()) {
                offset += 1;
            }
            if begin == offset {
                return Err("truncated PPM header".into());
            }
            tokens.push(std::str::from_utf8(&bytes[begin..offset]).map_err(|e| e.to_string())?);
        }
        if tokens[0] != "P6" || tokens[3] != "255" {
            return Err("unsupported PPM".into());
        }
        let width = tokens[1].parse::<usize>().map_err(|e| e.to_string())?;
        let height = tokens[2].parse::<usize>().map_err(|e| e.to_string())?;
        let data = bytes.get(offset + 1..).ok_or("PPM data")?;
        Self::new(width, height, data.to_vec())
    }
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.width + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
    #[cfg(test)]
    pub fn fill_rect(&mut self, r: Rect, colour: [u8; 3]) {
        for y in r.y0..r.y1.min(self.height) {
            for x in r.x0..r.x1.min(self.width) {
                let i = (y * self.width + x) * 3;
                self.rgb[i..i + 3].copy_from_slice(&colour);
            }
        }
    }
}

/// CIE L*a*b* (D65) of an sRGB colour.
pub fn lab(c: [u8; 3]) -> [f64; 3] {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}
/// CIE76 distance (cheap; ink and band detection).
pub fn delta_e76(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (p, q) = (lab(a), lab(b));
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}
/// CIEDE2000 (Sharma, Wu and Dalal 2005), the colour-dimension metric.
pub fn delta_e2000(a: [u8; 3], b: [u8; 3]) -> f64 {
    delta_e2000_lab(lab(a), lab(b))
}
/// CIEDE2000 between two L*a*b* colours.
pub fn delta_e2000_lab([l1, a1, b1]: [f64; 3], [l2, a2, b2]: [f64; 3]) -> f64 {
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |bb: f64, ap: f64| {
        if bb == 0.0 && ap == 0.0 {
            0.0
        } else {
            let h = bb.atan2(ap).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1, h2) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hm).to_radians().cos()
        + 0.32 * (3.0 * hm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hm - 63.0).to_radians().cos();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let rt = -rc * (2.0 * dtheta).to_radians().sin();
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
        .sqrt()
}
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Physical pixel rectangle `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}
impl Rect {
    pub fn width(&self) -> usize {
        self.x1 - self.x0
    }
    pub fn height(&self) -> usize {
        self.y1 - self.y0
    }
    fn json(&self) -> Value {
        json!([self.x0, self.y0, self.width(), self.height()])
    }
}

/// A region in logical units, offset from an anchor of the capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub anchor: Anchor,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    BottomCenter,
    Center,
}
impl Region {
    fn parse(v: &Value) -> Result<Self, String> {
        let anchor = match v["anchor"].as_str().ok_or("region anchor")? {
            "top_left" => Anchor::TopLeft,
            "top_right" => Anchor::TopRight,
            "bottom_left" => Anchor::BottomLeft,
            "bottom_right" => Anchor::BottomRight,
            "bottom_center" => Anchor::BottomCenter,
            "center" => Anchor::Center,
            other => return Err(format!("unknown region anchor {other}")),
        };
        let n = |k: &str| v[k].as_f64().ok_or_else(|| format!("region {k}"));
        Ok(Self {
            anchor,
            x: n("x")?,
            y: n("y")?,
            w: n("w")?,
            h: n("h")?,
        })
    }
    /// Physical rectangle at `scale` (1.5 = 150 %), clamped to the image.
    pub fn physical(&self, image: &Image, scale: f64) -> Result<Rect, String> {
        let (w, h) = (image.width as f64, image.height as f64);
        let (ax, ay) = match self.anchor {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::TopRight => (w, 0.0),
            Anchor::BottomLeft => (0.0, h),
            Anchor::BottomRight => (w, h),
            Anchor::BottomCenter => (w / 2.0, h),
            Anchor::Center => (w / 2.0, h / 2.0),
        };
        let x0 = (ax + self.x * scale).round().clamp(0.0, w) as usize;
        let y0 = (ay + self.y * scale).round().clamp(0.0, h) as usize;
        let x1 = (ax + (self.x + self.w) * scale).round().clamp(0.0, w) as usize;
        let y1 = (ay + (self.y + self.h) * scale).round().clamp(0.0, h) as usize;
        if x1 <= x0 || y1 <= y0 {
            return Err("probe region lies outside the capture".into());
        }
        Ok(Rect { x0, y0, x1, y1 })
    }
}

fn median(mut values: Vec<u8>) -> u8 {
    values.sort_unstable();
    values[values.len() / 2]
}
fn median_colour(pixels: impl Iterator<Item = [u8; 3]>) -> [u8; 3] {
    let (mut r, mut g, mut b) = (Vec::new(), Vec::new(), Vec::new());
    for p in pixels {
        r.push(p[0]);
        g.push(p[1]);
        b.push(p[2]);
    }
    if r.is_empty() {
        return [0; 3];
    }
    [median(r), median(g), median(b)]
}

/// Height in rows of the uniform band at the bottom of `image`, sampled in
/// columns `[x0, x1)`: rows are walked upward while their median colour
/// stays within `band_delta_e` of the bottom rows.
pub fn band_height(image: &Image, x0: usize, x1: usize, max_rows: usize, t: &Thresholds) -> usize {
    let row = |y: usize| median_colour((x0..x1).map(|x| image.pixel(x, y)));
    let bottom = image.height - 1;
    let band = median_colour((0..3.min(image.height)).map(|i| row(bottom - i)));
    let mut rows = 0;
    while rows < max_rows.min(image.height) && delta_e76(row(bottom - rows), band) <= t.band_delta_e
    {
        rows += 1;
    }
    rows
}
/// Bounding box of the ink in `r`: pixels farther than `ink_delta_e` from the
/// median colour of the region's border.
pub fn ink_box(image: &Image, r: Rect, t: &Thresholds) -> Option<Rect> {
    let border = (r.x0..r.x1)
        .flat_map(|x| [(x, r.y0), (x, r.y1 - 1)])
        .chain((r.y0..r.y1).flat_map(|y| [(r.x0, y), (r.x1 - 1, y)]))
        .map(|(x, y)| image.pixel(x, y));
    let background = median_colour(border);
    let mut found: Option<Rect> = None;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            if delta_e76(image.pixel(x, y), background) > t.ink_delta_e {
                found = Some(found.map_or(
                    Rect {
                        x0: x,
                        y0: y,
                        x1: x + 1,
                        y1: y + 1,
                    },
                    |f| Rect {
                        x0: f.x0.min(x),
                        y0: f.y0.min(y),
                        x1: f.x1.max(x + 1),
                        y1: f.y1.max(y + 1),
                    },
                ));
            }
        }
    }
    found
}
/// The first ink cluster in a horizontal band, scanning columns rightward
/// from `start_x`: the ink box of the `cluster_width` columns beginning at
/// the first column that holds ink (the taskbar's first button glyph).
pub fn first_ink_box(
    image: &Image,
    band: Rect,
    start_x: usize,
    cluster_width: usize,
    t: &Thresholds,
) -> Option<Rect> {
    // The band's own colour: the median over a sparse grid of the whole band
    // (glyphs and a 1 px edge are a small minority).
    let background = median_colour((band.y0..band.y1).step_by(3).flat_map(|y| {
        (band.x0..band.x1)
            .step_by(7)
            .map(move |x| image.pixel(x, y))
    }));
    let first = (start_x.max(band.x0)..band.x1).find(|&x| {
        (band.y0..band.y1).any(|y| delta_e76(image.pixel(x, y), background) > t.ink_delta_e)
    })?;
    let cluster = Rect {
        x0: first,
        y0: band.y0,
        x1: (first + cluster_width).min(band.x1),
        y1: band.y1,
    };
    let mut found: Option<Rect> = None;
    for y in cluster.y0..cluster.y1 {
        for x in cluster.x0..cluster.x1 {
            if delta_e76(image.pixel(x, y), background) > t.ink_delta_e {
                found = Some(found.map_or(
                    Rect {
                        x0: x,
                        y0: y,
                        x1: x + 1,
                        y1: y + 1,
                    },
                    |f| Rect {
                        x0: f.x0.min(x),
                        y0: f.y0.min(y),
                        x1: f.x1.max(x + 1),
                        y1: f.y1.max(y + 1),
                    },
                ));
            }
        }
    }
    found
}
/// Most frequent colour in `r` (5-bit buckets; the bucket's mean) and its share.
pub fn dominant_colour(image: &Image, r: Rect) -> ([u8; 3], f64) {
    let mut counts = vec![(0u32, [0u64; 3]); 1 << 15];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let p = image.pixel(x, y);
            let key = (usize::from(p[0] >> 3) << 10)
                | (usize::from(p[1] >> 3) << 5)
                | usize::from(p[2] >> 3);
            let entry = &mut counts[key];
            entry.0 += 1;
            for (sum, v) in entry.1.iter_mut().zip(p) {
                *sum += u64::from(v);
            }
        }
    }
    let (count, sums) = counts
        .iter()
        .max_by_key(|(count, _)| *count)
        .copied()
        .unwrap_or_default();
    let mean = sums.map(|s| (s / u64::from(count.max(1))) as u8);
    (mean, f64::from(count) / (r.width() * r.height()) as f64)
}
/// Widest horizontal run, over the rows of `r`, of pixels within
/// `run_delta_e` of `colour` that contains the region's centre column.
pub fn run_width(image: &Image, r: Rect, colour: [u8; 3], t: &Thresholds) -> usize {
    let centre = (r.x0 + r.x1) / 2;
    let near = |x: usize, y: usize| delta_e76(image.pixel(x, y), colour) <= t.run_delta_e;
    (r.y0..r.y1)
        .filter(|&y| near(centre, y))
        .map(|y| {
            let left = (0..centre).rev().take_while(|&x| near(x, y)).count();
            let right = (centre..image.width).take_while(|&x| near(x, y)).count();
            left + right
        })
        .max()
        .unwrap_or(0)
}
/// Time from the trigger (t = 0) to the first frame after which `r` stays
/// identical to the final frame; `None` if the sequence never settles.
pub fn settle_ms(
    frames: &[(f64, Image)],
    r: &dyn Fn(&Image) -> Result<Rect, String>,
) -> Result<Option<f64>, String> {
    let Some((_, last)) = frames.last() else {
        return Ok(None);
    };
    let region = r(last)?;
    let same = |a: &Image| {
        (region.y0..region.y1)
            .all(|y| (region.x0..region.x1).all(|x| a.pixel(x, y) == last.pixel(x, y)))
    };
    let mut settled = frames.len() - 1;
    while settled > 0 && same(&frames[settled - 1].1) {
        settled -= 1;
    }
    if settled + 1 >= frames.len() && frames.len() > 1 {
        // Still changing in the last pair: not settled within the sequence.
        return Ok(None);
    }
    Ok(Some(frames[settled].0))
}

/// Duration of the visible motion: from the first frame in which `r` differs
/// from the trigger frame (frame 0) to the settled frame (`settle_ms`). The
/// input path before the first change (a real keyboard on the reference, an
/// emulator's input queue on a guest) is not part of the animation. `None`
/// if the region never changes or never settles.
pub fn animation_ms(
    frames: &[(f64, Image)],
    r: &dyn Fn(&Image) -> Result<Rect, String>,
) -> Result<Option<f64>, String> {
    let Some(settled) = settle_ms(frames, r)? else {
        return Ok(None);
    };
    let (Some((_, first)), Some((_, last))) = (frames.first(), frames.last()) else {
        return Ok(None);
    };
    let region = r(last)?;
    let differs = |a: &Image| {
        (region.y0..region.y1)
            .any(|y| (region.x0..region.x1).any(|x| a.pixel(x, y) != first.pixel(x, y)))
    };
    Ok(frames
        .iter()
        .skip(1)
        .find(|(_, image)| differs(image))
        .map(|(started, _)| ((settled - started) * 100.0).round() / 100.0))
}
/// The runs of a reference motion sequence (`sequence.json` beside its
/// frames): each run's kept frames with their times, every frame checked
/// against the SHA-256 the sequence records.
pub fn reference_runs(root: &Path, sequence: &str) -> Result<Vec<Vec<(f64, Image)>>, String> {
    let path = root.join(sequence);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{sequence}: {e}"))?;
    let record: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("{sequence}: {e}"))?;
    if record["record_kind"] != "windows_motion_reference" {
        return Err(format!("{sequence}: not a motion reference"));
    }
    let dir = path.parent().ok_or("sequence directory")?;
    let mut runs = Vec::new();
    for run in record["runs"].as_array().ok_or("sequence runs")? {
        let mut frames = Vec::new();
        for frame in run["kept"].as_array().ok_or("sequence frames")? {
            let file = dir.join(frame["file"].as_str().ok_or("frame file")?);
            if crate::evidence::file_hash(&file)? != frame["sha256"] {
                return Err(format!("{} changed since it was recorded", file.display()));
            }
            let at = frame["t_ms"].as_f64().ok_or("frame time")?;
            frames.push((at, Image::read(&file)?));
        }
        runs.push(frames);
    }
    if runs.is_empty() {
        return Err(format!("{sequence}: no runs"));
    }
    Ok(runs)
}
/// A motion probe's reference value: the mean over the reference runs of
/// `animation_ms` in the probe's region, to 0.1 ms.
pub fn reference_animation_ms(root: &Path, probe: &Value) -> Result<f64, String> {
    let sequence = probe["reference"]["sequence"]
        .as_str()
        .ok_or("reference sequence")?;
    let scale = probe["reference"]["scale_percent"]
        .as_f64()
        .ok_or("reference scale")?
        / 100.0;
    let region = Region::parse(&probe["params"]["region"])?;
    let mut total = 0.0;
    let runs = reference_runs(root, sequence)?;
    for frames in &runs {
        total += animation_ms(frames, &|image| region.physical(image, scale))?
            .ok_or_else(|| format!("{sequence}: a run never settles"))?;
    }
    Ok((total / runs.len() as f64 * 10.0).round() / 10.0)
}
/// One guest or reference capture of a declared surface.
pub struct Capture {
    pub surface: String,
    pub image: Image,
    /// Display scale, percent (150 for the reference).
    pub scale_percent: u32,
    pub path: String,
    pub sha256: String,
}
/// Timestamped frames after a motion trigger (ms since the trigger).
pub struct Sequence {
    pub surface: String,
    pub frames: Vec<(f64, Image)>,
    pub scale_percent: u32,
    /// Mean interval between samples; bounds the measurement resolution.
    pub sample_interval_ms: f64,
}

/// Measures one probe on one capture; values are logical px (or colours).
pub fn measure(
    probe: &Value,
    image: &Image,
    scale_percent: u32,
    t: &Thresholds,
) -> Result<Value, String> {
    let scale = f64::from(scale_percent) / 100.0;
    let logical = |px: f64| (px / scale * 100.0).round() / 100.0;
    let kind = probe["kind"].as_str().ok_or("probe kind")?;
    let params = &probe["params"];
    match kind {
        "band_height" => {
            let f = params["columns_fraction"]
                .as_array()
                .ok_or("columns_fraction")?;
            let fraction = |i: usize| f.get(i).and_then(Value::as_f64).ok_or("columns_fraction");
            let x0 = (fraction(0)? * image.width as f64) as usize;
            let x1 = ((fraction(1)? * image.width as f64) as usize)
                .max(x0 + 1)
                .min(image.width);
            let max = (params["max_logical"].as_f64().ok_or("max_logical")? * scale) as usize;
            Ok(json!({"height":logical(band_height(image, x0, x1, max, t) as f64)}))
        }
        "first_ink_box" => {
            let band =
                (params["band_logical"].as_f64().ok_or("band_logical")? * scale).round() as usize;
            // Edge highlights (a 1-2 px top border) are not glyph ink.
            let inset = (params["inset_logical"].as_f64().unwrap_or(0.0) * scale).round() as usize;
            let band = Rect {
                x0: 0,
                y0: image.height.saturating_sub(band) + inset,
                x1: image.width,
                y1: image.height.saturating_sub(inset),
            };
            if band.y1 <= band.y0 {
                return Err("band inset leaves no rows".into());
            }
            let start = (params["start_fraction"].as_f64().ok_or("start_fraction")?
                * image.width as f64) as usize;
            let cluster = (params["cluster_logical"]
                .as_f64()
                .ok_or("cluster_logical")?
                * scale)
                .round() as usize;
            let b = first_ink_box(image, band, start, cluster, t).ok_or("no ink in the band")?;
            let centre = (b.y0 + b.y1) as f64 / 2.0 - (band.y0 + band.y1) as f64 / 2.0;
            Ok(
                json!({"width":logical(b.width() as f64),"height":logical(b.height() as f64),
                "centre_dy":logical(centre),"box_physical":b.json()}),
            )
        }
        "ink_box" => {
            let r = Region::parse(&params["region"])?.physical(image, scale)?;
            let b = ink_box(image, r, t).ok_or("no ink in the region")?;
            let cx = (b.x0 + b.x1) as f64 / 2.0 - (r.x0 + r.x1) as f64 / 2.0;
            let cy = (b.y0 + b.y1) as f64 / 2.0 - (r.y0 + r.y1) as f64 / 2.0;
            Ok(
                json!({"width":logical(b.width() as f64),"height":logical(b.height() as f64),
                "centre_dx":logical(cx),"centre_dy":logical(cy),
                "top":logical((b.y0 - r.y0) as f64),"box_physical":b.json()}),
            )
        }
        "dominant_colour" => {
            let r = Region::parse(&params["region"])?.physical(image, scale)?;
            let (colour, share) = dominant_colour(image, r);
            Ok(
                json!({"rgb":hex(colour),"share":(share * 1000.0).round() / 1000.0,"region_physical":r.json()}),
            )
        }
        "run_width" => {
            let r = Region::parse(&params["region"])?.physical(image, scale)?;
            let (colour, _) = dominant_colour(image, r);
            Ok(json!({"width":logical(run_width(image, r, colour, t) as f64),"rgb":hex(colour)}))
        }
        other => Err(format!("unknown probe kind {other}")),
    }
}

fn field_check(
    dimension: Dimension,
    field: &str,
    measured: &Value,
    expected: &Value,
    t: &Thresholds,
) -> Result<(bool, Value), String> {
    if dimension == Dimension::Colour {
        let m = measured[field]
            .as_str()
            .and_then(parse_hex)
            .ok_or("measured colour")?;
        let e = expected[field]
            .as_str()
            .and_then(parse_hex)
            .ok_or("expected colour")?;
        let d = (delta_e2000(m, e) * 100.0).round() / 100.0;
        return Ok((
            d <= t.colour_delta_e,
            json!({"field":field,"measured":hex(m),"expected":hex(e),"delta_e2000":d,"tolerance":t.colour_delta_e}),
        ));
    }
    let m = measured[field]
        .as_f64()
        .ok_or_else(|| format!("measured {field}"))?;
    let e = expected[field]
        .as_f64()
        .ok_or_else(|| format!("expected {field}"))?;
    let tolerance = dimension.tolerance(t);
    let d = ((m - e).abs() * 100.0).round() / 100.0;
    Ok((
        d <= tolerance,
        json!({"field":field,"measured":m,"expected":e,"difference":d,"tolerance":tolerance}),
    ))
}

/// Compares guest captures (and motion sequences) with the reference profile.
/// Returns the full per-probe report; `outcome` is `passed` only when every
/// dimension passed. Errors are reserved for a malformed or tampered profile.
pub fn compare(
    root: &Path,
    profile: &Value,
    captures: &[Capture],
    sequences: &[Sequence],
) -> Result<Value, String> {
    if profile["record_kind"] != "windows_reference" || profile["schema_version"] != 1 {
        return Err("not a windows_reference profile".into());
    }
    let t = FROZEN;
    if !thresholds_match(&profile["thresholds"]) {
        return Err("profile thresholds differ from the frozen harness thresholds (fidelity::FROZEN); thresholds cannot change after freezing".into());
    }
    let mut reference_files = Vec::new();
    for surface in profile["surfaces"].as_array().ok_or("profile surfaces")? {
        for capture in surface["captures"].as_array().into_iter().flatten() {
            let path = capture["path"].as_str().ok_or("reference capture path")?;
            let absolute = root.join(path);
            let state = if absolute.exists() {
                if crate::evidence::file_hash(&absolute)? != capture["sha256"] {
                    return Err(format!(
                        "reference capture {path} changed since the profile was frozen"
                    ));
                }
                "verified"
            } else {
                "absent (frozen values used)"
            };
            reference_files.push(json!({"path":path,"state":state}));
        }
    }
    // A motion reference recorded as a sequence must still give its frozen
    // value (every frame is checked against the hash the sequence records).
    for probe in profile["probes"].as_array().ok_or("profile probes")? {
        let Some(sequence) = probe["reference"]["sequence"].as_str() else {
            continue;
        };
        if root.join(sequence).exists()
            && probe["expected"]["duration_ms"].as_f64()
                != Some(reference_animation_ms(root, probe)?)
        {
            return Err(format!(
                "reference sequence {sequence} no longer gives the frozen {} value",
                probe["id"]
            ));
        }
    }
    let mut checks = Vec::new();
    for probe in profile["probes"].as_array().ok_or("profile probes")? {
        let id = probe["id"].as_str().ok_or("probe id")?;
        let dimension = Dimension::parse(probe["dimension"].as_str().ok_or("probe dimension")?)?;
        let surface = probe["surface"].as_str().ok_or("probe surface")?;
        let status = probe["status"].as_str().ok_or("probe status")?;
        let fields: Vec<&str> = probe["compare"]
            .as_array()
            .ok_or("probe compare")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let mut record = json!({"id":id,"dimension":dimension.name(),"surface":surface,"status":status,
            "kind":probe["kind"],"spec":probe["spec"],"expected":probe["expected"]});
        let motion = probe["kind"] == "settle_ms" || probe["kind"] == "animation_ms";
        let measured = if motion {
            match sequences.iter().find(|s| s.surface == surface) {
                Some(seq) => {
                    let region = Region::parse(&probe["params"]["region"])?;
                    let scale = f64::from(seq.scale_percent) / 100.0;
                    let at = |image: &Image| region.physical(image, scale);
                    let settled = if probe["kind"] == "animation_ms" {
                        animation_ms(&seq.frames, &at)?
                    } else {
                        settle_ms(&seq.frames, &at)?
                    };
                    Some(
                        json!({"duration_ms":settled,"frames":seq.frames.len(),"sample_interval_ms":seq.sample_interval_ms}),
                    )
                }
                None => None,
            }
        } else {
            captures.iter().find(|c| c.surface == surface).map(|c| {
                match measure(probe, &c.image, c.scale_percent, &t) {
                    Ok(mut m) => {
                        m["capture"] = json!(c.path);
                        m["capture_sha256"] = json!(c.sha256);
                        m["scale_percent"] = json!(c.scale_percent);
                        m
                    }
                    Err(error) => json!({"error":error,"capture":c.path}),
                }
            })
        };
        let outcome = match (status, &measured) {
            ("not_captured", _) => "unresolved",
            (_, None) => "missing_guest_capture",
            (_, Some(m)) if !m["error"].is_null() || (motion && m["duration_ms"].is_null()) => {
                if status == "uncontrolled" {
                    "reported"
                } else {
                    "failed"
                }
            }
            (_, Some(m)) => {
                let mut ok = true;
                let mut details = Vec::new();
                for field in &fields {
                    let (pass, detail) = field_check(dimension, field, m, &probe["expected"], &t)?;
                    ok &= pass;
                    details.push(detail);
                }
                record["fields"] = json!(details);
                match (status, ok) {
                    ("uncontrolled", _) => "reported",
                    ("approved_deviation", true) => "passed",
                    ("approved_deviation", false) => "exception",
                    ("frozen", true) => "passed",
                    ("frozen", false) => "failed",
                    (other, _) => return Err(format!("probe {id}: unknown status {other}")),
                }
            }
        };
        if outcome == "exception" {
            record["exception"] = probe["deviation"].clone();
        }
        record["measured"] = measured.unwrap_or(Value::Null);
        record["outcome"] = json!(outcome);
        checks.push(record);
    }
    let mut dimensions = serde_json::Map::new();
    for d in Dimension::ALL {
        let mine: Vec<&Value> = checks
            .iter()
            .filter(|c| c["dimension"] == d.name())
            .collect();
        let count = |o: &str| mine.iter().filter(|c| c["outcome"] == o).count();
        let blocking = count("failed") + count("missing_guest_capture");
        let outcome = if blocking > 0 {
            "failed"
        } else if count("unresolved") > 0 {
            "unresolved"
        } else if count("passed") + count("exception") == 0 {
            "not_evaluated"
        } else {
            "passed"
        };
        dimensions.insert(d.name().into(), json!({"outcome":outcome,"tolerance":d.tolerance(&t),
            "passed":count("passed"),"exceptions":count("exception"),"failed":count("failed"),
            "missing_guest_capture":count("missing_guest_capture"),"unresolved":count("unresolved"),"reported":count("reported")}));
    }
    let passed = dimensions.values().all(|d| d["outcome"] == "passed");
    let deviations: Vec<_> = checks
        .iter()
        .filter(|c| {
            matches!(
                c["outcome"].as_str(),
                Some("failed" | "missing_guest_capture" | "unresolved")
            )
        })
        .map(|c| c["id"].clone())
        .collect();
    let exceptions: Vec<_> = checks
        .iter()
        .filter(|c| c["outcome"] == "exception")
        .map(|c| json!({"id":c["id"],"deviation":c["exception"]}))
        .collect();
    let excluded: Vec<_> = profile["surfaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["claimed"] == false)
        .map(|s| json!({"surface":s["id"],"reason":s["exclusion"]}))
        .collect();
    Ok(
        json!({"schema_version":1,"record_kind":"vis01_comparison","profile":profile["id"],
        "reference":{"edition":profile["edition"],"display_version":profile["display_version"],"os_build":profile["os_build"]},
        "thresholds":t.to_json(),"reference_files":reference_files,"dimensions":dimensions,
        "checks":checks,"deviations":deviations,"exceptions":exceptions,"excluded_surfaces":excluded,
        "approved_deviations":profile["approved_deviations"],"outcome":if passed {"passed"} else {"failed"}}),
    )
}

/// The short form a suite reports as `reference_comparison`.
pub fn summary(report: &Value, profile_sha256: &str) -> Value {
    let dimensions: serde_json::Map<String, Value> = Dimension::ALL
        .iter()
        .map(|d| {
            (
                d.name().to_owned(),
                report["dimensions"][d.name()]["outcome"].clone(),
            )
        })
        .collect();
    json!({"profile":PROFILE,"profile_sha256":profile_sha256,"outcome":report["outcome"],
        "dimensions":dimensions,"deviations":report["deviations"],"exceptions":report["exceptions"]})
}

#[cfg(test)]
mod tests;
