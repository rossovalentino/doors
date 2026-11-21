//! GDI regions as y-x banded rectangle lists: rectangles are disjoint, sorted
//! by band (top) then left, every band holds non-touching spans, and
//! vertically adjacent bands with identical spans are coalesced. This is the
//! canonical form, so equal regions compare equal (`EqualRgn`).
use crate::{Error, Point, Rect, Result, try_push};
use alloc::vec::Vec;

/// Bound on rectangles in one region.
pub const MAX_RECTS: usize = 8192;

/// `CombineRgn` modes (RGN_AND .. RGN_COPY).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    And,
    Or,
    Xor,
    Diff,
    Copy,
}
impl Op {
    pub fn from_win32(mode: i32) -> Option<Self> {
        Some(match mode {
            1 => Self::And,
            2 => Self::Or,
            3 => Self::Xor,
            4 => Self::Diff,
            5 => Self::Copy,
            _ => return None,
        })
    }
    fn keep(self, a: bool, b: bool) -> bool {
        match self {
            Self::And => a && b,
            Self::Or => a || b,
            Self::Xor => a != b,
            Self::Diff => a && !b,
            Self::Copy => a,
        }
    }
}

/// `NULLREGION`, `SIMPLEREGION`, `COMPLEXREGION`.
pub const NULLREGION: i32 = 1;
pub const SIMPLEREGION: i32 = 2;
pub const COMPLEXREGION: i32 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    rects: Vec<Rect>,
}

impl Region {
    pub const fn new() -> Self {
        Self { rects: Vec::new() }
    }
    pub fn from_rect(r: Rect) -> Self {
        let r = r.normalized();
        let mut rects = Vec::new();
        if !r.is_empty() {
            rects.push(r);
        }
        Self { rects }
    }
    /// Build from arbitrary (possibly overlapping) rectangles.
    pub fn from_rects(list: &[Rect]) -> Result<Self> {
        let mut out = Self::new();
        for r in list {
            out = out.combine(&Self::from_rect(*r), Op::Or)?;
        }
        Ok(out)
    }
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    pub fn kind(&self) -> i32 {
        match self.rects.len() {
            0 => NULLREGION,
            1 => SIMPLEREGION,
            _ => COMPLEXREGION,
        }
    }
    pub fn bounds(&self) -> Rect {
        self.rects.iter().fold(Rect::EMPTY, |b, r| b.union(*r))
    }
    pub fn contains(&self, p: Point) -> bool {
        self.rects.iter().any(|r| r.contains(p))
    }
    /// `RectInRegion`: any overlap with a normalized rectangle.
    pub fn intersects(&self, r: Rect) -> bool {
        let r = r.normalized();
        self.rects.iter().any(|x| x.intersects(r))
    }
    pub fn offset(&mut self, dx: i32, dy: i32) {
        for r in &mut self.rects {
            *r = r.offset(dx, dy);
        }
    }
    /// Rectangles of this region clipped to `r`.
    pub fn clipped(&self, r: Rect) -> impl Iterator<Item = Rect> + '_ {
        self.rects
            .iter()
            .map(move |x| x.intersect(r))
            .filter(|x| !x.is_empty())
    }
    pub fn intersect_rect(&self, r: Rect) -> Result<Self> {
        self.combine(&Self::from_rect(r), Op::And)
    }
    pub fn union_rect(&self, r: Rect) -> Result<Self> {
        self.combine(&Self::from_rect(r), Op::Or)
    }
    pub fn subtract_rect(&self, r: Rect) -> Result<Self> {
        self.combine(&Self::from_rect(r), Op::Diff)
    }
    /// Spans of the band starting at `y0` (bands never cross an edge).
    fn spans(&self, y0: i32, out: &mut Vec<(i32, i32)>) {
        out.clear();
        // Banded form: every rectangle of a band shares its top and bottom.
        let end = self.rects.partition_point(|r| r.top <= y0);
        if end == 0 {
            return;
        }
        let top = self.rects[end - 1].top;
        let start = self.rects[..end].partition_point(|r| r.top < top);
        for r in &self.rects[start..end] {
            if y0 < r.bottom {
                out.push((r.left, r.right));
            }
        }
    }
    pub fn combine(&self, other: &Self, op: Op) -> Result<Self> {
        if op == Op::Copy {
            return Ok(self.clone());
        }
        let mut ys: Vec<i32> = Vec::new();
        ys.try_reserve(2 * (self.rects.len() + other.rects.len()))
            .map_err(|_| Error::Quota)?;
        for r in self.rects.iter().chain(other.rects.iter()) {
            ys.push(r.top);
            ys.push(r.bottom);
        }
        ys.sort_unstable();
        ys.dedup();
        let mut out = Self::new();
        let (mut a, mut b, mut xs) = (Vec::new(), Vec::new(), Vec::new());
        let mut cur: Vec<(i32, i32)> = Vec::new();
        let mut prev: Vec<(i32, i32)> = Vec::new();
        let mut prev_start = 0usize;
        for w in ys.windows(2) {
            let (y0, y1) = (w[0], w[1]);
            self.spans(y0, &mut a);
            other.spans(y0, &mut b);
            xs.clear();
            for &(l, r) in a.iter().chain(b.iter()) {
                xs.push(l);
                xs.push(r);
            }
            xs.sort_unstable();
            xs.dedup();
            cur.clear();
            for x in xs.windows(2) {
                let probe = x[0];
                let ina = a.iter().any(|&(l, r)| l <= probe && probe < r);
                let inb = b.iter().any(|&(l, r)| l <= probe && probe < r);
                if op.keep(ina, inb) {
                    match cur.last_mut() {
                        Some((_, right)) if *right == x[0] => *right = x[1],
                        _ => cur.push((x[0], x[1])),
                    }
                }
            }
            if cur.is_empty() {
                prev.clear();
                continue;
            }
            // Coalesce with the band directly above when the spans match.
            let touching = out
                .rects
                .last()
                .is_some_and(|r| r.bottom == y0 && prev == cur);
            if touching {
                for r in &mut out.rects[prev_start..] {
                    r.bottom = y1;
                }
            } else {
                prev_start = out.rects.len();
                for &(l, r) in &cur {
                    try_push(&mut out.rects, Rect::new(l, y0, r, y1), MAX_RECTS)?;
                }
                prev.clear();
                prev.extend_from_slice(&cur);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(l: i32, t: i32, rr: i32, b: i32) -> Region {
        Region::from_rect(Rect::new(l, t, rr, b))
    }
    #[test]
    fn set_operations_are_canonical() {
        let a = r(0, 0, 10, 10);
        let b = r(5, 5, 15, 15);
        let u = a.combine(&b, Op::Or).unwrap();
        assert_eq!(u.kind(), COMPLEXREGION);
        assert_eq!(u.bounds(), Rect::new(0, 0, 15, 15));
        assert_eq!(
            u.rects(),
            &[
                Rect::new(0, 0, 10, 5),
                Rect::new(0, 5, 15, 10),
                Rect::new(5, 10, 15, 15)
            ]
        );
        let i = a.combine(&b, Op::And).unwrap();
        assert_eq!(i.rects(), &[Rect::new(5, 5, 10, 10)]);
        assert_eq!(i.kind(), SIMPLEREGION);
        let d = a.combine(&b, Op::Diff).unwrap();
        assert!(d.contains(Point::new(0, 0)) && !d.contains(Point::new(7, 7)));
        let x = a.combine(&b, Op::Xor).unwrap();
        assert!(x.contains(Point::new(12, 12)) && !x.contains(Point::new(6, 6)));
        // Union of the pieces restores the original: canonical equality.
        let back = d.combine(&i, Op::Or).unwrap();
        assert_eq!(back, a);
        let two = r(0, 0, 4, 4).combine(&r(0, 4, 4, 8), Op::Or).unwrap();
        assert_eq!(two.rects(), &[Rect::new(0, 0, 4, 8)], "bands coalesce");
        let side = r(0, 0, 4, 4).combine(&r(4, 0, 8, 4), Op::Or).unwrap();
        assert_eq!(side.rects(), &[Rect::new(0, 0, 8, 4)], "spans merge");
        assert_eq!(a.combine(&a, Op::Diff).unwrap().kind(), NULLREGION);
        assert_eq!(
            Region::from_rect(Rect::new(5, 5, 1, 1)).bounds(),
            Rect::new(1, 1, 5, 5)
        );
        assert!(Region::from_rect(Rect::new(1, 1, 1, 9)).is_empty());
    }
    #[test]
    fn hit_testing_offsets_and_clipping() {
        let mut u =
            Region::from_rects(&[Rect::new(0, 0, 2, 2), Rect::new(10, 10, 12, 12)]).unwrap();
        assert!(u.contains(Point::new(1, 1)) && !u.contains(Point::new(2, 2)));
        assert!(u.intersects(Rect::new(11, 11, 20, 20)) && !u.intersects(Rect::new(3, 3, 9, 9)));
        u.offset(5, -1);
        assert!(u.contains(Point::new(15, 9)));
        let c: Vec<Rect> = u.clipped(Rect::new(0, 0, 16, 10)).collect();
        assert_eq!(c, [Rect::new(5, 0, 7, 1), Rect::new(15, 9, 16, 10)]);
        assert_eq!(Op::from_win32(6), None);
        assert_eq!(Op::from_win32(4), Some(Op::Diff));
    }
    #[test]
    fn hostile_fragmentation_hits_the_bound() {
        let mut checker = Region::new();
        let mut result = Ok(());
        for y in 0..200 {
            let row: Vec<Rect> = (0..50)
                .map(|i| {
                    let x = i * 4 + (y % 2) * 2;
                    Rect::new(x, y, x + 1, y + 1)
                })
                .collect();
            let row = Region::from_rects(&row).unwrap();
            match checker.combine(&row, Op::Or) {
                Ok(next) => checker = next,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        assert_eq!(result, Err(Error::Quota));
        assert!(checker.rects().len() <= MAX_RECTS);
    }
}
