//! Layout tables and the Windows translation rules: shift states (base,
//! Shift, AltGr, Shift+AltGr), CAPLOK/CAPLOKALTGR Caps Lock attributes,
//! AltGr = Right Alt or Ctrl+Alt (KLLF_ALTGR), dead keys with DEADTRANS-style
//! composition, Num Lock keypad meaning, and Ctrl/Alt chords that produce no
//! text (Windows WM_CHAR control characters and WM_SYSCHAR stay legacy-only).
use crate::physical;
use crate::vk;
use door_abi::graphics as g;
pub const NONE: char = '\0';
/// Caps Lock affects the base/Shift pair (CAPLOK).
pub const CAPLOK: u8 = 1;
/// Caps Lock affects the AltGr/Shift+AltGr pair (CAPLOKALTGR).
pub const CAPLOKALTGR: u8 = 2;
/// One character key: its virtual key, Caps Lock attributes, the character
/// of each shift state and which of those are dead keys (bit = level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDef {
    pub code: u16,
    pub vk: u8,
    pub caps: u8,
    pub dead: u8,
    pub chars: [char; 4],
}
/// A dead key: its spacing character (dead key + Space) and compositions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadKey {
    pub dead: char,
    pub space: char,
    pub pairs: &'static [(char, char)],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub id: u8,
    /// Windows keyboard layout identifier (KLID).
    pub klid: u32,
    pub name: &'static str,
    /// Right Alt is AltGr (and Ctrl+Alt aliases it).
    pub altgr: bool,
    /// Numeric keypad decimal key character.
    pub decimal: char,
    /// Sorted by code.
    pub keys: &'static [KeyDef],
    pub dead: &'static [DeadKey],
    /// Korean 101/103 keyboards: Right Alt = Hangul, Right Ctrl = Hanja.
    pub right_alt_hangul: bool,
    /// Japanese 106/109 keyboard: Hankaku/Zenkaku, Kana, (Non)Convert.
    pub jis: bool,
}
impl Layout {
    pub fn key(&self, code: u32) -> Option<&KeyDef> {
        let code = u16::try_from(code).ok()?;
        self.keys
            .binary_search_by_key(&code, |k| k.code)
            .ok()
            .map(|i| &self.keys[i])
    }
    pub fn dead_key(&self, dead: char) -> Option<&DeadKey> {
        self.dead.iter().find(|d| d.dead == dead)
    }
    /// The composition of a pending dead key with the next character.
    pub fn compose(&self, dead: char, next: char) -> Option<char> {
        self.dead_key(dead)?
            .pairs
            .iter()
            .find(|p| p.0 == next)
            .map(|p| p.1)
    }
    /// Dead key followed by Space.
    pub fn spacing(&self, dead: char) -> char {
        self.dead_key(dead).map_or(dead, |d| d.space)
    }
}
/// Up to four characters produced by one key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Text {
    chars: [char; 4],
    len: u8,
}
impl Text {
    pub fn push(&mut self, c: char) {
        if (self.len as usize) < self.chars.len() {
            self.chars[self.len as usize] = c;
            self.len += 1;
        }
    }
    pub fn chars(&self) -> &[char] {
        &self.chars[..self.len as usize]
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// UTF-8 into `out` (16 bytes always suffice); returns the length.
    pub fn encode(&self, out: &mut [u8; 16]) -> usize {
        let mut n = 0;
        for c in self.chars() {
            n += c.encode_utf8(&mut out[n..]).len();
        }
        n
    }
}
/// Translation status bits (same values as the text-input KEY status).
pub const DEAD: u8 = 1;
pub const COMPOSED: u8 = 2;
pub const ALTGR: u8 = 8;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Translation {
    pub vk: u8,
    pub usage: u16,
    pub text: Text,
    pub status: u8,
    /// Legacy one-byte translation for windows without a text-input
    /// context: printable ASCII, a Ctrl+letter control code, or the
    /// Enter/Tab/Backspace/Escape/Delete codes the US decoder produced.
    pub legacy: u8,
}
/// Per-session keyboard translation state (the pending dead key).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Translator {
    dead: Option<char>,
}
fn control_code(code: u32) -> u8 {
    match code {
        0x1c | 0x11c => 10,
        0x0f => 9,
        0x0e => 8,
        0x01 => 27,
        0x153 => 127,
        _ => 0,
    }
}
fn keypad_text(code: u32, digits: bool, decimal: char) -> Option<char> {
    match code {
        0x37 => Some('*'),
        0x4a => Some('-'),
        0x4e => Some('+'),
        0x135 => Some('/'),
        0x53 if digits => Some(decimal),
        0x47..=0x52 if digits && physical::numpad_navigation(code) => {
            Some(b"789-456+1230"[(code - 0x47) as usize] as char)
        }
        _ => None,
    }
}
impl Translator {
    pub const fn new() -> Self {
        Self { dead: None }
    }
    pub fn pending_dead(&self) -> Option<char> {
        self.dead
    }
    /// Focus changes and device resets drop a pending dead key.
    pub fn reset(&mut self) {
        self.dead = None;
    }
    pub fn translate(&mut self, layout: &Layout, code: u32, flags: u32) -> Translation {
        let phys = physical::lookup(code);
        let shift = flags & g::SHIFT != 0;
        let caps = flags & g::CAPS_LOCK != 0;
        let sided = |any: u32, left: u32, right: u32| {
            flags & any != 0 && (flags & left != 0 || flags & (left | right) == 0)
        };
        // Korean Type 1: the right-hand keys are Hangul/Hanja, not modifiers.
        let (alt_held, ctrl_held) = if layout.right_alt_hangul {
            (
                sided(g::ALT, g::LEFT_ALT, g::RIGHT_ALT),
                sided(g::CONTROL, g::LEFT_CONTROL, g::RIGHT_CONTROL),
            )
        } else {
            (flags & g::ALT != 0, flags & g::CONTROL != 0)
        };
        let altgr = layout.altgr && (flags & g::RIGHT_ALT != 0 || alt_held && ctrl_held);
        let ctrl = ctrl_held && !altgr;
        let alt = alt_held && !altgr;
        let def = layout.key(code);
        let mut vk = def.map(|d| d.vk).or(phys.map(|p| p.vk)).unwrap_or(0);
        let digits = flags & g::NUM_LOCK != 0 && !shift;
        if physical::numpad_navigation(code) && !digits {
            vk = physical::numpad_off_vk(code);
        }
        if layout.right_alt_hangul {
            match code {
                0x138 => vk = vk::HANGUL,
                0x11d => vk = vk::HANJA,
                _ => (),
            }
        }
        if layout.jis && code == 0x29 {
            vk = vk::KANJI;
        }
        let mut out = Translation {
            vk,
            usage: phys.map_or(0, |p| p.usage),
            ..Translation::default()
        };
        if flags & g::DOWN == 0 {
            return out;
        }
        let produced = if let Some(d) = def {
            let (band, capsable) = if altgr {
                (2, d.caps & CAPLOKALTGR != 0)
            } else {
                (0, d.caps & CAPLOK != 0)
            };
            let level = band + usize::from(shift ^ (caps && capsable));
            let ch = d.chars[level];
            (ch != NONE).then_some((ch, d.dead & (1 << level) != 0))
        } else if altgr {
            None
        } else {
            keypad_text(code, digits, layout.decimal).map(|c| (c, false))
        };
        if ctrl || alt {
            // Chords produce no text; the legacy byte keeps the US decoder's
            // convention (Ctrl+letter control codes, Alt+key its character).
            let base = def.map_or(NONE, |d| d.chars[0]);
            out.legacy = if ctrl && !alt && base.is_ascii_lowercase() {
                base as u8 - b'a' + 1
            } else if ctrl {
                control_code(code)
            } else {
                produced
                    .filter(|p| !p.1 && p.0.is_ascii_graphic() || p.0 == ' ')
                    .map_or(control_code(code), |p| p.0 as u8)
            };
            return out;
        }
        match produced {
            None => {
                out.legacy = control_code(code);
                if out.legacy != 0 {
                    // Escape, Enter, Tab and Backspace cancel a dead key.
                    self.dead = None;
                }
            }
            Some((ch, is_dead)) => {
                if let Some(prev) = self.dead.take() {
                    if let Some(c) = layout.compose(prev, ch) {
                        out.text.push(c);
                        out.status |= COMPOSED;
                    } else if ch == ' ' && !is_dead {
                        out.text.push(layout.spacing(prev));
                    } else {
                        out.text.push(layout.spacing(prev));
                        out.text.push(if is_dead { layout.spacing(ch) } else { ch });
                    }
                } else if is_dead {
                    self.dead = Some(ch);
                    out.status |= DEAD;
                } else {
                    out.text.push(ch);
                }
                if altgr && !out.text.is_empty() {
                    out.status |= ALTGR;
                }
                out.legacy = out
                    .text
                    .chars()
                    .last()
                    .filter(|c| c.is_ascii_graphic() || **c == ' ')
                    .map_or(0, |c| *c as u8);
            }
        }
        out
    }
}
/// Table helpers. `l` is a letter key (CAPLOK, VK = upper case).
pub const fn l(code: u16, c: char) -> KeyDef {
    KeyDef {
        code,
        vk: c.to_ascii_uppercase() as u8,
        caps: CAPLOK,
        dead: 0,
        chars: [c, c.to_ascii_uppercase(), NONE, NONE],
    }
}
/// A letter with AltGr characters (and CAPLOKALTGR when both are letters).
pub const fn la(code: u16, c: char, g0: char, g1: char) -> KeyDef {
    let caps = if g1 != NONE && g0 != g1 {
        CAPLOK | CAPLOKALTGR
    } else {
        CAPLOK
    };
    KeyDef {
        code,
        vk: c.to_ascii_uppercase() as u8,
        caps,
        dead: 0,
        chars: [c, c.to_ascii_uppercase(), g0, g1],
    }
}
/// A letter key carrying another letter's VK (AZERTY/QWERTZ moves).
pub const fn lv(code: u16, c: char) -> KeyDef {
    l(code, c)
}
/// A symbol key with explicit VK and characters.
pub const fn s(code: u16, vk: u8, chars: [char; 4]) -> KeyDef {
    KeyDef {
        code,
        vk,
        caps: 0,
        dead: 0,
        chars,
    }
}
/// A symbol key whose shift states follow Caps Lock (CAPLOK).
pub const fn sc(code: u16, vk: u8, chars: [char; 4]) -> KeyDef {
    KeyDef {
        code,
        vk,
        caps: CAPLOK,
        dead: 0,
        chars,
    }
}
/// A key with dead characters at the levels in `dead`.
pub const fn d(code: u16, vk: u8, dead: u8, chars: [char; 4]) -> KeyDef {
    KeyDef {
        code,
        vk,
        caps: 0,
        dead,
        chars,
    }
}
