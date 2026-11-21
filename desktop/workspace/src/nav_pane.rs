//! Shared navigation-pane model: grouped destination rows, disclosure headers,
//! pixel scrolling and a draggable thumb. Painting and hit testing both read
//! `Pane::layout`, so a visible row is exactly the region that answers clicks.
extern crate alloc;
use crate::fluent::Rect;
use alloc::vec::Vec;

/// One logical row. Headers own a group; items name an app destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row<K> {
    Header(u8),
    Item(K),
}

/// Destination rows and expanded headers share one pitch; a compact
/// (icon-only) pane shows headers as thin separators.
pub const PITCH: i32 = 40;
pub const ROW_HEIGHT: u32 = 36;
const SEPARATOR_PITCH: i32 = 13;
/// Destination row pitch at the applied Accessibility > Text size: rows hold
/// one 14 px label and grow with its line box (`PITCH` at 100%).
pub fn pitch() -> i32 {
    crate::palette::text_grow_i(PITCH, door_graphics_text::TextStyle::Regular14)
}
/// Destination row height at the applied text size (`ROW_HEIGHT` at 100%).
pub fn row_height() -> u32 {
    crate::palette::text_grow(ROW_HEIGHT, door_graphics_text::TextStyle::Regular14)
}
/// Thumb hit band and paint offset from the pane's right edge.
pub const THUMB_BAND: i32 = 10;
const MIN_THUMB: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed<K> {
    /// Index into the row slice passed to `layout`.
    pub index: usize,
    pub row: Row<K>,
    /// Unclipped row geometry; paint inside the pane viewport.
    pub rect: Rect,
    /// The part of `rect` inside the viewport; the only clickable region.
    pub visible: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    pub x: i32,
    pub width: u32,
    /// Viewport top and bottom (exclusive) in window coordinates.
    pub top: i32,
    pub bottom: i32,
    pub scroll: u32,
    pub compact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub top: i32,
    pub height: i32,
    pub travel: i32,
}

impl Pane {
    fn pitch<K>(self, row: Row<K>) -> i32 {
        match row {
            Row::Header(_) if self.compact => SEPARATOR_PITCH,
            _ => pitch(),
        }
    }
    fn viewport(self) -> u32 {
        (self.bottom - self.top).max(0) as u32
    }
    pub fn content<K: Copy>(self, rows: &[Row<K>]) -> u32 {
        rows.iter().map(|r| self.pitch(*r) as u32).sum()
    }
    pub fn max_scroll<K: Copy>(self, rows: &[Row<K>]) -> u32 {
        // The final row needs no trailing gap below it.
        let used = self
            .content(rows)
            .saturating_sub((pitch() as u32).saturating_sub(row_height()));
        used.saturating_sub(self.viewport())
    }
    pub fn clamped<K: Copy>(mut self, rows: &[Row<K>]) -> Self {
        self.scroll = self.scroll.min(self.max_scroll(rows));
        self
    }
    /// Rows that intersect the viewport, in order.
    pub fn layout<K: Copy>(self, rows: &[Row<K>]) -> Vec<Placed<K>> {
        let mut placed = Vec::new();
        let mut y = self.top - self.scroll.min(self.max_scroll(rows)) as i32;
        let width = self.width.saturating_sub(24);
        for (index, row) in rows.iter().copied().enumerate() {
            let pitch = self.pitch(row);
            let height = if pitch == self::pitch() {
                row_height()
            } else {
                pitch as u32
            };
            let rect = Rect::new(self.x + 12, y, width, height);
            let top = y.max(self.top);
            let bottom = (y + height as i32).min(self.bottom);
            if bottom > top {
                placed.push(Placed {
                    index,
                    row,
                    rect,
                    visible: Rect::new(rect.x, top, rect.w, (bottom - top) as u32),
                });
            }
            y += pitch;
            if y >= self.bottom {
                break;
            }
        }
        placed
    }
    /// The clickable row under a point; separators never answer.
    pub fn hit<K: Copy>(self, rows: &[Row<K>], x: i32, y: i32) -> Option<Placed<K>> {
        self.layout(rows)
            .into_iter()
            .find(|p| p.visible.hit(x, y) && !(self.compact && matches!(p.row, Row::Header(_))))
    }
    /// Smallest scroll change that shows `index` completely.
    pub fn reveal<K: Copy>(self, rows: &[Row<K>], index: usize) -> u32 {
        let start: i32 = rows.iter().take(index).map(|r| self.pitch(*r)).sum();
        let height = rows
            .get(index)
            .map_or(0, |r| self.pitch(*r).min(row_height() as i32));
        let mut scroll = self.scroll as i32;
        if start < scroll {
            scroll = start;
        }
        if start + height - scroll > self.viewport() as i32 {
            scroll = start + height - self.viewport() as i32;
        }
        (scroll.max(0) as u32).min(self.max_scroll(rows))
    }
    pub fn thumb<K: Copy>(self, rows: &[Row<K>]) -> Option<Thumb> {
        let max = self.max_scroll(rows);
        if max == 0 {
            return None;
        }
        let track = self.viewport() as i64;
        let height = (track * track / (track + i64::from(max)))
            .max(MIN_THUMB as i64)
            .min(track) as i32;
        let travel = track as i32 - height;
        Some(Thumb {
            top: self.top
                + (i64::from(travel) * i64::from(self.scroll.min(max)) / i64::from(max)) as i32,
            height,
            travel,
        })
    }
    /// The thumb's drag band: the full track height at the pane's right edge.
    pub fn track<K: Copy>(self, rows: &[Row<K>]) -> Option<Rect> {
        self.thumb(rows)?;
        Some(Rect::new(
            self.x + self.width as i32 - THUMB_BAND,
            self.top,
            THUMB_BAND as u32,
            self.viewport(),
        ))
    }
    /// Where a press on the track grabs the thumb: inside it keeps the offset,
    /// elsewhere centers it under the pointer.
    pub fn grab<K: Copy>(self, rows: &[Row<K>], y: i32) -> Option<i32> {
        let t = self.thumb(rows)?;
        Some(if (t.top..t.top + t.height).contains(&y) {
            y - t.top
        } else {
            t.height / 2
        })
    }
    /// Restricts painting to the viewport; false when no row can be visible.
    /// Call `View::reset_viewport` after painting the rows.
    pub fn clip(self, v: &mut crate::View<'_>) -> bool {
        match door_graphics_core::Rect::new(self.x, self.top, self.width, self.viewport()) {
            Ok(r) if r.height() > 0 => {
                v.viewport(r, 0);
                true
            }
            _ => false,
        }
    }
    pub fn drag<K: Copy>(self, rows: &[Row<K>], y: i32, grab: i32) -> u32 {
        let Some(t) = self.thumb(rows) else {
            return 0;
        };
        let max = self.max_scroll(rows);
        ((y - self.top - grab).clamp(0, t.travel) as u64 * u64::from(max) / t.travel.max(1) as u64)
            as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn rows() -> Vec<Row<u32>> {
        let mut r = vec![Row::Item(0), Row::Item(1), Row::Header(0)];
        r.extend((2..9).map(Row::Item));
        r.push(Row::Header(1));
        r.extend((9..15).map(Row::Item));
        r
    }
    fn pane(h: i32) -> Pane {
        Pane {
            x: 0,
            width: 220,
            top: 68,
            bottom: h,
            scroll: 0,
            compact: false,
        }
    }
    #[test]
    fn every_row_is_reachable_and_hit_regions_never_leave_the_viewport() {
        let rows = rows();
        for bottom in [200, 300, 500, 760, 1200] {
            let base = pane(bottom);
            let mut reached = vec![false; rows.len()];
            for scroll in 0..=base.max_scroll(&rows) {
                let p = Pane { scroll, ..base };
                for placed in p.layout(&rows) {
                    assert!(placed.visible.y >= p.top);
                    assert!(placed.visible.y + placed.visible.h as i32 <= p.bottom);
                    let probe = placed.visible.y + placed.visible.h as i32 / 2;
                    assert_eq!(p.hit(&rows, 40, probe).map(|h| h.index), Some(placed.index));
                    if placed.visible == placed.rect {
                        reached[placed.index] = true;
                    }
                }
                assert!(p.hit(&rows, 40, p.top - 1).is_none());
                assert!(p.hit(&rows, 40, p.bottom).is_none());
            }
            assert!(
                reached.iter().all(|r| *r),
                "unreachable row at bottom {bottom}"
            );
        }
    }
    #[test]
    fn reveal_thumb_and_drag_are_consistent() {
        let rows = rows();
        let p = pane(400);
        let max = p.max_scroll(&rows);
        assert!(max > 0);
        let last = rows.len() - 1;
        let shown = Pane {
            scroll: p.reveal(&rows, last),
            ..p
        };
        assert!(
            shown
                .layout(&rows)
                .iter()
                .any(|r| r.index == last && r.visible == r.rect)
        );
        assert_eq!(Pane { scroll: max, ..p }.reveal(&rows, 0), 0);
        let t = p.thumb(&rows).unwrap();
        assert_eq!(t.top, p.top);
        assert_eq!(p.drag(&rows, p.bottom + 50, 0), max);
        assert_eq!(p.drag(&rows, p.top - 50, 0), 0);
        assert!(pane(2000).thumb(&rows).is_none());
        assert!(pane(2000).track(&rows).is_none());
    }
    #[test]
    fn compact_headers_are_separators_without_targets() {
        let rows = rows();
        let p = Pane {
            width: 64,
            compact: true,
            ..pane(1200)
        };
        let header = p
            .layout(&rows)
            .into_iter()
            .find(|r| r.row == Row::Header(0))
            .unwrap();
        assert!(p.hit(&rows, 30, header.visible.y + 2).is_none());
        assert!(p.content(&rows) < pane(1200).content(&rows));
    }
}
