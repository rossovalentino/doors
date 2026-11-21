use crate::{Error, Result};

/// Checked, half-open physical-pixel rectangle. Coordinates may be offscreen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}
impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self> {
        if width == 0
            || height == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || x.checked_add(width as i32).is_none()
            || y.checked_add(height as i32).is_none()
        {
            return Err(Error::InvalidGeometry);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
    pub const fn x(self) -> i32 {
        self.x
    }
    pub const fn y(self) -> i32 {
        self.y
    }
    pub const fn width(self) -> u32 {
        self.width
    }
    pub const fn height(self) -> u32 {
        self.height
    }
    pub fn right(self) -> i32 {
        self.x + self.width as i32
    }
    pub fn bottom(self) -> i32 {
        self.y + self.height as i32
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| Self {
            x,
            y,
            width: (r - x) as u32,
            height: (b - y) as u32,
        })
    }
    /// Bounding union, saturating only at the representable coordinate envelope.
    pub fn union(self, other: Self) -> Option<Self> {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let w = self.right().max(other.right()) as i64 - x as i64;
        let h = self.bottom().max(other.bottom()) as i64 - y as i64;
        if w > i32::MAX as i64 || h > i32::MAX as i64 {
            return None;
        }
        Self::new(x, y, w as u32, h as u32).ok()
    }
    pub fn expanded(self, margin: i32) -> Option<Self> {
        if margin < 0 {
            return None;
        }
        let x = self.x.checked_sub(margin)?;
        let y = self.y.checked_sub(margin)?;
        let twice = (margin as u32).checked_mul(2)?;
        Self::new(
            x,
            y,
            self.width.checked_add(twice)?,
            self.height.checked_add(twice)?,
        )
        .ok()
    }
}

/// Output scale as a percentage of 96 DPI (Windows display scale). Logical
/// (device-independent) coordinates map to physical pixels by
/// `round(logical * percent / 100)`, rounding exact halves down, so for every
/// supported scale the rounded logical size of an output maps back onto its
/// exact physical edge. Rectangles snap from their logical *edges* (never
/// origin plus rounded size): abutting logical rectangles stay abutting
/// physical rectangles with no seam or overlap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Scale(u32);
impl Default for Scale {
    fn default() -> Self {
        Self::ONE
    }
}
impl Scale {
    pub const ONE: Self = Self(100);
    /// Windows display scales offered by the shell.
    pub const PERCENTS: [u32; 8] = [100, 125, 150, 175, 200, 225, 250, 300];
    pub const fn from_percent(percent: u32) -> Option<Self> {
        match percent {
            100 | 125 | 150 | 175 | 200 | 225 | 250 | 300 => Some(Self(percent)),
            _ => None,
        }
    }
    /// Integer device scale (1x, 2x, 3x).
    pub const fn from_factor(factor: u32) -> Option<Self> {
        match factor {
            1..=3 => Some(Self(factor * 100)),
            _ => None,
        }
    }
    pub const fn percent(self) -> u32 {
        self.0
    }
    /// 96 DPI at 100%.
    pub const fn dpi(self) -> u32 {
        96 * self.0 / 100
    }
    /// The integer factor when the scale is a whole multiple of 100%.
    pub const fn factor(self) -> Option<u32> {
        if self.0.is_multiple_of(100) {
            Some(self.0 / 100)
        } else {
            None
        }
    }
    /// Whole density whose native assets are sampled at this scale: the
    /// next factor at or above it (125% and 150% sample 2x), 1..=3.
    pub const fn asset_factor(self) -> u32 {
        let f = self.0.div_ceil(100);
        if f > 3 { 3 } else { f }
    }
    /// Integer scale offered to clients that only render whole factors:
    /// the nearest factor with exact halves rounding up (150% -> 2x), 1..=3.
    pub const fn legacy_factor(self) -> u32 {
        let f = (self.0 + 50) / 100;
        if f < 1 {
            1
        } else if f > 3 {
            3
        } else {
            f
        }
    }
    /// Physical coordinate of logical coordinate `logical`.
    pub fn px(self, logical: i32) -> i32 {
        (i64::from(logical) * i64::from(self.0) + 49).div_euclid(100) as i32
    }
    /// Physical length of `logical` device-independent pixels measured from
    /// a physical edge (equal to `px` of that length).
    pub fn len(self, logical: u32) -> u32 {
        ((u64::from(logical) * u64::from(self.0) + 49) / 100) as u32
    }
    /// Logical coordinate containing physical pixel `physical`: the largest
    /// `l` with `px(l) <= physical`.
    pub fn logical(self, physical: i32) -> i32 {
        (i64::from(physical) * 100 + 50).div_euclid(i64::from(self.0)) as i32
    }
    /// Logical extent of a physical output dimension: the largest logical
    /// length whose snapped physical edge stays inside it (`len(l) <=
    /// physical`, less than one logical pixel short), e.g. 2560 x 1600 px at
    /// 150% is 1707 x 1067 logical. Integer scales floor, as before.
    pub fn logical_len(self, physical: u32) -> u32 {
        ((u64::from(physical) * 100 + 50) / u64::from(self.0)) as u32
    }
    /// Physical rectangle covering a logical rectangle, snapped from its
    /// logical edges.
    pub fn rect(self, logical: Rect) -> Result<Rect> {
        let x = self.px(logical.x());
        let y = self.px(logical.y());
        let right = self.px(logical.right());
        let bottom = self.px(logical.bottom());
        Rect::new(x, y, (right - x) as u32, (bottom - y) as u32)
    }
    /// Physical offset of `logical` from a physical edge `origin`.
    pub fn from_edge(self, origin: i32, logical: i32) -> i32 {
        origin.saturating_add(self.px(logical))
    }
}
#[cfg(test)]
mod scale_tests {
    use super::*;
    #[test]
    fn rounding_is_monotonic_invertible_and_snaps_output_edges() {
        for percent in Scale::PERCENTS {
            let s = Scale::from_percent(percent).unwrap();
            assert_eq!(s.px(0), 0);
            assert_eq!(s.dpi(), 96 * percent / 100);
            let mut previous = s.px(-2000);
            for l in -1999..4000 {
                let p = s.px(l);
                assert!(p > previous, "{percent}% strictly increasing at {l}");
                assert!((i64::from(p) * 100 - i64::from(l) * i64::from(percent)).abs() <= 50);
                previous = p;
            }
            // Every physical pixel belongs to exactly one logical unit.
            for p in -3000..6000 {
                let l = s.logical(p);
                assert!(s.px(l) <= p && p < s.px(l + 1), "{percent}% pixel {p}");
            }
            // Common outputs: the logical size maps back to the exact edge.
            for physical in [1280u32, 1366, 1920, 2560, 1600, 1440, 1080, 2160, 3840] {
                let logical = s.logical_len(physical);
                assert!(s.len(logical) <= physical && physical < s.len(logical + 1));
            }
        }
        let s = Scale::from_percent(150).unwrap();
        assert_eq!((s.logical_len(2560), s.logical_len(1600)), (1707, 1067));
        assert_eq!((s.len(1707), s.len(1067)), (2560, 1600));
        assert_eq!(
            (s.len(32), s.len(46), s.len(48), s.len(8)),
            (48, 69, 72, 12)
        );
        // The 48-DIP taskbar at the bottom of 1067 logical rows: 72 rows.
        assert_eq!(s.px(1067) - s.px(1067 - 48), 72);
        assert_eq!(Scale::from_factor(2), Scale::from_percent(200));
        assert_eq!(
            Scale::PERCENTS.map(|p| Scale::from_percent(p).unwrap().legacy_factor()),
            [1, 1, 2, 2, 2, 2, 3, 3]
        );
        assert!(Scale::from_percent(130).is_none());
    }
    #[test]
    fn abutting_logical_rectangles_tile_physical_pixels_without_seams() {
        for percent in Scale::PERCENTS {
            let s = Scale::from_percent(percent).unwrap();
            // A row of logical cells of varying widths, and a 2D grid.
            let widths = [1u32, 2, 3, 5, 7, 8, 13, 46, 46, 46, 47, 99];
            let mut x = -17;
            let mut covered = 0u32;
            let start = s.px(x);
            for w in widths {
                let r = s.rect(Rect::new(x, 3, w, 5).unwrap()).unwrap();
                assert_eq!(r.x(), start + covered as i32, "{percent}% seam before {x}");
                covered += r.width();
                x += w as i32;
            }
            assert_eq!(start + covered as i32, s.px(x));
            for gy in 0..6 {
                for gx in 0..6 {
                    let a = s
                        .rect(Rect::new(gx * 7 - 9, gy * 5 + 1, 7, 5).unwrap())
                        .unwrap();
                    let right = s
                        .rect(Rect::new(gx * 7 - 2, gy * 5 + 1, 7, 5).unwrap())
                        .unwrap();
                    let below = s
                        .rect(Rect::new(gx * 7 - 9, gy * 5 + 6, 7, 5).unwrap())
                        .unwrap();
                    assert_eq!(a.right(), right.x());
                    assert_eq!(a.bottom(), below.y());
                    assert!(a.intersection(right).is_none() && a.intersection(below).is_none());
                }
            }
        }
    }
}
