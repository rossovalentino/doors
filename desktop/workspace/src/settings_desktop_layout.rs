//! Reference-derived taskbar/context page geometry shared by paint and input.
use crate::settings_layout::Page;
use door_graphics_core::Rect;
pub struct DesktopLayout {
    pub width: u32,
}
impl DesktopLayout {
    pub fn position(&self, index: usize) -> Option<Rect> {
        if index >= 4 {
            return None;
        }
        let columns = if self.width >= 600 { 4 } else { 2 };
        Rect::new(
            334 + (index % columns) as i32 * 136,
            340 + (index / columns) as i32 * 118,
            120,
            104,
        )
        .ok()
    }
    pub fn behavior_top(&self) -> i32 {
        if self.width >= 600 { 464 } else { 582 }
    }
    pub fn taskbar_bottom(&self) -> u32 {
        (self.behavior_top() + 750) as u32
    }
    pub fn taskbar_control(&self, id: u32) -> Option<Rect> {
        match id {
            700..=703 => self.position((id - 700) as usize),
            704 => Rect::new(
                310 + self.width as i32 - 164,
                self.behavior_top() + 14,
                140,
                32,
            )
            .ok(),
            705 => Rect::new(
                310 + self.width as i32 - 64,
                self.behavior_top() + 92,
                40,
                20,
            )
            .ok(),
            _ => None,
        }
    }
    pub fn context_control(&self, id: u32) -> Option<Rect> {
        let y = match id {
            706 => 692,
            707 => 908,
            _ => return None,
        };
        Rect::new(310 + self.width as i32 - 64, y + 24, 40, 20).ok()
    }
    pub fn target(&self, page: Page, x: i32, y: i32) -> Option<u32> {
        if self.width < 280 {
            return None;
        }
        match page {
            Page::Taskbar => {
                (700..=705).find(|id| self.taskbar_control(*id).is_some_and(|r| r.contains(x, y)))
            }
            Page::ContextMenu => {
                (706..=707).find(|id| self.context_control(*id).is_some_and(|r| r.contains(x, y)))
            }
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_control_has_its_visible_reference_hit_rectangle_and_no_disabled_row_target() {
        for width in [280, 480, 797, 1200] {
            let l = DesktopLayout { width };
            for id in 700..=707 {
                let page = if id < 706 {
                    Page::Taskbar
                } else {
                    Page::ContextMenu
                };
                let r = if id < 706 {
                    l.taskbar_control(id)
                } else {
                    l.context_control(id)
                }
                .unwrap();
                assert!(r.x() >= 310 && r.right() <= (310 + width) as i32);
                assert_eq!(l.target(page, r.x(), r.y()), Some(id));
                assert_ne!(l.target(page, r.right(), r.y()), Some(id));
            }
            assert_eq!(l.target(Page::ContextMenu, 334, 224), None);
            assert_eq!(l.target(Page::Taskbar, 334, l.behavior_top() + 160), None);
            assert!(l.taskbar_bottom() > l.behavior_top() as u32 + 700);
        }
    }
}
