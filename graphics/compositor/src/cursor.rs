//! Pinned Windows cursor resources exported outside the guest. Alpha-zero
//! white is the explicit monochrome inversion operation, not a premultiplied pixel.
//! Exports exist at 32/64/96 px (1x/2x/3x); fractional scales area-filter the
//! next larger export to `32 * scale` (40 px at 125%, 48 px at 150%).
use crate::Scale;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorIcon {
    #[default]
    Arrow,
    Text,
    EastWest,
    NorthSouth,
    NorthWestSouthEast,
    NorthEastSouthWest,
}
const INVERT: [u8; 4] = [255, 255, 255, 0];

impl CursorIcon {
    /// Logical cursor size.
    pub const SIZE: usize = 32;
    /// Physical cursor size at `scale`.
    pub fn size(scale: Scale) -> usize {
        scale.len(Self::SIZE as u32) as usize
    }
    pub fn resize_edges(rect: crate::Rect, x: i32, y: i32) -> u8 {
        Self::resize_edges_scaled(rect, x, y, Scale::ONE)
    }
    /// Logical width of the invisible resize band outside a normal window's
    /// frame (Windows 11 keeps 7 px of invisible border beyond the 1 px
    /// frame).
    pub const RESIZE_OUTSIDE: u32 = 7;
    /// Logical length along an edge, from each corner, that resizes both
    /// axes (the diagonal cursor), as Windows' larger corner targets do.
    pub const RESIZE_CORNER: u32 = 16;
    /// Edges (1 left, 2 right, 4 top, 8 bottom) a resize press at (x, y)
    /// moves: within 4 px inside the frame or anywhere outside it; on a side
    /// within `RESIZE_CORNER` of a corner, both of that corner's edges.
    pub fn resize_edges_scaled(rect: crate::Rect, x: i32, y: i32, scale: Scale) -> u8 {
        let margin = scale.len(4) as i32;
        let corner = scale.len(Self::RESIZE_CORNER) as i32;
        let (dx, dy) = (x - rect.x(), y - rect.y());
        let (w, h) = (rect.width() as i32, rect.height() as i32);
        let mut edges = u8::from(dx < margin)
            | (u8::from(dx >= w - margin) << 1)
            | (u8::from(dy < margin) << 2)
            | (u8::from(dy >= h - margin) << 3);
        if edges & 3 != 0 && edges & 12 == 0 {
            edges |= (u8::from(dy < corner) << 2) | (u8::from(dy >= h - corner) << 3);
        } else if edges & 12 != 0 && edges & 3 == 0 {
            edges |= u8::from(dx < corner) | (u8::from(dx >= w - corner) << 1);
        }
        edges
    }

    /// Export of integer factor 1..=3.
    fn raster(self, factor: u32) -> &'static [u8] {
        macro_rules! asset {
            ($n:literal) => {
                if factor == 3 {
                    &include_bytes!(concat!(
                        "../../../assets/cursors/windows/",
                        $n,
                        ".96.cursor"
                    ))[..]
                } else if factor == 2 {
                    &include_bytes!(concat!(
                        "../../../assets/cursors/windows/",
                        $n,
                        ".64.cursor"
                    ))[..]
                } else {
                    &include_bytes!(concat!("../../../assets/cursors/windows/", $n, ".cursor"))[..]
                }
            };
        }
        match self {
            Self::Arrow => asset!("aero_arrow"),
            Self::Text => asset!("beam_r"),
            Self::EastWest => asset!("aero_ew"),
            Self::NorthSouth => asset!("aero_ns"),
            Self::NorthWestSouthEast => asset!("aero_nwse"),
            Self::NorthEastSouthWest => asset!("aero_nesw"),
        }
    }
    pub fn hotspot(self) -> (i32, i32) {
        self.hotspot_scaled(Scale::ONE)
    }
    pub fn hotspot_scaled(self, scale: Scale) -> (i32, i32) {
        let factor = scale.percent().div_ceil(100);
        let b = self.raster(factor);
        let (source, target) = (32 * factor as i32, Self::size(scale) as i32);
        let map = |v: u8| (i32::from(v) * target + source / 2) / source;
        (map(b[0]), map(b[1]))
    }
    pub fn pixel(self, x: i32, y: i32) -> Option<[u8; 4]> {
        self.pixel_scaled(x, y, Scale::ONE)
    }
    /// Premultiplied pixel, the inversion marker, or None (transparent).
    pub fn pixel_scaled(self, x: i32, y: i32, scale: Scale) -> Option<[u8; 4]> {
        let size = Self::size(scale) as i32;
        if !(0..size).contains(&x) || !(0..size).contains(&y) {
            return None;
        }
        let factor = scale.percent().div_ceil(100);
        let b = self.raster(factor);
        let source = 32 * factor as usize;
        let texel = |sx: usize, sy: usize| -> [u8; 4] {
            let at = 2 + (sy * source + sx) * 4;
            b[at..at + 4].try_into().unwrap()
        };
        let target = size as usize;
        if source == target {
            let p = texel(x as usize, y as usize);
            return (p != [0; 4]).then_some(p);
        }
        // Area filter: premultiplied colour over the covered source area;
        // inversion wins when it covers at least half of the pixel.
        let span = |t: usize| {
            let start = t * source;
            let end = start + source;
            (start / target, end.div_ceil(target).min(source), start, end)
        };
        let (x0, x1, xs, xe) = span(x as usize);
        let (y0, y1, ys, ye) = span(y as usize);
        let (mut sum, mut invert) = ([0u64; 4], 0u64);
        for sy in y0..y1 {
            let wy = (((sy + 1) * target).min(ye) - (sy * target).max(ys)) as u64;
            for sx in x0..x1 {
                let w = wy * (((sx + 1) * target).min(xe) - (sx * target).max(xs)) as u64;
                let p = texel(sx, sy);
                if p == INVERT {
                    invert += w;
                } else {
                    for c in 0..4 {
                        sum[c] += u64::from(p[c]) * w;
                    }
                }
            }
        }
        let total = (source * source) as u64;
        if invert * 2 >= total {
            return Some(INVERT);
        }
        let p = sum.map(|v| ((v + total / 2) / total) as u8);
        (p != [0; 4]).then_some(p)
    }
    pub fn resize(edges: u8) -> Self {
        match (edges & 3 != 0, edges & 12 != 0) {
            (true, true) if matches!(edges, 5 | 10) => Self::NorthWestSouthEast,
            (true, true) => Self::NorthEastSouthWest,
            (true, false) => Self::EastWest,
            (false, true) => Self::NorthSouth,
            _ => Self::Arrow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resources_preserve_hotspots_clipping_and_valid_alpha_or_inversion() {
        for icon in [
            CursorIcon::Arrow,
            CursorIcon::Text,
            CursorIcon::EastWest,
            CursorIcon::NorthSouth,
            CursorIcon::NorthWestSouthEast,
            CursorIcon::NorthEastSouthWest,
        ] {
            let (x, y) = icon.hotspot();
            assert!((0..32).contains(&x) && (0..32).contains(&y));
            assert_eq!(icon.pixel(-1, 0), None);
            assert_eq!(icon.pixel(32, 0), None);
            for y in 0..32 {
                for x in 0..32 {
                    if let Some(p) = icon.pixel(x, y) {
                        assert!(p == [255, 255, 255, 0] || p[..3].iter().all(|v| *v <= p[3]));
                    }
                }
            }
        }
    }
}
