//! The EDIT control's text model: UTF-16 buffer, anchor/caret selection,
//! CR-LF line structure, the WM_CHAR/WM_KEYDOWN editing rules, single-level
//! undo, the modify flag, the text limit and the EN_CHANGE/EN_UPDATE/
//! EN_MAXTEXT notifications each edit produces (replacing a selection by
//! typing is two changes, as measured on Windows 11). Lines are hard
//! (CR-LF) lines; soft word-wrap only affects painting.
use crate::keyboard::vk;
use crate::msg::es;
use alloc::vec::Vec;

const CR: u16 = 13;
const LF: u16 = 10;
pub const DEFAULT_LIMIT: usize = 30_000;
pub const MAX_LIMIT: usize = 0x7fff_fffe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notify {
    Update,
    Change,
    MaxText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Cut,
    Paste,
}

/// Result of one input message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub notify: Vec<Notify>,
    pub clipboard: Option<Clip>,
    pub moved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    text: Vec<u16>,
    anchor: usize,
    caret: usize,
    pub style: u32,
    pub limit: usize,
    pub modified: bool,
    undo: Option<(Vec<u16>, usize, usize)>,
    /// First visible line (EM_GETFIRSTVISIBLELINE / scrolling).
    pub first_line: usize,
    pub password_char: u16,
}

/// Soft word-wrap of one hard line for painting (a multiline edit without
/// ES_AUTOHSCROLL): `advances` are the characters' widths, `width` the
/// room. Each row breaks after the last space that fits, or inside a word
/// longer than a row; a row's trailing spaces may overhang, as on Windows.
/// The rows as (start, end) offsets into the line; an empty line is one
/// empty row.
pub fn wrap_rows(text: &[u16], advances: &[i32], width: i32) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut used = 0;
        let mut end = start;
        let mut last_break = None;
        while end < text.len() {
            let w = advances.get(end).copied().unwrap_or(0);
            if text[end] == u16::from(b' ') {
                // Spaces never start a row and may hang past the edge.
                end += 1;
                used += w;
                last_break = Some(end);
                continue;
            }
            if used + w > width && end > start {
                break;
            }
            used += w;
            end += 1;
        }
        if end < text.len()
            && let Some(b) = last_break
        {
            end = b;
        }
        rows.push((start, end));
        start = end;
    }
    if rows.is_empty() {
        rows.push((0, 0));
    }
    rows
}

impl Edit {
    pub fn new(style: u32) -> Self {
        Self {
            text: Vec::new(),
            anchor: 0,
            caret: 0,
            style,
            limit: DEFAULT_LIMIT,
            modified: false,
            undo: None,
            first_line: 0,
            password_char: if style & es::PASSWORD != 0 {
                u16::from(b'*')
            } else {
                0
            },
        }
    }
    pub fn multiline(&self) -> bool {
        self.style & es::MULTILINE != 0
    }
    pub fn text(&self) -> &[u16] {
        &self.text
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn caret(&self) -> usize {
        self.caret
    }
    /// EM_GETSEL: ordered (start, end).
    pub fn selection(&self) -> (usize, usize) {
        (self.anchor.min(self.caret), self.anchor.max(self.caret))
    }
    /// EM_SETSEL: -1 start removes the selection; -1 end selects to the end.
    pub fn set_selection(&mut self, start: isize, end: isize) {
        let n = self.text.len();
        if start < 0 {
            self.anchor = self.caret;
            return;
        }
        let s = (start as usize).min(n);
        let e = if end < 0 { n } else { (end as usize).min(n) };
        self.anchor = self.snap(s);
        self.caret = self.snap(e);
    }
    /// Never place the caret between CR and LF.
    fn snap(&self, at: usize) -> usize {
        if at > 0 && at < self.text.len() && self.text[at - 1] == CR && self.text[at] == LF {
            at - 1
        } else {
            at
        }
    }
    /// WM_SETTEXT: resets selection, undo and the modify flag.
    pub fn set_text(&mut self, text: &[u16]) -> Outcome {
        let filtered = self.filter(text);
        self.text = filtered;
        if self.text.len() > self.limit {
            self.text.truncate(self.limit);
        }
        self.anchor = 0;
        self.caret = 0;
        self.undo = None;
        self.modified = false;
        self.first_line = 0;
        let mut out = Outcome::default();
        // Multiline controls do not notify for WM_SETTEXT.
        if !self.multiline() {
            out.notify.extend([Notify::Update, Notify::Change]);
        }
        out
    }
    fn filter(&self, text: &[u16]) -> Vec<u16> {
        let mut out = Vec::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            let mut u = text[i];
            if !self.multiline() && (u == CR || u == LF) {
                // Single-line text stops at the first line break.
                break;
            }
            if self.style & es::UPPERCASE != 0 && (u16::from(b'a')..=u16::from(b'z')).contains(&u) {
                u -= 32;
            } else if self.style & es::LOWERCASE != 0
                && (u16::from(b'A')..=u16::from(b'Z')).contains(&u)
            {
                u += 32;
            }
            out.push(u);
            i += 1;
        }
        out
    }
    /// EM_REPLACESEL / typing: replace the selection with `text`.
    pub fn replace_selection(&mut self, text: &[u16], can_undo: bool) -> Outcome {
        let mut out = Outcome::default();
        let insert = self.filter(text);
        let (s, e) = self.selection();
        let room = self.limit.saturating_sub(self.text.len() - (e - s));
        let take = insert.len().min(room);
        if take < insert.len() {
            out.notify.push(Notify::MaxText);
        }
        if e == s && take == 0 {
            return out;
        }
        if can_undo {
            self.undo = Some((self.text.clone(), self.anchor, self.caret));
        }
        self.text.splice(s..e, insert[..take].iter().copied());
        self.anchor = s + take;
        self.caret = s + take;
        self.modified = true;
        out.notify.extend([Notify::Update, Notify::Change]);
        out
    }
    fn delete_range(&mut self, s: usize, e: usize) -> Outcome {
        let mut out = Outcome::default();
        if s >= e {
            return out;
        }
        self.undo = Some((self.text.clone(), self.anchor, self.caret));
        self.text.drain(s..e);
        self.anchor = s;
        self.caret = s;
        self.modified = true;
        out.notify.extend([Notify::Update, Notify::Change]);
        out
    }
    fn prev(&self, at: usize) -> usize {
        if at >= 2 && self.text[at - 2] == CR && self.text[at - 1] == LF {
            at - 2
        } else {
            at.saturating_sub(1)
        }
    }
    fn next(&self, at: usize) -> usize {
        if at + 1 < self.text.len() && self.text[at] == CR && self.text[at + 1] == LF {
            at + 2
        } else {
            (at + 1).min(self.text.len())
        }
    }
    fn word_class(u: u16) -> u8 {
        match u {
            32 | 9 => 0,
            CR | LF => 1,
            _ if u < 128 && !(u as u8).is_ascii_alphanumeric() && u != u16::from(b'_') => 2,
            _ => 3,
        }
    }
    fn next_word(&self, mut at: usize) -> usize {
        let n = self.text.len();
        if at >= n {
            return n;
        }
        let class = Self::word_class(self.text[at]);
        while at < n && Self::word_class(self.text[at]) == class && class != 1 {
            at += 1;
        }
        if class == 1 {
            return self.next(at);
        }
        while at < n && Self::word_class(self.text[at]) == 0 {
            at += 1;
        }
        at
    }
    fn prev_word(&self, mut at: usize) -> usize {
        while at > 0 && Self::word_class(self.text[at - 1]) == 0 {
            at -= 1;
        }
        if at == 0 {
            return 0;
        }
        let class = Self::word_class(self.text[at - 1]);
        if class == 1 {
            return self.prev(at);
        }
        while at > 0 && Self::word_class(self.text[at - 1]) == class {
            at -= 1;
        }
        at
    }
    /// (start, end-without-break) of every hard line.
    pub fn lines(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < self.text.len() {
            if self.text[i] == CR && i + 1 < self.text.len() && self.text[i + 1] == LF {
                out.push((start, i));
                i += 2;
                start = i;
            } else {
                i += 1;
            }
        }
        out.push((start, self.text.len()));
        out
    }
    pub fn line_count(&self) -> usize {
        if self.multiline() {
            self.lines().len()
        } else {
            1
        }
    }
    /// EM_LINEFROMCHAR (`usize::MAX` selects the caret line).
    pub fn line_from_char(&self, at: usize) -> usize {
        let at = if at == usize::MAX {
            self.selection().0
        } else {
            at.min(self.text.len())
        };
        self.lines()
            .iter()
            .rposition(|&(s, _)| s <= at)
            .unwrap_or(0)
    }
    /// EM_LINEINDEX: None for a line past the end.
    pub fn line_index(&self, line: Option<usize>) -> Option<usize> {
        let lines = self.lines();
        let line = line.unwrap_or_else(|| self.line_from_char(self.caret));
        lines.get(line).map(|l| l.0)
    }
    /// EM_LINELENGTH for the line containing character `at`.
    pub fn line_length(&self, at: Option<usize>) -> usize {
        let lines = self.lines();
        let line = self.line_from_char(at.unwrap_or(self.caret));
        lines.get(line).map_or(0, |l| l.1 - l.0)
    }
    pub fn line_text(&self, line: usize) -> Option<&[u16]> {
        self.lines().get(line).map(|&(s, e)| &self.text[s..e])
    }
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }
    /// EM_UNDO: swaps the buffer with the undo state (undo is reversible).
    pub fn undo(&mut self) -> Outcome {
        let mut out = Outcome::default();
        if let Some((text, a, c)) = self.undo.take() {
            let current = (
                core::mem::replace(&mut self.text, text),
                self.anchor,
                self.caret,
            );
            self.anchor = a.min(self.text.len());
            self.caret = c.min(self.text.len());
            self.undo = Some(current);
            self.modified = true;
            out.notify.extend([Notify::Update, Notify::Change]);
        }
        out
    }
    pub fn empty_undo(&mut self) {
        self.undo = None;
    }
    fn readonly(&self) -> bool {
        self.style & es::READONLY != 0
    }
    /// WM_CHAR.
    pub fn char(&mut self, c: u16) -> Outcome {
        match c {
            1 => {
                // Ctrl+A selects everything.
                self.anchor = 0;
                self.caret = self.text.len();
                Outcome {
                    moved: true,
                    ..Outcome::default()
                }
            }
            3 => Outcome {
                clipboard: Some(Clip::Copy),
                ..Outcome::default()
            },
            0x16 if !self.readonly() => Outcome {
                clipboard: Some(Clip::Paste),
                ..Outcome::default()
            },
            0x18 if !self.readonly() => Outcome {
                clipboard: Some(Clip::Cut),
                ..Outcome::default()
            },
            0x1a if !self.readonly() => self.undo(),
            _ if self.readonly() => Outcome::default(),
            8 => {
                let (s, e) = self.selection();
                if s != e {
                    self.delete_range(s, e)
                } else {
                    let p = self.prev(s);
                    self.delete_range(p, s)
                }
            }
            CR | LF => {
                if self.multiline() {
                    self.typed(&[CR, LF])
                } else {
                    Outcome::default()
                }
            }
            9 if !self.multiline() => Outcome::default(),
            c if c < 32 && c != 9 => Outcome::default(),
            0x7f => Outcome::default(),
            c => {
                if self.style & es::NUMBER != 0 && !(u16::from(b'0')..=u16::from(b'9')).contains(&c)
                {
                    return Outcome::default();
                }
                self.typed(&[c])
            }
        }
    }
    /// Typing over a selection deletes it first (its own change), then
    /// inserts (a second change), as Windows does.
    fn typed(&mut self, units: &[u16]) -> Outcome {
        let (s, e) = self.selection();
        let mut out = Outcome::default();
        if s != e {
            out = self.delete_range(s, e);
        }
        let more = self.replace_selection(units, s == e);
        out.notify.extend(more.notify);
        out
    }
    /// WM_KEYDOWN.
    pub fn key(&mut self, key: u8, shift: bool, ctrl: bool) -> Outcome {
        let mut out = Outcome::default();
        let (s, e) = self.selection();
        let target = match key {
            vk::LEFT => Some(if !shift && s != e {
                s
            } else if ctrl {
                self.prev_word(self.caret)
            } else {
                self.prev(self.caret)
            }),
            vk::RIGHT => Some(if !shift && s != e {
                e
            } else if ctrl {
                self.next_word(self.caret)
            } else {
                self.next(self.caret)
            }),
            vk::HOME => Some(if ctrl {
                0
            } else {
                self.lines()[self.line_from_char(self.caret)].0
            }),
            vk::END => Some(if ctrl {
                self.text.len()
            } else {
                self.lines()[self.line_from_char(self.caret)].1
            }),
            vk::UP | vk::DOWN | vk::PRIOR | vk::NEXT if self.multiline() => {
                let lines = self.lines();
                let line = self.line_from_char(self.caret);
                let column = self.caret - lines[line].0;
                let step = if matches!(key, vk::PRIOR | vk::NEXT) {
                    10
                } else {
                    1
                };
                let next = if matches!(key, vk::UP | vk::PRIOR) {
                    line.saturating_sub(step)
                } else {
                    (line + step).min(lines.len() - 1)
                };
                let (ls, le) = lines[next];
                Some((ls + column).min(le))
            }
            vk::DELETE => {
                if self.readonly() {
                    return out;
                }
                if shift {
                    out.clipboard = Some(Clip::Cut);
                    return out;
                }
                return if s != e {
                    self.delete_range(s, e)
                } else {
                    let n = self.next(s);
                    self.delete_range(s, n)
                };
            }
            vk::INSERT => {
                out.clipboard = if shift && !self.readonly() {
                    Some(Clip::Paste)
                } else if ctrl {
                    Some(Clip::Copy)
                } else {
                    None
                };
                return out;
            }
            _ => None,
        };
        if let Some(t) = target {
            self.caret = t;
            if !shift {
                self.anchor = t;
            }
            out.moved = true;
        }
        out
    }
    /// Text for a copy: the selection.
    pub fn selected_text(&self) -> &[u16] {
        let (s, e) = self.selection();
        &self.text[s..e]
    }
    /// Characters as displayed (password masking).
    pub fn display_char(&self, u: u16) -> u16 {
        if self.password_char != 0 {
            self.password_char
        } else {
            u
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    #[test]
    fn soft_wrap_breaks_after_spaces_and_inside_long_words() {
        let rows = |s: &str, width: i32| {
            let t = w(s);
            wrap_rows(&t, &alloc::vec![10; t.len()], width)
        };
        // 10 px per character, 50 px rows.
        assert_eq!(rows("one two three", 50), [(0, 4), (4, 8), (8, 13)]);
        // A row's trailing space hangs past the edge.
        assert_eq!(rows("abcd efgh", 40), [(0, 5), (5, 9)]);
        // A word longer than a row breaks inside it.
        assert_eq!(rows("abcdefghij", 40), [(0, 4), (4, 8), (8, 10)]);
        assert_eq!(rows("", 40), [(0, 0)]);
        assert_eq!(rows("fits", 40), [(0, 4)]);
    }
    fn changes(o: &Outcome) -> usize {
        o.notify.iter().filter(|n| **n == Notify::Change).count()
    }
    /// The F1 input sequence and its Windows 11 EN_CHANGE count (14).
    #[test]
    fn fixture_sequence_matches_windows() {
        let mut e = Edit::new(es::MULTILINE | es::AUTOVSCROLL | es::WANTRETURN);
        let mut n = 0;
        for c in w("Doors\r\u{3bb}\u{9580}") {
            n += changes(&e.char(c));
        }
        assert_eq!(n, 8);
        n += changes(&e.key(vk::LEFT, false, false));
        n += changes(&e.char(u16::from(b'x')));
        n += changes(&e.key(vk::HOME, false, false));
        n += changes(&e.key(vk::DELETE, false, false));
        n += changes(&e.key(vk::END, false, false));
        n += changes(&e.char(8));
        n += changes(&e.char(0xe9));
        assert_eq!(n, 12);
        assert_eq!(e.text(), w("Doors\r\nx\u{e9}"));
        assert_eq!(e.line_count(), 2);
        assert_eq!(e.line_index(Some(1)), Some(7));
        e.set_selection(7, 8);
        assert_eq!(e.selection(), (7, 8));
        n += changes(&e.char(u16::from(b'Z')));
        assert_eq!(n, 14);
        assert_eq!(e.text(), w("Doors\r\nZ\u{e9}"));
        assert!(e.modified);
    }
    #[test]
    fn crlf_is_one_unit_and_selection_extends() {
        let mut e = Edit::new(es::MULTILINE);
        e.set_text(&w("ab\r\ncd"));
        e.set_selection(4, 4);
        e.key(vk::LEFT, false, false);
        assert_eq!(e.caret(), 2, "jumps over CR-LF");
        e.char(8);
        assert_eq!(e.text(), w("a\r\ncd"));
        e.set_selection(1, 1);
        e.key(vk::DELETE, false, false);
        assert_eq!(e.text(), w("acd"), "CR-LF deleted together");
        e.set_selection(0, 0);
        e.key(vk::RIGHT, true, false);
        e.key(vk::RIGHT, true, false);
        assert_eq!(e.selection(), (0, 2));
        assert_eq!(e.selected_text(), w("ac"));
        e.key(vk::END, false, true);
        assert_eq!(e.selection(), (3, 3));
        e.set_selection(3, 1);
        assert_eq!(e.selection(), (1, 3), "ordered");
        e.set_selection(-1, 0);
        assert_eq!(e.selection(), (1, 1));
        e.set_selection(0, -1);
        assert_eq!(e.selection(), (0, 3));
        assert_eq!(e.char(3).clipboard, Some(Clip::Copy));
        assert!(e.char(1).moved);
    }
    #[test]
    fn lines_words_undo_and_limits() {
        let mut e = Edit::new(es::MULTILINE);
        e.set_text(&w("one two\r\nthree\r\n"));
        assert_eq!(e.line_count(), 3);
        assert_eq!(e.line_from_char(10), 1);
        assert_eq!(e.line_length(Some(10)), 5);
        assert_eq!(e.line_text(2), Some(&[][..]));
        assert_eq!(e.line_index(Some(9)), None);
        e.set_selection(0, 0);
        e.key(vk::RIGHT, false, true);
        assert_eq!(e.caret(), 4, "word right");
        e.key(vk::DOWN, false, false);
        assert_eq!(e.caret(), 13);
        e.key(vk::LEFT, false, true);
        assert_eq!(e.caret(), 9);
        assert!(!e.can_undo());
        e.char(u16::from(b'X'));
        assert!(e.can_undo());
        e.undo();
        assert_eq!(e.text(), w("one two\r\nthree\r\n"));
        e.undo();
        assert!(e.text().contains(&u16::from(b'X')), "undo of undo");
        let mut s = Edit::new(0);
        s.limit = 3;
        assert_eq!(
            changes(&s.set_text(&w("ab\r\nc"))),
            1,
            "single-line notifies"
        );
        assert_eq!(s.text(), w("ab"));
        s.set_selection(2, 2);
        let o = s.replace_selection(&w("xyz"), true);
        assert!(o.notify.contains(&Notify::MaxText));
        assert_eq!(s.text(), w("abx"));
        assert_eq!(changes(&s.char(13)), 0, "no CR in single-line");
        let mut n = Edit::new(es::NUMBER | es::UPPERCASE);
        n.char(u16::from(b'a'));
        n.char(u16::from(b'7'));
        assert_eq!(n.text(), w("7"));
        let mut u = Edit::new(es::UPPERCASE);
        u.char(u16::from(b'q'));
        assert_eq!(u.text(), w("Q"));
        let mut r = Edit::new(es::READONLY | es::MULTILINE);
        r.set_text(&w("x"));
        assert_eq!(changes(&r.char(u16::from(b'y'))), 0);
        assert_eq!(changes(&r.key(vk::DELETE, false, false)), 0);
        assert_eq!(
            Edit::new(es::PASSWORD).display_char(u16::from(b'a')),
            u16::from(b'*')
        );
    }
}
