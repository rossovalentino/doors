//! Win32 geometry: `RECT` is half-open (`right`/`bottom` excluded), coordinates
//! are signed 32-bit and every derived quantity is computed in `i64` so hostile
//! extremes cannot overflow. Empty rectangles (`right <= left` or
//! `bottom <= top`) contain no pixels, exactly as `IsRectEmpty` defines them.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x.saturating_add(dx),
            y: self.y.saturating_add(dy),
        }
    }
    /// `MAKEPOINTS`-style signed 16-bit halves of an `LPARAM`.
    pub const fn from_lparam(value: isize) -> Self {
        Self {
            x: value as u16 as i16 as i32,
            y: (value >> 16) as u16 as i16 as i32,
        }
    }
    /// `MAKELPARAM(x, y)` with the documented 16-bit truncation.
    pub const fn to_lparam(self) -> isize {
        ((self.x as u16 as u32) | ((self.y as u16 as u32) << 16)) as i32 as isize
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub cx: i32,
    pub cy: i32,
}
impl Size {
    pub const fn new(cx: i32, cy: i32) -> Self {
        Self { cx, cy }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
    /// Origin plus extent; extents saturate instead of wrapping.
    pub const fn from_size(x: i32, y: i32, cx: i32, cy: i32) -> Self {
        Self::new(x, y, x.saturating_add(cx), y.saturating_add(cy))
    }
    pub const fn width(self) -> i32 {
        let w = self.right as i64 - self.left as i64;
        if w > i32::MAX as i64 {
            i32::MAX
        } else if w < i32::MIN as i64 {
            i32::MIN
        } else {
            w as i32
        }
    }
    pub const fn height(self) -> i32 {
        let h = self.bottom as i64 - self.top as i64;
        if h > i32::MAX as i64 {
            i32::MAX
        } else if h < i32::MIN as i64 {
            i32::MIN
        } else {
            h as i32
        }
    }
    pub const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
    pub const fn contains(self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }
    /// `IntersectRect`: the empty rectangle (all zero) when disjoint.
    pub fn intersect(self, other: Self) -> Self {
        let r = Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        if r.is_empty() { Self::EMPTY } else { r }
    }
    pub fn intersects(self, other: Self) -> bool {
        !self.intersect(other).is_empty()
    }
    /// `UnionRect`: empty inputs are ignored; both empty gives the empty rectangle.
    pub fn union(self, other: Self) -> Self {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => Self::EMPTY,
            (true, false) => other,
            (false, true) => self,
            (false, false) => Self::new(
                self.left.min(other.left),
                self.top.min(other.top),
                self.right.max(other.right),
                self.bottom.max(other.bottom),
            ),
        }
    }
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.left.saturating_add(dx),
            self.top.saturating_add(dy),
            self.right.saturating_add(dx),
            self.bottom.saturating_add(dy),
        )
    }
    /// `InflateRect` (negative values deflate). No normalization is applied.
    pub const fn inflate(self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.left.saturating_sub(dx),
            self.top.saturating_sub(dy),
            self.right.saturating_add(dx),
            self.bottom.saturating_add(dy),
        )
    }
    /// Swap reversed edges (GDI accepts reversed Rectangle/Ellipse/PatBlt bounds).
    pub fn normalized(self) -> Self {
        Self::new(
            self.left.min(self.right),
            self.top.min(self.bottom),
            self.left.max(self.right),
            self.top.max(self.bottom),
        )
    }
    pub fn contains_rect(self, inner: Self) -> bool {
        inner.is_empty()
            || (inner.left >= self.left
                && inner.top >= self.top
                && inner.right <= self.right
                && inner.bottom <= self.bottom)
    }
    /// `SubtractRect` semantics: the result is `self` unless `other` covers a full
    /// side, in which case the remaining strip is returned.
    pub fn subtract(self, other: Self) -> Self {
        let i = self.intersect(other);
        if i.is_empty() {
            return self;
        }
        if i == self {
            return Self::EMPTY;
        }
        if i.left == self.left && i.right == self.right {
            if i.top == self.top {
                return Self::new(self.left, i.bottom, self.right, self.bottom);
            }
            if i.bottom == self.bottom {
                return Self::new(self.left, self.top, self.right, i.top);
            }
        }
        if i.top == self.top && i.bottom == self.bottom {
            if i.left == self.left {
                return Self::new(i.right, self.top, self.right, self.bottom);
            }
            if i.right == self.right {
                return Self::new(self.left, self.top, i.left, self.bottom);
            }
        }
        self
    }
    pub const fn top_left(self) -> Point {
        Point::new(self.left, self.top)
    }
    pub const fn size(self) -> Size {
        Size::new(self.width(), self.height())
    }
    /// Little-endian Win32 `RECT` (16 bytes).
    pub fn to_bytes(self) -> [u8; 16] {
        let mut b = [0; 16];
        b[..4].copy_from_slice(&self.left.to_le_bytes());
        b[4..8].copy_from_slice(&self.top.to_le_bytes());
        b[8..12].copy_from_slice(&self.right.to_le_bytes());
        b[12..].copy_from_slice(&self.bottom.to_le_bytes());
        b
    }
    pub fn from_bytes(b: &[u8; 16]) -> Self {
        let at = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Self::new(at(0), at(4), at(8), at(12))
    }
}

/// Win32 `COLORREF` (0x00BBGGRR). Bit 24 (`PALETTEINDEX`) and 25
/// (`PALETTERGB`) selectors are accepted only as explicit RGB in a true-color
/// profile, matching the documented behavior on non-palette devices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ColorRef(pub u32);
impl ColorRef {
    pub const BLACK: Self = Self(0);
    pub const WHITE: Self = Self(0x00ff_ffff);
    pub const INVALID: u32 = 0xffff_ffff;
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r as u32 | (g as u32) << 8 | (b as u32) << 16)
    }
    pub const fn r(self) -> u8 {
        self.0 as u8
    }
    pub const fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub const fn b(self) -> u8 {
        (self.0 >> 16) as u8
    }
    /// 0x00RRGGBB device pixel (BGRX in memory, high byte zero as GDI writes it).
    pub const fn pixel(self) -> u32 {
        (self.r() as u32) << 16 | (self.g() as u32) << 8 | self.b() as u32
    }
    pub const fn from_pixel(pixel: u32) -> Self {
        Self::rgb((pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8)
    }
    /// `PALETTERGB`/explicit RGB are true-color requests; a `PALETTEINDEX` or
    /// unknown selector byte is rejected (`CLR_INVALID` callers).
    pub const fn validated(value: u32) -> Option<Self> {
        match value >> 24 {
            0 | 2 => Some(Self(value & 0x00ff_ffff)),
            _ => None,
        }
    }
}

/// Bounded UTF-16 helpers. Text never exceeds the caller-supplied bound.
pub mod wide {
    use alloc::{string::String, vec::Vec};
    /// Lossy UTF-16 -> UTF-8 (unpaired surrogates become U+FFFD).
    pub fn to_string(units: &[u16]) -> String {
        char::decode_utf16(units.iter().copied())
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    }
    pub fn from_str(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }
    /// Length up to (not including) the first NUL, bounded by `units.len()`.
    pub fn nul_len(units: &[u16]) -> usize {
        units.iter().position(|u| *u == 0).unwrap_or(units.len())
    }
    /// Case-insensitive compare used for class names and atoms (simple case
    /// folding of ASCII and Latin-1, as user32 atom comparisons do for these).
    pub fn eq_ignore_case(a: &[u16], b: &[u16]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fold(*x) == fold(*y))
    }
    pub fn fold(u: u16) -> u16 {
        match u {
            0x41..=0x5a | 0xc0..=0xd6 | 0xd8..=0xde => u + 32,
            _ => u,
        }
    }
    pub fn upper(u: u16) -> u16 {
        match u {
            0x61..=0x7a | 0xe0..=0xf6 | 0xf8..=0xfe => u - 32,
            _ => u,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rect_semantics_follow_win32() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.contains(Point::new(9, 9)));
        assert!(!a.contains(Point::new(10, 0)));
        assert_eq!(a.intersect(Rect::new(10, 0, 20, 5)), Rect::EMPTY);
        assert_eq!(a.union(Rect::EMPTY), a);
        assert_eq!(a.subtract(Rect::new(0, 0, 10, 4)), Rect::new(0, 4, 10, 10));
        assert_eq!(a.subtract(Rect::new(2, 2, 4, 4)), a);
        assert_eq!(Rect::new(10, 10, 0, 0).normalized(), a);
        assert_eq!(Rect::new(i32::MIN, 0, i32::MAX, 1).width(), i32::MAX);
        let r = Rect::new(-5, 7, 900, -3);
        assert_eq!(Rect::from_bytes(&r.to_bytes()), r);
    }
    #[test]
    fn lparam_points_sign_extend() {
        let p = Point::new(-2, 300);
        assert_eq!(Point::from_lparam(p.to_lparam()), p);
        assert_eq!(Point::from_lparam(0xffff_fffe), Point::new(-2, -1));
    }
    #[test]
    fn colorref_maps_to_bgrx_pixels() {
        let c = ColorRef::rgb(0x12, 0x34, 0x56);
        assert_eq!(c.0, 0x0056_3412);
        assert_eq!(c.pixel(), 0x0012_3456);
        assert_eq!(ColorRef::from_pixel(c.pixel()), c);
        assert_eq!(ColorRef::validated(0x0200_0102), Some(ColorRef(0x102)));
        assert_eq!(ColorRef::validated(0x0100_0001), None);
    }
    #[test]
    fn wide_helpers_are_bounded() {
        let w = wide::from_str("Ab\u{e9}");
        assert_eq!(wide::to_string(&w), "Ab\u{e9}");
        assert_eq!(wide::nul_len(&[65, 0, 66]), 1);
        assert!(wide::eq_ignore_case(
            &wide::from_str("Button"),
            &wide::from_str("BUTTON")
        ));
        assert_eq!(wide::to_string(&[0xd800]), "\u{fffd}");
    }
}
