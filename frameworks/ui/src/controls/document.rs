//! Text-field document model (master specification section 113), separate
//! from glyph caches: text, selection anchor and caret with affinity, IME
//! preedit kept apart from committed text, coalesced undo groups with
//! bounded history, validation and dirty state. Movement is grapheme- and
//! word-aware through Door Text segmentation; offsets are UTF-8 bytes on
//! character boundaries (Windows-facing bridges convert to UTF-16 units).
use alloc::{string::String, vec::Vec};
use core::ops::Range;
use door_graphics_text::{next_grapheme, next_word, prev_grapheme, prev_word};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    before: (usize, usize),
    after: (usize, usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Typing,
    Deleting,
}

#[derive(Clone, Debug)]
pub struct TextDocument {
    text: String,
    anchor: usize,
    caret: usize,
    pub affinity: Affinity,
    multiline: bool,
    max_bytes: usize,
    preedit: Option<(String, usize)>,
    undo: Vec<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
    group: Option<Group>,
    saved: u64,
    revision: u64,
}
impl TextDocument {
    pub const HISTORY_GROUPS: usize = 128;
    pub const HISTORY_BYTES: usize = 256 * 1024;
    pub fn new(text: &str, multiline: bool, max_bytes: usize) -> Self {
        let mut d = Self {
            text: String::new(),
            anchor: 0,
            caret: 0,
            affinity: Affinity::Downstream,
            multiline,
            max_bytes,
            preedit: None,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            saved: 0,
            revision: 0,
        };
        d.text = d.filter(text);
        d.text.truncate(floor(&d.text, max_bytes));
        d.anchor = d.text.len();
        d.caret = d.text.len();
        d
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn caret(&self) -> usize {
        self.caret
    }
    pub fn anchor(&self) -> usize {
        self.anchor
    }
    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret)
    }
    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }
    pub fn multiline(&self) -> bool {
        self.multiline
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Changed since the last `mark_saved`.
    pub fn dirty(&self) -> bool {
        self.revision != self.saved
    }
    pub fn mark_saved(&mut self) {
        self.saved = self.revision;
    }
    pub fn preedit(&self) -> Option<(&str, usize)> {
        self.preedit.as_ref().map(|(s, c)| (s.as_str(), *c))
    }
    /// Characters a field accepts: no controls except LF (multi-line) and TAB.
    fn filter(&self, s: &str) -> String {
        s.chars()
            .filter_map(|c| match c {
                '\r' => None,
                '\n' if self.multiline => Some('\n'),
                '\n' => Some(' '),
                '\t' => Some('\t'),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect()
    }
    /// Replace everything (not undoable; e.g. loading a document).
    pub fn set_text(&mut self, text: &str) {
        self.text = self.filter(text);
        self.text.truncate(floor(&self.text, self.max_bytes));
        self.anchor = self.text.len();
        self.caret = self.text.len();
        self.undo.clear();
        self.redo.clear();
        self.group = None;
        self.preedit = None;
        self.revision += 1;
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
        self.group = None;
    }
    pub fn set_selection(&mut self, anchor: usize, caret: usize) -> bool {
        let ok = |i: usize| i <= self.text.len() && self.text.is_char_boundary(i);
        if !ok(anchor) || !ok(caret) {
            return false;
        }
        self.anchor = anchor;
        self.caret = caret;
        self.group = None;
        true
    }
    /// Move the caret to `at` (snapped back to a character boundary).
    pub fn place(&mut self, at: usize, extend: bool) {
        let at = floor(&self.text, at.min(self.text.len()));
        self.caret = at;
        if !extend {
            self.anchor = at;
        }
        self.group = None;
    }
    fn apply(&mut self, at: usize, remove: usize, insert: &str, group: Option<Group>) -> bool {
        let removed: String = self.text[at..at + remove].into();
        if removed.is_empty() && insert.is_empty() {
            return false;
        }
        let before = (self.anchor, self.caret);
        self.text.replace_range(at..at + remove, insert);
        let caret = at + insert.len();
        self.anchor = caret;
        self.caret = caret;
        self.affinity = Affinity::Downstream;
        let edit = Edit {
            at,
            removed,
            inserted: insert.into(),
            before,
            after: (caret, caret),
        };
        let coalesce = group.is_some()
            && self.group == group
            && !self.undo.is_empty()
            && !insert.starts_with(' ');
        if coalesce {
            self.undo.last_mut().unwrap().push(edit);
        } else {
            self.undo.push(alloc::vec![edit]);
        }
        self.group = group;
        self.redo.clear();
        self.trim_history();
        self.revision += 1;
        true
    }
    fn trim_history(&mut self) {
        let bytes = |h: &Vec<Vec<Edit>>| {
            h.iter()
                .flatten()
                .map(|e| e.removed.len() + e.inserted.len())
                .sum::<usize>()
        };
        while self.undo.len() > Self::HISTORY_GROUPS
            || (self.undo.len() > 1 && bytes(&self.undo) > Self::HISTORY_BYTES)
        {
            self.undo.remove(0);
        }
    }
    /// Insert (typing or paste) replacing the selection. Returns false when
    /// nothing fits.
    pub fn insert(&mut self, s: &str) -> bool {
        self.insert_as(s, Some(Group::Typing))
    }
    /// Insert as its own undo group (paste, set value).
    pub fn paste(&mut self, s: &str) -> bool {
        self.insert_as(s, None)
    }
    fn insert_as(&mut self, s: &str, group: Option<Group>) -> bool {
        let s = self.filter(s);
        let sel = self.selection();
        let room = self.max_bytes.saturating_sub(self.text.len() - sel.len());
        let s = &s[..floor(&s, room)];
        if s.is_empty() && sel.is_empty() {
            return false;
        }
        self.apply(sel.start, sel.len(), s, group)
    }
    pub fn backspace(&mut self, word: bool) -> bool {
        let sel = self.selection();
        if !sel.is_empty() {
            return self.apply(sel.start, sel.len(), "", Some(Group::Deleting));
        }
        if self.caret == 0 {
            return false;
        }
        let start = if word {
            prev_word(&self.text, self.caret)
        } else {
            prev_grapheme(&self.text, self.caret)
        };
        self.apply(start, self.caret - start, "", Some(Group::Deleting))
    }
    pub fn delete(&mut self, word: bool) -> bool {
        let sel = self.selection();
        if !sel.is_empty() {
            return self.apply(sel.start, sel.len(), "", Some(Group::Deleting));
        }
        if self.caret >= self.text.len() {
            return false;
        }
        let end = if word {
            next_word(&self.text, self.caret)
        } else {
            next_grapheme(&self.text, self.caret)
        };
        self.apply(self.caret, end - self.caret, "", Some(Group::Deleting))
    }
    /// Left/Right: by grapheme or word; without `extend` a selection
    /// collapses to its edge first.
    pub fn horizontal(&mut self, right: bool, extend: bool, word: bool) {
        let sel = self.selection();
        let at = if !extend && !sel.is_empty() && !word {
            if right { sel.end } else { sel.start }
        } else if right {
            if word {
                next_word(&self.text, self.caret)
            } else {
                next_grapheme(&self.text, self.caret)
            }
        } else if word {
            prev_word(&self.text, self.caret)
        } else {
            prev_grapheme(&self.text, self.caret)
        };
        self.place(at, extend);
    }
    /// Start of the line (logical) containing `at`.
    pub fn line_start(&self, at: usize) -> usize {
        self.text[..at].rfind('\n').map_or(0, |i| i + 1)
    }
    pub fn line_end(&self, at: usize) -> usize {
        self.text[at..]
            .find('\n')
            .map_or(self.text.len(), |i| at + i)
    }
    pub fn undo(&mut self) -> bool {
        let Some(group) = self.undo.pop() else {
            return false;
        };
        for e in group.iter().rev() {
            self.text
                .replace_range(e.at..e.at + e.inserted.len(), &e.removed);
        }
        let first = &group[0];
        (self.anchor, self.caret) = first.before;
        self.redo.push(group);
        self.group = None;
        self.revision += 1;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for e in &group {
            self.text
                .replace_range(e.at..e.at + e.removed.len(), &e.inserted);
        }
        (self.anchor, self.caret) = group.last().unwrap().after;
        self.undo.push(group);
        self.group = None;
        self.revision += 1;
        true
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// IME preedit: shown at the caret, never part of the text.
    pub fn set_preedit(&mut self, text: &str, caret: usize) {
        if text.is_empty() {
            self.preedit = None;
        } else {
            self.preedit = Some((text.into(), floor(text, caret.min(text.len()))));
        }
    }
    pub fn commit_preedit(&mut self, text: &str) -> bool {
        self.preedit = None;
        self.paste(text)
    }
    pub fn cancel_preedit(&mut self) {
        self.preedit = None;
    }
}
/// Largest character boundary at or below `at`.
pub fn floor(s: &str, at: usize) -> usize {
    let mut at = at.min(s.len());
    while !s.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_selection_and_undo_groups() {
        let mut d = TextDocument::new("", false, 64);
        for c in ["H", "e", "l", "l", "o"] {
            d.insert(c);
        }
        d.insert(" world");
        assert_eq!(d.text(), "Hello world");
        assert!(d.undo());
        assert_eq!(
            d.text(),
            "Hello",
            "typing coalesces; a space starts a group"
        );
        assert!(d.undo());
        assert_eq!(d.text(), "");
        assert!(d.redo() && d.redo());
        assert_eq!(d.text(), "Hello world");
        d.horizontal(false, false, true);
        assert_eq!(d.caret(), 6);
        d.horizontal(true, true, false);
        assert_eq!(d.selected(), "w");
        d.insert("W");
        assert_eq!(d.text(), "Hello World");
        d.select_all();
        d.backspace(false);
        assert_eq!(d.text(), "");
        assert!(d.dirty());
        d.mark_saved();
        assert!(!d.dirty());
    }
    #[test]
    fn graphemes_limits_filtering_and_preedit() {
        let mut d = TextDocument::new("e\u{301}x", false, 16);
        d.place(d.text().len(), false);
        d.horizontal(false, false, false);
        d.horizontal(false, false, false);
        assert_eq!(d.caret(), 0, "the combining sequence is one step");
        d.place(3, false);
        d.backspace(false);
        assert_eq!(d.text(), "x", "backspace removes the whole cluster");
        assert!(d.paste("a\nb\u{7}c"));
        assert_eq!(
            d.text(),
            "a bcx",
            "single-line fields map newlines, drop controls"
        );
        d.select_all();
        d.paste(&"z".repeat(40));
        assert_eq!(d.text().len(), 16);
        d.set_preedit("\u{306b}", 3);
        assert_eq!(d.text().len(), 16, "preedit is not committed text");
        d.cancel_preedit();
        assert!(d.preedit().is_none());
        let mut m = TextDocument::new("one\ntwo", true, 1 << 20);
        assert_eq!(m.line_start(5), 4);
        assert_eq!(m.line_end(1), 3);
        m.commit_preedit("!");
        assert_eq!(m.text(), "one\ntwo!");
        assert!(!m.set_selection(0, 100));
    }
}
