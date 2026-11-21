//! PS/2 translated set-1 keyboard decoder (§53). State belongs to the console
//! owner. `event` reports physical keys with the complete sided-modifier and
//! lock state and typematic repeat; the window server applies the session
//! keyboard layout. `feed` remains the US text console translation.
pub struct Decoder {
    left_shift: bool,
    right_shift: bool,
    left_ctrl: bool,
    right_ctrl: bool,
    left_alt: bool,
    right_alt: bool,
    left_logo: bool,
    right_logo: bool,
    caps: bool,
    num: bool,
    scroll: bool,
    extended: bool,
    pause: u8,
    /// Physical codes currently down (0..512), for repeat detection.
    pressed: [u64; 8],
}
/// Non-key bytes a set-1 keyboard may interleave (error/overrun, ACK,
/// resend, echo, self-test failure). Their 0x80 bit is not a release.
fn control_byte(scan: u8) -> bool {
    matches!(scan, 0x00 | 0xff | 0xfa | 0xfe | 0xee | 0xfc)
}
impl Decoder {
    /// Physical make/break events preserve modifiers and key-up independently
    /// of the text-console translation. Called by the exclusive display owner.
    pub fn event(&mut self, scan: u8) -> Option<door_abi::graphics::Input> {
        use door_abi::graphics as g;
        let extended = self.extended;
        let pause = self.pause != 0 || scan == 0xe1;
        let was_down = {
            let code = usize::from(scan & 0x7f) | (usize::from(extended) << 8);
            self.pressed[code / 64] & (1 << (code % 64)) != 0
        };
        let text = self.feed(scan);
        if pause || scan == 0xe0 || scan == 0xe1 || control_byte(scan) && !extended {
            return None;
        }
        // Korean Hanja/Hangul: make-only codes without a prefix.
        if !extended && matches!(u32::from(scan), g::HANJA | g::HANGUL) {
            return Some(g::Input {
                kind: g::KEY,
                flags: self.flags() | g::DOWN,
                code: u32::from(scan),
                ..g::Input::default()
            });
        }
        // E0 2A/E0 36 (and breaks) are fake shifts around extended keys.
        if extended && matches!(scan & 0x7f, 0x2a | 0x36) {
            return None;
        }
        let pressed = scan & 0x80 == 0;
        let code = u32::from(scan & 0x7f) | (u32::from(extended) * 256);
        if code == 0 {
            return None;
        }
        let mut flags = self.flags();
        if pressed {
            flags |= g::DOWN;
            if was_down {
                flags |= g::REPEAT;
            }
        }
        Some(g::Input {
            kind: g::KEY,
            flags,
            code,
            text: u32::from(text.unwrap_or(0)),
            ..g::Input::default()
        })
    }
    /// Modifier and lock state as KEY flags (without DOWN/REPEAT).
    pub fn flags(&self) -> u32 {
        use door_abi::graphics as g;
        let mut f = 0;
        for (on, bits) in [
            (self.left_shift, g::SHIFT | g::LEFT_SHIFT),
            (self.right_shift, g::SHIFT | g::RIGHT_SHIFT),
            (self.left_ctrl, g::CONTROL | g::LEFT_CONTROL),
            (self.right_ctrl, g::CONTROL | g::RIGHT_CONTROL),
            (self.left_alt, g::ALT | g::LEFT_ALT),
            (self.right_alt, g::ALT | g::RIGHT_ALT),
            (self.left_logo, g::LOGO | g::LEFT_LOGO),
            (self.right_logo, g::LOGO | g::RIGHT_LOGO),
            (self.caps, g::CAPS_LOCK),
            (self.num, g::NUM_LOCK),
            (self.scroll, g::SCROLL_LOCK),
        ] {
            if on {
                f |= bits;
            }
        }
        f
    }
    pub const fn new() -> Self {
        Self {
            left_shift: false,
            right_shift: false,
            left_ctrl: false,
            right_ctrl: false,
            left_alt: false,
            right_alt: false,
            left_logo: false,
            right_logo: false,
            caps: false,
            // Desktop firmware default; the window server shows it as such.
            num: true,
            scroll: false,
            extended: false,
            pause: 0,
            pressed: [0; 8],
        }
    }
    /// The first make of a key (locks toggle here, never on repeat).
    fn first_make(&self, code: usize, release: bool) -> bool {
        !release && self.pressed[code / 64] & (1u64 << (code % 64)) == 0
    }
    pub fn feed(&mut self, scan: u8) -> Option<u8> {
        if self.pause != 0 {
            self.pause -= 1;
            return None;
        }
        if scan == 0xe1 {
            self.pause = 5;
            return None;
        }
        if scan == 0xe0 {
            self.extended = true;
            return None;
        }
        let extended = core::mem::take(&mut self.extended);
        if !extended && (control_byte(scan) || matches!(scan, 0xf1 | 0xf2)) {
            return None;
        }
        let release = scan & 0x80 != 0;
        let key = scan & 0x7f;
        let code = usize::from(key) | (usize::from(extended) << 8);
        let first = self.first_make(code, release);
        if extended && matches!(key, 0x2a | 0x36) {
            return None;
        }
        let bit = 1u64 << (code % 64);
        if release {
            self.pressed[code / 64] &= !bit;
        } else {
            self.pressed[code / 64] |= bit;
        }
        if extended {
            match key {
                0x38 => {
                    self.right_alt = !release;
                    return None;
                }
                0x1d => {
                    self.right_ctrl = !release;
                    return None;
                }
                0x5b => {
                    self.left_logo = !release;
                    return None;
                }
                0x5c => {
                    self.right_logo = !release;
                    return None;
                }
                _ => {}
            }
            if release {
                return None;
            }
            return match key {
                0x1c => Some(b'\n'),
                0x35 => Some(b'/'),
                0x53 => Some(127),
                _ => None,
            };
        }
        match key {
            0x38 => {
                self.left_alt = !release;
                return None;
            }
            0x2a => {
                self.left_shift = !release;
                return None;
            }
            0x36 => {
                self.right_shift = !release;
                return None;
            }
            0x1d => {
                self.left_ctrl = !release;
                return None;
            }
            0x3a => {
                if first {
                    self.caps = !self.caps;
                }
                return None;
            }
            0x45 => {
                if first {
                    self.num = !self.num;
                }
                return None;
            }
            0x46 => {
                if first {
                    self.scroll = !self.scroll;
                }
                return None;
            }
            _ => {}
        }
        if release {
            return None;
        }
        let shift = self.left_shift || self.right_shift;
        let letter = match key {
            0x10..=0x19 => Some(b"qwertyuiop"[(key - 0x10) as usize]),
            0x1e..=0x26 => Some(b"asdfghjkl"[(key - 0x1e) as usize]),
            0x2c..=0x32 => Some(b"zxcvbnm"[(key - 0x2c) as usize]),
            _ => None,
        };
        if let Some(letter) = letter {
            return Some(if self.left_ctrl || self.right_ctrl {
                letter - b'a' + 1
            } else if shift ^ self.caps {
                letter.to_ascii_uppercase()
            } else {
                letter
            });
        }
        // Numeric keypad digits follow Num Lock; Shift temporarily inverts
        // it (Windows), giving navigation keys, which carry no text.
        if matches!(key, 0x47..=0x49 | 0x4b..=0x4d | 0x4f..=0x53) {
            if self.num == shift {
                return None;
            }
            return Some(b"789-456+1230."[(key - 0x47) as usize]);
        }
        let (plain, shifted) = match key {
            1 => (27, 27),
            14 => (8, 8),
            15 => (9, 9),
            28 => (10, 10),
            57 => (32, 32),
            2..=11 => (
                b"1234567890"[(key - 2) as usize],
                b"!@#$%^&*()"[(key - 2) as usize],
            ),
            12 => (b'-', b'_'),
            13 => (b'=', b'+'),
            26 => (b'[', b'{'),
            27 => (b']', b'}'),
            39 => (b';', b':'),
            40 => (39, 34),
            41 => (b'`', b'~'),
            43 => (92, b'|'),
            51 => (b',', b'<'),
            52 => (b'.', b'>'),
            53 => (b'/', b'?'),
            55 => (b'*', b'*'),
            74 => (b'-', b'-'),
            78 => (b'+', b'+'),
            _ => return None,
        };
        Some(if shift { shifted } else { plain })
    }
}
impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_abi::graphics as g;
    #[test]
    fn physical_make_break_extended_and_both_alt_keys_keep_independent_state() {
        let mut d = Decoder::new();
        let n = g::NUM_LOCK;
        assert_eq!(
            d.event(0x38).unwrap().flags,
            g::ALT | g::LEFT_ALT | g::DOWN | n
        );
        assert!(d.event(0xe0).is_none());
        assert_eq!(d.event(0x38).unwrap().code, 0x138);
        assert_eq!(
            d.event(0xb8).unwrap().flags,
            g::ALT | g::RIGHT_ALT | n,
            "right Alt still held"
        );
        let key = d.event(0x1e).unwrap();
        assert_eq!(
            (key.text, key.flags),
            (u32::from(b'a'), g::ALT | g::RIGHT_ALT | g::DOWN | n)
        );
        let up = d.event(0x9e).unwrap();
        assert_eq!((up.text, up.flags), (0, g::ALT | g::RIGHT_ALT | n));
        d.event(0xe0);
        assert_eq!(d.event(0xb8).unwrap().flags, n);
        d.event(0xe0);
        assert_eq!(d.event(0x48).unwrap().code, 0x148);
    }
    #[test]
    fn right_alt_logo_and_sided_modifiers_are_reported_separately() {
        let mut d = Decoder::new();
        d.event(0xe0);
        let altgr = d.event(0x38).unwrap();
        assert_eq!(altgr.flags & !g::NUM_LOCK, g::DOWN | g::ALT | g::RIGHT_ALT);
        d.event(0xe0);
        let ctrl = d.event(0x1d).unwrap();
        assert!(ctrl.flags & g::RIGHT_CONTROL != 0 && ctrl.flags & g::LEFT_CONTROL == 0);
        d.event(0xe0);
        let logo = d.event(0x5b).unwrap();
        assert_eq!(logo.code, 0x15b);
        assert!(logo.flags & (g::LOGO | g::LEFT_LOGO) == g::LOGO | g::LEFT_LOGO);
        d.event(0xe0);
        let up = d.event(0xdb).unwrap();
        assert_eq!(up.flags & g::LOGO, 0);
        assert!(g::key_flags_consistent(up.flags));
        d.event(0x36);
        assert!(d.event(0x1e).unwrap().flags & g::RIGHT_SHIFT != 0);
    }
    #[test]
    fn typematic_repeat_is_flagged_and_locks_toggle_once_per_press() {
        let mut d = Decoder::new();
        let first = d.event(0x1e).unwrap();
        assert_eq!(first.flags & g::REPEAT, 0);
        let again = d.event(0x1e).unwrap();
        assert_eq!(again.flags & (g::REPEAT | g::DOWN), g::REPEAT | g::DOWN);
        assert_eq!(again.text, u32::from(b'a'));
        assert_eq!(d.event(0x9e).unwrap().flags & g::REPEAT, 0);
        assert_eq!(d.event(0x1e).unwrap().flags & g::REPEAT, 0, "new press");
        // Caps Lock held with typematic repeat toggles once.
        for _ in 0..3 {
            d.event(0x3a);
        }
        assert!(d.event(0xba).unwrap().flags & g::CAPS_LOCK != 0);
        d.event(0x3a);
        assert_eq!(d.event(0xba).unwrap().flags & g::CAPS_LOCK, 0);
        // Num Lock starts on and toggles; Scroll Lock likewise.
        assert!(d.flags() & g::NUM_LOCK != 0);
        d.event(0x45);
        d.event(0xc5);
        assert_eq!(d.flags() & g::NUM_LOCK, 0);
        d.event(0x46);
        assert!(d.event(0xc6).unwrap().flags & g::SCROLL_LOCK != 0);
        // Ctrl+Break (E0 46) is not Scroll Lock.
        d.event(0xe0);
        d.event(0x46);
        assert!(d.flags() & g::SCROLL_LOCK != 0);
    }
    #[test]
    fn korean_make_only_keys_fake_shifts_and_controls_bytes() {
        let mut d = Decoder::new();
        let hangul = d.event(0xf2).unwrap();
        assert_eq!((hangul.code, hangul.flags & g::DOWN), (g::HANGUL, g::DOWN));
        assert_eq!(d.event(0xf1).unwrap().code, g::HANJA);
        // Print Screen: E0 2A E0 37 ... E0 B7 E0 AA.
        for scan in [0xe0, 0x2a] {
            assert!(d.event(scan).is_none());
        }
        d.event(0xe0);
        assert_eq!(d.event(0x37).unwrap().code, 0x137);
        d.event(0xe0);
        d.event(0xb7);
        d.event(0xe0);
        assert!(d.event(0xaa).is_none());
        assert_eq!(d.flags() & g::SHIFT, 0, "fake shift never latches");
        for scan in [0x00, 0xff, 0xfa, 0xfe] {
            assert!(d.event(scan).is_none(), "{scan:#x}");
        }
        // A real left-shift break (0xAA) still releases Shift.
        d.event(0x2a);
        assert!(d.flags() & g::SHIFT != 0);
        d.event(0xaa);
        assert_eq!(d.flags() & g::SHIFT, 0);
        for scan in [0x1e, 0x9e, 0x2a, 0xaa, 0xe0, 0x38, 0xe0, 0xb8] {
            if let Some(e) = d.event(scan) {
                let mut b = [0; g::INPUT_BYTES];
                e.encode(&mut b).unwrap();
                assert_eq!(g::Input::decode(&b), Ok(e), "every event is ABI-valid");
            }
        }
    }
    #[test]
    fn numeric_keypad_follows_num_lock_and_shift() {
        let mut d = Decoder::new();
        assert_eq!(d.feed(0x47), Some(b'7'));
        assert_eq!(d.feed(0x52), Some(b'0'));
        assert_eq!(d.feed(0x53), Some(b'.'));
        d.feed(0x2a);
        assert_eq!(d.feed(0x48), None, "Shift inverts Num Lock");
        d.feed(0xaa);
        d.feed(0x45);
        d.feed(0xc5);
        assert_eq!(d.feed(0x48), None, "Num Lock off: navigation");
        assert_eq!(d.feed(0x4a), Some(b'-'), "keypad minus has no navigation");
    }
    #[test]
    fn paths_and_quotes_support_both_shift_keys_and_release() {
        let mut d = Decoder::new();
        assert_eq!(d.feed(53), Some(b'/'));
        assert_eq!(d.feed(52), Some(b'.'));
        assert_eq!(d.feed(12), Some(b'-'));
        assert_eq!(d.feed(42), None);
        assert_eq!(d.feed(40), Some(b'"'));
        d.feed(54);
        d.feed(0xaa);
        assert_eq!(d.feed(30), Some(b'A'));
        d.feed(0xb6);
        assert_eq!(d.feed(30), Some(b'a'));
        assert_eq!(d.feed(0x9e), None);
    }
    #[test]
    fn caps_ctrl_and_extended_sequences_do_not_invent_letters() {
        let mut d = Decoder::new();
        d.feed(58);
        d.feed(0xba);
        assert_eq!(d.feed(30), Some(b'A'));
        d.feed(42);
        assert_eq!(d.feed(30), Some(b'a'));
        d.feed(0xaa);
        d.feed(29);
        assert_eq!(d.feed(46), Some(3));
        d.feed(0x9d);
        d.feed(0xe0);
        assert_eq!(d.feed(0x48), None);
        d.feed(0xe0);
        assert_eq!(d.feed(0x1c), Some(b'\n'));
        d.feed(0xe1);
        for scan in [0x1d, 0x45, 0xe1, 0x9d, 0xc5] {
            assert_eq!(d.feed(scan), None);
        }
        assert_eq!(d.feed(30), Some(b'A'));
    }
}
