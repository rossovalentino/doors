//! Notes text document (master specification §113 text-field model), kept
//! apart from layout and glyph caches. UTF-8 lives in a gap buffer; the caret
//! and selection stop only on grapheme-cluster boundaries (CRLF is one
//! cluster), Ctrl+arrow stops come from UAX #29 word segmentation, typing
//! coalesces into undo groups, and an IME composition is a display overlay
//! that never enters the text or the history until its commit arrives. The
//! file's line endings and UTF-8 BOM are kept byte for byte; new line breaks
//! use the document's own ending (`NEW_DOCUMENT_EOL` for a new document).
extern crate alloc;
use alloc::{borrow::Cow, collections::VecDeque, string::String, vec::Vec};
use core::ops::Range;
use door_graphics_text::{grapheme_floor, next_grapheme, next_word, prev_grapheme, prev_word};

/// Largest document Notes edits: 4 MiB of UTF-8 (the ordinary volume bounds
/// one file at 4,243,456 bytes). Larger files and edits are refused whole,
/// never truncated.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Undo history bound: groups and removed + inserted payload bytes. A single
/// edit larger than the payload bound clears the history (it cannot be undone).
const HISTORY_GROUPS: usize = 256;
const HISTORY_BYTES: usize = 8 * 1024 * 1024;
/// Text either side of the caret given to grapheme/word segmentation.
const CONTEXT: usize = 8 * 1024;
const GAP: usize = 4096;
const BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The result would exceed `MAX_BYTES`.
    TooLong,
    /// Not UTF-8 text (a NUL, another encoding or binary data).
    Unsupported,
}
/// A soft-wrap boundary has two visual caret positions at the same byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    Upstream,
    #[default]
    Downstream,
}
/// The document's line ending (Notepad's status bar names it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    CrLf,
    Lf,
    Cr,
}
impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CrLf => "\r\n",
            Self::Lf => "\n",
            Self::Cr => "\r",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::CrLf => "Windows (CRLF)",
            Self::Lf => "Unix (LF)",
            Self::Cr => "Macintosh (CR)",
        }
    }
    /// The first line ending in `text`; `NEW_DOCUMENT_EOL` when there is none.
    pub fn detect(text: &[u8]) -> Self {
        match text.iter().position(|b| matches!(b, b'\r' | b'\n')) {
            Some(i) if text[i] == b'\n' => Self::Lf,
            Some(i) if text.get(i + 1) == Some(&b'\n') => Self::CrLf,
            Some(_) => Self::Cr,
            None => NEW_DOCUMENT_EOL,
        }
    }
}
/// Line ending of a document without one yet. Windows Notepad uses CRLF;
/// Door keeps LF until the guest scenarios that audit LF bytes typed into new
/// documents (Explorer, file mutation, multiwindow, Preview) move together.
pub const NEW_DOCUMENT_EOL: LineEnding = LineEnding::Lf;
/// Every line break in `text` (CRLF, CR or LF) rewritten as `eol`; NULs are
/// dropped. Pasted text adopts the document's ending, as Notepad does.
pub fn normalize_newlines(text: &str, eol: LineEnding) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(eol.as_str());
            }
            '\n' => out.push_str(eol.as_str()),
            '\0' => (),
            c => out.push(c),
        }
    }
    out
}
/// IME preedit shown at the caret; `cursor` is a byte offset into `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    pub at: usize,
    pub text: String,
    pub cursor: Option<usize>,
}

/// UTF-8 bytes with a movable gap at the last edit.
struct Gap {
    buf: Vec<u8>,
    start: usize,
    end: usize,
}
impl Gap {
    fn new(text: &[u8]) -> Self {
        let mut buf = Vec::with_capacity(text.len() + GAP);
        buf.extend_from_slice(text);
        buf.resize(text.len() + GAP, 0);
        Self {
            buf,
            start: text.len(),
            end: text.len() + GAP,
        }
    }
    fn len(&self) -> usize {
        self.buf.len() - (self.end - self.start)
    }
    fn byte(&self, i: usize) -> Option<u8> {
        if i < self.start {
            Some(self.buf[i])
        } else if i < self.len() {
            Some(self.buf[i + self.end - self.start])
        } else {
            None
        }
    }
    fn slice(&self, r: Range<usize>) -> Cow<'_, [u8]> {
        let (a, b) = (&self.buf[..self.start], &self.buf[self.end..]);
        if r.end <= a.len() {
            Cow::Borrowed(&a[r])
        } else if r.start >= a.len() {
            Cow::Borrowed(&b[r.start - a.len()..r.end - a.len()])
        } else {
            let mut v = Vec::with_capacity(r.len());
            v.extend_from_slice(&a[r.start..]);
            v.extend_from_slice(&b[..r.end - a.len()]);
            Cow::Owned(v)
        }
    }
    fn move_gap(&mut self, at: usize) {
        if at < self.start {
            let n = self.start - at;
            self.buf.copy_within(at..self.start, self.end - n);
            self.start = at;
            self.end -= n;
        } else if at > self.start {
            let n = at - self.start;
            self.buf.copy_within(self.end..self.end + n, self.start);
            self.start += n;
            self.end += n;
        }
    }
    fn replace(&mut self, r: Range<usize>, text: &[u8]) {
        self.move_gap(r.end);
        self.start = r.start;
        let gap = self.end - self.start;
        if gap < text.len() {
            let extra = text.len() - gap + (self.len() / 16).max(GAP);
            let old = self.buf.len();
            self.buf.resize(old + extra, 0);
            self.buf.copy_within(self.end..old, self.end + extra);
            self.end += extra;
        }
        self.buf[self.start..self.start + text.len()].copy_from_slice(text);
        self.start += text.len();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Selection {
    anchor: usize,
    caret: usize,
    affinity: Affinity,
}
impl Selection {
    fn at(at: usize) -> Self {
        Self {
            anchor: at,
            caret: at,
            affinity: Affinity::Downstream,
        }
    }
}
struct Edit {
    at: usize,
    removed: Vec<u8>,
    inserted: Vec<u8>,
}
struct Group {
    edits: Vec<Edit>,
    before: Selection,
    after: Selection,
    before_state: u64,
    after_state: u64,
}
impl Group {
    fn bytes(&self) -> usize {
        self.edits
            .iter()
            .map(|e| e.removed.len() + e.inserted.len())
            .sum()
    }
}
/// How an edit may join the previous undo group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Typing: a word and the spaces after it form one group.
    Typing {
        space: bool,
    },
    DeleteBackward,
    DeleteForward,
}

pub struct Document {
    text: Gap,
    selection: Selection,
    /// Byte offset of every line start (index 0 is 0).
    lines: Vec<usize>,
    scalars: usize,
    crlf: usize,
    eol: LineEnding,
    bom: bool,
    composition: Option<Composition>,
    undo: VecDeque<Group>,
    redo: Vec<Group>,
    history_bytes: usize,
    /// The open undo group may still grow: its kind and the caret it ends at.
    open: Option<(Kind, usize)>,
    state: u64,
    next_state: u64,
    saved: Option<u64>,
    revision: u64,
}
fn validate(text: &str) -> Result<(), EditError> {
    if text.len() > MAX_BYTES {
        return Err(EditError::TooLong);
    }
    if text.contains('\0') {
        return Err(EditError::Unsupported);
    }
    Ok(())
}
fn scalars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|b| (**b & 0xc0) != 0x80).count()
}
impl Document {
    /// A document holding `text` (no BOM), caret at the end.
    pub fn new(text: &str) -> Result<Self, EditError> {
        validate(text)?;
        let bytes = text.as_bytes();
        let mut d = Self {
            text: Gap::new(bytes),
            selection: Selection::at(bytes.len()),
            lines: alloc::vec![0],
            scalars: scalars(bytes),
            crlf: bytes.windows(2).filter(|p| p == b"\r\n").count(),
            eol: LineEnding::detect(bytes),
            bom: false,
            composition: None,
            undo: VecDeque::new(),
            redo: Vec::new(),
            history_bytes: 0,
            open: None,
            state: 0,
            next_state: 1,
            saved: Some(0),
            revision: 0,
        };
        for s in 1..=bytes.len() {
            if d.is_line_start(s) {
                d.lines.push(s);
            }
        }
        Ok(d)
    }
    /// Decode file bytes: UTF-8 with or without a BOM (kept for save). Other
    /// encodings, NULs and files over `MAX_BYTES` are refused.
    pub fn from_file(bytes: &[u8]) -> Result<Self, EditError> {
        let (bom, body) = match bytes.strip_prefix(BOM) {
            Some(rest) => (true, rest),
            None => (false, bytes),
        };
        if body.len() > MAX_BYTES {
            return Err(EditError::TooLong);
        }
        if !bom && (bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF")) {
            return Err(EditError::Unsupported);
        }
        let text = core::str::from_utf8(body).map_err(|_| EditError::Unsupported)?;
        let mut d = Self::new(text)?;
        d.bom = bom;
        Ok(d)
    }
    /// The bytes to store: the BOM if the file had one, then the text.
    pub fn file_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.len() + 3);
        if self.bom {
            out.extend_from_slice(BOM);
        }
        let (a, b) = (
            &self.text.buf[..self.text.start],
            &self.text.buf[self.text.end..],
        );
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out
    }
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn byte(&self, at: usize) -> Option<u8> {
        self.text.byte(at)
    }
    /// Text of a range on character boundaries (copied only across the gap).
    pub fn text(&self, range: Range<usize>) -> Cow<'_, str> {
        match self.text.slice(range) {
            Cow::Borrowed(b) => Cow::Borrowed(core::str::from_utf8(b).expect("UTF-8 document")),
            Cow::Owned(v) => Cow::Owned(String::from_utf8(v).expect("UTF-8 document")),
        }
    }
    pub fn to_text(&self) -> String {
        self.text(0..self.len()).into_owned()
    }
    pub fn eol(&self) -> LineEnding {
        self.eol
    }
    pub fn bom(&self) -> bool {
        self.bom
    }
    pub fn caret(&self) -> usize {
        self.selection.caret
    }
    pub fn anchor(&self) -> usize {
        self.selection.anchor
    }
    pub fn selection(&self) -> Range<usize> {
        self.selection.anchor.min(self.caret())..self.selection.anchor.max(self.caret())
    }
    pub fn affinity(&self) -> Affinity {
        self.selection.affinity
    }
    pub fn selected_text(&self) -> String {
        self.text(self.selection()).into_owned()
    }
    /// Characters as Notepad's status bar counts them: Unicode scalars, each
    /// line break (CRLF included) one character.
    pub fn char_count(&self) -> usize {
        self.scalars - self.crlf
    }
    pub fn chars_in(&self, range: Range<usize>) -> usize {
        let bytes = self.text.slice(range);
        scalars(&bytes) - bytes.windows(2).filter(|p| p == b"\r\n").count()
    }
    /// Layout observers rebuild when this changes (text and composition);
    /// selection and savepoints do not change it. None once exhausted.
    pub fn content_revision(&self) -> Option<u64> {
        (self.revision != u64::MAX).then_some(self.revision)
    }
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.state)
    }
    /// Call after an acknowledged durable save. Undo/redo back to this
    /// history state is clean again; any other state is dirty.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.state);
        self.open = None;
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    // --- lines ------------------------------------------------------------
    fn is_line_start(&self, s: usize) -> bool {
        match self.text.byte(s.wrapping_sub(1)) {
            Some(b'\n') => true,
            Some(b'\r') => self.text.byte(s) != Some(b'\n'),
            _ => false,
        }
    }
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
    /// Logical line containing `at` (a position inside CRLF stays on its line).
    pub fn line_of(&self, at: usize) -> usize {
        self.lines.partition_point(|&s| s <= at).saturating_sub(1)
    }
    pub fn line_start(&self, line: usize) -> usize {
        self.lines[line]
    }
    /// Start of the following line (after this line's ending), or the length.
    pub fn line_next(&self, line: usize) -> usize {
        self.lines.get(line + 1).copied().unwrap_or(self.len())
    }
    /// End of the line's text, before its ending.
    pub fn line_end(&self, line: usize) -> usize {
        let (start, mut end) = (self.lines[line], self.line_next(line));
        if end > start && self.byte(end - 1) == Some(b'\n') {
            end -= 1;
        }
        if end > start && self.byte(end - 1) == Some(b'\r') {
            end -= 1;
        }
        end
    }
    /// Notepad's "Ln, Col": 1-based logical line and character column.
    pub fn line_column(&self, at: usize) -> (usize, usize) {
        let line = self.line_of(at);
        (
            line + 1,
            self.chars_in(self.lines[line]..at.max(self.lines[line])) + 1,
        )
    }
    // --- segmentation -----------------------------------------------------
    fn floor_char(&self, mut at: usize) -> usize {
        at = at.min(self.len());
        while self.byte(at).is_some_and(|b| (b & 0xc0) == 0x80) {
            at -= 1;
        }
        at
    }
    /// Text around `at` for segmentation: (base offset, text).
    fn context(&self, at: usize) -> (usize, Cow<'_, str>) {
        let start = self.floor_char(at.saturating_sub(CONTEXT));
        let end = self.floor_char(at.saturating_add(CONTEXT));
        (start, self.text(start..end))
    }
    pub fn grapheme_floor(&self, at: usize) -> usize {
        let at = self.floor_char(at);
        let (base, text) = self.context(at);
        base + grapheme_floor(&text, at - base)
    }
    pub fn next_grapheme(&self, at: usize) -> usize {
        let (base, text) = self.context(at);
        (base + next_grapheme(&text, at - base)).max(at)
    }
    pub fn prev_grapheme(&self, at: usize) -> usize {
        let (base, text) = self.context(at);
        (base + prev_grapheme(&text, at - base)).min(at)
    }
    /// Ctrl+Right: the next word start; at a line end, the next line.
    pub fn next_word(&self, at: usize) -> usize {
        let (base, text) = self.context(at);
        let next = base + next_word(&text, at - base);
        if next > at {
            next
        } else {
            self.next_grapheme(at)
        }
    }
    /// Ctrl+Left: the start of the word before `at`.
    pub fn prev_word(&self, at: usize) -> usize {
        let (base, text) = self.context(at);
        base + prev_word(&text, at - base)
    }
    // --- caret and selection ---------------------------------------------
    pub fn place(&mut self, at: usize, extend: bool) {
        self.place_visual(at, extend, Affinity::Downstream);
    }
    /// Place the caret on the grapheme boundary at or before `at`.
    pub fn place_visual(&mut self, at: usize, extend: bool, affinity: Affinity) {
        let at = self.grapheme_floor(at);
        self.selection.caret = at;
        self.selection.affinity = affinity;
        if !extend {
            self.selection.anchor = at;
        }
        self.open = None;
    }
    /// Select `range` with the caret at its end.
    pub fn select(&mut self, range: Range<usize>) {
        self.place(range.start, false);
        self.place(range.end, true);
    }
    pub fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            caret: self.len(),
            affinity: Affinity::Downstream,
        };
        self.open = None;
    }
    pub fn horizontal(&mut self, right: bool, extend: bool, word: bool) {
        let selected = self.selection();
        let at = if !extend && !word && !selected.is_empty() {
            if right { selected.end } else { selected.start }
        } else if word {
            if right {
                self.next_word(self.caret())
            } else {
                self.prev_word(self.caret())
            }
        } else if right {
            self.next_grapheme(self.caret())
        } else {
            self.prev_grapheme(self.caret())
        };
        self.place(at, extend);
    }
    /// Logical line or document Home/End (the view uses visual rows).
    pub fn home_end(&mut self, end: bool, document: bool, extend: bool) {
        let line = self.line_of(self.caret());
        let at = match (document, end) {
            (true, true) => self.len(),
            (true, false) => 0,
            (false, true) => self.line_end(line),
            (false, false) => self.line_start(line),
        };
        self.place(at, extend);
    }
    // --- editing ----------------------------------------------------------
    fn count_crlf(&self, from: usize, to: usize) -> usize {
        (from..to)
            .filter(|&p| self.byte(p) == Some(b'\r') && self.byte(p + 1) == Some(b'\n'))
            .count()
    }
    /// Replace bytes and keep the line index and counts current.
    fn apply(&mut self, at: usize, removed: usize, inserted: &[u8]) {
        let low = at.saturating_sub(1);
        let before = self.count_crlf(low, at + removed);
        self.scalars -= scalars(&self.text.slice(at..at + removed));
        self.text.replace(at..at + removed, inserted);
        self.scalars += scalars(inserted);
        self.crlf = self.crlf - before + self.count_crlf(low, at + inserted.len());
        let first = self.lines.partition_point(|&s| s < at.max(1));
        let last = self.lines.partition_point(|&s| s <= at + removed + 1);
        let delta = inserted.len() as isize - removed as isize;
        for s in &mut self.lines[last..] {
            *s = s.wrapping_add_signed(delta);
        }
        let fresh: Vec<usize> = (at.max(1)..=(at + inserted.len() + 1).min(self.len()))
            .filter(|&s| self.is_line_start(s))
            .collect();
        self.lines.splice(first..last, fresh);
        self.revision = self.revision.saturating_add(1);
    }
    fn record(&mut self, edit: Edit, before: Selection, kind: Option<Kind>) {
        for old in self.redo.drain(..) {
            self.history_bytes -= old.bytes();
        }
        let state = self.next_state;
        self.next_state += 1;
        let previous = core::mem::replace(&mut self.state, state);
        // The open group continues only where its caret was left: where this
        // edit starts (typing, forward delete) or ends (backspace).
        let joins = self.undo.back().is_some()
            && match (kind, self.open) {
                (Some(Kind::Typing { space }), Some((Kind::Typing { space: was }, caret))) => {
                    caret == edit.at && (!was || space)
                }
                (Some(Kind::DeleteBackward), Some((Kind::DeleteBackward, caret))) => {
                    caret == edit.at + edit.removed.len()
                }
                (Some(Kind::DeleteForward), Some((Kind::DeleteForward, caret))) => caret == edit.at,
                _ => false,
            };
        let bytes = edit.removed.len() + edit.inserted.len();
        let caret = edit.at + edit.inserted.len();
        if bytes > HISTORY_BYTES {
            self.undo.clear();
            self.history_bytes = 0;
            self.open = None;
            return;
        }
        if joins {
            let group = self.undo.back_mut().expect("open group");
            group.edits.push(edit);
            group.after = self.selection;
            group.after_state = state;
        } else {
            self.undo.push_back(Group {
                edits: alloc::vec![edit],
                before,
                after: self.selection,
                before_state: previous,
                after_state: state,
            });
        }
        self.history_bytes += bytes;
        while self.undo.len() > HISTORY_GROUPS || self.history_bytes > HISTORY_BYTES {
            let dropped = self.undo.pop_front().expect("bounded history");
            self.history_bytes -= dropped.bytes();
        }
        self.open = kind.map(|k| (k, caret));
    }
    fn edit(
        &mut self,
        range: Range<usize>,
        text: &str,
        kind: Option<Kind>,
    ) -> Result<bool, EditError> {
        validate(text)?;
        if self.len() - range.len() + text.len() > MAX_BYTES {
            return Err(EditError::TooLong);
        }
        let at = range.start + text.len();
        if *self.text.slice(range.clone()) == *text.as_bytes() {
            self.place(at, false);
            return Ok(false);
        }
        let before = self.selection;
        let removed = self.text.slice(range.clone()).into_owned();
        self.apply(range.start, range.len(), text.as_bytes());
        // A CR typed before LF forms one CRLF ending: stay outside it.
        let at = if self.byte(at.wrapping_sub(1)) == Some(b'\r') && self.byte(at) == Some(b'\n') {
            at + 1
        } else {
            at
        };
        self.selection = Selection::at(at);
        self.record(
            Edit {
                at: range.start,
                removed,
                inserted: text.as_bytes().to_vec(),
            },
            before,
            kind,
        );
        Ok(true)
    }
    /// Replace the selection with `text` as its own undo group.
    pub fn insert(&mut self, text: &str) -> Result<bool, EditError> {
        self.composition = None;
        self.edit(self.selection(), text, None)
    }
    /// Typed text: consecutive typing at the caret coalesces into one undo
    /// group per word (and the spaces after it); replacing a selection or
    /// moving the caret starts a new group.
    pub fn type_text(&mut self, text: &str) -> Result<bool, EditError> {
        self.composition = None;
        let selected = self.selection();
        let space = text.chars().all(char::is_whitespace);
        let kind = selected.is_empty().then_some(Kind::Typing { space });
        self.edit(selected, text, kind)
    }
    /// Paste: line breaks adopt the document's ending; one undo group.
    pub fn paste(&mut self, text: &str) -> Result<bool, EditError> {
        let text = normalize_newlines(text, self.eol);
        if self.len() - self.selection().len() + text.len() > MAX_BYTES {
            return Err(EditError::TooLong);
        }
        self.insert(&text)
    }
    pub fn erase(&mut self, backward: bool, word: bool) -> bool {
        self.composition = None;
        let selected = self.selection();
        let at = self.caret();
        let (range, kind) = if !selected.is_empty() {
            (selected, None)
        } else if backward {
            let from = if word {
                self.prev_word(at)
            } else {
                self.prev_grapheme(at)
            };
            (from..at, Some(Kind::DeleteBackward))
        } else {
            let to = if word {
                self.next_word(at)
            } else {
                self.next_grapheme(at)
            };
            (at..to, Some(Kind::DeleteForward))
        };
        if range.is_empty() {
            return false;
        }
        let kind = if word { None } else { kind };
        self.edit(range, "", kind).expect("bounded deletion")
    }
    /// IME_DELETE: remove `before`/`after` bytes around the caret, widened to
    /// character boundaries.
    pub fn delete_surrounding(&mut self, before: usize, after: usize) -> bool {
        let at = self.caret();
        let from = self.floor_char(at.saturating_sub(before));
        let mut to = at.saturating_add(after).min(self.len());
        while self.byte(to).is_some_and(|b| (b & 0xc0) == 0x80) {
            to += 1;
        }
        from < to && self.edit(from..to, "", None).unwrap_or(false)
    }
    // --- composition ------------------------------------------------------
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }
    /// Show (or with empty text, clear) the IME preedit at the caret. A
    /// selection is replaced when composition starts, as in Windows.
    pub fn set_composition(&mut self, text: &str, cursor: Option<usize>) -> bool {
        if text.is_empty() {
            return self.cancel_composition();
        }
        if self.composition.is_none() && !self.selection().is_empty() {
            let _ = self.edit(self.selection(), "", None);
        }
        let cursor = cursor.filter(|c| *c <= text.len() && text.is_char_boundary(*c));
        let next = Composition {
            at: self.caret(),
            text: text.into(),
            cursor,
        };
        if self.composition.as_ref() == Some(&next) {
            return false;
        }
        self.composition = Some(next);
        self.revision = self.revision.saturating_add(1);
        true
    }
    /// IME commit: the text replaces the preedit as one undo group.
    pub fn commit_composition(&mut self, text: &str) -> Result<bool, EditError> {
        self.cancel_composition();
        self.open = None;
        let result = self.edit(self.selection(), text, None);
        self.open = None;
        result
    }
    /// Commit the current preedit locally (a caret move or edit during
    /// composition); the caller then cancels the IME's composition.
    pub fn finish_composition(&mut self) -> bool {
        match self.composition.take() {
            Some(c) => {
                self.revision = self.revision.saturating_add(1);
                let _ = self.commit_composition(&c.text);
                true
            }
            None => false,
        }
    }
    pub fn cancel_composition(&mut self) -> bool {
        let had = self.composition.take().is_some();
        if had {
            self.revision = self.revision.saturating_add(1);
        }
        had
    }
    // --- history ----------------------------------------------------------
    pub fn undo(&mut self) -> bool {
        self.cancel_composition();
        let Some(group) = self.undo.pop_back() else {
            return false;
        };
        for edit in group.edits.iter().rev() {
            self.apply(edit.at, edit.inserted.len(), &edit.removed);
        }
        self.selection = group.before;
        self.state = group.before_state;
        self.open = None;
        self.redo.push(group);
        true
    }
    pub fn redo(&mut self) -> bool {
        self.cancel_composition();
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for edit in &group.edits {
            self.apply(edit.at, edit.removed.len(), &edit.inserted);
        }
        self.selection = group.after;
        self.state = group.after_state;
        self.open = None;
        self.undo.push_back(group);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::ToString};
    fn doc(text: &str) -> Document {
        Document::new(text).unwrap()
    }
    #[test]
    fn grapheme_clusters_are_single_caret_steps_and_erasures() {
        // e + combining acute, a ZWJ family emoji, a flag, CJK and CRLF.
        let text = "e\u{301}👨\u{200d}👩\u{200d}👧🇮🇹日\r\nZ";
        let mut d = doc(text);
        d.place(0, false);
        let mut stops = alloc::vec![0];
        while d.caret() < d.len() {
            d.horizontal(true, false, false);
            stops.push(d.caret());
        }
        let family = "👨\u{200d}👩\u{200d}👧".len();
        assert_eq!(
            stops,
            [
                0,
                3,
                3 + family,
                11 + family,
                14 + family,
                16 + family,
                17 + family
            ]
        );
        // Placement inside a cluster snaps back to its start.
        d.place(1, false);
        assert_eq!(d.caret(), 0);
        d.place(15 + family, false);
        assert_eq!(d.caret(), 14 + family, "never between CR and LF");
        d.place(11 + family, false);
        assert!(d.erase(true, false));
        assert_eq!(d.to_text(), "e\u{301}👨\u{200d}👩\u{200d}👧日\r\nZ");
        d.place(3, false);
        assert!(d.erase(false, false));
        assert_eq!(d.to_text(), "e\u{301}日\r\nZ");
        assert!(d.erase(true, false));
        assert_eq!(d.to_text(), "日\r\nZ");
        d.undo();
        assert_eq!(d.to_text(), "e\u{301}日\r\nZ");
    }
    #[test]
    fn text01_grapheme_key_sequence_leaves_the_expected_line() {
        // TEXT-01: `[`, two IME-committed ZWJ families, `]`, Left x3, `<`,
        // Right, `-`, Right, Backspace, `>`, End, Enter.
        let family = "👨\u{200d}👩\u{200d}👧";
        let mut d = doc("");
        d.type_text("[").unwrap();
        for _ in 0..2 {
            d.set_composition("かぞく", None);
            d.commit_composition(family).unwrap();
        }
        assert_eq!(d.caret(), 1 + 2 * family.len());
        d.type_text("]").unwrap();
        for _ in 0..3 {
            d.horizontal(false, false, false);
        }
        assert_eq!(d.caret(), 1, "Left steps over whole clusters");
        d.type_text("<").unwrap();
        d.horizontal(true, false, false);
        d.type_text("-").unwrap();
        d.horizontal(true, false, false);
        assert!(d.erase(true, false));
        d.type_text(">").unwrap();
        assert_eq!(d.to_text(), format!("[<{family}->]"));
        assert_eq!(d.caret(), d.len() - 1, "the caret stays before `]`");
        d.home_end(true, false, false);
        d.type_text("\n").unwrap();
        assert_eq!(d.to_text(), format!("[<{family}->]\n"));
        assert_eq!(d.caret(), d.len());
    }
    #[test]
    fn word_movement_uses_unicode_segmentation_and_crosses_line_ends() {
        let mut d = doc("città, ünïcode 日本\r\nnext");
        d.place(0, false);
        let mut stops = Vec::new();
        for _ in 0..5 {
            d.horizontal(true, false, true);
            stops.push(d.text(0..d.caret()).into_owned());
        }
        // UAX #29: ideographs are single word segments; Ctrl+Right at a line
        // end moves to the next line.
        assert_eq!(
            stops,
            [
                "città, ",
                "città, ünïcode ",
                "città, ünïcode 日",
                "città, ünïcode 日本",
                "città, ünïcode 日本\r\n"
            ]
        );
        d.horizontal(false, false, true);
        assert_eq!(&d.text(0..d.caret()), "città, ünïcode 日");
        d.erase(true, true);
        assert_eq!(d.to_text(), "città, ünïcode 本\r\nnext");
        d.horizontal(true, true, true);
        assert_eq!(d.selected_text(), "本");
    }
    #[test]
    fn crlf_lf_and_cr_endings_round_trip_and_new_breaks_use_the_document_ending() {
        for (file, typed) in [
            (&b"one\r\ntwo\r\n"[..], &b"one\r\ntwo\r\nthree\r\n"[..]),
            (b"one\ntwo\n", b"one\ntwo\nthree\n"),
            (b"one\rtwo\r", b"one\rtwo\rthree\r"),
            (
                b"\xEF\xBB\xBFone\r\ntwo\r\n",
                b"\xEF\xBB\xBFone\r\ntwo\r\nthree\r\n",
            ),
        ] {
            let mut d = Document::from_file(file).unwrap();
            assert_eq!(d.file_bytes(), file, "unchanged bytes round-trip");
            let eol = d.eol();
            d.type_text("three").unwrap();
            d.type_text(eol.as_str()).unwrap();
            assert_eq!(d.file_bytes(), typed);
            d.undo();
            assert_eq!(d.file_bytes(), file);
        }
        // Mixed endings are preserved exactly; the first one names the file.
        let mixed = b"a\nb\r\nc\rd";
        let d = Document::from_file(mixed).unwrap();
        assert_eq!(
            (d.eol(), d.file_bytes().as_slice()),
            (LineEnding::Lf, &mixed[..])
        );
        assert_eq!(d.line_count(), 4);
        assert_eq!((d.line_end(1), d.line_next(1)), (3, 5));
        assert_eq!(Document::new("").unwrap().eol(), NEW_DOCUMENT_EOL);
        assert_eq!(LineEnding::CrLf.label(), "Windows (CRLF)");
        assert_eq!(
            Document::from_file(b"\xFF\xFEa\0").err(),
            Some(EditError::Unsupported)
        );
        assert_eq!(
            Document::from_file(b"a\xC3").err(),
            Some(EditError::Unsupported)
        );
        assert_eq!(
            normalize_newlines("a\r\nb\nc\rd\0", LineEnding::CrLf),
            "a\r\nb\r\nc\r\nd"
        );
    }
    #[test]
    fn documents_well_beyond_four_kib_edit_and_count_through_the_gap() {
        let line = "Ünïcödé line with 日本語 and emoji 😀\r\n";
        let text = line.repeat(40_000); // ~1.9 MB
        assert!(text.len() > 1024 * 1024);
        let mut d = doc(&text);
        assert_eq!(d.line_count(), 40_001);
        assert_eq!(d.char_count(), text.chars().count() - 40_000);
        // Edit near the start, the middle and the end; the gap moves.
        for at in [line.len() * 3, text.len() / 2, text.len() - 2] {
            let at = d.grapheme_floor(at);
            d.place(at, false);
            d.type_text("X\r\n").unwrap();
        }
        assert_eq!(d.line_count(), 40_004);
        assert_eq!(d.len(), text.len() + 9);
        let (line_no, col) = d.line_column(d.caret());
        assert_eq!(d.line_of(d.caret()) + 1, line_no);
        assert!(col >= 1);
        let lines: Vec<usize> = (0..d.line_count()).map(|i| d.line_start(i)).collect();
        let fresh = doc(&d.to_text());
        assert_eq!(
            lines,
            (0..fresh.line_count())
                .map(|i| fresh.line_start(i))
                .collect::<Vec<_>>()
        );
        assert_eq!(d.char_count(), fresh.char_count());
        while d.undo() {}
        assert_eq!(d.to_text(), text);
        assert!(!d.dirty());
        // The bound is exact and refusals change nothing.
        let mut full = doc(&"a".repeat(MAX_BYTES));
        assert_eq!(full.type_text("b"), Err(EditError::TooLong));
        assert_eq!(full.paste("bb"), Err(EditError::TooLong));
        assert!(!full.dirty() && !full.can_undo());
        full.select_all();
        assert_eq!(full.type_text("ok"), Ok(true));
        assert_eq!(
            Document::from_file(&alloc::vec![b'a'; MAX_BYTES + 1]).err(),
            Some(EditError::TooLong)
        );
    }
    #[test]
    fn composition_is_an_overlay_until_commit_and_one_undo_group() {
        let mut d = doc("ab");
        d.place(1, false);
        let revision = d.content_revision();
        assert!(d.set_composition("ni", Some(2)));
        assert_ne!(d.content_revision(), revision, "layout sees the preedit");
        assert_eq!(d.to_text(), "ab", "preedit is not document text");
        assert!(!d.dirty() && !d.can_undo());
        d.set_composition("你", None);
        assert_eq!(
            d.composition().map(|c| (c.at, c.text.as_str())),
            Some((1, "你"))
        );
        d.commit_composition("你好").unwrap();
        assert_eq!((d.to_text().as_str(), d.caret()), ("a你好b", 7));
        assert!(d.composition().is_none());
        assert!(d.undo());
        assert_eq!(d.to_text(), "ab");
        assert!(!d.can_undo());
        // Cancel restores nothing to history; a selection is replaced when
        // composition starts; a local finish commits the preedit.
        d.select_all();
        d.set_composition("k", None);
        assert_eq!(d.to_text(), "");
        assert!(d.cancel_composition());
        d.set_composition("か", Some(3));
        assert!(d.finish_composition());
        assert_eq!(d.to_text(), "か");
        d.undo();
        d.undo();
        assert_eq!(d.to_text(), "ab");
    }
    #[test]
    fn typing_coalesces_by_word_and_savepoints_follow_history_states() {
        let mut d = doc("");
        for c in "hello world".chars() {
            d.type_text(&c.to_string()).unwrap();
        }
        d.undo();
        assert_eq!(d.to_text(), "hello ");
        d.undo();
        assert_eq!(d.to_text(), "");
        d.redo();
        d.mark_saved();
        assert!(!d.dirty());
        d.type_text("x").unwrap();
        assert!(d.dirty());
        d.undo();
        assert!(!d.dirty(), "back at the saved state");
        d.redo();
        assert!(d.dirty());
        // Moving the caret ends the group; branching drops redo.
        d.place(0, false);
        d.type_text("A").unwrap();
        d.undo();
        assert_eq!(d.to_text(), "hello x");
        d.type_text("B").unwrap();
        assert!(!d.can_redo());
        // Backspace runs coalesce; a selection replacement restores it.
        let mut d = doc("one two");
        for _ in 0..3 {
            d.erase(true, false);
        }
        assert_eq!(d.to_text(), "one ");
        d.undo();
        assert_eq!(d.to_text(), "one two");
        d.place(0, false);
        d.place(3, true);
        d.type_text("1").unwrap();
        d.undo();
        assert_eq!((d.selection(), d.caret()), (0..3, 3));
    }
    #[test]
    fn clipboard_text_round_trips_selection_and_adopts_line_endings() {
        let mut d = Document::from_file(b"first\nsecond\n").unwrap();
        d.select(0..12);
        assert_eq!(d.selected_text(), "first\nsecond");
        d.place(d.len(), false);
        d.paste("a\r\nb\rc").unwrap();
        assert_eq!(d.to_text(), "first\nsecond\na\nb\nc");
        assert_eq!(d.chars_in(0..5), 5);
        d.undo();
        assert_eq!(d.to_text(), "first\nsecond\n");
        // Surrounding deletion widens to characters.
        let mut d = doc("aé日");
        d.place(d.len(), false);
        assert!(d.delete_surrounding(2, 0));
        assert_eq!(d.to_text(), "aé");
        assert_eq!(format!("{:?}", d.insert("\0")), "Err(Unsupported)");
    }
    #[test]
    fn affinity_survives_undo_and_history_bounds_hold() {
        let mut d = doc("abcdef");
        d.place_visual(3, false, Affinity::Upstream);
        d.insert("X").unwrap();
        assert_eq!(d.affinity(), Affinity::Downstream);
        d.undo();
        assert_eq!((d.caret(), d.affinity()), (3, Affinity::Upstream));
        let mut d = doc(&"a".repeat(64 * 1024));
        for i in 0..1000 {
            d.select_all();
            d.insert(&(if i % 2 == 0 { "b" } else { "a" }).repeat(64 * 1024))
                .unwrap();
            assert!(d.undo.len() <= HISTORY_GROUPS && d.history_bytes <= HISTORY_BYTES);
        }
        let mut undone = 0;
        while d.undo() {
            undone += 1;
        }
        assert_eq!(undone, HISTORY_BYTES / (128 * 1024));
    }
}
