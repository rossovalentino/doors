//! Logical geometry. Door UI lays out in device-independent pixels (DIPs, 96
//! per inch) held as 26.6 fixed point (`Lu`, DIP x 64); there are no floats,
//! so NaN/infinite geometry cannot exist. Physical pixels come only from
//! `Lu::physical` / `LRect::physical`, which snap every *edge* with the same
//! rule as `door_graphics_core::Scale` (round, exact halves down): abutting
//! logical rectangles stay abutting physical rectangles at every scale.
use door_graphics_core::Scale;

/// 26.6 fixed-point logical length or coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lu(pub i32);
impl Lu {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(64);
    /// "Unbounded" available space for measurement (about 8 million DIPs,
    /// far beyond any surface, while sums of a few cannot overflow i32).
    pub const INFINITE: Self = Self(1 << 29);
    /// Whole DIPs.
    pub const fn px(v: i32) -> Self {
        Self(v.saturating_mul(64))
    }
    /// `num / den` DIPs (den > 0), rounded to the nearest 1/64.
    pub const fn ratio(num: i32, den: i32) -> Self {
        let den = if den <= 0 { 1 } else { den } as i64;
        Self(((num as i64 * 64 * 2 + den) / (2 * den)) as i32)
    }
    pub const fn raw(self) -> i32 {
        self.0
    }
    pub const fn is_infinite(self) -> bool {
        self.0 >= Self::INFINITE.0
    }
    /// Nearest whole DIP (halves up).
    pub const fn round(self) -> i32 {
        (self.0 + 32).div_euclid(64)
    }
    pub const fn floor(self) -> i32 {
        self.0.div_euclid(64)
    }
    pub const fn ceil(self) -> i32 {
        (self.0 + 63).div_euclid(64)
    }
    pub const fn max(self, o: Self) -> Self {
        if self.0 >= o.0 { self } else { o }
    }
    pub const fn min(self, o: Self) -> Self {
        if self.0 <= o.0 { self } else { o }
    }
    pub const fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi.max(lo))
    }
    /// `self * num / den`, rounded to nearest (den > 0).
    pub const fn mul_div(self, num: i32, den: i32) -> Self {
        let den = if den <= 0 { 1 } else { den } as i64;
        let v = self.0 as i64 * num as i64;
        let r = if v >= 0 {
            (v * 2 + den) / (2 * den)
        } else {
            -((-v * 2 + den) / (2 * den))
        };
        Self(if r > i32::MAX as i64 {
            i32::MAX
        } else if r < i32::MIN as i64 {
            i32::MIN
        } else {
            r as i32
        })
    }
    /// Physical pixel edge at `scale`: `round(dip * percent / 100)`, exact
    /// halves down, identical to `Scale::px` for whole DIPs.
    pub fn physical(self, scale: Scale) -> i32 {
        (i64::from(self.0) * i64::from(scale.percent()) + 3199).div_euclid(6400) as i32
    }
    /// Logical coordinate of physical pixel `p` (its top-left edge).
    pub fn from_physical(p: i32, scale: Scale) -> Self {
        Self(((i64::from(p) * 6400) / i64::from(scale.percent())) as i32)
    }
}
impl core::ops::Add for Lu {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self(self.0.saturating_add(o.0))
    }
}
impl core::ops::Sub for Lu {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self(self.0.saturating_sub(o.0))
    }
}
impl core::ops::AddAssign for Lu {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}
impl core::ops::SubAssign for Lu {
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}
impl core::ops::Neg for Lu {
    type Output = Self;
    fn neg(self) -> Self {
        Self(self.0.saturating_neg())
    }
}
impl core::ops::Mul<i32> for Lu {
    type Output = Self;
    fn mul(self, o: i32) -> Self {
        Self(self.0.saturating_mul(o))
    }
}
impl core::ops::Div<i32> for Lu {
    type Output = Self;
    fn div(self, o: i32) -> Self {
        if o == 0 { self } else { Self(self.0 / o) }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LPoint {
    pub x: Lu,
    pub y: Lu,
}
impl LPoint {
    pub const fn new(x: Lu, y: Lu) -> Self {
        Self { x, y }
    }
    pub const fn px(x: i32, y: i32) -> Self {
        Self {
            x: Lu::px(x),
            y: Lu::px(y),
        }
    }
    pub fn offset(self, d: LPoint) -> Self {
        Self {
            x: self.x + d.x,
            y: self.y + d.y,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LSize {
    pub w: Lu,
    pub h: Lu,
}
impl LSize {
    pub const ZERO: Self = Self {
        w: Lu::ZERO,
        h: Lu::ZERO,
    };
    pub const INFINITE: Self = Self {
        w: Lu::INFINITE,
        h: Lu::INFINITE,
    };
    pub const fn new(w: Lu, h: Lu) -> Self {
        Self { w, h }
    }
    pub const fn px(w: i32, h: i32) -> Self {
        Self {
            w: Lu::px(w),
            h: Lu::px(h),
        }
    }
    pub fn max(self, o: Self) -> Self {
        Self {
            w: self.w.max(o.w),
            h: self.h.max(o.h),
        }
    }
    pub fn min(self, o: Self) -> Self {
        Self {
            w: self.w.min(o.w),
            h: self.h.min(o.h),
        }
    }
}
/// Logical rectangle: origin plus non-negative size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LRect {
    pub x: Lu,
    pub y: Lu,
    pub w: Lu,
    pub h: Lu,
}
impl LRect {
    pub const ZERO: Self = Self {
        x: Lu::ZERO,
        y: Lu::ZERO,
        w: Lu::ZERO,
        h: Lu::ZERO,
    };
    pub fn new(x: Lu, y: Lu, w: Lu, h: Lu) -> Self {
        Self {
            x,
            y,
            w: w.max(Lu::ZERO),
            h: h.max(Lu::ZERO),
        }
    }
    pub fn px(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self::new(Lu::px(x), Lu::px(y), Lu::px(w), Lu::px(h))
    }
    pub fn from_size(origin: LPoint, size: LSize) -> Self {
        Self::new(origin.x, origin.y, size.w, size.h)
    }
    pub fn origin(self) -> LPoint {
        LPoint::new(self.x, self.y)
    }
    pub fn size(self) -> LSize {
        LSize::new(self.w, self.h)
    }
    pub fn right(self) -> Lu {
        self.x + self.w
    }
    pub fn bottom(self) -> Lu {
        self.y + self.h
    }
    pub fn center(self) -> LPoint {
        LPoint::new(self.x + self.w / 2, self.y + self.h / 2)
    }
    pub fn is_empty(self) -> bool {
        self.w.0 <= 0 || self.h.0 <= 0
    }
    pub fn contains(self, p: LPoint) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
    pub fn intersect(self, o: Self) -> Option<Self> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > x && b > y).then(|| Self::new(x, y, r - x, b - y))
    }
    pub fn intersects(self, o: Self) -> bool {
        self.intersect(o).is_some()
    }
    pub fn union(self, o: Self) -> Self {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Self::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }
    pub fn inset(self, i: Insets) -> Self {
        Self::new(
            self.x + i.left,
            self.y + i.top,
            self.w - i.left - i.right,
            self.h - i.top - i.bottom,
        )
    }
    /// Grow by `d` on every side (negative shrinks).
    pub fn inflate(self, d: Lu) -> Self {
        Self::new(self.x - d, self.y - d, self.w + d * 2, self.h + d * 2)
    }
    pub fn translate(self, d: LPoint) -> Self {
        Self {
            x: self.x + d.x,
            y: self.y + d.y,
            ..self
        }
    }
    /// Physical pixel rectangle covering this one at `scale`, snapped from
    /// its edges; `None` when it covers no whole pixel.
    pub fn physical(self, scale: Scale) -> Option<PRect> {
        let x0 = self.x.physical(scale);
        let y0 = self.y.physical(scale);
        let x1 = self.right().physical(scale);
        let y1 = self.bottom().physical(scale);
        (x1 > x0 && y1 > y0).then_some(PRect { x0, y0, x1, y1 })
    }
}
/// Physical half-open pixel rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}
impl PRect {
    pub fn width(self) -> i32 {
        self.x1 - self.x0
    }
    pub fn height(self) -> i32 {
        self.y1 - self.y0
    }
    pub fn intersect(self, o: Self) -> Option<Self> {
        let r = Self {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        (r.x1 > r.x0 && r.y1 > r.y0).then_some(r)
    }
    pub fn area(self) -> u64 {
        self.width().max(0) as u64 * self.height().max(0) as u64
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Insets {
    pub left: Lu,
    pub top: Lu,
    pub right: Lu,
    pub bottom: Lu,
}
impl Insets {
    pub const ZERO: Self = Self {
        left: Lu::ZERO,
        top: Lu::ZERO,
        right: Lu::ZERO,
        bottom: Lu::ZERO,
    };
    pub const fn all(v: i32) -> Self {
        Self {
            left: Lu::px(v),
            top: Lu::px(v),
            right: Lu::px(v),
            bottom: Lu::px(v),
        }
    }
    pub const fn xy(x: i32, y: i32) -> Self {
        Self {
            left: Lu::px(x),
            top: Lu::px(y),
            right: Lu::px(x),
            bottom: Lu::px(y),
        }
    }
    pub const fn ltrb(l: i32, t: i32, r: i32, b: i32) -> Self {
        Self {
            left: Lu::px(l),
            top: Lu::px(t),
            right: Lu::px(r),
            bottom: Lu::px(b),
        }
    }
    pub fn horizontal(self) -> Lu {
        self.left + self.right
    }
    pub fn vertical(self) -> Lu {
        self.top + self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_point_edges_match_graphics_core_and_never_seam() {
        for p in Scale::PERCENTS {
            let s = Scale::from_percent(p).unwrap();
            for l in -500..2000 {
                assert_eq!(Lu::px(l).physical(s), s.px(l), "{p}% {l}");
            }
            // Fractional widths tile without gaps or overlap.
            let mut x = Lu::px(3);
            let mut edge = x.physical(s);
            for i in 0..200 {
                let w = Lu::ratio(100 + i * 7, 3);
                let r = LRect::new(x, Lu::ZERO, w, Lu::px(10)).physical(s);
                if let Some(r) = r {
                    assert_eq!(r.x0, edge);
                    edge = r.x1;
                }
                x += w;
            }
            assert_eq!(edge, x.physical(s));
            for phys in -300..900 {
                let l = Lu::from_physical(phys, s);
                assert!(l.physical(s) <= phys + 1 && l.physical(s) >= phys - 1);
            }
        }
        assert_eq!(Lu::ratio(1, 3), Lu(21));
        assert_eq!(Lu::px(10).mul_div(3, 2), Lu::px(15));
        assert_eq!(Lu(95).round(), 1);
        assert_eq!(Lu(-1).floor(), -1);
    }
    #[test]
    fn rect_algebra() {
        let a = LRect::px(0, 0, 10, 10);
        let b = LRect::px(5, 5, 10, 10);
        assert_eq!(a.intersect(b), Some(LRect::px(5, 5, 5, 5)));
        assert_eq!(a.union(b), LRect::px(0, 0, 15, 15));
        assert!(a.intersect(LRect::px(10, 0, 5, 5)).is_none());
        assert_eq!(a.inset(Insets::all(2)), LRect::px(2, 2, 6, 6));
        assert_eq!(LRect::px(0, 0, 2, 2).inset(Insets::all(4)).w, Lu::ZERO);
        assert!(a.contains(LPoint::px(9, 9)) && !a.contains(LPoint::px(10, 0)));
    }
}
