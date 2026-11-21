//! Logical-to-device mapping (`SetMapMode`, window/viewport origins and
//! extents) and dialog base units. Device coordinates are pixels of a 96 DPI
//! device. The transform is the documented
//! `D = (L - WindowOrg) * ViewportExt / WindowExt + ViewportOrg`, evaluated in
//! 64-bit arithmetic with GDI's round-half-away-from-zero `MulDiv`.
use crate::msg::mm;
use crate::{Point, Rect, Size};

/// Device resolution used by the fixed metric modes.
pub const DPI: i32 = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub mode: i32,
    pub window_org: Point,
    pub window_ext: Size,
    pub viewport_org: Point,
    pub viewport_ext: Size,
}
impl Default for Mapping {
    fn default() -> Self {
        Self {
            mode: mm::TEXT,
            window_org: Point::new(0, 0),
            window_ext: Size::new(1, 1),
            viewport_org: Point::new(0, 0),
            viewport_ext: Size::new(1, 1),
        }
    }
}

/// `MulDiv` with rounding half away from zero; a zero divisor yields -1.
pub fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let n = i64::from(a) * i64::from(b);
    let c = i64::from(c);
    let q = if (n < 0) != (c < 0) {
        (n - c / 2) / c
    } else {
        (n + c / 2) / c
    };
    q.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

impl Mapping {
    /// Change the mode; fixed modes reset both extents (y grows upwards).
    pub fn set_mode(&mut self, mode: i32) -> Option<i32> {
        if !(mm::TEXT..=mm::ANISOTROPIC).contains(&mode) {
            return None;
        }
        let old = self.mode;
        self.mode = mode;
        // Units per inch of each fixed mode.
        let per_inch = match mode {
            mm::LOMETRIC => 254,
            mm::HIMETRIC => 2540,
            mm::LOENGLISH => 100,
            mm::HIENGLISH => 1000,
            mm::TWIPS => 1440,
            _ => 0,
        };
        if mode == mm::TEXT {
            self.window_ext = Size::new(1, 1);
            self.viewport_ext = Size::new(1, 1);
        } else if per_inch != 0 {
            self.window_ext = Size::new(per_inch, per_inch);
            self.viewport_ext = Size::new(DPI, -DPI);
        } else if mode == mm::ISOTROPIC && old != mm::ANISOTROPIC {
            // Isotropic starts from the metric extents, as GDI does.
            self.window_ext = Size::new(254, 254);
            self.viewport_ext = Size::new(DPI, -DPI);
        }
        Some(old)
    }
    /// Extents are only settable in the two scalable modes.
    pub fn set_window_ext(&mut self, ext: Size) -> bool {
        if !matches!(self.mode, mm::ISOTROPIC | mm::ANISOTROPIC) {
            return true;
        }
        if ext.cx == 0 || ext.cy == 0 {
            return false;
        }
        self.window_ext = ext;
        self.fix_isotropic();
        true
    }
    pub fn set_viewport_ext(&mut self, ext: Size) -> bool {
        if !matches!(self.mode, mm::ISOTROPIC | mm::ANISOTROPIC) {
            return true;
        }
        if ext.cx == 0 || ext.cy == 0 {
            return false;
        }
        self.viewport_ext = ext;
        self.fix_isotropic();
        true
    }
    /// MM_ISOTROPIC keeps one logical unit square by shrinking the larger
    /// viewport extent while preserving its sign.
    fn fix_isotropic(&mut self) {
        if self.mode != mm::ISOTROPIC {
            return;
        }
        let (wx, wy) = (
            i64::from(self.window_ext.cx).abs(),
            i64::from(self.window_ext.cy).abs(),
        );
        let (vx, vy) = (
            i64::from(self.viewport_ext.cx).abs(),
            i64::from(self.viewport_ext.cy).abs(),
        );
        let (sx, sy) = (self.viewport_ext.cx.signum(), self.viewport_ext.cy.signum());
        // Compare vx/wx with vy/wy.
        if vx * wy > vy * wx {
            let nx = (vy * wx / wy.max(1)).max(1);
            self.viewport_ext.cx = sx * nx as i32;
        } else if vx * wy < vy * wx {
            let ny = (vx * wy / wx.max(1)).max(1);
            self.viewport_ext.cy = sy * ny as i32;
        }
    }
    pub fn lp_to_dp(&self, p: Point) -> Point {
        if self.mode == mm::TEXT {
            return Point::new(
                p.x.saturating_sub(self.window_org.x)
                    .saturating_add(self.viewport_org.x),
                p.y.saturating_sub(self.window_org.y)
                    .saturating_add(self.viewport_org.y),
            );
        }
        Point::new(
            mul_div(
                p.x.saturating_sub(self.window_org.x),
                self.viewport_ext.cx,
                self.window_ext.cx,
            )
            .saturating_add(self.viewport_org.x),
            mul_div(
                p.y.saturating_sub(self.window_org.y),
                self.viewport_ext.cy,
                self.window_ext.cy,
            )
            .saturating_add(self.viewport_org.y),
        )
    }
    pub fn dp_to_lp(&self, p: Point) -> Point {
        if self.mode == mm::TEXT {
            return Point::new(
                p.x.saturating_sub(self.viewport_org.x)
                    .saturating_add(self.window_org.x),
                p.y.saturating_sub(self.viewport_org.y)
                    .saturating_add(self.window_org.y),
            );
        }
        Point::new(
            mul_div(
                p.x.saturating_sub(self.viewport_org.x),
                self.window_ext.cx,
                self.viewport_ext.cx,
            )
            .saturating_add(self.window_org.x),
            mul_div(
                p.y.saturating_sub(self.viewport_org.y),
                self.window_ext.cy,
                self.viewport_ext.cy,
            )
            .saturating_add(self.window_org.y),
        )
    }
    /// Map a logical rectangle to a normalized device rectangle.
    pub fn rect_to_dp(&self, r: Rect) -> Rect {
        let a = self.lp_to_dp(r.top_left());
        let b = self.lp_to_dp(Point::new(r.right, r.bottom));
        Rect::new(a.x, a.y, b.x, b.y).normalized()
    }
    /// Scale a logical length along x (pen widths, text heights).
    pub fn scale_x(&self, len: i32) -> i32 {
        if self.mode == mm::TEXT {
            return len;
        }
        mul_div(len, self.viewport_ext.cx, self.window_ext.cx).saturating_abs()
    }
    pub fn scale_y(&self, len: i32) -> i32 {
        if self.mode == mm::TEXT {
            return len;
        }
        mul_div(len, self.viewport_ext.cy, self.window_ext.cy).saturating_abs()
    }
}

/// Dialog base units: horizontal units are 1/4 of the average character
/// width, vertical units 1/8 of the character height (`MapDialogRect`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogUnits {
    pub base_x: i32,
    pub base_y: i32,
}
impl DialogUnits {
    pub fn map(self, r: Rect) -> Rect {
        Rect::new(
            mul_div(r.left, self.base_x, 4),
            mul_div(r.top, self.base_y, 8),
            mul_div(r.right, self.base_x, 4),
            mul_div(r.bottom, self.base_y, 8),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muldiv_rounds_half_away_from_zero() {
        assert_eq!(mul_div(3, 1, 2), 2);
        assert_eq!(mul_div(-3, 1, 2), -2);
        assert_eq!(mul_div(5, 7, 0), -1);
        assert_eq!(mul_div(i32::MAX, 2, 1), i32::MAX);
        assert_eq!(mul_div(10, 10, 3), 33);
    }
    #[test]
    fn text_mode_translates_origins() {
        let mut m = Mapping {
            viewport_org: Point::new(280, 150),
            ..Mapping::default()
        };
        assert_eq!(m.lp_to_dp(Point::new(4, 4)), Point::new(284, 154));
        assert_eq!(m.dp_to_lp(Point::new(284, 154)), Point::new(4, 4));
        m.window_org = Point::new(10, 0);
        assert_eq!(m.lp_to_dp(Point::new(10, 0)), Point::new(280, 150));
        assert!(m.set_window_ext(Size::new(5, 5)), "ignored in MM_TEXT");
        assert_eq!(m.window_ext, Size::new(1, 1));
    }
    #[test]
    fn metric_and_scalable_modes() {
        let mut m = Mapping::default();
        assert_eq!(m.set_mode(mm::LOENGLISH), Some(mm::TEXT));
        // 100 logical units = one inch = 96 pixels, y up.
        assert_eq!(m.lp_to_dp(Point::new(100, 100)), Point::new(96, -96));
        assert_eq!(m.set_mode(99), None);
        m.set_mode(mm::ANISOTROPIC);
        assert!(m.set_window_ext(Size::new(100, 100)));
        assert!(m.set_viewport_ext(Size::new(200, 50)));
        assert_eq!(m.lp_to_dp(Point::new(10, 10)), Point::new(20, 5));
        assert!(!m.set_window_ext(Size::new(0, 1)));
        m.set_mode(mm::ISOTROPIC);
        m.set_window_ext(Size::new(100, 100));
        m.set_viewport_ext(Size::new(200, 50));
        assert_eq!(m.viewport_ext, Size::new(50, 50), "square units");
        assert_eq!(m.rect_to_dp(Rect::new(0, 0, 10, 10)), Rect::new(0, 0, 5, 5));
        assert_eq!(m.scale_y(-20), 10);
        let du = DialogUnits {
            base_x: 7,
            base_y: 15,
        };
        assert_eq!(du.map(Rect::new(4, 8, 8, 16)), Rect::new(7, 15, 14, 30));
    }
}
