//! Layout tables following the Windows keyboard layouts (kbd*.dll shift
//! states, Caps Lock attributes and dead-key tables). Characters were taken
//! from the documented Windows layouts; each table is host-tested against
//! the reference sequences in this module's tests.
use crate::layout::{CAPLOK, DeadKey, KeyDef, Layout, NONE as N, d, l, la, s, sc};
use crate::vk;
const fn n(code: u16, vk: u8, base: char, shift: char) -> KeyDef {
    s(code, vk, [base, shift, N, N])
}
const fn digit(code: u16, base: char, shift: char) -> KeyDef {
    n(code, base as u8, base, shift)
}
const SPACE: KeyDef = s(0x39, vk::SPACE, [' ', ' ', ' ', ' ']);
// --- Shared dead-key compositions ----------------------------------------
const ACUTE: &[(char, char)] = &[
    ('a', 'á'),
    ('e', 'é'),
    ('i', 'í'),
    ('o', 'ó'),
    ('u', 'ú'),
    ('y', 'ý'),
    ('A', 'Á'),
    ('E', 'É'),
    ('I', 'Í'),
    ('O', 'Ó'),
    ('U', 'Ú'),
    ('Y', 'Ý'),
];
const ACUTE_CEDILLA: &[(char, char)] = &[
    ('a', 'á'),
    ('e', 'é'),
    ('i', 'í'),
    ('o', 'ó'),
    ('u', 'ú'),
    ('y', 'ý'),
    ('A', 'Á'),
    ('E', 'É'),
    ('I', 'Í'),
    ('O', 'Ó'),
    ('U', 'Ú'),
    ('Y', 'Ý'),
    ('c', 'ç'),
    ('C', 'Ç'),
];
const GRAVE: &[(char, char)] = &[
    ('a', 'à'),
    ('e', 'è'),
    ('i', 'ì'),
    ('o', 'ò'),
    ('u', 'ù'),
    ('A', 'À'),
    ('E', 'È'),
    ('I', 'Ì'),
    ('O', 'Ò'),
    ('U', 'Ù'),
];
const CIRCUMFLEX: &[(char, char)] = &[
    ('a', 'â'),
    ('e', 'ê'),
    ('i', 'î'),
    ('o', 'ô'),
    ('u', 'û'),
    ('A', 'Â'),
    ('E', 'Ê'),
    ('I', 'Î'),
    ('O', 'Ô'),
    ('U', 'Û'),
];
const DIAERESIS: &[(char, char)] = &[
    ('a', 'ä'),
    ('e', 'ë'),
    ('i', 'ï'),
    ('o', 'ö'),
    ('u', 'ü'),
    ('y', 'ÿ'),
    ('A', 'Ä'),
    ('E', 'Ë'),
    ('I', 'Ï'),
    ('O', 'Ö'),
    ('U', 'Ü'),
    ('Y', 'Ÿ'),
];
const TILDE: &[(char, char)] = &[
    ('a', 'ã'),
    ('o', 'õ'),
    ('n', 'ñ'),
    ('A', 'Ã'),
    ('O', 'Õ'),
    ('N', 'Ñ'),
];
// --- US (00000409) --------------------------------------------------------
const US_KEYS: &[KeyDef] = &[
    digit(0x02, '1', '!'),
    digit(0x03, '2', '@'),
    digit(0x04, '3', '#'),
    digit(0x05, '4', '$'),
    digit(0x06, '5', '%'),
    digit(0x07, '6', '^'),
    digit(0x08, '7', '&'),
    digit(0x09, '8', '*'),
    digit(0x0a, '9', '('),
    digit(0x0b, '0', ')'),
    n(0x0c, vk::OEM_MINUS, '-', '_'),
    n(0x0d, vk::OEM_PLUS, '=', '+'),
    l(0x10, 'q'),
    l(0x11, 'w'),
    l(0x12, 'e'),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'y'),
    l(0x16, 'u'),
    l(0x17, 'i'),
    l(0x18, 'o'),
    l(0x19, 'p'),
    n(0x1a, vk::OEM_4, '[', '{'),
    n(0x1b, vk::OEM_6, ']', '}'),
    l(0x1e, 'a'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    n(0x27, vk::OEM_1, ';', ':'),
    n(0x28, vk::OEM_7, '\'', '"'),
    n(0x29, vk::OEM_3, '`', '~'),
    n(0x2b, vk::OEM_5, '\\', '|'),
    l(0x2c, 'z'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    l(0x32, 'm'),
    n(0x33, vk::OEM_COMMA, ',', '<'),
    n(0x34, vk::OEM_PERIOD, '.', '>'),
    n(0x35, vk::OEM_2, '/', '?'),
    SPACE,
    n(0x56, vk::OEM_102, '\\', '|'),
];
pub const US: Layout = Layout {
    id: 1,
    klid: 0x0000_0409,
    name: "US",
    altgr: false,
    decimal: '.',
    keys: US_KEYS,
    dead: &[],
    right_alt_hangul: false,
    jis: false,
};
// --- United States-International (00020409) ------------------------------
const US_INTL_KEYS: &[KeyDef] = &[
    s(0x02, b'1', ['1', '!', '¡', '¹']),
    s(0x03, b'2', ['2', '@', '²', N]),
    s(0x04, b'3', ['3', '#', '³', N]),
    s(0x05, b'4', ['4', '$', '¤', '£']),
    s(0x06, b'5', ['5', '%', '€', N]),
    d(0x07, b'6', 0b0010, ['6', '^', '¼', N]),
    s(0x08, b'7', ['7', '&', '½', N]),
    s(0x09, b'8', ['8', '*', '¾', N]),
    s(0x0a, b'9', ['9', '(', '‘', N]),
    s(0x0b, b'0', ['0', ')', '’', N]),
    s(0x0c, vk::OEM_MINUS, ['-', '_', '¥', N]),
    s(0x0d, vk::OEM_PLUS, ['=', '+', '×', '÷']),
    la(0x10, 'q', 'ä', 'Ä'),
    la(0x11, 'w', 'å', 'Å'),
    la(0x12, 'e', 'é', 'É'),
    la(0x13, 'r', '®', N),
    la(0x14, 't', 'þ', 'Þ'),
    la(0x15, 'y', 'ü', 'Ü'),
    la(0x16, 'u', 'ú', 'Ú'),
    la(0x17, 'i', 'í', 'Í'),
    la(0x18, 'o', 'ó', 'Ó'),
    la(0x19, 'p', 'ö', 'Ö'),
    s(0x1a, vk::OEM_4, ['[', '{', '«', N]),
    s(0x1b, vk::OEM_6, [']', '}', '»', N]),
    la(0x1e, 'a', 'á', 'Á'),
    la(0x1f, 's', 'ß', '§'),
    la(0x20, 'd', 'ð', 'Ð'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    la(0x26, 'l', 'ø', 'Ø'),
    s(0x27, vk::OEM_1, [';', ':', '¶', '°']),
    d(0x28, vk::OEM_7, 0b0011, ['\'', '"', '´', '¨']),
    d(0x29, vk::OEM_3, 0b0011, ['`', '~', N, N]),
    s(0x2b, vk::OEM_5, ['\\', '|', '¬', '¦']),
    la(0x2c, 'z', 'æ', 'Æ'),
    l(0x2d, 'x'),
    la(0x2e, 'c', '©', '¢'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    la(0x31, 'n', 'ñ', 'Ñ'),
    la(0x32, 'm', 'µ', N),
    s(0x33, vk::OEM_COMMA, [',', '<', 'ç', 'Ç']),
    n(0x34, vk::OEM_PERIOD, '.', '>'),
    s(0x35, vk::OEM_2, ['/', '?', '¿', N]),
    SPACE,
    n(0x56, vk::OEM_102, '\\', '|'),
];
const US_INTL_DEAD: &[DeadKey] = &[
    DeadKey {
        dead: '\'',
        space: '\'',
        pairs: ACUTE_CEDILLA,
    },
    DeadKey {
        dead: '"',
        space: '"',
        pairs: DIAERESIS,
    },
    DeadKey {
        dead: '`',
        space: '`',
        pairs: GRAVE,
    },
    DeadKey {
        dead: '~',
        space: '~',
        pairs: TILDE,
    },
    DeadKey {
        dead: '^',
        space: '^',
        pairs: CIRCUMFLEX,
    },
];
pub const US_INTERNATIONAL: Layout = Layout {
    id: 2,
    klid: 0x0002_0409,
    name: "United States-International",
    altgr: true,
    decimal: '.',
    keys: US_INTL_KEYS,
    dead: US_INTL_DEAD,
    right_alt_hangul: false,
    jis: false,
};
// --- United Kingdom (00000809) -------------------------------------------
const UK_KEYS: &[KeyDef] = &[
    digit(0x02, '1', '!'),
    digit(0x03, '2', '"'),
    digit(0x04, '3', '£'),
    s(0x05, b'4', ['4', '$', '€', N]),
    digit(0x06, '5', '%'),
    digit(0x07, '6', '^'),
    digit(0x08, '7', '&'),
    digit(0x09, '8', '*'),
    digit(0x0a, '9', '('),
    digit(0x0b, '0', ')'),
    n(0x0c, vk::OEM_MINUS, '-', '_'),
    n(0x0d, vk::OEM_PLUS, '=', '+'),
    l(0x10, 'q'),
    l(0x11, 'w'),
    la(0x12, 'e', 'é', 'É'),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'y'),
    la(0x16, 'u', 'ú', 'Ú'),
    la(0x17, 'i', 'í', 'Í'),
    la(0x18, 'o', 'ó', 'Ó'),
    l(0x19, 'p'),
    n(0x1a, vk::OEM_4, '[', '{'),
    n(0x1b, vk::OEM_6, ']', '}'),
    la(0x1e, 'a', 'á', 'Á'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    n(0x27, vk::OEM_1, ';', ':'),
    n(0x28, vk::OEM_3, '\'', '@'),
    s(0x29, vk::OEM_8, ['`', '¬', '¦', N]),
    n(0x2b, vk::OEM_7, '#', '~'),
    l(0x2c, 'z'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    l(0x32, 'm'),
    n(0x33, vk::OEM_COMMA, ',', '<'),
    n(0x34, vk::OEM_PERIOD, '.', '>'),
    n(0x35, vk::OEM_2, '/', '?'),
    SPACE,
    n(0x56, vk::OEM_5, '\\', '|'),
];
pub const UNITED_KINGDOM: Layout = Layout {
    id: 3,
    klid: 0x0000_0809,
    name: "United Kingdom",
    altgr: true,
    decimal: '.',
    keys: UK_KEYS,
    dead: &[],
    right_alt_hangul: false,
    jis: false,
};
// --- Italian (00000410) ---------------------------------------------------
const IT_KEYS: &[KeyDef] = &[
    digit(0x02, '1', '!'),
    digit(0x03, '2', '"'),
    digit(0x04, '3', '£'),
    digit(0x05, '4', '$'),
    s(0x06, b'5', ['5', '%', '€', N]),
    digit(0x07, '6', '&'),
    digit(0x08, '7', '/'),
    digit(0x09, '8', '('),
    digit(0x0a, '9', ')'),
    digit(0x0b, '0', '='),
    n(0x0c, vk::OEM_4, '\'', '?'),
    n(0x0d, vk::OEM_6, 'ì', '^'),
    l(0x10, 'q'),
    l(0x11, 'w'),
    la(0x12, 'e', '€', N),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'y'),
    l(0x16, 'u'),
    l(0x17, 'i'),
    l(0x18, 'o'),
    l(0x19, 'p'),
    s(0x1a, vk::OEM_1, ['è', 'é', '[', '{']),
    s(0x1b, vk::OEM_PLUS, ['+', '*', ']', '}']),
    l(0x1e, 'a'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    s(0x27, vk::OEM_3, ['ò', 'ç', '@', N]),
    s(0x28, vk::OEM_7, ['à', '°', '#', N]),
    n(0x29, vk::OEM_5, '\\', '|'),
    n(0x2b, vk::OEM_2, 'ù', '§'),
    l(0x2c, 'z'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    l(0x32, 'm'),
    n(0x33, vk::OEM_COMMA, ',', ';'),
    n(0x34, vk::OEM_PERIOD, '.', ':'),
    n(0x35, vk::OEM_MINUS, '-', '_'),
    SPACE,
    n(0x56, vk::OEM_102, '<', '>'),
];
pub const ITALIAN: Layout = Layout {
    id: 4,
    klid: 0x0000_0410,
    name: "Italian",
    altgr: true,
    decimal: '.',
    keys: IT_KEYS,
    dead: &[],
    right_alt_hangul: false,
    jis: false,
};
// --- German (00000407) ----------------------------------------------------
const DE_KEYS: &[KeyDef] = &[
    digit(0x02, '1', '!'),
    s(0x03, b'2', ['2', '"', '²', N]),
    s(0x04, b'3', ['3', '§', '³', N]),
    digit(0x05, '4', '$'),
    digit(0x06, '5', '%'),
    digit(0x07, '6', '&'),
    s(0x08, b'7', ['7', '/', '{', N]),
    s(0x09, b'8', ['8', '(', '[', N]),
    s(0x0a, b'9', ['9', ')', ']', N]),
    s(0x0b, b'0', ['0', '=', '}', N]),
    s(0x0c, vk::OEM_4, ['ß', '?', '\\', N]),
    d(0x0d, vk::OEM_6, 0b0011, ['´', '`', N, N]),
    la(0x10, 'q', '@', N),
    l(0x11, 'w'),
    la(0x12, 'e', '€', N),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'z'),
    l(0x16, 'u'),
    l(0x17, 'i'),
    l(0x18, 'o'),
    l(0x19, 'p'),
    sc(0x1a, vk::OEM_1, ['ü', 'Ü', N, N]),
    s(0x1b, vk::OEM_PLUS, ['+', '*', '~', N]),
    l(0x1e, 'a'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    sc(0x27, vk::OEM_3, ['ö', 'Ö', N, N]),
    sc(0x28, vk::OEM_7, ['ä', 'Ä', N, N]),
    d(0x29, vk::OEM_5, 0b0001, ['^', '°', N, N]),
    n(0x2b, vk::OEM_2, '#', '\''),
    l(0x2c, 'y'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    la(0x32, 'm', 'µ', N),
    n(0x33, vk::OEM_COMMA, ',', ';'),
    n(0x34, vk::OEM_PERIOD, '.', ':'),
    n(0x35, vk::OEM_MINUS, '-', '_'),
    SPACE,
    s(0x56, vk::OEM_102, ['<', '>', '|', N]),
];
const DE_DEAD: &[DeadKey] = &[
    DeadKey {
        dead: '^',
        space: '^',
        pairs: CIRCUMFLEX,
    },
    DeadKey {
        dead: '´',
        space: '´',
        pairs: ACUTE,
    },
    DeadKey {
        dead: '`',
        space: '`',
        pairs: GRAVE,
    },
];
pub const GERMAN: Layout = Layout {
    id: 5,
    klid: 0x0000_0407,
    name: "German",
    altgr: true,
    decimal: ',',
    keys: DE_KEYS,
    dead: DE_DEAD,
    right_alt_hangul: false,
    jis: false,
};
// --- French (0000040C), AZERTY -------------------------------------------
const FR_KEYS: &[KeyDef] = &[
    sc(0x02, b'1', ['&', '1', N, N]),
    KeyDef {
        code: 0x03,
        vk: b'2',
        caps: CAPLOK,
        dead: 0b0100,
        chars: ['é', '2', '~', N],
    },
    sc(0x04, b'3', ['"', '3', '#', N]),
    sc(0x05, b'4', ['\'', '4', '{', N]),
    sc(0x06, b'5', ['(', '5', '[', N]),
    sc(0x07, b'6', ['-', '6', '|', N]),
    KeyDef {
        code: 0x08,
        vk: b'7',
        caps: CAPLOK,
        dead: 0b0100,
        chars: ['è', '7', '`', N],
    },
    sc(0x09, b'8', ['_', '8', '\\', N]),
    sc(0x0a, b'9', ['ç', '9', '^', N]),
    sc(0x0b, b'0', ['à', '0', '@', N]),
    s(0x0c, vk::OEM_4, [')', '°', ']', N]),
    s(0x0d, vk::OEM_PLUS, ['=', '+', '}', N]),
    l(0x10, 'a'),
    l(0x11, 'z'),
    la(0x12, 'e', '€', N),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'y'),
    l(0x16, 'u'),
    l(0x17, 'i'),
    l(0x18, 'o'),
    l(0x19, 'p'),
    d(0x1a, vk::OEM_6, 0b0011, ['^', '¨', N, N]),
    s(0x1b, vk::OEM_1, ['$', '£', '¤', N]),
    l(0x1e, 'q'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    l(0x27, 'm'),
    n(0x28, vk::OEM_3, 'ù', '%'),
    n(0x29, vk::OEM_7, '²', N),
    n(0x2b, vk::OEM_5, '*', 'µ'),
    l(0x2c, 'w'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    n(0x32, vk::OEM_COMMA, ',', '?'),
    n(0x33, vk::OEM_PERIOD, ';', '.'),
    n(0x34, vk::OEM_2, ':', '/'),
    n(0x35, vk::OEM_8, '!', '§'),
    SPACE,
    n(0x56, vk::OEM_102, '<', '>'),
];
const FR_DEAD: &[DeadKey] = &[
    DeadKey {
        dead: '^',
        space: '^',
        pairs: CIRCUMFLEX,
    },
    DeadKey {
        dead: '¨',
        space: '¨',
        pairs: DIAERESIS,
    },
    DeadKey {
        dead: '~',
        space: '~',
        pairs: TILDE,
    },
    DeadKey {
        dead: '`',
        space: '`',
        pairs: GRAVE,
    },
];
pub const FRENCH: Layout = Layout {
    id: 6,
    klid: 0x0000_040c,
    name: "French",
    altgr: true,
    decimal: '.',
    keys: FR_KEYS,
    dead: FR_DEAD,
    right_alt_hangul: false,
    jis: false,
};
// --- Japanese 106/109 (00000411), used with the Japanese IME -------------
const JIS_KEYS: &[KeyDef] = &[
    digit(0x02, '1', '!'),
    digit(0x03, '2', '"'),
    digit(0x04, '3', '#'),
    digit(0x05, '4', '$'),
    digit(0x06, '5', '%'),
    digit(0x07, '6', '&'),
    digit(0x08, '7', '\''),
    digit(0x09, '8', '('),
    digit(0x0a, '9', ')'),
    digit(0x0b, '0', N),
    n(0x0c, vk::OEM_MINUS, '-', '='),
    n(0x0d, vk::OEM_7, '^', '~'),
    l(0x10, 'q'),
    l(0x11, 'w'),
    l(0x12, 'e'),
    l(0x13, 'r'),
    l(0x14, 't'),
    l(0x15, 'y'),
    l(0x16, 'u'),
    l(0x17, 'i'),
    l(0x18, 'o'),
    l(0x19, 'p'),
    n(0x1a, vk::OEM_3, '@', '`'),
    n(0x1b, vk::OEM_4, '[', '{'),
    l(0x1e, 'a'),
    l(0x1f, 's'),
    l(0x20, 'd'),
    l(0x21, 'f'),
    l(0x22, 'g'),
    l(0x23, 'h'),
    l(0x24, 'j'),
    l(0x25, 'k'),
    l(0x26, 'l'),
    n(0x27, vk::OEM_PLUS, ';', '+'),
    n(0x28, vk::OEM_1, ':', '*'),
    n(0x2b, vk::OEM_6, ']', '}'),
    l(0x2c, 'z'),
    l(0x2d, 'x'),
    l(0x2e, 'c'),
    l(0x2f, 'v'),
    l(0x30, 'b'),
    l(0x31, 'n'),
    l(0x32, 'm'),
    n(0x33, vk::OEM_COMMA, ',', '<'),
    n(0x34, vk::OEM_PERIOD, '.', '>'),
    n(0x35, vk::OEM_2, '/', '?'),
    SPACE,
    n(0x73, vk::OEM_102, '\\', '_'),
    // Windows shows U+005C as the yen sign in Japanese fonts.
    n(0x7d, vk::OEM_5, '\\', '|'),
];
pub const JAPANESE: Layout = Layout {
    id: 7,
    klid: 0x0000_0411,
    name: "Japanese",
    altgr: false,
    decimal: '.',
    keys: JIS_KEYS,
    dead: &[],
    right_alt_hangul: false,
    jis: true,
};
// --- Korean (00000412): US positions, Type 1 Hangul/Hanja keys ------------
pub const KOREAN: Layout = Layout {
    id: 8,
    klid: 0x0000_0412,
    name: "Korean",
    altgr: false,
    decimal: '.',
    keys: US_KEYS,
    dead: &[],
    right_alt_hangul: true,
    jis: false,
};
pub const ALL: [&Layout; 8] = [
    &US,
    &US_INTERNATIONAL,
    &UNITED_KINGDOM,
    &ITALIAN,
    &GERMAN,
    &FRENCH,
    &JAPANESE,
    &KOREAN,
];
pub fn by_id(id: u8) -> Option<&'static Layout> {
    ALL.iter().copied().find(|l| l.id == id)
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::layout::Translator;
    use door_abi::graphics as g;
    use std::string::String;
    /// Type a sequence: `(code, flags)` presses (each followed by a release).
    fn typed(layout: &Layout, keys: &[(u32, u32)]) -> String {
        let mut t = Translator::new();
        let mut out = String::new();
        for &(code, flags) in keys {
            let r = t.translate(layout, code, flags | g::DOWN);
            out.extend(r.text.chars());
            t.translate(layout, code, flags);
        }
        out
    }
    const SH: u32 = g::SHIFT | g::LEFT_SHIFT;
    const AG: u32 = g::ALT | g::RIGHT_ALT;
    const CAPS: u32 = g::CAPS_LOCK;
    #[test]
    fn tables_are_sorted_and_use_valid_codes() {
        for layout in ALL {
            for pair in layout.keys.windows(2) {
                assert!(pair[0].code < pair[1].code, "{} {:?}", layout.name, pair);
            }
            for k in layout.keys {
                assert!(
                    crate::physical::lookup(u32::from(k.code)).is_some(),
                    "{} {:#x}",
                    layout.name,
                    k.code
                );
                assert!(k.vk != 0);
                for (level, c) in k.chars.iter().enumerate() {
                    if k.dead & (1 << level) != 0 {
                        assert!(layout.dead_key(*c).is_some(), "{} dead {c}", layout.name);
                    }
                }
            }
        }
        let mut ids: std::vec::Vec<u8> = ALL.iter().map(|l| l.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
    }
    #[test]
    fn us_and_uk_basics() {
        assert_eq!(typed(&US, &[(0x23, SH), (0x17, 0), (0x02, SH)]), "Hi!");
        assert_eq!(
            typed(&US, &[(0x1e, CAPS), (0x1e, CAPS | SH), (0x02, CAPS)]),
            "Aa1"
        );
        assert_eq!(
            typed(
                &UNITED_KINGDOM,
                &[(0x03, SH), (0x04, SH), (0x28, SH), (0x2b, 0), (0x2b, SH)]
            ),
            "\"£@#~"
        );
        assert_eq!(
            typed(
                &UNITED_KINGDOM,
                &[(0x05, AG), (0x1e, AG), (0x1e, AG | SH), (0x1e, AG | CAPS)]
            ),
            "€áÁÁ"
        );
        assert_eq!(typed(&UNITED_KINGDOM, &[(0x29, AG), (0x29, SH)]), "¦¬");
    }
    #[test]
    fn us_international_dead_keys_and_altgr() {
        let l = &US_INTERNATIONAL;
        // ' e → é, " u → ü, ` a → à, ~ n → ñ, Shift+6 o → ô.
        assert_eq!(
            typed(
                l,
                &[
                    (0x28, 0),
                    (0x12, 0),
                    (0x28, SH),
                    (0x16, 0),
                    (0x29, 0),
                    (0x1e, 0),
                    (0x29, SH),
                    (0x31, 0),
                    (0x07, SH),
                    (0x18, 0)
                ]
            ),
            "éüàñô"
        );
        // Dead key + Space gives the spacing character; unmatched gives both.
        assert_eq!(
            typed(
                l,
                &[
                    (0x28, 0),
                    (0x39, 0),
                    (0x28, 0),
                    (0x14, 0),
                    (0x28, 0),
                    (0x2e, SH)
                ]
            ),
            "''tÇ"
        );
        // Same dead key twice: both characters (no DEADTRANS entry).
        assert_eq!(typed(l, &[(0x28, SH), (0x28, SH)]), "\"\"");
        // AltGr layer and Shift+AltGr, Ctrl+Alt alias, Caps on AltGr letters.
        assert_eq!(
            typed(
                l,
                &[
                    (0x10, AG),
                    (0x10, AG | SH),
                    (0x06, AG),
                    (0x1f, AG),
                    (0x10, AG | CAPS)
                ]
            ),
            "äÄ€ßÄ"
        );
        assert_eq!(
            typed(
                l,
                &[(0x12, g::CONTROL | g::LEFT_CONTROL | g::ALT | g::LEFT_ALT)]
            ),
            "é"
        );
        // Escape cancels a pending dead key; a modifier does not.
        assert_eq!(typed(l, &[(0x28, 0), (0x01, 0), (0x12, 0)]), "e");
        assert_eq!(typed(l, &[(0x28, 0), (0x2a, SH), (0x12, SH)]), "É");
    }
    #[test]
    fn italian_layout() {
        let l = &ITALIAN;
        assert_eq!(
            typed(
                l,
                &[
                    (0x1a, 0),
                    (0x1a, SH),
                    (0x27, 0),
                    (0x28, 0),
                    (0x2b, 0),
                    (0x0d, 0)
                ]
            ),
            "èéòàùì"
        );
        assert_eq!(
            typed(
                l,
                &[
                    (0x1a, AG),
                    (0x1a, AG | SH),
                    (0x1b, AG),
                    (0x27, AG),
                    (0x28, AG),
                    (0x12, AG),
                    (0x06, AG)
                ]
            ),
            "[{]@#€€"
        );
        assert_eq!(
            typed(
                l,
                &[(0x08, SH), (0x0b, SH), (0x56, 0), (0x56, SH), (0x35, SH)]
            ),
            "/=<>_"
        );
        // Caps Lock does not capitalise the accented keys (no CAPLOK).
        assert_eq!(typed(l, &[(0x1a, CAPS), (0x1e, CAPS)]), "èA");
    }
    #[test]
    fn german_layout_dead_keys_and_qwertz() {
        let l = &GERMAN;
        assert_eq!(
            typed(
                l,
                &[
                    (0x15, 0),
                    (0x2c, 0),
                    (0x1a, 0),
                    (0x27, SH),
                    (0x28, CAPS),
                    (0x0c, 0)
                ]
            ),
            "zyüÖÄß"
        );
        assert_eq!(
            typed(
                l,
                &[
                    (0x10, AG),
                    (0x12, AG),
                    (0x0c, AG),
                    (0x56, AG),
                    (0x08, AG),
                    (0x32, AG)
                ]
            ),
            "@€\\|{µ"
        );
        // ^ e → ê, ´ a → á, Shift+´ (`) o → ò, ^ Space → ^, ´ x → ´x.
        assert_eq!(
            typed(
                l,
                &[
                    (0x29, 0),
                    (0x12, 0),
                    (0x0d, 0),
                    (0x1e, 0),
                    (0x0d, SH),
                    (0x18, 0),
                    (0x29, 0),
                    (0x39, 0),
                    (0x0d, 0),
                    (0x2d, 0)
                ]
            ),
            "êáò^´x"
        );
        let mut t = Translator::new();
        let ctrl_z = t.translate(l, 0x15, g::DOWN | g::CONTROL | g::LEFT_CONTROL);
        assert_eq!(
            (ctrl_z.vk, ctrl_z.legacy, ctrl_z.text.is_empty()),
            (b'Z', 26, true),
            "VK follows the label"
        );
        assert_eq!(
            t.translate(l, 0x53, g::DOWN | g::NUM_LOCK).text.chars(),
            &[',']
        );
    }
    #[test]
    fn french_azerty_numbers_dead_keys_and_caps() {
        let l = &FRENCH;
        assert_eq!(
            typed(l, &[(0x10, 0), (0x11, 0), (0x1e, 0), (0x2c, 0), (0x27, 0)]),
            "azqwm"
        );
        assert_eq!(
            typed(
                l,
                &[(0x02, 0), (0x03, 0), (0x02, SH), (0x03, CAPS), (0x0b, AG)]
            ),
            "&é12@"
        );
        // ^ e → ê; Shift+^ (¨) i → ï; AltGr+é (~) n → ñ; AltGr+è (`) a → à.
        assert_eq!(
            typed(
                l,
                &[
                    (0x1a, 0),
                    (0x12, 0),
                    (0x1a, SH),
                    (0x17, 0),
                    (0x03, AG),
                    (0x31, 0),
                    (0x08, AG),
                    (0x10, 0)
                ]
            ),
            "êïñà"
        );
        let mut t = Translator::new();
        assert_eq!(t.translate(l, 0x10, g::DOWN).vk, b'A');
        assert_eq!(t.translate(l, 0x1e, g::DOWN).vk, b'Q');
        assert_eq!(t.translate(l, 0x27, g::DOWN).vk, b'M');
    }
    #[test]
    fn japanese_and_korean_special_keys() {
        let mut t = Translator::new();
        let zen = t.translate(&JAPANESE, 0x29, g::DOWN);
        assert_eq!((zen.vk, zen.text.is_empty()), (vk::KANJI, true));
        assert_eq!(t.translate(&JAPANESE, 0x79, g::DOWN).vk, vk::CONVERT);
        assert_eq!(
            typed(
                &JAPANESE,
                &[(0x1a, 0), (0x28, SH), (0x7d, 0), (0x73, SH), (0x0d, SH)]
            ),
            "@*\\_~"
        );
        let hangul = t.translate(&KOREAN, 0x138, g::DOWN | g::ALT | g::RIGHT_ALT);
        assert_eq!(hangul.vk, vk::HANGUL);
        assert_eq!(
            t.translate(&KOREAN, 0x11d, g::DOWN | g::CONTROL | g::RIGHT_CONTROL)
                .vk,
            vk::HANJA
        );
        assert_eq!(t.translate(&KOREAN, 0xf2, g::DOWN).vk, vk::HANGUL);
        // Right Alt held on Korean is not Alt: letters still type.
        assert_eq!(typed(&KOREAN, &[(0x1e, AG)]), "a");
    }
    #[test]
    fn numpad_chords_and_legacy_bytes() {
        let mut t = Translator::new();
        let digit = t.translate(&US, 0x48, g::DOWN | g::NUM_LOCK);
        assert_eq!(
            (digit.vk, digit.text.chars(), digit.legacy),
            (vk::NUMPAD0 + 8, &['8'][..], b'8')
        );
        let nav = t.translate(&US, 0x48, g::DOWN);
        assert_eq!((nav.vk, nav.text.is_empty()), (vk::UP, true));
        let shifted = t.translate(&US, 0x48, g::DOWN | g::NUM_LOCK | SH);
        assert_eq!(shifted.vk, vk::UP, "Shift inverts Num Lock");
        let enter = t.translate(&US, 0x1c, g::DOWN);
        assert_eq!(
            (enter.vk, enter.legacy, enter.text.is_empty()),
            (vk::RETURN, 10, true)
        );
        let ctrl_c = t.translate(&US, 0x2e, g::DOWN | g::CONTROL | g::LEFT_CONTROL);
        assert_eq!((ctrl_c.legacy, ctrl_c.text.is_empty()), (3, true));
        let alt_f = t.translate(&US, 0x21, g::DOWN | g::ALT | g::LEFT_ALT);
        assert_eq!((alt_f.legacy, alt_f.text.is_empty()), (b'f', true));
        let release = t.translate(&US, 0x1e, 0);
        assert!(release.text.is_empty() && release.legacy == 0 && release.vk == b'A');
        // AltGr text is flagged so legacy packing can drop Ctrl/Alt.
        let at = t.translate(&ITALIAN, 0x27, g::DOWN | AG);
        assert_eq!(
            (at.legacy, at.status & crate::layout::ALTGR),
            (b'@', crate::layout::ALTGR)
        );
        let euro = t.translate(&GERMAN, 0x12, g::DOWN | AG);
        assert_eq!((euro.legacy, euro.text.chars()), (0, &['€'][..]));
        let mut buf = [0u8; 16];
        let len = euro.text.encode(&mut buf);
        assert_eq!(&buf[..len], "€".as_bytes());
    }
}
