//! door.text-input v1 (`specs/contracts/text-input-v1.md`, §53, §109).
//!
//! A focused text field enables a *context* on its window: content type,
//! hints and the caret rectangle (candidate placement), optionally the
//! surrounding text. While a context is enabled, every key for that window
//! reaches its owner as an ordered `KEY` record stream (physical key,
//! Windows virtual key, sided modifiers, locks, repeat and the Unicode text
//! of the session keyboard layout), interleaved with `PREEDIT`, `COMMIT` and
//! `DELETE` records from the input method. Windows without an enabled
//! context keep receiving legacy window INPUT key events (`window.rs`).
//!
//! The IME is one authenticated process per session (manifest
//! `IME_MANIFEST`). It alone receives `IME_KEY`, `ACTIVATE` and surrounding
//! text, and it alone may send preedit/commit. Secure contexts (`PASSWORD`,
//! `PIN`) never reach it, never store surrounding text, and a locked session
//! deactivates everything. Owners come from authenticated channel metadata.
use crate::window::Rect;
use crate::{BODY_BYTES, Code};
/// Opcodes 0x5240..=0x527F are reserved for text input (native-ui-v1.md).
pub const FIRST: u32 = 0x5240;
pub const LAST: u32 = 0x527F;
pub const VERSION: u32 = 1;
/// Process manifest the window server accepts as the session IME. The kernel
/// catalog entry is added with the IME process (coordinated with Init).
pub const IME_MANIFEST: u64 = 16;
/// HELLO capability (bit 26): text-input records for this client raise one
/// coalesced `PENDING` window event until the client polls.
pub const CAP_TEXT_INPUT: u32 = 1 << 26;
/// HELLO capability (bit 27), granted only to the `IME_MANIFEST` process:
/// `PENDING` events for the IME record stream.
pub const CAP_IME: u32 = 1 << 27;
/// Event kinds 0x130..=0x13F are reserved for text input. PENDING: records
/// wait in POLL (a = queued records); delivered once until the next POLL.
pub const PENDING: u32 = 0x130;
pub const LAST_EVENT: u32 = 0x13F;
// --- Application (text field) operations -------------------------------
/// `[window u64][content u32][hints u32][caret Rect]` (32 bytes): the
/// window's text field took focus. Reply `[serial u32]`; a later ENABLE on
/// the same window replaces the context (new serial, preedit discarded).
pub const ENABLE: u32 = 0x5240;
/// `[window u64]`: the field lost focus (pending composition is committed
/// by the IME first, Windows behaviour).
pub const DISABLE: u32 = 0x5241;
/// `[window u64][serial u32][0 u32][caret Rect]`: caret rectangle in
/// content-local physical pixels (top of line, line height).
pub const CURSOR: u32 = 0x5242;
/// `[window u64][serial u32][cursor u16][anchor u16][utf8 text]`: text
/// around the caret (byte offsets into `text`, on character boundaries).
/// Unsupported for secure contexts; at most `MAX_SURROUNDING` bytes.
pub const SURROUNDING: u32 = 0x5243;
/// `[window u64][serial u32][mode u32]`: the application changed the text
/// itself; the IME cancels (`RESET_CANCEL`) or completes (`RESET_COMPLETE`)
/// its composition.
pub const RESET: u32 = 0x5244;
/// Empty: reply up to `BODY_BYTES` of records (`Record`); WouldBlock if none.
/// Shared by applications and the IME.
pub const POLL: u32 = 0x5245;
/// Empty: reply `State` (`STATE_BYTES`).
pub const STATE: u32 = 0x5246;
/// `[preferences u32]`: Settings > Time & language > Language & region >
/// Keyboard (workspace apps only). Applies live; Settings persists it.
pub const SET_METHODS: u32 = 0x5247;
/// `[index u32]`: select an installed input method (workspace apps: the
/// Settings page and the taskbar input indicator flyout).
pub const SELECT: u32 = 0x5248;
/// `[preferences u32]`: Init (session 0) applies the session user's
/// persisted input methods at sign-in.
pub const SET_PREFERENCES: u32 = 0x5249;
// --- IME operations (IME_MANIFEST only) --------------------------------
/// `[version u32][methods u32]` (bit n = input method id n handled; bit
/// `NO_CANDIDATES_SHIFT` + n: method n composes without its candidate
/// dictionary). Reply `[current method u32]`. One IME per session.
pub const IME_REGISTER: u32 = 0x5260;
/// IME_REGISTER/State: methods running without a candidate dictionary
/// are reported in the high half (bit 16 + method id).
pub const NO_CANDIDATES_SHIFT: u32 = 16;
/// State flags: an IME process is registered.
pub const STATE_IME: u32 = 1;
/// `[serial u32][cursor u16][count u8][flags u8]` + count × `Attr` + text.
/// Empty text clears the preedit. `cursor` = `HIDDEN_CURSOR` hides it.
pub const IME_PREEDIT: u32 = 0x5261;
/// `[serial u32][0 u32]` + text (1..=`MAX_COMMIT` bytes). Clears preedit.
pub const IME_COMMIT: u32 = 0x5262;
/// `[serial u32][before u32][after u32]`: delete bytes around the caret.
pub const IME_DELETE: u32 = 0x5263;
/// `[key serial u32][verdict u32]`: the IME finished a key: `FORWARD` (the
/// application receives it unchanged) or `CONSUMED`. In order.
pub const IME_KEY_DONE: u32 = 0x5264;
/// `CandidateList` encoding; count 0 hides the candidate window.
pub const IME_CANDIDATES: u32 = 0x5265;
/// `[mode u32]`: conversion mode shown by the taskbar input indicator.
pub const IME_MODE: u32 = 0x5266;
pub const FORWARD: u32 = 1;
pub const CONSUMED: u32 = 0;
pub const RESET_CANCEL: u32 = 0;
pub const RESET_COMPLETE: u32 = 1;
/// IME conversion modes.
pub const MODE_DIRECT: u32 = 0;
pub const MODE_NATIVE: u32 = 1;
pub const MODE_KATAKANA: u32 = 2;
pub const MODE_FULLWIDTH: u32 = 3;
/// Content types. Secure types bypass the IME entirely.
pub const NORMAL: u32 = 0;
pub const PASSWORD: u32 = 1;
pub const URL: u32 = 2;
pub const NUMBER: u32 = 3;
pub const EMAIL: u32 = 4;
pub const TERMINAL: u32 = 5;
pub const PIN: u32 = 6;
pub const fn secure(content: u32) -> bool {
    matches!(content, PASSWORD | PIN)
}
/// Windows input scopes URL/e-mail/number open the IME in direct
/// (alphanumeric) input: these contexts bypass composition.
pub const fn latin(content: u32) -> bool {
    matches!(content, URL | NUMBER | EMAIL)
}
/// Hints.
pub const HINT_NO_PREDICTION: u32 = 1;
/// Private (InPrivate) input: the IME neither learns nor keeps history.
pub const HINT_PRIVATE: u32 = 2;
pub const HINT_MULTILINE: u32 = 4;
pub const HINTS: u32 = 7;
pub const MAX_SURROUNDING: usize = BODY_BYTES - 16;
pub const MAX_ATTRS: usize = 8;
/// Largest record (`[len u8][kind u8][0 u16]` header + payload).
pub const MAX_RECORD: usize = BODY_BYTES;
/// Record header bytes (`[len u8][kind u8][0 u16]`).
pub const RECORD_HEADER: usize = 4;
/// Fixed PREEDIT record payload: window, serial, cursor, count, padding.
const PREEDIT_FIXED: usize = 16;
/// Preedit text cap: one PREEDIT record (header + 16 fixed bytes + 6 bytes
/// for each of `MAX_ATTRS` attributes + text) always fits `MAX_RECORD`, so
/// the encoder, the IME (which truncates on a grapheme boundary) and the
/// broker share this one bound (240 - 4 - 16 - 48 = 172 bytes).
pub const MAX_PREEDIT: usize = MAX_RECORD - RECORD_HEADER - PREEDIT_FIXED - 6 * MAX_ATTRS;
/// Commit text per IME_COMMIT request, equal to what one COMMIT record
/// carries (header + window + serial + text), so the broker never splits a
/// commit; the IME splits longer ones on grapheme boundaries
/// (`grapheme_floor`).
pub const MAX_COMMIT: usize = MAX_RECORD - RECORD_HEADER - 12;
const _: () = assert!(RECORD_HEADER + PREEDIT_FIXED + 6 * MAX_ATTRS + MAX_PREEDIT <= MAX_RECORD);
const _: () = assert!(8 + 6 * MAX_ATTRS + MAX_PREEDIT <= BODY_BYTES);
const _: () = assert!(8 + MAX_COMMIT <= BODY_BYTES);
const _: () = assert!(RECORD_HEADER + 8 + MAX_SURROUNDING <= MAX_RECORD);
/// Largest Unicode text a KEY record carries (up to four scalars: a dead
/// key followed by an unmatched character yields two).
pub const MAX_KEY_TEXT: usize = 16;
pub const HIDDEN_CURSOR: u16 = 0xffff;
pub const MAX_CANDIDATES: usize = 9;
pub const MAX_CANDIDATE_BYTES: usize = 48;
/// Bounded server state. Contexts per application (owner), so one client
/// cannot exhaust the session; `MAX_TOTAL_CONTEXTS` bounds the server.
pub const MAX_CONTEXTS: usize = 16;
pub const MAX_TOTAL_CONTEXTS: usize = 64;
/// Records an application queue admits for KEY and preedit data. Control
/// records (commit, delete, disabled, preedit clear) may use a further
/// `CONTROL_RESERVE`: when data no longer fits, the queued data records are
/// dropped and the application's contexts are disabled, while queued
/// control records are kept in order.
pub const MAX_QUEUED_RECORDS: usize = 96;
pub const CONTROL_RESERVE: usize = MAX_CONTEXTS + 1;
pub const MAX_PENDING_KEYS: usize = 32;
/// A key the IME has not finished after this many ticks (10 ms) is
/// forwarded; the IME is bypassed until it answers again.
pub const IME_KEY_TIMEOUT_TICKS: u64 = 30;
/// Preedit attribute styles (Windows ATTR_*).
pub const STYLE_INPUT: u8 = 0;
pub const STYLE_TARGET_CONVERTED: u8 = 1;
pub const STYLE_CONVERTED: u8 = 2;
pub const STYLE_TARGET_NOTCONVERTED: u8 = 3;
pub const STYLE_INPUT_ERROR: u8 = 4;
/// KEY / IME_KEY status bits.
pub const KEY_DEAD: u8 = 1;
pub const KEY_COMPOSED: u8 = 2;
pub const KEY_FORWARDED: u8 = 4;
pub const KEY_ALTGR: u8 = 8;
/// Record kinds delivered to applications.
pub const R_KEY: u8 = 1;
pub const R_PREEDIT: u8 = 2;
pub const R_COMMIT: u8 = 3;
pub const R_DELETE: u8 = 4;
/// The server disabled the context (`reason`): the application re-enables
/// on its next focus change. Pending preedit is gone.
pub const R_DISABLED: u8 = 5;
/// Record kinds delivered to the IME.
pub const R_ACTIVATE: u8 = 16;
pub const R_DEACTIVATE: u8 = 17;
pub const R_SURROUNDING: u8 = 18;
pub const R_CURSOR: u8 = 19;
pub const R_IME_KEY: u8 = 20;
pub const R_METHOD: u8 = 21;
pub const R_RESET: u8 = 22;
pub const DISABLED_OVERFLOW: u32 = 1;
pub const DISABLED_LOCKED: u32 = 2;
pub const STATE_BYTES: usize = 16;
/// IME operations occupy 0x5260..=0x527F.
pub const fn ime_op(op: u32) -> bool {
    op >= IME_REGISTER && op <= LAST
}
/// Decode a PENDING window event (clients that negotiated CAP_TEXT_INPUT
/// or CAP_IME).
pub fn decode_event(b: &[u8]) -> Result<crate::window::Event, Code> {
    crate::clipboard::decode_event(b, PENDING..=PENDING)
}
fn w32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn w16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn w64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn text(b: &[u8]) -> Result<&str, Code> {
    let s = core::str::from_utf8(b).map_err(|_| Code::InvalidRequest)?;
    if s.contains('\0') {
        return Err(Code::InvalidRequest);
    }
    Ok(s)
}
/// Committed/preedit text may not carry C0/C1 controls except tab and line
/// feed (multi-line commits), nor U+FEFF/U+FFFE noise.
pub fn valid_text(s: &str) -> bool {
    s.chars()
        .all(|c| (!c.is_control() || c == '\t' || c == '\n') && c != '\u{feff}' && c != '\u{fffe}')
}
/// The largest split point at most `max` that does not divide an extended
/// grapheme cluster (UAX #29) of input-method text: never before a
/// combining mark, variation selector, emoji modifier, tag, joiner or
/// Hangul vowel/final jamo, never after a ZWJ or a continuing leading jamo,
/// never inside a regional-indicator pair. Conservative (it may keep
/// neighbouring clusters together); it falls back to a character boundary
/// only when a single cluster is longer than `max`.
pub fn grapheme_floor(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    let char_floor = i;
    while i > 0 {
        if cluster_boundary(s, i) {
            return i;
        }
        i -= 1;
        while !s.is_char_boundary(i) {
            i -= 1;
        }
    }
    char_floor
}
fn cluster_boundary(s: &str, at: usize) -> bool {
    let (before, after) = s.split_at(at);
    let (Some(p), Some(n)) = (before.chars().next_back(), after.chars().next()) else {
        return true;
    };
    let regional = |c: char| matches!(c as u32, 0x1F1E6..=0x1F1FF);
    let leading = |c: char| matches!(c as u32, 0x1100..=0x115F | 0xA960..=0xA97F);
    if grapheme_extend(n) || p == '\u{200d}' {
        return false;
    }
    if leading(p) && (leading(n) || matches!(n as u32, 0xAC00..=0xD7A3)) {
        return false;
    }
    if regional(p) && regional(n) {
        return before.chars().rev().take_while(|c| regional(*c)).count() % 2 == 0;
    }
    true
}
/// Characters that continue the preceding cluster (Extend, SpacingMark, ZWJ
/// and Hangul V/T jamo of the scripts the input methods produce).
fn grapheme_extend(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5 | 0x05C7 | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670
        | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED
        | 0x0900..=0x0903 | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x1160..=0x11FF | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF | 0x200C..=0x200D | 0x20D0..=0x20FF | 0x302A..=0x302F
        | 0x3099..=0x309A | 0xD7B0..=0xD7FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F
        | 0xFF9E..=0xFF9F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}
/// Session input-method state for Settings and the taskbar indicator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// `door_keyboard_layouts::Preferences` bits (installed methods, current).
    pub preferences: u32,
    /// Current input method id.
    pub method: u32,
    /// IME conversion mode (MODE_*), MODE_DIRECT for keyboards.
    pub mode: u32,
    /// `STATE_IME`: an IME process is registered; bits 16 + n: method n
    /// composes without candidates (its dictionary is not installed).
    pub flags: u32,
}
impl State {
    pub fn encode(self) -> [u8; STATE_BYTES] {
        let mut o = [0; STATE_BYTES];
        for (i, v) in [self.preferences, self.method, self.mode, self.flags]
            .into_iter()
            .enumerate()
        {
            o[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != STATE_BYTES
            || w32(b, 8) > MODE_FULLWIDTH
            || w32(b, 12) & !(STATE_IME | 0xffff << NO_CANDIDATES_SHIFT) != 0
        {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            preferences: w32(b, 0),
            method: w32(b, 4),
            mode: w32(b, 8),
            flags: w32(b, 12),
        })
    }
    /// The registered IME offers candidates for method `id` (false when
    /// no IME runs or its dictionary for that method is not installed).
    pub fn candidates_available(self, id: u32) -> bool {
        self.flags & STATE_IME != 0
            && (id >= 16 || self.flags & (1 << (NO_CANDIDATES_SHIFT + id)) == 0)
    }
}
/// ENABLE body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enable {
    pub window: u64,
    pub content: u32,
    pub hints: u32,
    pub caret: Rect,
}
impl Enable {
    pub fn encode(self) -> [u8; 32] {
        let mut o = [0; 32];
        o[..8].copy_from_slice(&self.window.to_le_bytes());
        o[8..12].copy_from_slice(&self.content.to_le_bytes());
        o[12..16].copy_from_slice(&self.hints.to_le_bytes());
        self.caret.encode(&mut o[16..32]);
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != 32 || w32(b, 8) > PIN || w32(b, 12) & !HINTS != 0 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            window: w64(b, 0),
            content: w32(b, 8),
            hints: w32(b, 12),
            caret: caret(&b[16..32])?,
        })
    }
}
/// A caret rectangle: non-negative size within the 16-bit coordinate space.
fn caret(b: &[u8]) -> Result<Rect, Code> {
    let r = Rect::decode(b)?;
    if r.width > 4096
        || r.height > 4096
        || !(-32768..=32767).contains(&r.x)
        || !(-32768..=32767).contains(&r.y)
    {
        return Err(Code::InvalidRequest);
    }
    Ok(r)
}
/// CURSOR body.
pub fn encode_cursor(window: u64, serial: u32, caret: Rect) -> [u8; 32] {
    let mut o = [0; 32];
    o[..8].copy_from_slice(&window.to_le_bytes());
    o[8..12].copy_from_slice(&serial.to_le_bytes());
    caret.encode(&mut o[16..32]);
    o
}
pub fn decode_cursor(b: &[u8]) -> Result<(u64, u32, Rect), Code> {
    if b.len() != 32 || w32(b, 12) != 0 {
        return Err(Code::InvalidRequest);
    }
    Ok((w64(b, 0), w32(b, 8), caret(&b[16..32])?))
}
/// Surrounding text with byte offsets on character boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surrounding<'a> {
    pub text: &'a str,
    pub cursor: u16,
    pub anchor: u16,
}
impl<'a> Surrounding<'a> {
    fn check(self) -> Result<Self, Code> {
        let (c, a) = (self.cursor as usize, self.anchor as usize);
        if self.text.len() > MAX_SURROUNDING
            || c > self.text.len()
            || a > self.text.len()
            || !self.text.is_char_boundary(c)
            || !self.text.is_char_boundary(a)
        {
            return Err(Code::InvalidRequest);
        }
        Ok(self)
    }
}
/// SURROUNDING body; returns its length.
pub fn encode_surrounding(
    window: u64,
    serial: u32,
    s: Surrounding<'_>,
    out: &mut [u8],
) -> Result<usize, Code> {
    let s = s.check()?;
    let end = 16 + s.text.len();
    if out.len() < end {
        return Err(Code::Resources);
    }
    out[..8].copy_from_slice(&window.to_le_bytes());
    out[8..12].copy_from_slice(&serial.to_le_bytes());
    out[12..14].copy_from_slice(&s.cursor.to_le_bytes());
    out[14..16].copy_from_slice(&s.anchor.to_le_bytes());
    out[16..end].copy_from_slice(s.text.as_bytes());
    Ok(end)
}
pub fn decode_surrounding(b: &[u8]) -> Result<(u64, u32, Surrounding<'_>), Code> {
    if b.len() < 16 {
        return Err(Code::InvalidRequest);
    }
    let s = Surrounding {
        text: text(&b[16..])?,
        cursor: w16(b, 12),
        anchor: w16(b, 14),
    }
    .check()?;
    Ok((w64(b, 0), w32(b, 8), s))
}
/// One preedit attribute range (byte offsets into the preedit text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Attr {
    pub start: u16,
    pub end: u16,
    pub style: u8,
}
/// A composition string. Attribute ranges are ordered, non-overlapping and
/// on character boundaries; `cursor` is a byte offset or `HIDDEN_CURSOR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preedit<'a> {
    pub text: &'a str,
    pub cursor: u16,
    pub attrs: &'a [Attr],
}
impl Preedit<'_> {
    pub fn validate(&self) -> Result<(), Code> {
        let t = self.text;
        if t.len() > MAX_PREEDIT
            || self.attrs.len() > MAX_ATTRS
            || !valid_text(t)
            || t.contains('\n')
            || (self.cursor != HIDDEN_CURSOR
                && (self.cursor as usize > t.len() || !t.is_char_boundary(self.cursor as usize)))
        {
            return Err(Code::InvalidRequest);
        }
        let mut last = 0;
        for a in self.attrs {
            let (s, e) = (a.start as usize, a.end as usize);
            if s < last
                || s >= e
                || e > t.len()
                || !t.is_char_boundary(s)
                || !t.is_char_boundary(e)
                || a.style > STYLE_INPUT_ERROR
            {
                return Err(Code::InvalidRequest);
            }
            last = e;
        }
        Ok(())
    }
}
fn put_attrs(attrs: &[Attr], out: &mut [u8]) {
    for (i, a) in attrs.iter().enumerate() {
        let o = &mut out[i * 6..i * 6 + 6];
        o[..2].copy_from_slice(&a.start.to_le_bytes());
        o[2..4].copy_from_slice(&a.end.to_le_bytes());
        o[4] = a.style;
        o[5] = 0;
    }
}
fn get_attrs(b: &[u8], out: &mut [Attr; MAX_ATTRS]) -> Result<(), Code> {
    for (i, c) in b.chunks_exact(6).enumerate() {
        if c[5] != 0 {
            return Err(Code::InvalidRequest);
        }
        out[i] = Attr {
            start: w16(c, 0),
            end: w16(c, 2),
            style: c[4],
        };
    }
    Ok(())
}
/// IME_PREEDIT body; returns its length.
pub fn encode_ime_preedit(serial: u32, p: Preedit<'_>, out: &mut [u8]) -> Result<usize, Code> {
    p.validate()?;
    let a = 8 + 6 * p.attrs.len();
    let end = a + p.text.len();
    if out.len() < end || end > BODY_BYTES {
        return Err(Code::Resources);
    }
    out[..4].copy_from_slice(&serial.to_le_bytes());
    out[4..6].copy_from_slice(&p.cursor.to_le_bytes());
    out[6] = p.attrs.len() as u8;
    out[7] = 0;
    put_attrs(p.attrs, &mut out[8..a]);
    out[a..end].copy_from_slice(p.text.as_bytes());
    Ok(end)
}
/// Decoded preedit owning its attribute storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreeditBuf<'a> {
    pub text: &'a str,
    pub cursor: u16,
    pub count: usize,
    pub attrs: [Attr; MAX_ATTRS],
}
impl<'a> PreeditBuf<'a> {
    pub fn preedit(&self) -> Preedit<'_> {
        Preedit {
            text: self.text,
            cursor: self.cursor,
            attrs: &self.attrs[..self.count],
        }
    }
}
pub fn decode_ime_preedit(b: &[u8]) -> Result<(u32, PreeditBuf<'_>), Code> {
    if b.len() < 8 || b[7] != 0 || b[6] as usize > MAX_ATTRS {
        return Err(Code::InvalidRequest);
    }
    let count = b[6] as usize;
    let a = 8 + 6 * count;
    if b.len() < a {
        return Err(Code::InvalidRequest);
    }
    let mut attrs = [Attr::default(); MAX_ATTRS];
    get_attrs(&b[8..a], &mut attrs)?;
    let p = PreeditBuf {
        text: text(&b[a..])?,
        cursor: w16(b, 4),
        count,
        attrs,
    };
    p.preedit().validate()?;
    Ok((w32(b, 0), p))
}
pub fn encode_ime_commit(serial: u32, t: &str, out: &mut [u8]) -> Result<usize, Code> {
    if t.is_empty() || t.len() > MAX_COMMIT || !valid_text(t) || out.len() < 8 + t.len() {
        return Err(Code::InvalidRequest);
    }
    out[..4].copy_from_slice(&serial.to_le_bytes());
    out[4..8].fill(0);
    out[8..8 + t.len()].copy_from_slice(t.as_bytes());
    Ok(8 + t.len())
}
pub fn decode_ime_commit(b: &[u8]) -> Result<(u32, &str), Code> {
    if b.len() < 9 || w32(b, 4) != 0 {
        return Err(Code::InvalidRequest);
    }
    let t = text(&b[8..])?;
    if t.len() > MAX_COMMIT || !valid_text(t) {
        return Err(Code::InvalidRequest);
    }
    Ok((w32(b, 0), t))
}
/// Candidate window contents (one page). The window server draws it near
/// the caret of the active context (Windows 11 candidate window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidates<'a> {
    pub serial: u32,
    pub selected: u8,
    pub page: u8,
    pub pages: u8,
    /// Bit 0: vertical list (Japanese); otherwise horizontal (Pinyin).
    pub flags: u8,
    pub items: &'a [&'a str],
}
pub const CANDIDATES_VERTICAL: u8 = 1;
impl Candidates<'_> {
    fn check(&self) -> Result<(), Code> {
        if self.items.len() > MAX_CANDIDATES
            || self.flags & !CANDIDATES_VERTICAL != 0
            || (!self.items.is_empty()
                && (self.selected as usize >= self.items.len()
                    || self.pages == 0
                    || self.page >= self.pages))
            || self.items.iter().any(|s| {
                s.is_empty()
                    || s.len() > MAX_CANDIDATE_BYTES
                    || !valid_text(s)
                    || s.contains(['\n', '\t'])
            })
        {
            return Err(Code::InvalidRequest);
        }
        Ok(())
    }
}
/// IME_CANDIDATES body: `[serial u32][selected u8][count u8][page u8]
/// [pages u8][flags u8][0 u8 ×3]` + count lengths (u8) + strings.
pub fn encode_candidates(c: Candidates<'_>, out: &mut [u8]) -> Result<usize, Code> {
    c.check()?;
    let n = c.items.len();
    let end = 12 + n + c.items.iter().map(|s| s.len()).sum::<usize>();
    if out.len() < end || end > BODY_BYTES {
        return Err(Code::Resources);
    }
    out[..4].copy_from_slice(&c.serial.to_le_bytes());
    out[4] = c.selected;
    out[5] = n as u8;
    out[6] = c.page;
    out[7] = c.pages;
    out[8] = c.flags;
    out[9..12].fill(0);
    let mut at = 12 + n;
    for (i, s) in c.items.iter().enumerate() {
        out[12 + i] = s.len() as u8;
        out[at..at + s.len()].copy_from_slice(s.as_bytes());
        at += s.len();
    }
    Ok(end)
}
/// Decode into `items`; returns the header and item count.
pub fn decode_candidates<'a>(
    b: &'a [u8],
    items: &mut [&'a str; MAX_CANDIDATES],
) -> Result<Candidates<'static>, Code> {
    if b.len() < 12 || b[9..12].iter().any(|&v| v != 0) || b[5] as usize > MAX_CANDIDATES {
        return Err(Code::InvalidRequest);
    }
    let n = b[5] as usize;
    let mut at = 12 + n;
    if b.len() < at {
        return Err(Code::InvalidRequest);
    }
    for i in 0..n {
        let len = b[12 + i] as usize;
        if at + len > b.len() {
            return Err(Code::InvalidRequest);
        }
        items[i] = text(&b[at..at + len])?;
        at += len;
    }
    if at != b.len() {
        return Err(Code::InvalidRequest);
    }
    let header = Candidates {
        serial: w32(b, 0),
        selected: b[4],
        page: b[6],
        pages: b[7],
        flags: b[8],
        items: &[],
    };
    Candidates {
        items: &items[..n],
        ..header
    }
    .check()?;
    Ok(header)
}
/// One key as delivered to an application (`R_KEY`) or the IME (`R_IME_KEY`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Key {
    /// Physical set-1 code (`door_abi::graphics` KEY code).
    pub code: u32,
    /// `door_abi::graphics` KEY flags (DOWN, REPEAT, sided modifiers, locks).
    pub flags: u32,
    /// The legacy window key code for the same key (scan | flags << 9 |
    /// ASCII << 16), so a client can reuse its existing key handling.
    pub legacy: u32,
    /// Windows virtual-key code under the session layout.
    pub vk: u16,
    /// USB HID usage (page 7) of the physical key; 0 if none.
    pub usage: u16,
    /// KEY_* status bits.
    pub status: u8,
    pub timestamp: u64,
    pub text_len: u8,
    pub text: [u8; MAX_KEY_TEXT],
}
impl Key {
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len as usize]).unwrap_or("")
    }
    pub fn down(&self) -> bool {
        self.flags & door_abi::graphics::DOWN != 0
    }
    pub fn with_text(mut self, t: &str) -> Self {
        let n = t.len().min(MAX_KEY_TEXT);
        let n = (0..=n).rev().find(|&i| t.is_char_boundary(i)).unwrap_or(0);
        self.text = [0; MAX_KEY_TEXT];
        self.text[..n].copy_from_slice(&t.as_bytes()[..n]);
        self.text_len = n as u8;
        self
    }
    const BYTES: usize = 32 + MAX_KEY_TEXT;
    fn put(&self, o: &mut [u8]) {
        o[..8].copy_from_slice(&self.timestamp.to_le_bytes());
        o[8..12].copy_from_slice(&self.code.to_le_bytes());
        o[12..16].copy_from_slice(&self.flags.to_le_bytes());
        o[16..20].copy_from_slice(&self.legacy.to_le_bytes());
        o[20..22].copy_from_slice(&self.vk.to_le_bytes());
        o[22..24].copy_from_slice(&self.usage.to_le_bytes());
        o[24] = self.text_len;
        o[25] = self.status;
        o[26..32].fill(0);
        o[32..32 + MAX_KEY_TEXT].copy_from_slice(&self.text);
    }
    fn get(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES
            || b[26..32].iter().any(|&v| v != 0)
            || b[24] as usize > MAX_KEY_TEXT
        {
            return Err(Code::InvalidRequest);
        }
        let mut k = Self {
            timestamp: w64(b, 0),
            code: w32(b, 8),
            flags: w32(b, 12),
            legacy: w32(b, 16),
            vk: w16(b, 20),
            usage: w16(b, 22),
            text_len: b[24],
            status: b[25],
            text: [0; MAX_KEY_TEXT],
        };
        k.text.copy_from_slice(&b[32..32 + MAX_KEY_TEXT]);
        let used = &k.text[..k.text_len as usize];
        if !door_abi::graphics::valid_key_code(k.code)
            || !door_abi::graphics::key_flags_consistent(k.flags)
            || k.status & !(KEY_DEAD | KEY_COMPOSED | KEY_FORWARDED | KEY_ALTGR) != 0
            || k.text[k.text_len as usize..].iter().any(|&v| v != 0)
            || core::str::from_utf8(used).is_err()
        {
            return Err(Code::InvalidRequest);
        }
        Ok(k)
    }
}
/// Records in a POLL reply: `[len u8][kind u8][0 u16]` + payload, packed
/// back to back; `len` counts the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record<'a> {
    Key {
        window: u64,
        key: Key,
    },
    Preedit {
        window: u64,
        serial: u32,
        preedit: PreeditBuf<'a>,
    },
    Commit {
        window: u64,
        serial: u32,
        text: &'a str,
    },
    Delete {
        window: u64,
        serial: u32,
        before: u32,
        after: u32,
    },
    Disabled {
        window: u64,
        serial: u32,
        reason: u32,
    },
    Activate {
        serial: u32,
        content: u32,
        hints: u32,
        method: u32,
        scale: u32,
        caret: Rect,
    },
    Deactivate {
        serial: u32,
    },
    Surrounding {
        serial: u32,
        surrounding: Surrounding<'a>,
    },
    Cursor {
        serial: u32,
        scale: u32,
        caret: Rect,
    },
    ImeKey {
        key_serial: u32,
        serial: u32,
        key: Key,
    },
    Method {
        method: u32,
    },
    Reset {
        serial: u32,
        mode: u32,
    },
}
impl Record<'_> {
    /// Encode at the start of `out`; returns the record length. Never
    /// panics: variable-length payloads are bounds-checked (`Resources`)
    /// and validated (`InvalidRequest`) before they are copied.
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        let mut b = [0u8; MAX_RECORD - RECORD_HEADER];
        let (kind, n) = match *self {
            Record::Key { window, key } => {
                b[..8].copy_from_slice(&window.to_le_bytes());
                key.put(&mut b[8..8 + Key::BYTES]);
                (R_KEY, 8 + Key::BYTES)
            }
            Record::Preedit {
                window,
                serial,
                preedit,
            } => {
                let p = preedit.preedit();
                p.validate()?;
                b[..8].copy_from_slice(&window.to_le_bytes());
                b[8..12].copy_from_slice(&serial.to_le_bytes());
                b[12..14].copy_from_slice(&p.cursor.to_le_bytes());
                b[14] = p.attrs.len() as u8;
                let a = 16 + 6 * p.attrs.len();
                put_attrs(p.attrs, b.get_mut(16..a).ok_or(Code::Resources)?);
                (R_PREEDIT, put(&mut b, a, p.text.as_bytes())?)
            }
            Record::Commit {
                window,
                serial,
                text,
            } => {
                if text.is_empty() || text.len() > MAX_COMMIT || !valid_text(text) {
                    return Err(Code::InvalidRequest);
                }
                b[..8].copy_from_slice(&window.to_le_bytes());
                b[8..12].copy_from_slice(&serial.to_le_bytes());
                (R_COMMIT, put(&mut b, 12, text.as_bytes())?)
            }
            Record::Delete {
                window,
                serial,
                before,
                after,
            } => {
                b[..8].copy_from_slice(&window.to_le_bytes());
                for (i, v) in [serial, before, after].into_iter().enumerate() {
                    b[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
                }
                (R_DELETE, 20)
            }
            Record::Disabled {
                window,
                serial,
                reason,
            } => {
                b[..8].copy_from_slice(&window.to_le_bytes());
                b[8..12].copy_from_slice(&serial.to_le_bytes());
                b[12..16].copy_from_slice(&reason.to_le_bytes());
                (R_DISABLED, 16)
            }
            Record::Activate {
                serial,
                content,
                hints,
                method,
                scale,
                caret,
            } => {
                for (i, v) in [serial, content, hints, method, scale]
                    .into_iter()
                    .enumerate()
                {
                    b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
                }
                caret.encode(&mut b[20..36]);
                (R_ACTIVATE, 36)
            }
            Record::Deactivate { serial } => {
                b[..4].copy_from_slice(&serial.to_le_bytes());
                (R_DEACTIVATE, 4)
            }
            Record::Surrounding {
                serial,
                surrounding,
            } => {
                let s = surrounding.check()?;
                b[..4].copy_from_slice(&serial.to_le_bytes());
                b[4..6].copy_from_slice(&s.cursor.to_le_bytes());
                b[6..8].copy_from_slice(&s.anchor.to_le_bytes());
                (R_SURROUNDING, put(&mut b, 8, s.text.as_bytes())?)
            }
            Record::Cursor {
                serial,
                scale,
                caret,
            } => {
                b[..4].copy_from_slice(&serial.to_le_bytes());
                b[4..8].copy_from_slice(&scale.to_le_bytes());
                caret.encode(&mut b[8..24]);
                (R_CURSOR, 24)
            }
            Record::ImeKey {
                key_serial,
                serial,
                key,
            } => {
                b[..4].copy_from_slice(&key_serial.to_le_bytes());
                b[4..8].copy_from_slice(&serial.to_le_bytes());
                key.put(&mut b[8..8 + Key::BYTES]);
                (R_IME_KEY, 8 + Key::BYTES)
            }
            Record::Method { method } => {
                b[..4].copy_from_slice(&method.to_le_bytes());
                (R_METHOD, 4)
            }
            Record::Reset { serial, mode } => {
                b[..4].copy_from_slice(&serial.to_le_bytes());
                b[4..8].copy_from_slice(&mode.to_le_bytes());
                (R_RESET, 8)
            }
        };
        let len = RECORD_HEADER + n;
        let o = out.get_mut(..len).ok_or(Code::Resources)?;
        o[0] = len as u8;
        o[1] = kind;
        o[2..4].fill(0);
        o[4..].copy_from_slice(&b[..n]);
        Ok(len)
    }
}
/// Copy `bytes` into `b` at `at`; returns the end offset.
fn put(b: &mut [u8], at: usize, bytes: &[u8]) -> Result<usize, Code> {
    let end = at.checked_add(bytes.len()).ok_or(Code::Resources)?;
    b.get_mut(at..end)
        .ok_or(Code::Resources)?
        .copy_from_slice(bytes);
    Ok(end)
}
/// Decode the record at the start of `b`; returns it and its length.
pub fn decode_record(b: &[u8]) -> Result<(Record<'_>, usize), Code> {
    if b.len() < 4 || b[2] != 0 || b[3] != 0 || (b[0] as usize) < 4 || b[0] as usize > b.len() {
        return Err(Code::InvalidRequest);
    }
    let len = b[0] as usize;
    let p = &b[4..len];
    let exact = |n: usize| {
        if p.len() == n {
            Ok(())
        } else {
            Err(Code::InvalidRequest)
        }
    };
    let record = match b[1] {
        R_KEY => {
            exact(8 + Key::BYTES)?;
            Record::Key {
                window: w64(p, 0),
                key: Key::get(&p[8..])?,
            }
        }
        R_PREEDIT => {
            if p.len() < 16
                || p[15] != 0
                || p[14] as usize > MAX_ATTRS
                || p.len() < 16 + 6 * p[14] as usize
            {
                return Err(Code::InvalidRequest);
            }
            let count = p[14] as usize;
            let a = 16 + 6 * count;
            let mut attrs = [Attr::default(); MAX_ATTRS];
            get_attrs(&p[16..a], &mut attrs)?;
            let preedit = PreeditBuf {
                text: text(&p[a..])?,
                cursor: w16(p, 12),
                count,
                attrs,
            };
            preedit.preedit().validate()?;
            Record::Preedit {
                window: w64(p, 0),
                serial: w32(p, 8),
                preedit,
            }
        }
        R_COMMIT => {
            if p.len() < 13 {
                return Err(Code::InvalidRequest);
            }
            let t = text(&p[12..])?;
            if !valid_text(t) {
                return Err(Code::InvalidRequest);
            }
            Record::Commit {
                window: w64(p, 0),
                serial: w32(p, 8),
                text: t,
            }
        }
        R_DELETE => {
            exact(20)?;
            Record::Delete {
                window: w64(p, 0),
                serial: w32(p, 8),
                before: w32(p, 12),
                after: w32(p, 16),
            }
        }
        R_DISABLED => {
            exact(16)?;
            Record::Disabled {
                window: w64(p, 0),
                serial: w32(p, 8),
                reason: w32(p, 12),
            }
        }
        R_ACTIVATE => {
            exact(36)?;
            if w32(p, 4) > PIN || w32(p, 8) & !HINTS != 0 || secure(w32(p, 4)) {
                return Err(Code::InvalidRequest);
            }
            Record::Activate {
                serial: w32(p, 0),
                content: w32(p, 4),
                hints: w32(p, 8),
                method: w32(p, 12),
                scale: w32(p, 16),
                caret: Rect::decode(&p[20..36])?,
            }
        }
        R_DEACTIVATE => {
            exact(4)?;
            Record::Deactivate { serial: w32(p, 0) }
        }
        R_SURROUNDING => {
            if p.len() < 8 {
                return Err(Code::InvalidRequest);
            }
            let s = Surrounding {
                text: text(&p[8..])?,
                cursor: w16(p, 4),
                anchor: w16(p, 6),
            }
            .check()?;
            Record::Surrounding {
                serial: w32(p, 0),
                surrounding: s,
            }
        }
        R_CURSOR => {
            exact(24)?;
            Record::Cursor {
                serial: w32(p, 0),
                scale: w32(p, 4),
                caret: Rect::decode(&p[8..24])?,
            }
        }
        R_IME_KEY => {
            exact(8 + Key::BYTES)?;
            Record::ImeKey {
                key_serial: w32(p, 0),
                serial: w32(p, 4),
                key: Key::get(&p[8..])?,
            }
        }
        R_METHOD => {
            exact(4)?;
            Record::Method { method: w32(p, 0) }
        }
        R_RESET => {
            exact(8)?;
            if w32(p, 4) > RESET_COMPLETE {
                return Err(Code::InvalidRequest);
            }
            Record::Reset {
                serial: w32(p, 0),
                mode: w32(p, 4),
            }
        }
        _ => return Err(Code::InvalidRequest),
    };
    Ok((record, len))
}
/// Iterate the records of a POLL reply; the first malformed record ends
/// iteration with an error.
pub fn records(b: &[u8]) -> impl Iterator<Item = Result<Record<'_>, Code>> {
    let mut at = 0;
    let mut failed = false;
    core::iter::from_fn(move || {
        if failed || at >= b.len() {
            return None;
        }
        match decode_record(&b[at..]) {
            Ok((r, n)) => {
                at += n;
                Some(Ok(r))
            }
            Err(e) => {
                failed = true;
                Some(Err(e))
            }
        }
    })
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    fn rect() -> Rect {
        Rect {
            x: 40,
            y: -8,
            width: 2,
            height: 20,
        }
    }
    #[test]
    fn requests_roundtrip_and_reject_malformed_bodies() {
        let e = Enable {
            window: 9,
            content: URL,
            hints: HINT_PRIVATE,
            caret: rect(),
        };
        assert_eq!(Enable::decode(&e.encode()), Ok(e));
        let mut bad = e.encode();
        bad[8] = 7;
        assert!(Enable::decode(&bad).is_err(), "unknown content type");
        let mut bad = e.encode();
        bad[12] = 8;
        assert!(Enable::decode(&bad).is_err(), "unknown hint");
        let mut bad = e.encode();
        bad[24..28].copy_from_slice(&5000u32.to_le_bytes());
        assert!(Enable::decode(&bad).is_err(), "caret too large");
        assert_eq!(
            decode_cursor(&encode_cursor(3, 4, rect())),
            Ok((3, 4, rect()))
        );
        let mut out = [0u8; BODY_BYTES];
        let s = Surrounding {
            text: "città è",
            cursor: 6,
            anchor: 0,
        };
        let n = encode_surrounding(5, 6, s, &mut out).unwrap();
        assert_eq!(decode_surrounding(&out[..n]), Ok((5, 6, s)));
        assert!(
            encode_surrounding(5, 6, Surrounding { cursor: 5, ..s }, &mut out).is_err(),
            "inside à"
        );
        assert!(encode_surrounding(5, 6, Surrounding { anchor: 99, ..s }, &mut out).is_err());
        let st = State {
            preferences: 0x0100_0071,
            method: 7,
            mode: MODE_NATIVE,
            flags: 1,
        };
        assert_eq!(State::decode(&st.encode()), Ok(st));
    }
    #[test]
    fn preedit_commit_and_candidates_are_validated() {
        let attrs = [
            Attr {
                start: 0,
                end: 6,
                style: STYLE_TARGET_CONVERTED,
            },
            Attr {
                start: 6,
                end: 9,
                style: STYLE_INPUT,
            },
        ];
        let p = Preedit {
            text: "日本ご",
            cursor: 9,
            attrs: &attrs,
        };
        let mut out = [0u8; BODY_BYTES];
        let n = encode_ime_preedit(4, p, &mut out).unwrap();
        let (serial, back) = decode_ime_preedit(&out[..n]).unwrap();
        assert_eq!((serial, back.preedit()), (4, p));
        for bad in [
            Preedit { cursor: 1, ..p },
            Preedit {
                text: "a\nb",
                cursor: 0,
                attrs: &[],
            },
            Preedit {
                attrs: &[attrs[1], attrs[0]],
                ..p
            },
            Preedit {
                attrs: &[Attr {
                    start: 0,
                    end: 2,
                    style: 0,
                }],
                ..p
            },
            Preedit {
                attrs: &[Attr {
                    start: 0,
                    end: 3,
                    style: 9,
                }],
                ..p
            },
        ] {
            assert!(encode_ime_preedit(4, bad, &mut out).is_err(), "{bad:?}");
        }
        let n = encode_ime_commit(4, "你好", &mut out).unwrap();
        assert_eq!(decode_ime_commit(&out[..n]), Ok((4, "你好")));
        assert!(encode_ime_commit(4, "", &mut out).is_err());
        assert!(encode_ime_commit(4, "a\u{7}", &mut out).is_err());
        let items = ["你好", "拟好", "你", "尼"];
        let c = Candidates {
            serial: 4,
            selected: 1,
            page: 0,
            pages: 3,
            flags: 0,
            items: &items,
        };
        let n = encode_candidates(c, &mut out).unwrap();
        {
            let mut slots = [""; MAX_CANDIDATES];
            let h = decode_candidates(&out[..n], &mut slots).unwrap();
            assert_eq!((h.serial, h.selected, h.pages), (4, 1, 3));
            assert_eq!(&slots[..4], &items);
            assert!(
                decode_candidates(&out[..n - 1], &mut slots).is_err(),
                "truncated"
            );
        }
        assert!(encode_candidates(Candidates { selected: 4, ..c }, &mut out).is_err());
        assert!(encode_candidates(Candidates { page: 3, ..c }, &mut out).is_err());
        let mut slots = [""; MAX_CANDIDATES];
        let hide = Candidates {
            items: &[],
            selected: 0,
            page: 0,
            pages: 0,
            ..c
        };
        let n = encode_candidates(hide, &mut out).unwrap();
        assert_eq!(decode_candidates(&out[..n], &mut slots).unwrap().pages, 0);
    }
    #[test]
    fn records_pack_back_to_back_and_reject_corruption() {
        use door_abi::graphics as g;
        let key = Key {
            code: 0x10,
            flags: g::DOWN | g::ALT | g::RIGHT_ALT,
            legacy: 0x10 | (g::RIGHT_ALT << 9) | (u32::from(b'@') << 16),
            vk: 0x51,
            usage: 0x14,
            status: KEY_ALTGR,
            timestamp: 77,
            ..Key::default()
        }
        .with_text("@");
        let attrs = [Attr {
            start: 0,
            end: 2,
            style: STYLE_INPUT,
        }];
        let mut pb = PreeditBuf {
            text: "ni",
            cursor: 2,
            count: 1,
            attrs: [Attr::default(); MAX_ATTRS],
        };
        pb.attrs[0] = attrs[0];
        let all = [
            Record::Key { window: 3, key },
            Record::Preedit {
                window: 3,
                serial: 8,
                preedit: pb,
            },
            Record::Commit {
                window: 3,
                serial: 8,
                text: "你好",
            },
            Record::Delete {
                window: 3,
                serial: 8,
                before: 3,
                after: 0,
            },
            Record::Disabled {
                window: 3,
                serial: 8,
                reason: DISABLED_LOCKED,
            },
            Record::Activate {
                serial: 8,
                content: NORMAL,
                hints: 0,
                method: 7,
                scale: 150,
                caret: rect(),
            },
            Record::Deactivate { serial: 8 },
            Record::Surrounding {
                serial: 8,
                surrounding: Surrounding {
                    text: "ab",
                    cursor: 1,
                    anchor: 1,
                },
            },
            Record::Cursor {
                serial: 8,
                scale: 100,
                caret: rect(),
            },
            Record::ImeKey {
                key_serial: 99,
                serial: 8,
                key,
            },
            Record::Method { method: 9 },
            Record::Reset {
                serial: 8,
                mode: RESET_COMPLETE,
            },
        ];
        let mut buf = [0u8; 1024];
        let mut at = 0;
        for r in &all {
            at += r.encode(&mut buf[at..]).unwrap();
        }
        let back: Vec<_> = records_vec(&buf[..at]);
        assert_eq!(back, all);
        assert_eq!(key.text(), "@");
        // Every byte of corruption in the header is caught.
        let n = all[0].encode(&mut buf).unwrap();
        for i in [0usize, 2, 3] {
            let mut bad = buf;
            bad[i] ^= 0x41;
            assert!(decode_record(&bad[..n]).is_err(), "byte {i}");
        }
        let mut bad = buf;
        bad[1] = 99;
        assert!(decode_record(&bad[..n]).is_err(), "unknown kind");
        // A secure context can never be activated at the IME.
        let mut out = [0u8; 64];
        let n = Record::Activate {
            serial: 1,
            content: PASSWORD,
            hints: 0,
            method: 7,
            scale: 100,
            caret: rect(),
        }
        .encode(&mut out)
        .unwrap();
        assert!(decode_record(&out[..n]).is_err());
        // Key text must be valid UTF-8 within its length; bad flags rejected.
        let n = all[0].encode(&mut buf).unwrap();
        let mut bad = buf;
        bad[4 + 8 + 32] = 0xff;
        assert!(decode_record(&bad[..n]).is_err());
        let mut bad = buf;
        bad[4 + 8 + 12..4 + 8 + 16].copy_from_slice(&g::RIGHT_SHIFT.to_le_bytes());
        assert!(decode_record(&bad[..n]).is_err());
        assert!(matches!(records(&buf[..3]).next(), Some(Err(_))));
        assert_eq!(Key::default().with_text("ééééééééé").text_len, 16);
        assert_eq!(
            Key::default().with_text("日本語日本語").text(),
            "日本語日本",
            "cut on a boundary"
        );
    }
    fn records_vec(b: &[u8]) -> Vec<Record<'_>> {
        records(b).collect::<Result<Vec<_>, _>>().unwrap()
    }
    #[test]
    fn every_valid_preedit_fits_one_record_and_encode_never_panics() {
        let attrs: Vec<Attr> = (0..MAX_ATTRS as u16)
            .map(|i| Attr {
                start: i,
                end: i + 1,
                style: STYLE_INPUT,
            })
            .collect();
        let text = std::string::String::from("x").repeat(MAX_PREEDIT + 8);
        let mut out = [0u8; MAX_RECORD];
        for count in 0..=MAX_ATTRS {
            for len in [
                0,
                1,
                8,
                MAX_PREEDIT - 1,
                MAX_PREEDIT,
                MAX_PREEDIT + 1,
                176,
                180,
            ] {
                let preedit = PreeditBuf {
                    text: &text[..len],
                    cursor: 0,
                    count,
                    attrs: core::array::from_fn(
                        |i| if i < count { attrs[i] } else { Attr::default() },
                    ),
                };
                if count > 0 && len < count {
                    continue;
                }
                let r = Record::Preedit {
                    window: 1,
                    serial: 2,
                    preedit,
                };
                let encoded = r.encode(&mut out);
                assert_eq!(
                    encoded.is_ok(),
                    len <= MAX_PREEDIT,
                    "{count} attrs, {len} B"
                );
                if let Ok(n) = encoded {
                    assert!(n <= MAX_RECORD);
                    assert_eq!(decode_record(&out[..n]).unwrap(), (r, n));
                    let mut body = [0u8; BODY_BYTES];
                    assert!(encode_ime_preedit(2, preedit.preedit(), &mut body).is_ok());
                }
                // A short output buffer is an error, never a panic.
                assert!(r.encode(&mut out[..10]).is_err());
            }
        }
        let commit = std::string::String::from("y").repeat(MAX_COMMIT);
        let r = Record::Commit {
            window: 1,
            serial: 2,
            text: &commit,
        };
        assert_eq!(r.encode(&mut out), Ok(MAX_RECORD));
        let long = std::string::String::from("y").repeat(MAX_COMMIT + 1);
        let too_long = Record::Commit {
            window: 1,
            serial: 2,
            text: &long,
        };
        assert!(too_long.encode(&mut out).is_err());
        let mut body = [0u8; BODY_BYTES];
        assert!(encode_ime_commit(2, &commit, &mut body).is_ok());
        assert!(encode_ime_commit(2, &long, &mut body).is_err());
    }
    #[test]
    fn grapheme_floor_never_splits_clusters() {
        let family = "a👨\u{200d}👩\u{200d}👧";
        let cases: [(&str, usize, usize); 9] = [
            ("abc", 2, 2),
            ("abc", 9, 3),
            ("xe\u{301}y", 3, 1),
            ("ae\u{301}", 2, 1),
            (family, family.len() - 1, 1),
            ("日本語", 7, 6),
            ("🇯🇵🇰🇷", 12, 8),
            ("a🇯🇵", 5, 1),
            ("a👍🏽!", 6, 1),
        ];
        for (s, max, want) in cases {
            assert_eq!(grapheme_floor(s, max), want, "{s:?} at {max}");
        }
        // One cluster longer than the cap: a character boundary.
        let mut long = std::string::String::from("a");
        for _ in 0..100 {
            long.push('\u{301}');
        }
        assert_eq!(grapheme_floor(&long, 50), 49);
        // Leading jamo + vowel + final: one syllable block.
        assert_eq!(grapheme_floor("b\u{1112}\u{1161}\u{11ab}a", 10), 10);
        assert_eq!(grapheme_floor("b\u{1112}\u{1161}\u{11ab}a", 9), 1);
    }
    #[test]
    fn content_policy() {
        assert!(secure(PASSWORD) && secure(PIN) && !secure(NORMAL));
        assert!(latin(URL) && latin(NUMBER) && latin(EMAIL) && !latin(TERMINAL));
        assert!(valid_text("a\tb\nc") && !valid_text("a\u{1b}") && !valid_text("\u{feff}"));
    }
}
