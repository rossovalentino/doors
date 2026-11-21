//! Shared command popup geometry. Commands are the same IDs used by each app's
//! toolbar, so pointer and keyboard activation have identical service effects.

#[derive(Default)]
pub struct Menu {
    origin: Option<(i32, i32)>,
    selected: usize,
    rows: usize,
    offset: usize,
    header: u32,
    enabled: u64,
}
impl Menu {
    /// Item row height: 32 px grown with its 12 px label's line box at the
    /// applied text size (paint, hit testing and semantics share it).
    pub fn row_height() -> u32 {
        crate::palette::text_grow(32, door_graphics_text::TextStyle::Regular12)
    }
    pub fn open(&self) -> bool {
        self.origin.is_some()
    }
    pub fn show(&mut self, x: i32, y: i32, w: u32, h: u32, count: usize) {
        self.show_header(x, y, w, h, count, 0);
    }
    pub fn show_header(&mut self, x: i32, y: i32, w: u32, h: u32, count: usize, header: u32) {
        self.header = header.min(h.saturating_sub(40));
        self.enabled = u64::MAX;
        let row = Self::row_height();
        self.rows = ((h.saturating_sub(8 + self.header) / row) as usize)
            .max(1)
            .min(count);
        self.offset = 0;
        self.origin = Some((
            x.clamp(0, (w as i32 - 224).max(0)),
            y.clamp(
                0,
                (h as i32 - self.rows as i32 * row as i32 - 8 - self.header as i32).max(0),
            ),
        ));
        self.selected = 0;
    }
    pub fn set_enabled(&mut self, mask: u64, count: usize) {
        self.enabled = mask;
        if !self.enabled(self.selected) {
            self.selected = (0..count).find(|i| self.enabled(*i)).unwrap_or(0);
            self.offset = self.selected.saturating_sub(self.rows.saturating_sub(1));
        }
    }
    pub fn enabled(&self, index: usize) -> bool {
        index < 64 && self.enabled & (1u64 << index) != 0
    }
    pub fn header(&self) -> u32 {
        self.header
    }
    pub fn dismiss(&mut self) {
        self.origin = None;
    }
    pub fn target(&self, x: i32, y: i32, count: usize) -> Option<u32> {
        let (left, top) = self.origin?;
        let row = Self::row_height() as i32;
        Some(
            if (left..left + 224).contains(&x) && (top..top + 4 + self.header as i32).contains(&y) {
                998
            } else if (left + 4..left + 220).contains(&x)
                && (top + 4 + self.header as i32
                    ..top + 4 + self.header as i32 + count.min(self.rows) as i32 * row)
                    .contains(&y)
            {
                1000 + self.offset as u32 + ((y - top - 4 - self.header as i32) / row) as u32
            } else {
                999
            },
        )
    }
    pub fn click(&mut self, x: i32, y: i32, count: usize) -> Option<usize> {
        let id = self.target(x, y, count)?;
        if id == 998
            || id
                .checked_sub(1000)
                .is_some_and(|i| !self.enabled(i as usize))
        {
            return None;
        }
        self.dismiss();
        id.checked_sub(1000).map(|n| n as usize)
    }
    pub fn key(&mut self, scan: u32, count: usize) -> Option<usize> {
        if count == 0 {
            self.dismiss();
            return None;
        }
        match scan {
            0x148 | 0x150 => {
                for _ in 0..count {
                    self.selected = if scan == 0x148 {
                        (self.selected + count - 1) % count
                    } else {
                        (self.selected + 1) % count
                    };
                    if self.enabled(self.selected) {
                        break;
                    }
                }
            }
            1 => self.dismiss(),
            28 | 0x11c => {
                if !self.enabled(self.selected) {
                    return None;
                }
                let id = self.selected;
                self.dismiss();
                return Some(id);
            }
            _ => (),
        };
        if self.selected < self.offset {
            self.offset = self.selected;
        }
        if self.selected >= self.offset + self.rows {
            self.offset = self.selected + 1 - self.rows;
        }
        None
    }
    pub fn origin(&self) -> Option<(i32, i32)> {
        self.origin
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn hover(&mut self, x: i32, y: i32, count: usize) -> bool {
        if let Some(id) = self.target(x, y, count).and_then(|v| v.checked_sub(1000)) {
            if !self.enabled(id as usize) {
                return false;
            }
            let changed = self.selected != id as usize;
            self.selected = id as usize;
            return changed;
        }
        false
    }
    pub fn viewport(&self) -> (usize, usize) {
        (self.offset, self.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_actions_and_inline_header_never_dispatch_or_dismiss() {
        let mut m = Menu::default();
        m.show_header(400, 400, 640, 480, 10, 40);
        m.set_enabled(0b1111 << 6, 10);
        assert_eq!(m.selected(), 6);
        let (x, y) = m.origin().unwrap();
        assert!(y + m.viewport().1 as i32 * 32 + 48 <= 480);
        assert_eq!(m.click(x + 10, y + 20, 10), None);
        assert!(m.open());
        m.key(0x148, 10);
        assert_eq!(m.selected(), 9);
        m.key(0x150, 10);
        assert_eq!(m.selected(), 6);
        assert_eq!(m.key(28, 10), Some(6));
        m.show_header(0, 0, 640, 480, 10, 0);
        m.set_enabled(0b1111 << 6, 10);
        assert_eq!(m.click(10, 12, 10), None);
        assert!(m.open());
    }
    #[test]
    fn a_small_viewport_scrolls_keyboard_selection_into_view() {
        let mut m = Menu::default();
        m.show(200, 80, 240, 88, 4);
        assert_eq!(m.viewport(), (0, 2));
        m.key(0x148, 4);
        assert_eq!(m.viewport(), (2, 2));
        let (x, y) = m.origin().unwrap();
        assert_eq!(m.click(x + 12, y + 44, 4), Some(3));
    }
    #[test]
    fn outside_release_dismisses_without_a_command() {
        let mut m = Menu::default();
        m.show(20, 20, 490, 358, 2);
        assert_eq!(m.click(0, 0, 2), None);
        assert!(!m.open());
    }
    #[test]
    fn keyboard_wraps_and_escape_cancels() {
        let mut m = Menu::default();
        m.show(20, 20, 490, 358, 2);
        m.key(0x148, 2);
        assert_eq!(m.key(28, 2), Some(1));
        assert!(!m.open());
        m.show(20, 20, 490, 358, 2);
        m.key(1, 2);
        assert!(!m.open());
    }
    #[test]
    fn popup_stays_inside_default_output_and_only_rows_invoke() {
        let mut m = Menu::default();
        m.show(489, 357, 490, 358, 2);
        let (x, y) = m.origin().unwrap();
        assert!(x + 224 <= 490 && y + 72 <= 358);
        assert_eq!(m.click(x + 16, y + 12, 2), Some(0));
    }
}
