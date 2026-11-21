//! Bounded single-line editing with byte-safe cursor and selection boundaries.
extern crate alloc;
use alloc::string::String;
use core::ops::Range;

#[derive(Default)]
pub struct LineEdit {
    text: String,
    cursor: usize,
    anchor: usize,
}
impl LineEdit {
    pub fn set(&mut self, text: String, selected: bool) {
        self.cursor = text.len();
        self.anchor = if selected { 0 } else { self.cursor };
        self.text = text;
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    /// A copy showing one bullet per character (password boxes), with the
    /// cursor and selection on the same characters.
    pub fn masked(&self) -> Self {
        const BULLET: char = '\u{25CF}';
        let at = |byte: usize| self.text[..byte].chars().count() * BULLET.len_utf8();
        Self {
            text: core::iter::repeat_n(BULLET, self.text.chars().count()).collect(),
            cursor: at(self.cursor),
            anchor: at(self.anchor),
        }
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }
    pub fn place(&mut self, cursor: usize, extend: bool) {
        if cursor <= self.text.len() && self.text.is_char_boundary(cursor) {
            self.cursor = cursor;
            if !extend {
                self.anchor = cursor;
            }
        }
    }
    fn previous(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .last()
            .map_or(0, |(index, _)| index)
    }
    fn next(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }
    fn remove_selection(&mut self) -> bool {
        let selected = self.selection();
        if selected.is_empty() {
            return false;
        }
        self.text.replace_range(selected.clone(), "");
        self.cursor = selected.start;
        self.anchor = self.cursor;
        true
    }
    pub fn insert(&mut self, c: char) {
        self.insert_bounded(c, 192);
    }
    pub fn insert_bounded(&mut self, c: char, limit: usize) {
        if c < ' ' || c == '\u{7f}' {
            return;
        }
        let selected = self.selection();
        if self.text.len() - selected.len() + c.len_utf8() > limit {
            return;
        }
        self.remove_selection();
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.anchor = self.cursor;
    }
    pub fn backspace(&mut self) {
        if self.remove_selection() || self.cursor == 0 {
            return;
        }
        let start = self.previous(self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.place(start, false);
    }
    pub fn delete(&mut self) {
        if self.remove_selection() {
            return;
        }
        self.text
            .replace_range(self.cursor..self.next(self.cursor), "");
    }
    pub fn horizontal(&mut self, right: bool, extend: bool, word: bool) {
        let selected = self.selection();
        let mut at = if !extend && !selected.is_empty() {
            if right { selected.end } else { selected.start }
        } else if right {
            self.next(self.cursor)
        } else {
            self.previous(self.cursor)
        };
        if word {
            while at != 0 && at != self.text.len() {
                if self.text[at..].starts_with([' ', '/', '\\']) {
                    break;
                }
                at = if right {
                    self.next(at)
                } else {
                    self.previous(at)
                };
            }
        }
        self.place(at, extend);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_cursor_deletion_and_shift_selection_keep_text() {
        let mut e = LineEdit::default();
        e.set("C:\\Users\\1001\\Downloads".into(), true);
        e.insert('x');
        assert_eq!(e.text(), "x");
        e.set("Documents".into(), false);
        e.place(0, false);
        e.horizontal(true, false, false);
        e.delete();
        e.insert('O');
        assert_eq!(e.text(), "DOcuments");
        e.horizontal(false, true, false);
        e.insert('o');
        assert_eq!(e.text(), "Documents");
        e.place(e.text().len(), false);
        e.backspace();
        assert_eq!(e.text(), "Document");
    }
    #[test]
    fn unicode_boundaries_and_limit_are_enforced_before_replacing() {
        let mut e = LineEdit::default();
        e.set("a文b".into(), false);
        e.horizontal(false, false, false);
        e.backspace();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.cursor(), 1);
        e.set("a".repeat(192), false);
        e.insert('b');
        assert_eq!(e.text().len(), 192);
        e.select_all();
        e.insert('文');
        assert_eq!(e.text(), "文");
        e.place(1, false);
        assert_eq!(e.cursor(), 3);
    }
}
