//! Physical keys: PS/2 set-1 codes (the device ABI) with their USB HID usage
//! (page 7) and the US-position Windows virtual key. A physical key is not a
//! character; layouts give it meaning.
use crate::vk;
/// One physical key: set-1 code (0x100 = E0 prefix), HID usage, base VK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Physical {
    pub code: u16,
    pub usage: u16,
    pub vk: u8,
    pub name: &'static str,
}
const fn p(code: u16, usage: u16, vk: u8, name: &'static str) -> Physical {
    Physical {
        code,
        usage,
        vk,
        name,
    }
}
/// Every key the set-1 decoder reports, sorted by code.
pub const KEYS: &[Physical] = &[
    p(0x01, 0x29, vk::ESCAPE, "Escape"),
    p(0x02, 0x1e, b'1', "Digit1"),
    p(0x03, 0x1f, b'2', "Digit2"),
    p(0x04, 0x20, b'3', "Digit3"),
    p(0x05, 0x21, b'4', "Digit4"),
    p(0x06, 0x22, b'5', "Digit5"),
    p(0x07, 0x23, b'6', "Digit6"),
    p(0x08, 0x24, b'7', "Digit7"),
    p(0x09, 0x25, b'8', "Digit8"),
    p(0x0a, 0x26, b'9', "Digit9"),
    p(0x0b, 0x27, b'0', "Digit0"),
    p(0x0c, 0x2d, vk::OEM_MINUS, "Minus"),
    p(0x0d, 0x2e, vk::OEM_PLUS, "Equal"),
    p(0x0e, 0x2a, vk::BACK, "Backspace"),
    p(0x0f, 0x2b, vk::TAB, "Tab"),
    p(0x10, 0x14, b'Q', "KeyQ"),
    p(0x11, 0x1a, b'W', "KeyW"),
    p(0x12, 0x08, b'E', "KeyE"),
    p(0x13, 0x15, b'R', "KeyR"),
    p(0x14, 0x17, b'T', "KeyT"),
    p(0x15, 0x1c, b'Y', "KeyY"),
    p(0x16, 0x18, b'U', "KeyU"),
    p(0x17, 0x0c, b'I', "KeyI"),
    p(0x18, 0x12, b'O', "KeyO"),
    p(0x19, 0x13, b'P', "KeyP"),
    p(0x1a, 0x2f, vk::OEM_4, "BracketLeft"),
    p(0x1b, 0x30, vk::OEM_6, "BracketRight"),
    p(0x1c, 0x28, vk::RETURN, "Enter"),
    p(0x1d, 0xe0, vk::CONTROL, "ControlLeft"),
    p(0x1e, 0x04, b'A', "KeyA"),
    p(0x1f, 0x16, b'S', "KeyS"),
    p(0x20, 0x07, b'D', "KeyD"),
    p(0x21, 0x09, b'F', "KeyF"),
    p(0x22, 0x0a, b'G', "KeyG"),
    p(0x23, 0x0b, b'H', "KeyH"),
    p(0x24, 0x0d, b'J', "KeyJ"),
    p(0x25, 0x0e, b'K', "KeyK"),
    p(0x26, 0x0f, b'L', "KeyL"),
    p(0x27, 0x33, vk::OEM_1, "Semicolon"),
    p(0x28, 0x34, vk::OEM_7, "Quote"),
    p(0x29, 0x35, vk::OEM_3, "Backquote"),
    p(0x2a, 0xe1, vk::SHIFT, "ShiftLeft"),
    p(0x2b, 0x31, vk::OEM_5, "Backslash"),
    p(0x2c, 0x1d, b'Z', "KeyZ"),
    p(0x2d, 0x1b, b'X', "KeyX"),
    p(0x2e, 0x06, b'C', "KeyC"),
    p(0x2f, 0x19, b'V', "KeyV"),
    p(0x30, 0x05, b'B', "KeyB"),
    p(0x31, 0x11, b'N', "KeyN"),
    p(0x32, 0x10, b'M', "KeyM"),
    p(0x33, 0x36, vk::OEM_COMMA, "Comma"),
    p(0x34, 0x37, vk::OEM_PERIOD, "Period"),
    p(0x35, 0x38, vk::OEM_2, "Slash"),
    p(0x36, 0xe5, vk::SHIFT, "ShiftRight"),
    p(0x37, 0x55, vk::MULTIPLY, "NumpadMultiply"),
    p(0x38, 0xe2, vk::MENU, "AltLeft"),
    p(0x39, 0x2c, vk::SPACE, "Space"),
    p(0x3a, 0x39, vk::CAPITAL, "CapsLock"),
    p(0x3b, 0x3a, vk::F1, "F1"),
    p(0x3c, 0x3b, vk::F1 + 1, "F2"),
    p(0x3d, 0x3c, vk::F1 + 2, "F3"),
    p(0x3e, 0x3d, vk::F1 + 3, "F4"),
    p(0x3f, 0x3e, vk::F1 + 4, "F5"),
    p(0x40, 0x3f, vk::F1 + 5, "F6"),
    p(0x41, 0x40, vk::F1 + 6, "F7"),
    p(0x42, 0x41, vk::F1 + 7, "F8"),
    p(0x43, 0x42, vk::F1 + 8, "F9"),
    p(0x44, 0x43, vk::F1 + 9, "F10"),
    p(0x45, 0x53, vk::NUMLOCK, "NumLock"),
    p(0x46, 0x47, vk::SCROLL, "ScrollLock"),
    p(0x47, 0x5f, vk::NUMPAD0 + 7, "Numpad7"),
    p(0x48, 0x60, vk::NUMPAD0 + 8, "Numpad8"),
    p(0x49, 0x61, vk::NUMPAD0 + 9, "Numpad9"),
    p(0x4a, 0x56, vk::SUBTRACT, "NumpadSubtract"),
    p(0x4b, 0x5c, vk::NUMPAD0 + 4, "Numpad4"),
    p(0x4c, 0x5d, vk::NUMPAD0 + 5, "Numpad5"),
    p(0x4d, 0x5e, vk::NUMPAD0 + 6, "Numpad6"),
    p(0x4e, 0x57, vk::ADD, "NumpadAdd"),
    p(0x4f, 0x59, vk::NUMPAD0 + 1, "Numpad1"),
    p(0x50, 0x5a, vk::NUMPAD0 + 2, "Numpad2"),
    p(0x51, 0x5b, vk::NUMPAD0 + 3, "Numpad3"),
    p(0x52, 0x62, vk::NUMPAD0, "Numpad0"),
    p(0x53, 0x63, vk::DECIMAL, "NumpadDecimal"),
    p(0x54, 0x46, vk::SNAPSHOT, "PrintScreen"),
    p(0x56, 0x64, vk::OEM_102, "IntlBackslash"),
    p(0x57, 0x44, vk::F11, "F11"),
    p(0x58, 0x45, vk::F11 + 1, "F12"),
    p(0x64, 0x68, vk::F13, "F13"),
    p(0x65, 0x69, vk::F13 + 1, "F14"),
    p(0x66, 0x6a, vk::F13 + 2, "F15"),
    p(0x67, 0x6b, vk::F13 + 3, "F16"),
    p(0x68, 0x6c, vk::F13 + 4, "F17"),
    p(0x69, 0x6d, vk::F13 + 5, "F18"),
    p(0x6a, 0x6e, vk::F13 + 6, "F19"),
    p(0x6b, 0x6f, vk::F13 + 7, "F20"),
    p(0x6c, 0x70, vk::F13 + 8, "F21"),
    p(0x6d, 0x71, vk::F13 + 9, "F22"),
    p(0x6e, 0x72, vk::F13 + 10, "F23"),
    p(0x70, 0x88, vk::DBE_HIRAGANA, "KanaMode"),
    p(0x73, 0x87, vk::OEM_102, "IntlRo"),
    p(0x76, 0x73, vk::F13 + 11, "F24"),
    p(0x79, 0x8a, vk::CONVERT, "Convert"),
    p(0x7b, 0x8b, vk::NONCONVERT, "NonConvert"),
    p(0x7d, 0x89, vk::OEM_5, "IntlYen"),
    p(0x7e, 0x85, vk::SEPARATOR, "NumpadComma"),
    p(0xf1, 0x91, vk::HANJA, "Lang2"),
    p(0xf2, 0x90, vk::HANGUL, "Lang1"),
    p(0x11c, 0x58, vk::RETURN, "NumpadEnter"),
    p(0x11d, 0xe4, vk::CONTROL, "ControlRight"),
    p(0x135, 0x54, vk::DIVIDE, "NumpadDivide"),
    p(0x137, 0x46, vk::SNAPSHOT, "PrintScreen"),
    p(0x138, 0xe6, vk::MENU, "AltRight"),
    p(0x146, 0x48, vk::CANCEL, "Pause"),
    p(0x147, 0x4a, vk::HOME, "Home"),
    p(0x148, 0x52, vk::UP, "ArrowUp"),
    p(0x149, 0x4b, vk::PRIOR, "PageUp"),
    p(0x14b, 0x50, vk::LEFT, "ArrowLeft"),
    p(0x14d, 0x4f, vk::RIGHT, "ArrowRight"),
    p(0x14f, 0x4d, vk::END, "End"),
    p(0x150, 0x51, vk::DOWN, "ArrowDown"),
    p(0x151, 0x4e, vk::NEXT, "PageDown"),
    p(0x152, 0x49, vk::INSERT, "Insert"),
    p(0x153, 0x4c, vk::DELETE, "Delete"),
    p(0x15b, 0xe3, vk::LWIN, "MetaLeft"),
    p(0x15c, 0xe7, vk::RWIN, "MetaRight"),
    p(0x15d, 0x65, vk::APPS, "ContextMenu"),
    p(0x15e, 0x66, 0, "Power"),
    p(0x15f, 0, vk::SLEEP, "Sleep"),
];
/// The physical key for a set-1 code.
pub fn lookup(code: u32) -> Option<&'static Physical> {
    let code = u16::try_from(code).ok()?;
    KEYS.binary_search_by_key(&code, |k| k.code)
        .ok()
        .map(|i| &KEYS[i])
}
/// Numeric-keypad keys whose meaning follows Num Lock (no E0 prefix).
pub const fn numpad_navigation(code: u32) -> bool {
    matches!(code, 0x47..=0x49 | 0x4b..=0x4d | 0x4f..=0x53)
}
/// The navigation VK of a keypad key with Num Lock off (or Shift held).
pub const fn numpad_off_vk(code: u32) -> u8 {
    match code {
        0x47 => vk::HOME,
        0x48 => vk::UP,
        0x49 => vk::PRIOR,
        0x4b => vk::LEFT,
        0x4c => vk::CLEAR,
        0x4d => vk::RIGHT,
        0x4f => vk::END,
        0x50 => vk::DOWN,
        0x51 => vk::NEXT,
        0x52 => vk::INSERT,
        _ => vk::DELETE,
    }
}
/// Modifier keys (they never end a dead-key sequence nor dirty a chord).
pub const fn modifier(code: u32) -> bool {
    matches!(
        code,
        0x1d | 0x2a | 0x36 | 0x38 | 0x3a | 0x45 | 0x46 | 0x11d | 0x138 | 0x15b | 0x15c
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_is_sorted_unique_and_abi_valid() {
        for pair in KEYS.windows(2) {
            assert!(pair[0].code < pair[1].code, "{:?}", pair);
        }
        for k in KEYS {
            assert!(
                door_abi::graphics::valid_key_code(u32::from(k.code)),
                "{k:?}"
            );
        }
        assert_eq!(lookup(0x1e).unwrap().name, "KeyA");
        assert_eq!(lookup(0x14b).unwrap().vk, vk::LEFT);
        assert_eq!(lookup(0xf2).unwrap().usage, 0x90);
        assert!(lookup(0x80).is_none() && lookup(u32::MAX).is_none());
        assert_eq!(numpad_off_vk(0x53), vk::DELETE);
    }
}
