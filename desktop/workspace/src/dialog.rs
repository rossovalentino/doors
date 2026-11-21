//! One app-modal choice, with shared paint/hit bounds and an input focus trap.
extern crate alloc;
use crate::explorer_layout::Bounds;
use alloc::vec::Vec;
use core::ops::Range;

/// Word wrap with measured widths and scalar-safe fallback for a long token.
/// Ranges address the original text; no token is silently removed or reordered.
pub fn lines(text: &str, width: u32, measure: impl Fn(&str) -> u32) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut offset = 0;
    for paragraph in text.split('\n') {
        let end = offset + paragraph.len();
        let mut start = offset;
        if paragraph.is_empty() {
            result.push(start..start);
        }
        while start < end {
            let mut fit = start;
            let mut space = None;
            for (index, ch) in text[start..end].char_indices() {
                let next = start + index + ch.len_utf8();
                if fit > start && measure(&text[start..next]) > width {
                    if ch == ' ' {
                        space = Some(fit);
                    }
                    break;
                }
                fit = next;
                if ch == ' ' && index > 0 {
                    space = Some(start + index);
                }
            }
            let stop = if fit < end { space.unwrap_or(fit) } else { fit };
            result.push(start..stop);
            start = stop;
            while start < end && text.as_bytes()[start] == b' ' {
                start += 1;
            }
        }
        offset = end + 1;
    }
    result
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub panel: Bounds,
    pub text: Bounds,
    pub buttons: [Bounds; 3],
}
impl Layout {
    pub fn new(width: u32, height: u32, count: usize) -> Self {
        let count = count.clamp(1, 3) as u32;
        let w = width.saturating_sub(16).min(448);
        let compact = w < 336;
        // Text size grows the text area (title and body lines) and the
        // button row; the panel width and padding stay, as on Windows.
        let body = 140;
        let bh_grown = crate::palette::text_grow(32, door_graphics_text::TextStyle::Regular14);
        let grown = crate::palette::text_len(body) - body + bh_grown - 32;
        let h = height.saturating_sub(16).min(232 + grown);
        let panel = Bounds {
            x: ((width - w) / 2) as i32,
            y: ((height - h) / 2) as i32,
            width: w,
            height: h,
        };
        let pad = (if compact { 8 } else { 24 }).min(w / 4).min(h / 8);
        let inner = w.saturating_sub(pad * 2);
        let gap = (if compact { 4 } else { 8 }).min(inner / 8);
        let bh = bh_grown.min(h.saturating_sub(pad * 2));
        let footer = bh + pad;
        let bw = inner.saturating_sub(gap * (count - 1)) / count;
        let mut buttons = [Bounds {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }; 3];
        for (i, button) in buttons.iter_mut().enumerate().take(count as usize) {
            *button = Bounds {
                x: panel.x + pad as i32 + i as i32 * (bw + gap) as i32,
                y: panel.y + h.saturating_sub(footer) as i32,
                width: bw,
                height: bh,
            };
        }
        Self {
            panel,
            text: Bounds {
                x: panel.x + pad as i32,
                y: panel.y + pad as i32,
                width: inner,
                height: h.saturating_sub(footer + pad + 12),
            },
            buttons,
        }
    }
}

#[derive(Default)]
pub struct Dialog {
    count: usize,
    focus: usize,
    keyboard_focus: bool,
}
impl Dialog {
    /// The final button is always the safe cancel/close action.
    pub fn show(&mut self, count: usize) {
        self.show_focused(count, 0);
    }
    /// Destructive confirmations choose their safe initial keyboard action.
    pub fn show_focused(&mut self, count: usize, focus: usize) {
        assert!((1..=3).contains(&count));
        assert!(focus < count);
        if self.open() {
            return;
        }
        self.count = count;
        self.focus = focus;
        self.keyboard_focus = true;
    }
    pub fn open(&self) -> bool {
        self.count != 0
    }
    pub fn count(&self) -> usize {
        self.count
    }
    pub fn focus(&self) -> Option<usize> {
        self.keyboard_focus.then_some(self.focus)
    }
    pub fn dismiss(&mut self) {
        self.count = 0;
    }
    pub fn target(&self, layout: &Layout, x: i32, y: i32) -> Option<usize> {
        (0..self.count).find(|i| layout.buttons[*i].contains(x, y))
    }
    pub fn pointer(&mut self, target: Option<usize>) {
        self.keyboard_focus = false;
        if let Some(i) = target.filter(|i| *i < self.count) {
            self.focus = i;
        }
    }
    /// Caller consumes every key while open, including non-dialog shortcuts.
    /// A result does not dismiss: failed actions must keep their modal guard.
    pub fn key(&mut self, scan: u32, shift: bool) -> Option<usize> {
        if !self.open() {
            return None;
        }
        self.keyboard_focus = true;
        match scan {
            1 => Some(self.count - 1),
            15 => {
                self.focus = (self.focus + if shift { self.count - 1 } else { 1 }) % self.count;
                None
            }
            28 | 0x11c | 57 => Some(self.focus),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measured_paragraphs_wrap_at_words_and_long_scalars_without_invalid_slices() {
        let t = "Save changes to desktop.txt?\nKeep your edits";
        let rows = lines(t, 15, |s| s.chars().count() as u32);
        assert_eq!(
            rows.iter().map(|r| &t[r.clone()]).collect::<Vec<_>>(),
            ["Save changes to", "desktop.txt?", "Keep your edits"]
        );
        let t = "é🪟é";
        let rows = lines(t, 1, |s| s.chars().count() as u32);
        assert_eq!(
            rows.iter().map(|r| &t[r.clone()]).collect::<Vec<_>>(),
            ["é", "🪟", "é"]
        );
        assert_eq!(lines("\n", 0, |_| 0), [0..0, 1..1]);
    }
    #[test]
    fn keyboard_cycles_traps_and_never_dismisses_failed_actions() {
        let mut d = Dialog::default();
        d.show(3);
        assert_eq!(d.key(15, true), None);
        assert_eq!(d.key(28, false), Some(2));
        assert!(d.open());
        d.show(1);
        assert_eq!(d.count(), 3);
        d.key(15, false);
        assert_eq!(d.key(57, false), Some(0));
        d.key(15, false);
        assert_eq!(d.key(0x11c, false), Some(1));
        assert_eq!(d.key(31, false), None);
        assert_eq!(d.key(1, false), Some(2));
        d.dismiss();
        assert_eq!(d.key(28, false), None);
        d.show(1);
        assert_eq!(d.key(1, false), Some(0));
    }
    #[test]
    fn all_buttons_stay_disjoint_inside_small_and_density_logical_surfaces() {
        for width in [0, 32, 128, 240, 320, 490, 1280, 3840] {
            for height in [0, 32, 80, 160, 210, 358, 720, 2160] {
                for count in 1..=3 {
                    let l = Layout::new(width, height, count);
                    for b in l.buttons.iter().take(count) {
                        assert!(b.x >= l.panel.x && b.y >= l.panel.y);
                        assert!(b.x as u32 + b.width <= width && b.y as u32 + b.height <= height);
                        for other in l.buttons.iter().take(count) {
                            if core::ptr::eq(b, other) {
                                continue;
                            }
                            assert!(!other.contains(b.x, b.y) || b.width == 0 || b.height == 0);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn pointer_hit_bounds_and_focus_have_one_source() {
        let l = Layout::new(490, 358, 3);
        let mut d = Dialog::default();
        d.show(3);
        for i in 0..3 {
            let b = l.buttons[i];
            assert_eq!(d.target(&l, b.x, b.y), Some(i));
            assert_ne!(d.target(&l, b.x + b.width as i32, b.y), Some(i));
        }
        d.pointer(Some(1));
        assert_eq!(d.focus(), None);
        assert_eq!(d.key(28, false), Some(1));
        assert_eq!(d.target(&l, -1, -1), None);
    }
    #[test]
    fn minimum_native_client_keeps_warning_and_all_three_labels_readable() {
        let l = Layout::new(240, 178, 3);
        assert!(l.text.width >= 208 && l.text.height >= 100);
        for b in l.buttons {
            assert!(b.width >= 66 && b.height == 32);
        }
    }
}
