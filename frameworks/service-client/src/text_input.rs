//! door.text-input v1 client (allocation-free, `text-input-v1.md`).
//!
//! Applications: request `ti::CAP_TEXT_INPUT` with
//! `WindowClient::connect_with(registry, ti::CAP_TEXT_INPUT)`, call
//! `text_input_enable` when a text field takes focus, and on every
//! `ti::PENDING` window event `text_input_poll` and apply the records to the
//! field (`TextField` is the reference integration). The IME process uses
//! the `ime_*` calls. Init applies a user's input methods with
//! `apply_input_methods`.
use crate::window::WindowClient;
use crate::{Connection, Error, Result};
use door_abi::Handle;
use door_service_protocol::{
    BODY_BYTES, Code,
    text_input::{self as ti, Record},
    window::{Event, Rect},
};
/// PENDING events for a client that negotiated CAP_TEXT_INPUT or CAP_IME.
pub(crate) fn decode_event(b: &[u8], capabilities: u32) -> Option<Event> {
    if capabilities & (ti::CAP_TEXT_INPUT | ti::CAP_IME) != 0 {
        return ti::decode_event(b).ok();
    }
    None
}
/// Init (session 0): apply the session user's persisted input methods.
pub fn apply_input_methods(port: Handle, preferences: u32) -> Result<()> {
    let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
    c.call(ti::SET_PREFERENCES, &preferences.to_le_bytes(), &[])?
        .empty()
}
fn words<const N: usize>(values: [u32; N]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (i, v) in values.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}
impl WindowClient {
    pub fn supports_text_input(&self) -> bool {
        self.capabilities & (ti::CAP_TEXT_INPUT | ti::CAP_IME) != 0
    }
    fn ti_call(&mut self, op: u32, body: &[u8]) -> Result<crate::rpc::Reply> {
        self.connection.call(op, body, &[])
    }
    /// A text field of `window` took focus. Returns the context serial.
    pub fn text_input_enable(
        &mut self,
        window: u64,
        content: u32,
        hints: u32,
        caret: Rect,
    ) -> Result<u32> {
        let body = ti::Enable {
            window,
            content,
            hints,
            caret,
        }
        .encode();
        u32::try_from(self.ti_call(ti::ENABLE, &body)?.scalar()?).map_err(|_| Error::Malformed)
    }
    pub fn text_input_disable(&mut self, window: u64) -> Result<()> {
        self.ti_call(ti::DISABLE, &window.to_le_bytes())?.empty()
    }
    /// Caret rectangle in content-local physical pixels.
    pub fn text_input_cursor(&mut self, window: u64, serial: u32, caret: Rect) -> Result<()> {
        self.ti_call(ti::CURSOR, &ti::encode_cursor(window, serial, caret))?
            .empty()
    }
    /// Never call for secure fields (the server refuses it).
    pub fn text_input_surrounding(
        &mut self,
        window: u64,
        serial: u32,
        s: ti::Surrounding<'_>,
    ) -> Result<()> {
        let mut b = [0u8; BODY_BYTES];
        let n = ti::encode_surrounding(window, serial, s, &mut b).map_err(Error::Service)?;
        self.ti_call(ti::SURROUNDING, &b[..n])?.empty()
    }
    pub fn text_input_reset(&mut self, window: u64, serial: u32, mode: u32) -> Result<()> {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&window.to_le_bytes());
        b[8..12].copy_from_slice(&serial.to_le_bytes());
        b[12..].copy_from_slice(&mode.to_le_bytes());
        self.ti_call(ti::RESET, &b)?.empty()
    }
    /// Queued records (`ti::records`) into `out`; 0 when none are pending.
    pub fn text_input_poll(&mut self, out: &mut [u8; BODY_BYTES]) -> Result<usize> {
        match self.ti_call(ti::POLL, &[]) {
            Ok(r) => {
                let data = r.data()?;
                if data.is_empty() || ti::records(data).any(|r| r.is_err()) {
                    return Err(Error::Malformed);
                }
                out[..data.len()].copy_from_slice(data);
                Ok(data.len())
            }
            Err(Error::Service(Code::WouldBlock)) => Ok(0),
            Err(e) => Err(e),
        }
    }
    /// Installed input methods, current method and IME mode.
    pub fn input_state(&mut self) -> Result<ti::State> {
        let r = self.ti_call(ti::STATE, &[])?;
        ti::State::decode(r.data()?).map_err(|_| Error::Malformed)
    }
    /// Settings > Time & language > Language & region > Keyboard (workspace
    /// apps): `door_keyboard_layouts::Preferences` bits.
    pub fn set_input_methods(&mut self, preferences: u32) -> Result<()> {
        self.ti_call(ti::SET_METHODS, &preferences.to_le_bytes())?
            .empty()
    }
    /// Taskbar input indicator flyout / Settings: switch to method `index`.
    pub fn select_input_method(&mut self, index: u32) -> Result<()> {
        self.ti_call(ti::SELECT, &index.to_le_bytes())?.empty()
    }
    // --- IME process ------------------------------------------------------
    /// Register as the session IME for `methods` (bit n = method id n).
    pub fn ime_register(&mut self, methods: u32) -> Result<u32> {
        let r = self.ti_call(ti::IME_REGISTER, &words([ti::VERSION, methods])[..8])?;
        u32::try_from(r.scalar()?).map_err(|_| Error::Malformed)
    }
    pub fn ime_preedit(&mut self, serial: u32, p: ti::Preedit<'_>) -> Result<()> {
        let mut b = [0u8; BODY_BYTES];
        let n = ti::encode_ime_preedit(serial, p, &mut b).map_err(Error::Service)?;
        self.ti_call(ti::IME_PREEDIT, &b[..n])?.empty()
    }
    /// Commit text, split on grapheme-cluster boundaries into protocol
    /// chunks (one COMMIT record each).
    pub fn ime_commit(&mut self, serial: u32, text: &str) -> Result<()> {
        let mut rest = text;
        while !rest.is_empty() {
            let n = ti::grapheme_floor(rest, ti::MAX_COMMIT);
            let mut b = [0u8; BODY_BYTES];
            let len = ti::encode_ime_commit(serial, &rest[..n], &mut b).map_err(Error::Service)?;
            self.ti_call(ti::IME_COMMIT, &b[..len])?.empty()?;
            rest = &rest[n..];
        }
        Ok(())
    }
    pub fn ime_delete(&mut self, serial: u32, before: u32, after: u32) -> Result<()> {
        self.ti_call(ti::IME_DELETE, &words([serial, before, after])[..12])?
            .empty()
    }
    pub fn ime_key_done(&mut self, key_serial: u32, forward: bool) -> Result<()> {
        let verdict = if forward { ti::FORWARD } else { ti::CONSUMED };
        self.ti_call(ti::IME_KEY_DONE, &words([key_serial, verdict])[..8])?
            .empty()
    }
    pub fn ime_candidates(&mut self, c: ti::Candidates<'_>) -> Result<()> {
        let mut b = [0u8; BODY_BYTES];
        let n = ti::encode_candidates(c, &mut b).map_err(Error::Service)?;
        self.ti_call(ti::IME_CANDIDATES, &b[..n])?.empty()
    }
    pub fn ime_mode(&mut self, mode: u32) -> Result<()> {
        self.ti_call(ti::IME_MODE, &mode.to_le_bytes())?.empty()
    }
}
/// What applying a record changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Not for this field (another window or a stale serial).
    Ignored,
    /// Text, caret or preedit changed: repaint and report the caret.
    Edited,
    /// A key the field does not handle (shortcuts, Escape, Tab ...).
    Unhandled(ti::Key),
    /// The server disabled the context: enable again on next focus.
    Disabled,
}
/// Reference integration: a single-line field over a fixed buffer that
/// adopts door.text-input. Keys insert their layout text, editing keys use
/// Windows virtual keys, preedit is shown at the caret and commits replace
/// the selection. Offsets are UTF-8 byte offsets on character boundaries.
pub struct TextField<const N: usize> {
    pub window: u64,
    pub serial: u32,
    pub content: u32,
    text: [u8; N],
    len: usize,
    cursor: usize,
    anchor: usize,
    preedit: [u8; ti::MAX_PREEDIT],
    preedit_len: usize,
    preedit_cursor: u16,
}
const VK_BACK: u16 = 0x08;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_RIGHT: u16 = 0x27;
const VK_DELETE: u16 = 0x2e;
impl<const N: usize> TextField<N> {
    pub const fn new(window: u64, content: u32) -> Self {
        Self {
            window,
            serial: 0,
            content,
            text: [0; N],
            len: 0,
            cursor: 0,
            anchor: 0,
            preedit: [0; ti::MAX_PREEDIT],
            preedit_len: 0,
            preedit_cursor: 0,
        }
    }
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.text[..self.len]).unwrap_or("")
    }
    pub fn preedit(&self) -> &str {
        core::str::from_utf8(&self.preedit[..self.preedit_len]).unwrap_or("")
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }
    /// Text as displayed: the preedit inserted at the caret (password fields
    /// show bullets, never the characters).
    pub fn display<'a>(&self, out: &'a mut [u8]) -> &'a str {
        let mut n = 0;
        let mut put = |s: &[u8]| {
            let m = s.len().min(out.len() - n);
            out[n..n + m].copy_from_slice(&s[..m]);
            n += m;
        };
        if ti::secure(self.content) {
            for _ in self.text().chars() {
                put("●".as_bytes());
            }
        } else {
            put(&self.text[..self.cursor]);
            put(&self.preedit[..self.preedit_len]);
            put(&self.text[self.cursor..self.len]);
        }
        let mut end = n;
        while core::str::from_utf8(&out[..end]).is_err() {
            end -= 1;
        }
        core::str::from_utf8(&out[..end]).unwrap()
    }
    /// Surrounding text for the IME: at most `MAX_SURROUNDING` bytes around
    /// the caret on character boundaries (None for secure fields).
    pub fn surrounding(&self) -> Option<ti::Surrounding<'_>> {
        if ti::secure(self.content) {
            return None;
        }
        let half = ti::MAX_SURROUNDING / 2;
        let mut start = self.cursor.saturating_sub(half);
        let t = self.text();
        while !t.is_char_boundary(start) {
            start += 1;
        }
        let mut end = (start + ti::MAX_SURROUNDING).min(self.len);
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        let clamp = |o: usize| (o.clamp(start, end) - start) as u16;
        Some(ti::Surrounding {
            text: &t[start..end],
            cursor: clamp(self.cursor),
            anchor: clamp(self.anchor),
        })
    }
    fn replace_selection(&mut self, s: &str) -> bool {
        let (a, b) = self.selection();
        if self.len - (b - a) + s.len() > N {
            return false;
        }
        self.text.copy_within(b..self.len, a + s.len());
        self.text[a..a + s.len()].copy_from_slice(s.as_bytes());
        self.len = self.len - (b - a) + s.len();
        self.cursor = a + s.len();
        self.anchor = self.cursor;
        true
    }
    fn step(&self, from: usize, right: bool) -> usize {
        let t = self.text();
        if right {
            t[from..]
                .chars()
                .next()
                .map_or(from, |c| from + c.len_utf8())
        } else {
            t[..from]
                .chars()
                .next_back()
                .map_or(from, |c| from - c.len_utf8())
        }
    }
    fn delete_range(&mut self, a: usize, b: usize) {
        self.text.copy_within(b..self.len, a);
        self.len -= b - a;
        self.cursor = a;
        self.anchor = a;
    }
    pub fn apply(&mut self, record: &Record<'_>) -> Applied {
        use door_abi::graphics as g;
        match *record {
            Record::Key { window, key } if window == self.window => {
                if !key.down() {
                    return Applied::Unhandled(key);
                }
                let chord =
                    key.flags & (g::CONTROL | g::ALT) != 0 && key.status & ti::KEY_ALTGR == 0;
                let shift = key.flags & g::SHIFT != 0;
                if !chord && !key.text().is_empty() {
                    return if self.replace_selection(key.text()) {
                        Applied::Edited
                    } else {
                        Applied::Ignored
                    };
                }
                let (a, b) = self.selection();
                match key.vk {
                    VK_BACK if a != b => self.delete_range(a, b),
                    VK_BACK => self.delete_range(self.step(a, false), a),
                    VK_DELETE if a != b => self.delete_range(a, b),
                    VK_DELETE => self.delete_range(a, self.step(a, true)),
                    VK_LEFT | VK_RIGHT | VK_HOME | VK_END => {
                        self.cursor = match key.vk {
                            VK_LEFT => self.step(self.cursor, false),
                            VK_RIGHT => self.step(self.cursor, true),
                            VK_HOME => 0,
                            _ => self.len,
                        };
                        if !shift {
                            self.anchor = self.cursor;
                        }
                    }
                    _ => return Applied::Unhandled(key),
                }
                Applied::Edited
            }
            Record::Preedit {
                window,
                serial,
                preedit,
            } if window == self.window && serial == self.serial => {
                let p = preedit.text.as_bytes();
                self.preedit[..p.len()].copy_from_slice(p);
                self.preedit_len = p.len();
                self.preedit_cursor = preedit.cursor;
                Applied::Edited
            }
            Record::Commit {
                window,
                serial,
                text,
            } if window == self.window && serial == self.serial => {
                self.preedit_len = 0;
                self.replace_selection(text);
                Applied::Edited
            }
            Record::Delete {
                window,
                serial,
                before,
                after,
            } if window == self.window && serial == self.serial => {
                let t = self.text();
                let mut a = self.cursor.saturating_sub(before as usize);
                while !t.is_char_boundary(a) {
                    a -= 1;
                }
                let mut b = (self.cursor + after as usize).min(self.len);
                while !t.is_char_boundary(b) {
                    b += 1;
                }
                self.delete_range(a, b);
                Applied::Edited
            }
            Record::Disabled { window, serial, .. }
                if window == self.window && serial == self.serial =>
            {
                self.preedit_len = 0;
                self.serial = 0;
                Applied::Disabled
            }
            _ => Applied::Ignored,
        }
    }
    /// Caret position within `display()` (after the preedit caret).
    pub fn display_caret(&self) -> usize {
        let pc = if self.preedit_cursor == ti::HIDDEN_CURSOR {
            self.preedit_len
        } else {
            self.preedit_cursor as usize
        };
        self.cursor + if self.preedit_len > 0 { pc } else { 0 }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_abi::graphics as g;
    fn key(vk: u16, text: &str, flags: u32) -> ti::Key {
        ti::Key {
            code: 0x1e,
            flags: flags | g::DOWN,
            vk,
            ..ti::Key::default()
        }
        .with_text(text)
    }
    #[test]
    fn reference_field_applies_keys_preedit_commit_and_delete() {
        let mut f = TextField::<64>::new(3, ti::NORMAL);
        f.serial = 7;
        for t in ["c", "i", "t", "t", "à"] {
            assert_eq!(
                f.apply(&Record::Key {
                    window: 3,
                    key: key(0x41, t, 0)
                }),
                Applied::Edited
            );
        }
        assert_eq!(f.text(), "città");
        f.apply(&Record::Key {
            window: 3,
            key: key(VK_LEFT, "", 0),
        });
        assert_eq!(f.cursor(), 4, "one character, two bytes");
        let mut pb = ti::PreeditBuf {
            text: "ni",
            cursor: 2,
            count: 0,
            attrs: [ti::Attr::default(); ti::MAX_ATTRS],
        };
        f.apply(&Record::Preedit {
            window: 3,
            serial: 7,
            preedit: pb,
        });
        let mut out = [0u8; 64];
        assert_eq!((f.display(&mut out), f.display_caret()), ("cittnià", 6));
        pb.text = "x";
        assert_eq!(
            f.apply(&Record::Preedit {
                window: 3,
                serial: 8,
                preedit: pb
            }),
            Applied::Ignored,
            "stale serial"
        );
        f.apply(&Record::Commit {
            window: 3,
            serial: 7,
            text: "你",
        });
        assert_eq!((f.text(), f.preedit()), ("citt你à", ""));
        f.apply(&Record::Delete {
            window: 3,
            serial: 7,
            before: 3,
            after: 0,
        });
        assert_eq!(f.text(), "città");
        // Ctrl chords are left to the application; AltGr text types.
        let ctrl = key(0x41, "", g::CONTROL | g::LEFT_CONTROL);
        assert_eq!(
            f.apply(&Record::Key {
                window: 3,
                key: ctrl
            }),
            Applied::Unhandled(ctrl)
        );
        let altgr = ti::Key {
            status: ti::KEY_ALTGR,
            ..key(0x51, "@", g::ALT | g::RIGHT_ALT)
        };
        f.apply(&Record::Key {
            window: 3,
            key: altgr,
        });
        assert_eq!(f.text(), "citt@à");
        f.apply(&Record::Key {
            window: 3,
            key: key(VK_HOME, "", g::SHIFT | g::LEFT_SHIFT),
        });
        f.apply(&Record::Key {
            window: 3,
            key: key(VK_BACK, "", 0),
        });
        assert_eq!(f.text(), "à");
        let s = f.surrounding().unwrap();
        assert_eq!((s.text, s.cursor), ("à", 0));
        assert_eq!(
            f.apply(&Record::Disabled {
                window: 3,
                serial: 7,
                reason: 1
            }),
            Applied::Disabled
        );
    }
    #[test]
    fn password_fields_expose_no_surrounding_text_and_mask_display() {
        let mut f = TextField::<32>::new(1, ti::PASSWORD);
        f.apply(&Record::Key {
            window: 1,
            key: key(0x41, "pw", 0),
        });
        assert!(f.surrounding().is_none());
        let mut out = [0u8; 32];
        assert_eq!(f.display(&mut out), "●●");
        assert_eq!(
            f.apply(&Record::Key {
                window: 2,
                key: key(0x41, "x", 0)
            }),
            Applied::Ignored
        );
        let e = Event {
            kind: ti::PENDING,
            a: 2,
            ..Event::default()
        };
        assert_eq!(decode_event(&e.encode(), ti::CAP_TEXT_INPUT), Some(e));
        assert_eq!(
            decode_event(&e.encode(), 0),
            None,
            "only for negotiated clients"
        );
    }
}
