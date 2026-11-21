//! Unicode analysis: grapheme clusters and words (UAX #29), line-break
//! opportunities (UAX #14), script itemization (UAX #24 runs) and emoji
//! presentation. All offsets are UTF-8 byte offsets into the caller's `str`;
//! UTF-16 helpers convert for Windows and IME adapters without splitting
//! surrogate pairs.
use alloc::vec::Vec;
use core::ops::Range;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// Byte offset of every grapheme cluster boundary, including 0 and `text.len()`.
pub fn grapheme_boundaries(text: &str) -> Vec<usize> {
    let mut out: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    out.push(text.len());
    out
}

fn floor_char(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Next grapheme boundary strictly after `offset` (or `text.len()`).
pub fn next_grapheme(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset);
    let mut cursor = unicode_segmentation::GraphemeCursor::new(offset, text.len(), true);
    cursor
        .next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

/// Previous grapheme boundary strictly before `offset` (or 0).
pub fn prev_grapheme(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset);
    let mut cursor = unicode_segmentation::GraphemeCursor::new(offset, text.len(), true);
    cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
}

/// Snap an arbitrary byte offset to the grapheme boundary at or before it.
pub fn grapheme_floor(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset);
    let mut cursor = unicode_segmentation::GraphemeCursor::new(offset, text.len(), true);
    if cursor.is_boundary(text, 0).unwrap_or(true) {
        offset
    } else {
        prev_grapheme(text, offset)
    }
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(|c| c.is_alphanumeric())
}

/// Ctrl+Right: start of the next word (UAX #29), skipping punctuation and spaces
/// the way Windows edit controls do; `text.len()` at the end.
pub fn next_word(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset);
    let mut passed_current = false;
    for (start, segment) in text.split_word_bound_indices() {
        let end = start + segment.len();
        if end <= offset {
            continue;
        }
        if start < offset || (start == offset && is_word(segment)) {
            passed_current = true;
            continue;
        }
        if is_word(segment) && (passed_current || start > offset) {
            return start;
        }
        if segment.contains(['\n', '\r', '\u{2029}']) {
            return start;
        }
        passed_current = true;
    }
    text.len()
}

/// Ctrl+Left: start of the word before `offset` (or 0).
pub fn prev_word(text: &str, offset: usize) -> usize {
    let offset = floor_char(text, offset);
    let mut best = 0;
    for (start, segment) in text.split_word_bound_indices() {
        if start >= offset {
            break;
        }
        if is_word(segment) {
            best = start;
        }
    }
    best
}

/// Word (or other UAX #29 segment) containing `offset`, for double-click selection.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let offset = floor_char(text, offset);
    let mut last = 0..0;
    for (start, segment) in text.split_word_bound_indices() {
        let range = start..start + segment.len();
        if range.contains(&offset) {
            return range;
        }
        last = range;
    }
    if offset == text.len() {
        last
    } else {
        offset..offset
    }
}

/// UAX #14 line-break opportunity before a byte offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Break {
    /// A line may wrap here.
    Allowed,
    /// The line must end here (paragraph end or explicit line break).
    Mandatory,
}

/// Break opportunities as `(offset, kind)`; the final entry is `text.len()`.
pub fn line_breaks(text: &str) -> Vec<(usize, Break)> {
    unicode_linebreak::linebreaks(text)
        .map(|(offset, kind)| {
            (
                offset,
                match kind {
                    unicode_linebreak::BreakOpportunity::Mandatory => Break::Mandatory,
                    unicode_linebreak::BreakOpportunity::Allowed => Break::Allowed,
                },
            )
        })
        .collect()
}

/// UTF-16 length of a string.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// UTF-8 byte offset to UTF-16 code-unit offset (offset is snapped to a char).
pub fn utf8_to_utf16(text: &str, offset: usize) -> usize {
    utf16_len(&text[..floor_char(text, offset)])
}

/// UTF-16 code-unit offset to UTF-8 byte offset. An offset inside a surrogate
/// pair snaps back to the start of that scalar value, never splitting it.
pub fn utf16_to_utf8(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (index, ch) in text.char_indices() {
        let next = seen + ch.len_utf16();
        if next > units {
            return index;
        }
        seen = next;
    }
    text.len()
}

/// Emoji_Presentation=Yes (Unicode 16 emoji-data.txt), sorted inclusive ranges.
const EMOJI_PRESENTATION: &[(u32, u32)] = &[
    (0x231A, 0x231B),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1E6, 0x1F1FF),
    (0x1F201, 0x1F201),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F236),
    (0x1F238, 0x1F23A),
    (0x1F250, 0x1F251),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D7),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FA89),
    (0x1FA8F, 0x1FAC6),
    (0x1FACE, 0x1FADC),
    (0x1FADF, 0x1FAE9),
    (0x1FAF0, 0x1FAF8),
];

fn in_ranges(table: &[(u32, u32)], c: u32) -> bool {
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                core::cmp::Ordering::Less
            } else if lo > c {
                core::cmp::Ordering::Greater
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Emoji_Presentation property (default colour presentation).
pub fn is_emoji_presentation(ch: char) -> bool {
    in_ranges(EMOJI_PRESENTATION, ch as u32)
}

/// Whether a grapheme cluster should be drawn from the colour emoji face:
/// VS16, emoji modifiers, ZWJ sequences, flags, keycaps and tag sequences, or
/// a default-emoji base without VS15.
pub fn cluster_is_emoji(cluster: &str) -> bool {
    let mut chars = cluster.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let mut emoji = is_emoji_presentation(first);
    for ch in cluster.chars() {
        match ch as u32 {
            0xFE0E => return false,
            0xFE0F | 0x20E3 | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F => emoji = true,
            0x200D if (first as u32) >= 0x2000 => emoji = true,
            _ => {}
        }
    }
    emoji
}

pub(crate) fn is_default_ignorable(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F
        | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8 | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A
        | 0xE0000..=0xE0FFF)
}

/// Resolved script runs of `text` (UAX #24): Common and Inherited characters
/// join the surrounding script, paired brackets take the script in effect at
/// their opening bracket, and a leading Common run takes the first real script.
pub(crate) fn script_runs(text: &str) -> Vec<(Range<usize>, Script)> {
    let mut runs: Vec<(Range<usize>, Script)> = Vec::new();
    let mut current = Script::Common;
    // Expected closing bracket and the script in effect at its opening bracket.
    let mut brackets: Vec<(char, Script)> = Vec::new();
    for (index, ch) in text.char_indices() {
        let raw = ch.script();
        let script = if matches!(raw, Script::Common | Script::Inherited | Script::Unknown) {
            if let Some(close) = closing_bracket(ch) {
                if brackets.len() < 64 {
                    brackets.push((close, current));
                }
                current
            } else if let Some(at) = brackets.iter().rposition(|(c, _)| *c == ch) {
                let saved = brackets[at].1;
                brackets.truncate(at);
                if saved == Script::Common {
                    current
                } else {
                    saved
                }
            } else {
                current
            }
        } else {
            if current == Script::Common {
                for entry in brackets.iter_mut().filter(|e| e.1 == Script::Common) {
                    entry.1 = raw;
                }
            }
            current = raw;
            raw
        };
        let end = index + ch.len_utf8();
        match runs.last_mut() {
            Some(last) if last.1 == script => last.0.end = end,
            _ => runs.push((index..end, script)),
        }
    }
    if runs.len() > 1 && runs[0].1 == Script::Common {
        let first = runs.remove(0);
        runs[0].0.start = first.0.start;
    }
    runs
}

const BRACKETS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('\u{ab}', '\u{bb}'),
    ('\u{2018}', '\u{2019}'),
    ('\u{201c}', '\u{201d}'),
    ('\u{2039}', '\u{203a}'),
    ('\u{3008}', '\u{3009}'),
    ('\u{300a}', '\u{300b}'),
    ('\u{300c}', '\u{300d}'),
    ('\u{300e}', '\u{300f}'),
    ('\u{3010}', '\u{3011}'),
    ('\u{ff08}', '\u{ff09}'),
    ('\u{ff3b}', '\u{ff3d}'),
    ('\u{ff5b}', '\u{ff5d}'),
];
fn closing_bracket(open: char) -> Option<char> {
    BRACKETS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

/// harfrust script for a Unicode script (ISO 15924 tag).
pub(crate) fn shaping_script(script: Script) -> Option<harfrust::Script> {
    let name = script.short_name().as_bytes();
    if name.len() != 4 {
        return None;
    }
    harfrust::Script::from_iso15924_tag(harfrust::Tag::new(&[name[0], name[1], name[2], name[3]]))
}
