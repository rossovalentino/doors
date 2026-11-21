//! Shared Explorer-style document caption geometry. Values are logical DIPs.
use crate::explorer_layout::Bounds;
static NEXT_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
/// Process-local model identity survives reorder, detach and display reconnection.
/// Zero means identity exhaustion and is never eligible for transfer.
pub fn next_id() -> u64 {
    NEXT_ID
        .fetch_update(
            core::sync::atomic::Ordering::Relaxed,
            core::sync::atomic::Ordering::Relaxed,
            |n| n.checked_add(1),
        )
        .unwrap_or(0)
}
/// All tab/window entry points share this allocator, including same-tick requests.
pub fn untitled_path(parent: &str, ticks: u64) -> Option<alloc::string::String> {
    let id = next_id();
    (id != 0).then(|| alloc::format!("{parent}/Untitled-{ticks}-{id}.txt"))
}
#[derive(Clone, Copy, Debug)]
pub struct Strip {
    pub width: u32,
    pub left: i32,
    pub count: usize,
    pub active: usize,
}
impl Strip {
    pub fn capacity(self) -> usize {
        (self.width.saturating_sub(self.left.max(0) as u32 + 184) / 96).max(1) as usize
    }
    pub fn start(self) -> usize {
        self.active
            .min(self.count.saturating_sub(1))
            .saturating_sub(self.capacity() - 1)
    }
    pub fn range(self) -> core::ops::Range<usize> {
        let s = self.start();
        s..(s + self.capacity()).min(self.count)
    }
    pub fn tab(self, index: usize) -> Option<Bounds> {
        if !self.range().contains(&index) {
            return None;
        }
        let available = self.width.saturating_sub(self.left.max(0) as u32 + 184);
        let stride = (available / self.range().len().max(1) as u32).min(294);
        if stride < 64 {
            return None;
        }
        Some(Bounds {
            x: self.left + (index - self.start()) as i32 * stride as i32,
            y: 3,
            width: stride.saturating_sub(4),
            height: 38,
        })
    }
    pub fn close(self, index: usize) -> Option<Bounds> {
        let r = self.tab(index)?;
        Some(Bounds {
            x: r.x + r.width as i32 - 32,
            y: r.y + 9,
            width: 24,
            height: 20,
        })
    }
    pub fn plus(self) -> Option<Bounds> {
        let x = self
            .range()
            .filter_map(|i| self.tab(i))
            .map(|r| r.x + r.width as i32 + 6)
            .max()
            .unwrap_or(self.left);
        (x + 32 <= self.width as i32 - 144).then_some(Bounds {
            x,
            y: 6,
            width: 32,
            height: 32,
        })
    }
    pub fn hit(self, x: i32, y: i32) -> Option<usize> {
        self.range().find(|&i| {
            self.tab(i).is_some_and(|r| r.contains(x, y))
                && !self.close(i).is_some_and(|r| r.contains(x, y))
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub moved: bool,
}
impl Drag {
    pub fn motion(&mut self, x: i32, y: i32) {
        self.moved |= (x - self.x).abs() >= 6 || (y - self.y).abs() >= 6;
    }
    pub fn detached(self, x: i32, y: i32, left: i32, right: i32) -> bool {
        self.moved && (!(-12..=56).contains(&y) || x < left - 12 || x > right + 12)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_tick_tab_and_window_names_cannot_alias() {
        let a = untitled_path("/users/1001/notes", 42).unwrap();
        let b = untitled_path("/users/1001/notes", 42).unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("/users/1001/notes/Untitled-42-"));
    }
    #[test]
    fn caption_controls_never_cover_trusted_system_buttons() {
        for width in [240, 490, 900, 1280, 1472] {
            for n in 1..=8 {
                for active in 0..n {
                    let s = Strip {
                        width,
                        left: 8,
                        count: n,
                        active,
                    };
                    for i in s.range() {
                        if let Some(r) = s.tab(i) {
                            assert!(r.x >= 8 && r.x + r.width as i32 <= width as i32 - 144);
                            assert_eq!(s.hit(r.x + 20, 20), Some(i));
                            let c = s.close(i).unwrap();
                            assert_eq!(s.hit(c.x + 5, c.y + 5), None);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn a_click_or_small_jitter_never_detaches_a_document() {
        let mut d = Drag {
            index: 2,
            x: 100,
            y: 20,
            moved: false,
        };
        d.motion(103, 22);
        assert!(!d.detached(103, 70, 8, 800));
        d.motion(110, 20);
        assert!(!d.detached(110, 22, 8, 800));
        assert!(d.detached(110, 70, 8, 800));
    }
}
