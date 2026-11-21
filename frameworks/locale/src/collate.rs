//! Explorer name ordering in the manner of Windows `StrCmpLogicalW`: runs of
//! digits compare by numeric value ("file2" < "file10") and letters compare
//! case-insensitively with accents as a secondary difference.
//!
//! Primary order: hyphen and apostrophe are ignorable (Windows word sort),
//! then punctuation/symbols < digits < letters < other scripts. Latin
//! letters fold case and Latin-1/Latin Extended-A accents to their base
//! letter; fullwidth ASCII folds to ASCII; katakana folds to hiragana; Greek
//! and Cyrillic fold case. Other characters (CJK ideographs, Hangul) order by
//! code point, which matches Hangul alphabetical order but not the Pinyin or
//! reading orders of zh-CN/ja-JP; all seven Door regions share this table,
//! as their Windows default sorts share the root Latin order.
//!
//! `compare_logical` returns Equal only for identical units: names equal
//! under StrCmpLogicalW (`equal_ignoring_case`) are ordered by accents, then
//! more leading zeros first ("file02" before "file2"), then lowercase before
//! uppercase, then code units. Expansions (æ, œ, ß as two letters) are not
//! applied: those letters sort by their first base letter.
use core::cmp::Ordering;

/// Base letters of U+00C0..=U+017F ('?' = not a letter).
const LATIN: &[u8; 192] = b"aaaaaaaceeeeiiiidnooooo?ouuuuytsaaaaaaaceeeeiiiidnooooo?ouuuuytyaaaaaaccccccccddddeeeeeeeeeegggggggghhhhiiiiiiiiiiiijjkkkllllllllllnnnnnnnnnoooooooorrrrrrssssssssttttttuuuuuuuuuuuuwwyyyzzzzzzs";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Class {
    Symbol = 0,
    Digit = 1,
    Letter = 2,
    Other = 3,
}
/// Primary key plus the secondary (accent/width/kana) and tertiary (case) bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Key {
    class: Class,
    primary: u32,
    accent: bool,
    upper: bool,
}
fn digit(unit: u16) -> Option<u8> {
    match unit {
        0x30..=0x39 => Some((unit - 0x30) as u8),
        0xff10..=0xff19 => Some((unit - 0xff10) as u8),
        _ => None,
    }
}
fn ignorable(unit: u16) -> bool {
    matches!(unit, 0x2d | 0x27)
}
fn key(unit: u16) -> Key {
    let (unit, width) = if (0xff01..=0xff5e).contains(&unit) {
        (unit - 0xfee0, true)
    } else {
        (unit, false)
    };
    let letter = |primary: u16, accent: bool, upper: bool| Key {
        class: Class::Letter,
        primary: u32::from(primary),
        accent: accent || width,
        upper,
    };
    match unit {
        0x41..=0x5a => letter(unit + 32, false, true),
        0x61..=0x7a => letter(unit, false, false),
        0x30..=0x39 => Key {
            class: Class::Digit,
            primary: u32::from(unit),
            accent: width,
            upper: false,
        },
        0xc0..=0x17f if LATIN[usize::from(unit - 0xc0)] != b'?' => {
            let upper = match unit {
                0xc0..=0xde => true,
                0xdf..=0xff => false,
                0x130 => true,
                0x131 | 0x138 | 0x149 | 0x17f => false,
                0x139..=0x148 | 0x179..=0x17e => unit % 2 == 1,
                0x178 => true,
                _ => unit % 2 == 0,
            };
            letter(u16::from(LATIN[usize::from(unit - 0xc0)]), true, upper)
        }
        // Greek and Cyrillic: fold capitals to small letters.
        0x391..=0x3a9 => letter(unit + 32, false, true),
        0x3b1..=0x3c9 => letter(unit, false, false),
        0x400..=0x40f => letter(unit + 0x50, false, true),
        0x410..=0x42f => letter(unit + 32, false, true),
        0x430..=0x45f => letter(unit, false, false),
        // Katakana sorts with hiragana; the script is a secondary difference.
        0x30a1..=0x30f6 => Key {
            class: Class::Other,
            primary: u32::from(unit - 0x60),
            accent: true,
            upper: false,
        },
        _ if unit < 0x80 || (0x80..0xc0).contains(&unit) || (0x2000..0x2070).contains(&unit) => {
            Key {
                class: Class::Symbol,
                primary: u32::from(unit),
                accent: width,
                upper: false,
            }
        }
        0xd7 | 0xf7 => Key {
            class: Class::Symbol,
            primary: u32::from(unit),
            accent: false,
            upper: false,
        },
        _ => Key {
            class: Class::Other,
            primary: u32::from(unit),
            accent: width,
            upper: false,
        },
    }
}
/// Differences found after an equal primary comparison, most significant first.
#[derive(Clone, Copy)]
struct Ties {
    accent: Ordering,
    zeros: Ordering,
    case: Ordering,
    ignorable: Ordering,
}
fn note(slot: &mut Ordering, order: Ordering) {
    if *slot == Ordering::Equal {
        *slot = order;
    }
}
/// Primary comparison (the StrCmpLogicalW result) and the tie-breakers.
fn primary(a: &[u16], b: &[u16]) -> (Ordering, Ties) {
    let mut ties = Ties {
        accent: Ordering::Equal,
        zeros: Ordering::Equal,
        case: Ordering::Equal,
        ignorable: Ordering::Equal,
    };
    let (mut i, mut j) = (0, 0);
    loop {
        // Ignorable punctuation contributes only a late difference.
        let (si, sj) = (i, j);
        while i < a.len() && ignorable(a[i]) {
            i += 1;
        }
        while j < b.len() && ignorable(b[j]) {
            j += 1;
        }
        note(&mut ties.ignorable, (i - si).cmp(&(j - sj)).reverse());
        match (i < a.len(), j < b.len()) {
            (false, false) => return (Ordering::Equal, ties),
            (false, true) => return (Ordering::Less, ties),
            (true, false) => return (Ordering::Greater, ties),
            _ => (),
        }
        if digit(a[i]).is_some() && digit(b[j]).is_some() {
            let (start_a, start_b) = (i, j);
            while i < a.len() && digit(a[i]).is_some() {
                i += 1;
            }
            while j < b.len() && digit(b[j]).is_some() {
                j += 1;
            }
            let (x, y) = (&a[start_a..i], &b[start_b..j]);
            let zx = x.iter().take_while(|u| digit(**u) == Some(0)).count();
            let zy = y.iter().take_while(|u| digit(**u) == Some(0)).count();
            // Value without overflow: significant length, then digits.
            let order = (x.len() - zx).cmp(&(y.len() - zy)).then_with(|| {
                x[zx..]
                    .iter()
                    .map(|u| digit(*u))
                    .cmp(y[zy..].iter().map(|u| digit(*u)))
            });
            if order != Ordering::Equal {
                return (order, ties);
            }
            note(&mut ties.zeros, zx.cmp(&zy).reverse());
            let width = x
                .iter()
                .any(|u| *u > 0x7f)
                .cmp(&y.iter().any(|u| *u > 0x7f));
            note(&mut ties.accent, width);
            continue;
        }
        let (ka, kb) = (key(a[i]), key(b[j]));
        let order = ka.class.cmp(&kb.class).then(ka.primary.cmp(&kb.primary));
        if order != Ordering::Equal {
            return (order, ties);
        }
        note(&mut ties.accent, ka.accent.cmp(&kb.accent));
        note(&mut ties.case, ka.upper.cmp(&kb.upper));
        i += 1;
        j += 1;
    }
}
/// StrCmpLogicalW equality: equal digits values, case and ignorable
/// punctuation aside (accents still differ).
pub fn equal_ignoring_case(a: &[u16], b: &[u16]) -> bool {
    let (order, ties) = primary(a, b);
    order == Ordering::Equal && ties.accent == Ordering::Equal
}
/// Total order for sorting Explorer names (see the module notes).
pub fn compare_logical(a: &[u16], b: &[u16]) -> Ordering {
    let (order, ties) = primary(a, b);
    order
        .then(ties.accent)
        .then(ties.zeros)
        .then(ties.case)
        .then(ties.ignorable)
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{string::String, vec::Vec};
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<Vec<u16>> = names.iter().map(|n| w(n)).collect();
        v.sort_by(|a, b| compare_logical(a, b));
        v.iter().map(|n| String::from_utf16(n).unwrap()).collect()
    }
    #[test]
    fn numbers_by_value_case_insensitive_like_strcmplogicalw() {
        assert_eq!(
            sorted(&[
                "file10.txt",
                "File2.txt",
                "file1.txt",
                "file02.txt",
                "a",
                "B",
                "file2.txt"
            ]),
            [
                "a",
                "B",
                "file1.txt",
                "file02.txt",
                "file2.txt",
                "File2.txt",
                "file10.txt"
            ]
        );
        assert_eq!(compare_logical(&w("file2"), &w("file10")), Ordering::Less);
        assert_eq!(
            compare_logical(
                &w("v99999999999999999999999"),
                &w("v100000000000000000000000")
            ),
            Ordering::Less
        );
        assert!(equal_ignoring_case(&w("Report 7"), &w("report 07")));
        assert!(!equal_ignoring_case(&w("resume"), &w("résumé")));
        assert_ne!(compare_logical(&w("x"), &w("X")), Ordering::Equal);
        assert_eq!(compare_logical(&w("abc"), &w("abc")), Ordering::Equal);
        assert_eq!(compare_logical(&w(""), &w("a")), Ordering::Less);
        assert_eq!(compare_logical(&w("2"), &w("10")), Ordering::Less);
        assert_eq!(compare_logical(&w("a2b"), &w("a2a")), Ordering::Greater);
    }
    #[test]
    fn symbols_digits_letters_accents_and_scripts() {
        assert_eq!(
            sorted(&["zeta", "_draft", "1st", "Äpfel", "apple", "émile", "eagle"]),
            ["_draft", "1st", "Äpfel", "apple", "eagle", "émile", "zeta"]
        );
        // Hyphen and apostrophe are ignorable in word sort.
        assert_eq!(
            sorted(&["coop", "co-op2", "cop"]),
            ["coop", "co-op2", "cop"]
        );
        // Fullwidth digits/letters fold to ASCII; Cyrillic folds case.
        assert!(!equal_ignoring_case(&w("ｆｉｌｅ２"), &w("file2")));
        assert_eq!(primary(&w("ｆｉｌｅ２"), &w("file2")).0, Ordering::Equal);
        assert_eq!(
            compare_logical(&w("ｆｉｌｅ２"), &w("file10")),
            Ordering::Less
        );
        assert_eq!(sorted(&["Яблоко", "арбуз"]), ["арбуз", "Яблоко"]);
        // Kana types share a primary; ideographs and Hangul follow letters.
        assert_eq!(primary(&w("カナ"), &w("かな")).0, Ordering::Equal);
        assert_eq!(
            sorted(&["한국", "日本", "zebra", "가나"]),
            ["zebra", "日本", "가나", "한국"]
        );
    }
}
