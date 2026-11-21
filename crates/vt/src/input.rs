//! Console input: key events as the Windows console reports them
//! (KEY_EVENT_RECORD), carried from the Terminal to the program's console
//! as win32-input-mode sequences (`ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`,
//! as Windows Terminal sends conhost), and turned back into virtual
//! terminal input (`ESC [ A`, ...) for a program that asks for it
//! (ENABLE_VIRTUAL_TERMINAL_INPUT) or reads the console as a byte stream.
//! Plain VT input and UTF-8 text decode too (pasted text, other sources).

/// dwControlKeyState bits.
pub mod control {
    pub const RIGHT_ALT: u32 = 0x01;
    pub const LEFT_ALT: u32 = 0x02;
    pub const RIGHT_CTRL: u32 = 0x04;
    pub const LEFT_CTRL: u32 = 0x08;
    pub const SHIFT: u32 = 0x10;
    pub const NUMLOCK: u32 = 0x20;
    pub const SCROLLLOCK: u32 = 0x40;
    pub const CAPSLOCK: u32 = 0x80;
    pub const ENHANCED: u32 = 0x100;
    pub const ALT: u32 = RIGHT_ALT | LEFT_ALT;
    pub const CTRL: u32 = RIGHT_CTRL | LEFT_CTRL;
}
/// Virtual-key codes.
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0d;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1b;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const INSERT: u16 = 0x2d;
    pub const DELETE: u16 = 0x2e;
    pub const LWIN: u16 = 0x5b;
    pub const MULTIPLY: u16 = 0x6a;
    pub const ADD: u16 = 0x6b;
    pub const SUBTRACT: u16 = 0x6d;
    pub const DIVIDE: u16 = 0x6f;
    pub const F1: u16 = 0x70;
    pub const F12: u16 = 0x7b;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyEvent {
    pub down: bool,
    pub repeat: u16,
    pub vk: u16,
    pub scan: u16,
    /// The UTF-16 unit typed, 0 for none.
    pub ch: u16,
    pub control: u32,
}

/// The US keyboard's main block: set-1 scan code, virtual key, character
/// and shifted character.
const LAYOUT: [(u8, u8, u8, u8); 48] = [
    (2, b'1', b'1', b'!'),
    (3, b'2', b'2', b'@'),
    (4, b'3', b'3', b'#'),
    (5, b'4', b'4', b'$'),
    (6, b'5', b'5', b'%'),
    (7, b'6', b'6', b'^'),
    (8, b'7', b'7', b'&'),
    (9, b'8', b'8', b'*'),
    (10, b'9', b'9', b'('),
    (11, b'0', b'0', b')'),
    (12, 0xbd, b'-', b'_'),
    (13, 0xbb, b'=', b'+'),
    (16, b'Q', b'q', b'Q'),
    (17, b'W', b'w', b'W'),
    (18, b'E', b'e', b'E'),
    (19, b'R', b'r', b'R'),
    (20, b'T', b't', b'T'),
    (21, b'Y', b'y', b'Y'),
    (22, b'U', b'u', b'U'),
    (23, b'I', b'i', b'I'),
    (24, b'O', b'o', b'O'),
    (25, b'P', b'p', b'P'),
    (26, 0xdb, b'[', b'{'),
    (27, 0xdd, b']', b'}'),
    (30, b'A', b'a', b'A'),
    (31, b'S', b's', b'S'),
    (32, b'D', b'd', b'D'),
    (33, b'F', b'f', b'F'),
    (34, b'G', b'g', b'G'),
    (35, b'H', b'h', b'H'),
    (36, b'J', b'j', b'J'),
    (37, b'K', b'k', b'K'),
    (38, b'L', b'l', b'L'),
    (39, 0xba, b';', b':'),
    (40, 0xde, b'\'', b'"'),
    (41, 0xc0, b'`', b'~'),
    (43, 0xdc, b'\\', b'|'),
    (44, b'Z', b'z', b'Z'),
    (45, b'X', b'x', b'X'),
    (46, b'C', b'c', b'C'),
    (47, b'V', b'v', b'V'),
    (48, b'B', b'b', b'B'),
    (49, b'N', b'n', b'N'),
    (50, b'M', b'm', b'M'),
    (51, 0xbc, b',', b'<'),
    (52, 0xbe, b'.', b'>'),
    (53, 0xbf, b'/', b'?'),
    (57, b' ', b' ', b' '),
];
/// Keys outside the main block: scan code (0x100 set for E0-prefixed
/// keys), virtual key, the character they type.
const SPECIAL: [(u16, u16, u16); 39] = [
    (1, vk::ESCAPE, 0x1b),
    (14, vk::BACK, 0x08),
    (15, vk::TAB, 0x09),
    (28, vk::RETURN, 0x0d),
    (0x11c, vk::RETURN, 0x0d),
    (29, vk::CONTROL, 0),
    (0x11d, vk::CONTROL, 0),
    (42, vk::SHIFT, 0),
    (54, vk::SHIFT, 0),
    (56, vk::MENU, 0),
    (0x138, vk::MENU, 0),
    (58, vk::CAPITAL, 0),
    (0x15b, vk::LWIN, 0),
    (55, vk::MULTIPLY, b'*' as u16),
    (74, vk::SUBTRACT, b'-' as u16),
    (78, vk::ADD, b'+' as u16),
    (0x135, vk::DIVIDE, b'/' as u16),
    (0x147, vk::HOME, 0),
    (0x148, vk::UP, 0),
    (0x149, vk::PRIOR, 0),
    (0x14b, vk::LEFT, 0),
    (0x14d, vk::RIGHT, 0),
    (0x14f, vk::END, 0),
    (0x150, vk::DOWN, 0),
    (0x151, vk::NEXT, 0),
    (0x152, vk::INSERT, 0),
    (0x153, vk::DELETE, 0),
    // The keypad without Num Lock.
    (71, vk::HOME, 0),
    (72, vk::UP, 0),
    (73, vk::PRIOR, 0),
    (75, vk::LEFT, 0),
    (77, vk::RIGHT, 0),
    (79, vk::END, 0),
    (80, vk::DOWN, 0),
    (81, vk::NEXT, 0),
    (82, vk::INSERT, 0),
    (83, vk::DELETE, 0),
    (87, 0x7a, 0),
    (88, 0x7b, 0),
];

/// The scan code Windows reports with a virtual key (US layout).
pub fn scan_of(key: u16) -> u16 {
    if (vk::F1..vk::F1 + 10).contains(&key) {
        return 59 + (key - vk::F1);
    }
    if let Some(&(scan, _, _, _)) = LAYOUT.iter().find(|e| u16::from(e.1) == key) {
        return u16::from(scan);
    }
    SPECIAL
        .iter()
        .find(|e| e.1 == key && e.1 != 0)
        .map_or(0, |e| e.0 & 0xff)
}

/// A key pressed in the Terminal (set-1 scan code, 0x100 for E0 keys; the
/// character it typed, if any; modifiers) as the console's key-down event.
pub fn from_scan(scan: u16, text: char, ctrl: bool, shift: bool, alt: bool) -> Option<KeyEvent> {
    let mut control = 0;
    if ctrl {
        control |= control::LEFT_CTRL;
    }
    if shift {
        control |= control::SHIFT;
    }
    if alt {
        control |= control::LEFT_ALT;
    }
    if scan & 0x100 != 0 {
        control |= control::ENHANCED;
    }
    let (key, mut ch) = if (59..=68).contains(&scan) {
        (vk::F1 + (scan - 59), 0)
    } else if let Some(&(_, key, base, shifted)) = LAYOUT.iter().find(|e| u16::from(e.0) == scan) {
        let typed = if text != '\0' {
            text as u32 as u16
        } else if shift {
            u16::from(shifted)
        } else {
            u16::from(base)
        };
        (u16::from(key), typed)
    } else if let Some(&(_, key, ch)) = SPECIAL.iter().find(|e| e.0 == scan && e.1 != 0) {
        (key, ch)
    } else {
        return None;
    };
    if ctrl {
        ch = match key {
            0x41..=0x5a => key - 0x40,
            0xdb => 0x1b,
            0xdc => 0x1c,
            0xdd => 0x1d,
            vk::SPACE => 0x20,
            vk::BACK => 0x7f,
            vk::RETURN => 0x0a,
            vk::TAB | vk::ESCAPE => ch,
            _ => 0,
        };
    }
    Some(KeyEvent {
        down: true,
        repeat: 1,
        vk: key,
        scan: scan & 0xff,
        ch,
        control,
    })
}

/// Decimal digits of `n` into `out` from `at`; the new end.
fn digits(out: &mut [u8], mut at: usize, n: u32) -> usize {
    let mut buffer = [0u8; 10];
    let (mut len, mut n) = (0, n);
    loop {
        buffer[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        out[at] = buffer[len];
        at += 1;
    }
    at
}

/// `ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _` for one key event (at most 48
/// bytes).
pub fn encode_win32(e: &KeyEvent, out: &mut [u8; 48]) -> usize {
    out[..2].copy_from_slice(b"\x1b[");
    let mut at = 2;
    for (i, value) in [
        u32::from(e.vk),
        u32::from(e.scan),
        u32::from(e.ch),
        u32::from(e.down),
        e.control,
        u32::from(e.repeat),
    ]
    .into_iter()
    .enumerate()
    {
        if i > 0 {
            out[at] = b';';
            at += 1;
        }
        at = digits(out, at, value);
    }
    out[at] = b'_';
    at + 1
}

/// The xterm modifier parameter (1 + shift 1, alt 2, ctrl 4), or 1.
fn modifier(control: u32) -> u32 {
    1 + u32::from(control & control::SHIFT != 0)
        + 2 * u32::from(control & control::ALT != 0)
        + 4 * u32::from(control & control::CTRL != 0)
}

/// A key-down event as virtual terminal input (what the program reads
/// with ENABLE_VIRTUAL_TERMINAL_INPUT, or as a byte stream); key-ups and
/// bare modifiers send nothing. At most 16 bytes.
pub fn to_vt(e: &KeyEvent, application_cursor: bool, out: &mut [u8; 16]) -> usize {
    if !e.down {
        return 0;
    }
    let m = modifier(e.control);
    let csi = |out: &mut [u8; 16], number: u32, last: u8| -> usize {
        out[..2].copy_from_slice(b"\x1b[");
        let mut at = 2;
        if number != 1 || m != 1 {
            at = digits(out, at, number);
        }
        if m != 1 {
            out[at] = b';';
            at = digits(out, at + 1, m);
        }
        out[at] = last;
        at + 1
    };
    let ss3 = |out: &mut [u8; 16], last: u8| -> usize {
        if m == 1 {
            out[..3].copy_from_slice(&[0x1b, b'O', last]);
            3
        } else {
            csi(out, 1, last)
        }
    };
    let arrow = |out: &mut [u8; 16], last: u8| -> usize {
        if application_cursor && m == 1 {
            ss3(out, last)
        } else {
            csi(out, 1, last)
        }
    };
    match e.vk {
        vk::UP => return arrow(out, b'A'),
        vk::DOWN => return arrow(out, b'B'),
        vk::RIGHT => return arrow(out, b'C'),
        vk::LEFT => return arrow(out, b'D'),
        vk::HOME => return arrow(out, b'H'),
        vk::END => return arrow(out, b'F'),
        vk::INSERT => return csi(out, 2, b'~'),
        vk::DELETE => return csi(out, 3, b'~'),
        vk::PRIOR => return csi(out, 5, b'~'),
        vk::NEXT => return csi(out, 6, b'~'),
        k if (vk::F1..vk::F1 + 4).contains(&k) => return ss3(out, b'P' + (k - vk::F1) as u8),
        k if (vk::F1 + 4..=vk::F12).contains(&k) => {
            const CODES: [u32; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            return csi(out, CODES[(k - vk::F1 - 4) as usize], b'~');
        }
        vk::TAB if e.control & control::SHIFT != 0 => {
            out[..3].copy_from_slice(b"\x1b[Z");
            return 3;
        }
        vk::BACK => {
            out[0] = if e.control & control::CTRL != 0 {
                0x08
            } else {
                0x7f
            };
            return 1;
        }
        _ => {}
    }
    if e.ch == 0 || (0xd800..0xe000).contains(&e.ch) {
        // A surrogate half travels as one event; the caller joins pairs.
        return 0;
    }
    let mut at = 0;
    if e.control & control::ALT != 0 && e.control & control::CTRL == 0 {
        out[0] = 0x1b;
        at = 1;
    }
    let ch = char::from_u32(u32::from(e.ch)).unwrap_or('\u{fffd}');
    at + ch.encode_utf8(&mut out[at..]).len()
}

/// Key events decoded from the input stream (two for a character
/// outside the BMP: its surrogate pair).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Keys {
    pub events: [KeyEvent; 2],
    pub count: usize,
}
impl Keys {
    fn one(e: KeyEvent) -> Self {
        Self {
            events: [e, KeyEvent::default()],
            count: 1,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decoded {
    /// Events and the bytes they took.
    Keys(Keys, usize),
    /// Bytes that carry no key (bracketed-paste and focus markers, an
    /// unknown sequence).
    Skip(usize),
    /// The bytes may be the start of a longer sequence: wait for more.
    Need,
}
/// The key that types `c` on a US keyboard (letters, digits, symbols).
fn key_for(c: u32, control: u32) -> KeyEvent {
    let mut e = KeyEvent {
        down: true,
        repeat: 1,
        ch: 0,
        control,
        ..Default::default()
    };
    if let Ok(b) = u8::try_from(c)
        && let Some(&(scan, key, base, shifted)) = LAYOUT.iter().find(|e| e.2 == b || e.3 == b)
    {
        e.vk = u16::from(key);
        e.scan = u16::from(scan);
        if b != base && b == shifted {
            e.control |= control::SHIFT;
        }
    }
    e
}
fn char_event(c: char, control: u32) -> Keys {
    let mut e = key_for(c as u32, control);
    let mut units = [0u16; 2];
    let units = c.encode_utf16(&mut units);
    e.ch = units[0];
    let mut keys = Keys::one(e);
    if units.len() == 2 {
        let mut low = e;
        low.ch = units[1];
        keys.events[1] = low;
        keys.count = 2;
    }
    keys
}
fn special(key: u16, ch: u16, control: u32) -> Keys {
    Keys::one(KeyEvent {
        down: true,
        repeat: 1,
        vk: key,
        scan: scan_of(key),
        ch,
        control,
    })
}
/// The xterm modifier parameter as control-key state.
fn control_of(m: u32) -> u32 {
    let m = m.saturating_sub(1);
    let mut c = 0;
    if m & 1 != 0 {
        c |= control::SHIFT;
    }
    if m & 2 != 0 {
        c |= control::LEFT_ALT;
    }
    if m & 4 != 0 {
        c |= control::LEFT_CTRL;
    }
    c
}

/// Decode the next key from `bytes`. `more`: further bytes may still
/// arrive (an ESC at the end then waits rather than being the Escape key).
pub fn decode(bytes: &[u8], more: bool) -> Decoded {
    let Some(&first) = bytes.first() else {
        return Decoded::Need;
    };
    if first == 0x1b {
        return escape(bytes, more);
    }
    let control_key = |key: u16, ch: u16, control: u32| Decoded::Keys(special(key, ch, control), 1);
    match first {
        0x0d | 0x0a => return control_key(vk::RETURN, 0x0d, 0),
        0x09 => return control_key(vk::TAB, 0x09, 0),
        0x7f => return control_key(vk::BACK, 0x08, 0),
        0x08 => return control_key(vk::BACK, 0x7f, control::LEFT_CTRL),
        0x00 => return control_key(vk::SPACE, 0x20, control::LEFT_CTRL),
        0x01..=0x1a => {
            return control_key(
                0x40 + u16::from(first),
                u16::from(first),
                control::LEFT_CTRL,
            );
        }
        0x1c..=0x1f => return control_key(0, u16::from(first), control::LEFT_CTRL),
        _ => {}
    }
    let need = match first {
        0x20..=0x7e => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Decoded::Skip(1),
    };
    if bytes.len() < need {
        return if more {
            Decoded::Need
        } else {
            Decoded::Skip(bytes.len())
        };
    }
    match core::str::from_utf8(&bytes[..need])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(c) => Decoded::Keys(char_event(c, 0), need),
        None => Decoded::Skip(1),
    }
}
fn escape(bytes: &[u8], more: bool) -> Decoded {
    let escape_key = Decoded::Keys(special(vk::ESCAPE, 0x1b, 0), 1);
    let Some(&second) = bytes.get(1) else {
        return if more { Decoded::Need } else { escape_key };
    };
    match second {
        b'[' => csi(bytes, more),
        b'O' => {
            let Some(&last) = bytes.get(2) else {
                return if more { Decoded::Need } else { escape_key };
            };
            match final_key(last, 1, 0) {
                Some(keys) => Decoded::Keys(keys, 3),
                None => Decoded::Skip(3),
            }
        }
        0x1b => escape_key,
        // Alt + a key.
        _ => match decode(&bytes[1..], more) {
            Decoded::Keys(mut keys, n) => {
                for e in &mut keys.events[..keys.count] {
                    e.control |= control::LEFT_ALT;
                }
                Decoded::Keys(keys, n + 1)
            }
            Decoded::Skip(_) => escape_key,
            Decoded::Need => Decoded::Need,
        },
    }
}
/// The key a CSI or SS3 final byte names.
fn final_key(last: u8, m: u32, number: u32) -> Option<Keys> {
    let control = control_of(m);
    let enhanced = control | control::ENHANCED;
    Some(match last {
        b'A' => special(vk::UP, 0, enhanced),
        b'B' => special(vk::DOWN, 0, enhanced),
        b'C' => special(vk::RIGHT, 0, enhanced),
        b'D' => special(vk::LEFT, 0, enhanced),
        b'H' => special(vk::HOME, 0, enhanced),
        b'F' => special(vk::END, 0, enhanced),
        b'P'..=b'S' => special(vk::F1 + u16::from(last - b'P'), 0, control),
        b'Z' => special(vk::TAB, 0x09, control::SHIFT),
        b'~' => {
            let key = match number {
                1 | 7 => vk::HOME,
                2 => vk::INSERT,
                3 => vk::DELETE,
                4 | 8 => vk::END,
                5 => vk::PRIOR,
                6 => vk::NEXT,
                11..=15 => vk::F1 + (number - 11) as u16,
                17..=21 => vk::F1 + 5 + (number - 17) as u16,
                23 | 24 => vk::F1 + 10 + (number - 23) as u16,
                _ => return None,
            };
            let control = if key < vk::F1 { enhanced } else { control };
            special(key, 0, control)
        }
        _ => return None,
    })
}
fn csi(bytes: &[u8], more: bool) -> Decoded {
    let mut params = [0u32; 6];
    let mut given = [false; 6];
    let mut count = 0usize;
    let mut private = false;
    for (i, &b) in bytes.iter().enumerate().skip(2) {
        match b {
            b'0'..=b'9' => {
                if count == 0 {
                    count = 1;
                }
                if count <= params.len() {
                    let p = &mut params[count - 1];
                    *p = p.saturating_mul(10).saturating_add(u32::from(b - b'0'));
                    given[count - 1] = true;
                }
            }
            b';' => count = count.max(1) + 1,
            b'<' | b'=' | b'>' | b'?' => private = true,
            0x40..=0x7e => {
                let n = i + 1;
                if private {
                    return Decoded::Skip(n);
                }
                if b == b'_' {
                    let value = |k: usize, d: u32| if given[k] { params[k] } else { d };
                    let e = KeyEvent {
                        vk: value(0, 0) as u16,
                        scan: value(1, 0) as u16,
                        ch: value(2, 0) as u16,
                        down: value(3, 0) != 0,
                        control: value(4, 0),
                        repeat: value(5, 1) as u16,
                    };
                    return Decoded::Keys(Keys::one(e), n);
                }
                let m = if count >= 2 { params[1].max(1) } else { 1 };
                let number = if count >= 1 { params[0] } else { 0 };
                return match b {
                    // Bracketed paste markers and focus reports carry no key.
                    b'~' if matches!(number, 200 | 201) => Decoded::Skip(n),
                    b'I' | b'O' => Decoded::Skip(n),
                    _ => match final_key(b, m, number) {
                        Some(keys) => Decoded::Keys(keys, n),
                        None => Decoded::Skip(n),
                    },
                };
            }
            _ => return Decoded::Skip(i + 1),
        }
        if i > 64 {
            return Decoded::Skip(i + 1);
        }
    }
    if more {
        Decoded::Need
    } else {
        Decoded::Skip(bytes.len())
    }
}
