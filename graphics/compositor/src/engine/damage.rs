//! Small list of damage rectangles. Two updates far apart (a caret in one
//! window, a counter in another) recompose two small regions instead of their
//! bounding box; rectangles that overlap or whose union wastes little area
//! are merged. The list is bounded, so the worst case degrades to the union.
use super::*;

pub const MAX_DAMAGE_RECTS: usize = 4;

/// Merge when the union is at most this many eighths of the parts' area.
const ECONOMY_EIGHTHS: u64 = 12;

fn area(r: Rect) -> u64 {
    u64::from(r.width()) * u64::from(r.height())
}
/// Merging `a` and `b` costs little: they overlap/touch, or their bounding
/// box wastes under half of their combined area.
fn economical(a: Rect, b: Rect) -> bool {
    let Some(union) = a.union(b) else {
        return true;
    };
    let touching =
        a.right() >= b.x() && b.right() >= a.x() && a.bottom() >= b.y() && b.bottom() >= a.y();
    touching || area(union) * 8 <= (area(a) + area(b)) * ECONOMY_EIGHTHS
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageList {
    rects: [Option<Rect>; MAX_DAMAGE_RECTS],
}
impl DamageList {
    pub fn single(r: Rect) -> Self {
        let mut list = Self::default();
        list.rects[0] = Some(r);
        list
    }
    pub fn iter(&self) -> impl Iterator<Item = Rect> + '_ {
        self.rects.iter().flatten().copied()
    }
    pub fn count(&self) -> usize {
        self.rects.iter().flatten().count()
    }
    pub fn bounds(&self) -> Option<Rect> {
        self.iter().reduce(|a, b| a.union(b).unwrap_or(a))
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// Add `r` keeping at most `limit` (1..=MAX) rectangles.
    pub fn add(&mut self, mut r: Rect, limit: usize) {
        let limit = limit.clamp(1, MAX_DAMAGE_RECTS);
        loop {
            let merge = self
                .rects
                .iter()
                .position(|p| p.is_some_and(|p| economical(p, r)));
            match merge {
                Some(i) => {
                    let p = self.rects[i].take().unwrap();
                    r = p.union(r).unwrap_or(p);
                }
                None => break,
            }
        }
        if self.count() < limit {
            let slot = self.rects.iter().position(Option::is_none).unwrap();
            self.rects[slot] = Some(r);
            return;
        }
        // Full: merge into the part whose union grows the least, then
        // re-merge any parts the grown rectangle now reaches.
        let i = self
            .rects
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.map(|p| (i, p)))
            .min_by_key(|(_, p)| p.union(r).map_or(u64::MAX, |u| area(u) - area(*p)))
            .map(|(i, _)| i)
            .unwrap();
        let p = self.rects[i].take().unwrap();
        let grown = p.union(r).unwrap_or(p);
        self.add(grown, limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect::new(x, y, w, h).unwrap()
    }
    #[test]
    fn distant_updates_stay_separate_and_overlaps_merge() {
        let mut list = DamageList::default();
        list.add(r(10, 10, 20, 20), 4);
        list.add(r(1000, 800, 20, 20), 4);
        assert_eq!(list.count(), 2);
        // Overlapping and adjacent rectangles merge.
        list.add(r(25, 25, 20, 20), 4);
        list.add(r(45, 10, 10, 10), 4);
        assert_eq!(list.count(), 2);
        assert!(list.iter().any(|p| p == r(10, 10, 45, 35)));
        // A nearby small update whose union wastes little merges too.
        list.add(r(1000, 822, 20, 4), 4);
        assert_eq!(list.count(), 2);
        assert_eq!(list.bounds(), Some(r(10, 10, 1010, 816)));
    }
    #[test]
    fn the_limit_bounds_the_list_and_cascades_merges() {
        let mut list = DamageList::default();
        for i in 0..10 {
            list.add(r(i * 200, i * 100, 10, 10), 3);
            assert!(list.count() <= 3);
        }
        assert_eq!(list.bounds(), Some(r(0, 0, 1810, 910)));
        let mut single = DamageList::default();
        single.add(r(0, 0, 10, 10), 1);
        single.add(r(500, 500, 10, 10), 1);
        assert_eq!(
            single.iter().collect::<std::vec::Vec<_>>(),
            [r(0, 0, 510, 510)]
        );
        // Every point of every added rectangle stays covered.
        let mut list = DamageList::default();
        let added: std::vec::Vec<_> = (0..40)
            .map(|i| r((i * 97) % 1800, (i * 53) % 1000, 30 + i as u32 % 7, 20))
            .collect();
        for rect in &added {
            list.add(*rect, 4);
        }
        for rect in added {
            assert!(list.iter().any(|p| p.intersection(rect) == Some(rect)));
        }
    }
}
