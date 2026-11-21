//! Normalized input. Physical key codes from the Door window protocol are
//! mapped to logical keys here; typed text arrives separately (the kernel's
//! layout translation today, the text-input/IME protocol later), so a
//! hardware key event is never mistaken for a character.
use crate::units::LPoint;
use alloc::string::String;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}
impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ctrl: false,
        alt: false,
    };
    pub const CTRL: Self = Self {
        shift: false,
        ctrl: true,
        alt: false,
    };
    pub const ALT: Self = Self {
        shift: false,
        ctrl: false,
        alt: true,
    };
    pub fn none(self) -> bool {
        !self.shift && !self.ctrl && !self.alt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Tab,
    Enter,
    Escape,
    Space,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1..F12.
    F(u8),
    /// The Application (context menu) key.
    Menu,
    /// A character key by its unshifted US-layout meaning ('a', '1', '/').
    Char(char),
    /// Modifier keys pressed alone.
    Shift,
    Control,
    Alt,
    /// Any other physical key (set-1 scan code, 0x100 = extended).
    Other(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
    /// Text this key typed, if any (after layout translation).
    pub text: Option<char>,
}
impl KeyEvent {
    pub fn new(key: Key, modifiers: Modifiers) -> Self {
        let text = match key {
            Key::Char(c) if !modifiers.ctrl && !modifiers.alt => {
                Some(if modifiers.shift { shifted(c) } else { c })
            }
            Key::Space if !modifiers.ctrl && !modifiers.alt => Some(' '),
            _ => None,
        };
        Self {
            key,
            modifiers,
            text,
        }
    }
    pub fn plain(key: Key) -> Self {
        Self::new(key, Modifiers::NONE)
    }
    /// Decode a Door window-protocol key code: scan code in bits 0..9,
    /// modifiers (`door_abi::graphics::{SHIFT, CONTROL, ALT}`) in bits 9..16,
    /// typed ASCII byte in bits 16..24.
    pub fn from_door(code: u32) -> Self {
        let scan = (code & 511) as u16;
        let mods = (code >> 9) & 127;
        let modifiers = Modifiers {
            shift: mods & 2 != 0,
            ctrl: mods & 4 != 0,
            alt: mods & 8 != 0,
        };
        let typed = (code >> 16) as u8;
        let key = key_for_scan(scan);
        let text = ((32..127).contains(&typed) && !modifiers.ctrl && !modifiers.alt)
            .then_some(typed as char);
        Self {
            key,
            modifiers,
            text,
        }
    }
}
fn shifted(c: char) -> char {
    match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        c => c,
    }
}
const ROW: [(u16, &str); 4] = [
    (2, "1234567890-="),
    (16, "qwertyuiop[]"),
    (30, "asdfghjkl;'`"),
    (44, "zxcvbnm,./"),
];
/// Logical key of a set-1 scan code (0x100 marks the E0 prefix).
pub fn key_for_scan(scan: u16) -> Key {
    match scan {
        1 => Key::Escape,
        14 => Key::Backspace,
        15 => Key::Tab,
        28 | 0x11c => Key::Enter,
        29 | 0x11d => Key::Control,
        42 | 54 => Key::Shift,
        43 => Key::Char('\\'),
        56 | 0x138 => Key::Alt,
        57 => Key::Space,
        59..=68 => Key::F((scan - 58) as u8),
        87 => Key::F(11),
        88 => Key::F(12),
        0x147 => Key::Home,
        0x148 => Key::Up,
        0x149 => Key::PageUp,
        0x14b => Key::Left,
        0x14d => Key::Right,
        0x14f => Key::End,
        0x150 => Key::Down,
        0x151 => Key::PageDown,
        0x152 => Key::Insert,
        0x153 => Key::Delete,
        0x15d => Key::Menu,
        s => ROW
            .iter()
            .find_map(|(first, chars)| {
                s.checked_sub(*first)
                    .and_then(|i| chars.chars().nth(i as usize))
            })
            .map_or(Key::Other(s), Key::Char),
    }
}
/// Scan code for a logical key (tests and injected input).
pub fn scan_for_key(key: Key) -> u16 {
    (0..0x200).find(|s| key_for_scan(*s) == key).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Button {
    #[default]
    Primary,
    Secondary,
    Middle,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pointer {
    /// In the receiving widget's content coordinates.
    pub pos: LPoint,
    pub button: Button,
    pub modifiers: Modifiers,
}

/// IME composition (preedit is never committed text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Composition {
    /// Preedit text and caret (UTF-8 byte offset within it).
    Update {
        text: String,
        caret: usize,
    },
    Commit(String),
    Cancel,
}

/// Routed to widgets; see `EventCx::phase` for capture/target/bubble.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    PointerDown(Pointer),
    PointerUp(Pointer),
    PointerMove(Pointer),
    PointerEnter,
    PointerLeave,
    /// Detents; positive scrolls up/left (away from the user).
    Wheel {
        pos: LPoint,
        delta: i32,
        horizontal: bool,
        modifiers: Modifiers,
    },
    /// Pointer capture ended without a release (window lost focus, a modal
    /// opened, the widget was hidden): cancel any armed state.
    CaptureLost,
    KeyDown(KeyEvent),
    KeyUp(KeyEvent),
    Composition(Composition),
    FocusIn,
    FocusOut,
    /// Light dismiss of this widget's open overlay (click outside, Escape,
    /// window deactivation).
    Dismiss,
    /// A routed command raised by a descendant (`EventCx::raise`), bubbling
    /// to ancestors; unhandled commands reach the application as
    /// `Notify::Command`.
    Command(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Root to target's parent, before the target.
    Capture,
    Target,
    /// Target's parent back to the root.
    Bubble,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn door_key_codes_map_to_logical_keys_and_text() {
        let code = |scan: u32, mods: u32, text: u8| scan | (mods << 9) | (u32::from(text) << 16);
        let e = KeyEvent::from_door(code(30, 2, b'A'));
        assert_eq!(
            (e.key, e.modifiers.shift, e.text),
            (Key::Char('a'), true, Some('A'))
        );
        let e = KeyEvent::from_door(code(46, 4, b'c'));
        assert_eq!(
            (e.key, e.modifiers.ctrl, e.text),
            (Key::Char('c'), true, None)
        );
        assert_eq!(KeyEvent::from_door(0x14b).key, Key::Left);
        assert_eq!(KeyEvent::from_door(15).key, Key::Tab);
        assert_eq!(KeyEvent::from_door(68).key, Key::F(10));
        assert_eq!(KeyEvent::from_door(0x15d).key, Key::Menu);
        assert_eq!(key_for_scan(11), Key::Char('0'));
        assert_eq!(key_for_scan(53), Key::Char('/'));
        assert_eq!(scan_for_key(Key::Char('q')), 16);
        assert_eq!(
            KeyEvent::new(Key::Char('1'), Modifiers::SHIFT).text,
            Some('!')
        );
    }
}
