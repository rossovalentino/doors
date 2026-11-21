//! Keyboard: physical set-1 scan codes to virtual keys (US layout, the
//! profile's `MapVirtualKeyW`), character translation for `TranslateMessage`
//! (`ToUnicode` on the US layout), the per-thread key state of
//! `GetKeyState`, and accelerator matching for `TranslateAcceleratorW`.
use crate::msg::{fvirt, wm};
use crate::resource::Accel;

pub mod vk {
    pub const BACK: u8 = 0x08;
    pub const TAB: u8 = 0x09;
    pub const CLEAR: u8 = 0x0c;
    pub const RETURN: u8 = 0x0d;
    pub const SHIFT: u8 = 0x10;
    pub const CONTROL: u8 = 0x11;
    pub const MENU: u8 = 0x12;
    pub const PAUSE: u8 = 0x13;
    pub const CAPITAL: u8 = 0x14;
    pub const ESCAPE: u8 = 0x1b;
    pub const SPACE: u8 = 0x20;
    pub const PRIOR: u8 = 0x21;
    pub const NEXT: u8 = 0x22;
    pub const END: u8 = 0x23;
    pub const HOME: u8 = 0x24;
    pub const LEFT: u8 = 0x25;
    pub const UP: u8 = 0x26;
    pub const RIGHT: u8 = 0x27;
    pub const DOWN: u8 = 0x28;
    pub const INSERT: u8 = 0x2d;
    pub const DELETE: u8 = 0x2e;
    pub const LWIN: u8 = 0x5b;
    pub const RWIN: u8 = 0x5c;
    pub const APPS: u8 = 0x5d;
    pub const NUMPAD0: u8 = 0x60;
    pub const MULTIPLY: u8 = 0x6a;
    pub const ADD: u8 = 0x6b;
    pub const SUBTRACT: u8 = 0x6d;
    pub const DECIMAL: u8 = 0x6e;
    pub const DIVIDE: u8 = 0x6f;
    pub const F1: u8 = 0x70;
    pub const F4: u8 = 0x73;
    pub const F5: u8 = 0x74;
    pub const F10: u8 = 0x79;
    pub const F11: u8 = 0x7a;
    pub const F12: u8 = 0x7b;
    pub const NUMLOCK: u8 = 0x90;
    pub const SCROLL: u8 = 0x91;
    pub const LSHIFT: u8 = 0xa0;
    pub const RSHIFT: u8 = 0xa1;
    pub const LCONTROL: u8 = 0xa2;
    pub const RCONTROL: u8 = 0xa3;
    pub const LMENU: u8 = 0xa4;
    pub const RMENU: u8 = 0xa5;
    pub const OEM_1: u8 = 0xba;
    pub const OEM_PLUS: u8 = 0xbb;
    pub const OEM_COMMA: u8 = 0xbc;
    pub const OEM_MINUS: u8 = 0xbd;
    pub const OEM_PERIOD: u8 = 0xbe;
    pub const OEM_2: u8 = 0xbf;
    pub const OEM_3: u8 = 0xc0;
    pub const OEM_4: u8 = 0xdb;
    pub const OEM_5: u8 = 0xdc;
    pub const OEM_6: u8 = 0xdd;
    pub const OEM_7: u8 = 0xde;
    pub const OEM_102: u8 = 0xe2;
}

/// Set-1 make codes 0x00..0x58 (no prefix) to virtual keys.
const BASE: [u8; 0x59] = [
    0,
    vk::ESCAPE,
    b'1',
    b'2',
    b'3',
    b'4',
    b'5',
    b'6',
    b'7',
    b'8',
    b'9',
    b'0',
    vk::OEM_MINUS,
    vk::OEM_PLUS,
    vk::BACK,
    vk::TAB,
    b'Q',
    b'W',
    b'E',
    b'R',
    b'T',
    b'Y',
    b'U',
    b'I',
    b'O',
    b'P',
    vk::OEM_4,
    vk::OEM_6,
    vk::RETURN,
    vk::LCONTROL,
    b'A',
    b'S',
    b'D',
    b'F',
    b'G',
    b'H',
    b'J',
    b'K',
    b'L',
    vk::OEM_1,
    vk::OEM_7,
    vk::OEM_3,
    vk::LSHIFT,
    vk::OEM_5,
    b'Z',
    b'X',
    b'C',
    b'V',
    b'B',
    b'N',
    b'M',
    vk::OEM_COMMA,
    vk::OEM_PERIOD,
    vk::OEM_2,
    vk::RSHIFT,
    vk::MULTIPLY,
    vk::LMENU,
    vk::SPACE,
    vk::CAPITAL,
    vk::F1,
    0x71,
    0x72,
    vk::F4,
    vk::F5,
    0x75,
    0x76,
    0x77,
    0x78,
    vk::F10,
    vk::NUMLOCK,
    vk::SCROLL,
    vk::HOME,
    vk::UP,
    vk::PRIOR,
    vk::SUBTRACT,
    vk::LEFT,
    vk::CLEAR,
    vk::RIGHT,
    vk::ADD,
    vk::END,
    vk::DOWN,
    vk::NEXT,
    vk::INSERT,
    vk::DELETE,
    0,
    0,
    vk::OEM_102,
    vk::F11,
    vk::F12,
];

/// Virtual key of a physical code (`0x100 |` marks an E0 prefix), with
/// left/right modifiers distinguished (MAPVK_VSC_TO_VK_EX).
pub fn scan_to_vk_ex(code: u16) -> u8 {
    if code & 0x100 != 0 {
        return match code & 0xff {
            0x1c => vk::RETURN,
            0x1d => vk::RCONTROL,
            0x35 => vk::DIVIDE,
            0x38 => vk::RMENU,
            0x47 => vk::HOME,
            0x48 => vk::UP,
            0x49 => vk::PRIOR,
            0x4b => vk::LEFT,
            0x4d => vk::RIGHT,
            0x4f => vk::END,
            0x50 => vk::DOWN,
            0x51 => vk::NEXT,
            0x52 => vk::INSERT,
            0x53 => vk::DELETE,
            0x5b => vk::LWIN,
            0x5c => vk::RWIN,
            0x5d => vk::APPS,
            _ => 0,
        };
    }
    BASE.get(code as usize).copied().unwrap_or(0)
}
/// The key transition a keyboard input message carries: the sided virtual
/// key from its scan code and extended bit (the generic wParam key when the
/// scan code maps to none) and whether it is a press. user32 applies it to
/// the retrieving thread's synchronous key state (`GetKeyState`) when the
/// message is retrieved, not when the device event arrives.
pub fn transition(message: u32, wparam: u64, lparam: i64) -> Option<(u8, bool)> {
    let down = match message {
        wm::KEYDOWN | wm::SYSKEYDOWN => true,
        wm::KEYUP | wm::SYSKEYUP => false,
        _ => return None,
    };
    let scan = ((lparam >> 16) & 0xff) as u16 | (((lparam >> 24) & 1) as u16) << 8;
    let generic_key = (wparam & 0xff) as u8;
    let sided = scan_to_vk_ex(scan);
    let key = if sided != 0 && generic(sided) == generic_key {
        sided
    } else {
        generic_key
    };
    (key != 0).then_some((key, down))
}
/// MAPVK_VSC_TO_VK: sided modifiers fold into SHIFT/CONTROL/MENU.
pub fn scan_to_vk(code: u16) -> u8 {
    generic(scan_to_vk_ex(code))
}
pub fn generic(key: u8) -> u8 {
    match key {
        vk::LSHIFT | vk::RSHIFT => vk::SHIFT,
        vk::LCONTROL | vk::RCONTROL => vk::CONTROL,
        vk::LMENU | vk::RMENU => vk::MENU,
        k => k,
    }
}
/// MAPVK_VK_TO_VSC: the unprefixed code (extended keys report their make
/// code without the E0 byte, as Windows does).
pub fn vk_to_scan(key: u8) -> u16 {
    let key = match key {
        vk::SHIFT => vk::LSHIFT,
        vk::CONTROL => vk::LCONTROL,
        vk::MENU => vk::LMENU,
        vk::DIVIDE => return 0x35,
        vk::RCONTROL => return 0x1d,
        vk::RMENU => return 0x38,
        vk::LWIN => return 0x5b,
        vk::RWIN => return 0x5c,
        vk::APPS => return 0x5d,
        k => k,
    };
    BASE.iter()
        .position(|&v| v == key && v != 0)
        .map_or(0, |p| p as u16)
}
/// Keys whose physical make code carries the E0 prefix.
pub fn is_extended(key: u8) -> bool {
    matches!(
        key,
        vk::PRIOR
            | vk::NEXT
            | vk::END
            | vk::HOME
            | vk::LEFT
            | vk::UP
            | vk::RIGHT
            | vk::DOWN
            | vk::INSERT
            | vk::DELETE
            | vk::DIVIDE
            | vk::RCONTROL
            | vk::RMENU
            | vk::LWIN
            | vk::RWIN
            | vk::APPS
    )
}

/// `ToUnicode` for the US layout. `None` for keys without a character.
pub fn to_char(key: u8, shift: bool, caps: bool, ctrl: bool, alt: bool) -> Option<u16> {
    if ctrl && alt {
        return None;
    }
    let c = match key {
        b'A'..=b'Z' => {
            if ctrl {
                return Some(u16::from(key - b'A' + 1));
            }
            if shift != caps { key } else { key + 32 }
        }
        b'0'..=b'9' => {
            if ctrl {
                return None;
            }
            if shift {
                b")!@#$%^&*("[(key - b'0') as usize]
            } else {
                key
            }
        }
        vk::SPACE => b' ',
        vk::RETURN => return Some(if ctrl { 10 } else { 13 }),
        vk::TAB if !ctrl => b'\t',
        vk::BACK => return Some(if ctrl { 0x7f } else { 8 }),
        vk::ESCAPE => 0x1b,
        vk::OEM_4 if ctrl => 0x1b,
        vk::OEM_5 if ctrl => 0x1c,
        vk::OEM_6 if ctrl => 0x1d,
        _ if ctrl => return None,
        vk::OEM_1 => pick(shift, b';', b':'),
        vk::OEM_PLUS => pick(shift, b'=', b'+'),
        vk::OEM_COMMA => pick(shift, b',', b'<'),
        vk::OEM_MINUS => pick(shift, b'-', b'_'),
        vk::OEM_PERIOD => pick(shift, b'.', b'>'),
        vk::OEM_2 => pick(shift, b'/', b'?'),
        vk::OEM_3 => pick(shift, b'`', b'~'),
        vk::OEM_4 => pick(shift, b'[', b'{'),
        vk::OEM_5 | vk::OEM_102 => pick(shift, b'\\', b'|'),
        vk::OEM_6 => pick(shift, b']', b'}'),
        vk::OEM_7 => pick(shift, b'\'', b'"'),
        0x60..=0x69 => b'0' + (key - vk::NUMPAD0),
        vk::MULTIPLY => b'*',
        vk::ADD => b'+',
        vk::SUBTRACT => b'-',
        vk::DECIMAL => b'.',
        vk::DIVIDE => b'/',
        _ => return None,
    };
    Some(u16::from(c))
}
fn pick(shift: bool, plain: u8, shifted: u8) -> u8 {
    if shift { shifted } else { plain }
}

/// Key state: bit 7 down, bit 0 toggled (`GetKeyState`).
#[derive(Clone, Debug)]
pub struct KeyState {
    keys: [u8; 256],
}
impl Default for KeyState {
    fn default() -> Self {
        Self::new()
    }
}
impl KeyState {
    pub const fn new() -> Self {
        Self { keys: [0; 256] }
    }
    pub fn get(&self, key: u8) -> i16 {
        let v = self.keys[key as usize];
        let mut r = 0i16;
        if v & 0x80 != 0 {
            r |= i16::MIN;
        }
        if v & 1 != 0 {
            r |= 1;
        }
        r
    }
    pub fn down(&self, key: u8) -> bool {
        self.keys[key as usize] & 0x80 != 0
    }
    pub fn toggled(&self, key: u8) -> bool {
        self.keys[key as usize] & 1 != 0
    }
    /// Apply a key transition (sided or generic code).
    pub fn apply(&mut self, key: u8, down: bool) {
        let mut set = |k: u8, down: bool| {
            let v = &mut self.keys[k as usize];
            if down {
                if *v & 0x80 == 0 {
                    *v ^= 1;
                }
                *v |= 0x80;
            } else {
                *v &= !0x80;
            }
        };
        set(key, down);
        let (l, r) = match generic(key) {
            vk::SHIFT => (vk::LSHIFT, vk::RSHIFT),
            vk::CONTROL => (vk::LCONTROL, vk::RCONTROL),
            vk::MENU => (vk::LMENU, vk::RMENU),
            _ => return,
        };
        let g = generic(key);
        if key == g {
            set(l, down);
        } else {
            let any = self.keys[l as usize] & 0x80 != 0 || self.keys[r as usize] & 0x80 != 0;
            let v = &mut self.keys[g as usize];
            if any {
                *v |= 0x80;
            } else {
                *v &= !0x80;
            }
        }
    }
    pub fn modifiers(&self) -> (bool, bool, bool) {
        (
            self.down(vk::SHIFT),
            self.down(vk::CONTROL),
            self.down(vk::MENU),
        )
    }
}

/// The command an accelerator table assigns to a keyboard message.
pub fn match_accelerator(
    table: &[Accel],
    message: u32,
    wparam: u64,
    keys: &KeyState,
) -> Option<u16> {
    let (shift, ctrl, alt) = keys.modifiers();
    let virt = match message {
        wm::KEYDOWN | wm::SYSKEYDOWN => true,
        wm::CHAR | wm::SYSCHAR => false,
        _ => return None,
    };
    let code = (wparam & 0xffff) as u16;
    table
        .iter()
        .find(|a| {
            let is_virt = a.virt & fvirt::VIRTKEY != 0;
            if is_virt != virt {
                return false;
            }
            if virt {
                a.key == code
                    && (a.virt & fvirt::SHIFT != 0) == shift
                    && (a.virt & fvirt::CONTROL != 0) == ctrl
                    && (a.virt & fvirt::ALT != 0) == alt
            } else {
                a.key == code && (a.virt & fvirt::ALT != 0) == (message == wm::SYSCHAR)
            }
        })
        .map(|a| a.cmd)
}

/// lParam of a key message: repeat count, scan code, extended, previous
/// state and transition bits.
pub fn key_lparam(key: u8, down: bool, repeat: bool, alt: bool) -> i64 {
    let mut v: u32 = 1 | u32::from(vk_to_scan(key)) << 16;
    if is_extended(key) {
        v |= 1 << 24;
    }
    if alt {
        v |= 1 << 29;
    }
    if repeat || !down {
        v |= 1 << 30;
    }
    if !down {
        v |= 1 << 31;
    }
    i64::from(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_codes_map_both_ways() {
        for (key, scan) in [
            (vk::LEFT, 0x4b),
            (vk::F5, 0x3f),
            (vk::DELETE, 0x53),
            (vk::HOME, 0x47),
            (vk::END, 0x4f),
            (b'A', 0x1e),
            (vk::SHIFT, 0x2a),
            (vk::RETURN, 0x1c),
        ] {
            assert_eq!(vk_to_scan(key), scan, "{key:#x}");
        }
        assert_eq!(scan_to_vk(0x14b), vk::LEFT);
        assert_eq!(scan_to_vk(0x36), vk::SHIFT);
        assert_eq!(scan_to_vk_ex(0x36), vk::RSHIFT);
        assert_eq!(scan_to_vk_ex(0x11d), vk::RCONTROL);
        assert_eq!(scan_to_vk(0x200), 0);
        assert!(is_extended(vk::DELETE) && !is_extended(b'A'));
    }
    #[test]
    fn characters_follow_shift_caps_and_control() {
        assert_eq!(
            to_char(b'A', false, false, false, false),
            Some(u16::from(b'a'))
        );
        assert_eq!(
            to_char(b'A', true, false, false, false),
            Some(u16::from(b'A'))
        );
        assert_eq!(
            to_char(b'A', true, true, false, false),
            Some(u16::from(b'a'))
        );
        assert_eq!(to_char(b'C', false, false, true, false), Some(3));
        assert_eq!(
            to_char(b'2', true, false, false, false),
            Some(u16::from(b'@'))
        );
        assert_eq!(
            to_char(vk::OEM_7, true, false, false, false),
            Some(u16::from(b'"'))
        );
        assert_eq!(to_char(vk::LEFT, false, false, false, false), None);
        assert_eq!(to_char(vk::DELETE, false, false, false, false), None);
        assert_eq!(to_char(vk::RETURN, false, false, false, false), Some(13));
        assert_eq!(
            to_char(b'A', false, false, true, true),
            None,
            "AltGr chords"
        );
    }
    #[test]
    fn key_state_tracks_sides_and_toggles() {
        let mut k = KeyState::default();
        k.apply(vk::LSHIFT, true);
        assert!(k.down(vk::SHIFT) && k.get(vk::SHIFT) < 0);
        k.apply(vk::RSHIFT, true);
        k.apply(vk::LSHIFT, false);
        assert!(k.down(vk::SHIFT), "right shift still held");
        k.apply(vk::RSHIFT, false);
        assert!(!k.down(vk::SHIFT));
        k.apply(vk::CAPITAL, true);
        k.apply(vk::CAPITAL, false);
        assert!(k.toggled(vk::CAPITAL) && k.get(vk::CAPITAL) == 1);
        k.apply(vk::CONTROL, true);
        assert!(k.down(vk::LCONTROL));
    }
    #[test]
    fn accelerators_match_exact_modifiers() {
        let table = [
            Accel {
                virt: fvirt::VIRTKEY,
                key: u16::from(vk::F5),
                cmd: 101,
            },
            Accel {
                virt: fvirt::VIRTKEY | fvirt::CONTROL | fvirt::SHIFT,
                key: u16::from(b'S'),
                cmd: 4,
            },
            Accel {
                virt: fvirt::VIRTKEY | fvirt::CONTROL,
                key: u16::from(b'S'),
                cmd: 3,
            },
            Accel {
                virt: 0,
                key: u16::from(b'q'),
                cmd: 9,
            },
        ];
        let mut k = KeyState::default();
        assert_eq!(
            match_accelerator(&table, wm::KEYDOWN, u64::from(vk::F5), &k),
            Some(101)
        );
        k.apply(vk::CONTROL, true);
        assert_eq!(
            match_accelerator(&table, wm::KEYDOWN, u64::from(b'S'), &k),
            Some(3)
        );
        k.apply(vk::SHIFT, true);
        assert_eq!(
            match_accelerator(&table, wm::KEYDOWN, u64::from(b'S'), &k),
            Some(4)
        );
        assert_eq!(
            match_accelerator(&table, wm::KEYDOWN, u64::from(vk::F5), &k),
            None
        );
        assert_eq!(
            match_accelerator(&table, wm::CHAR, u64::from(b'q'), &k),
            Some(9)
        );
        assert_eq!(
            match_accelerator(&table, wm::SYSCHAR, u64::from(b'q'), &k),
            None
        );
        assert_eq!(
            match_accelerator(&table, wm::KEYUP, u64::from(vk::F5), &k),
            None
        );
        let lp = key_lparam(vk::LEFT, true, false, false);
        assert_eq!(lp, 1 | 0x4b << 16 | 1 << 24);
        assert!(
            key_lparam(b'A', false, false, false) < 0
                || key_lparam(b'A', false, false, false) >> 31 & 1 == 1
        );
    }
}
