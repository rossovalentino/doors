//! Single-line shell text fields (Start search, Run): Unicode insertion from
//! key text and IME commits, a byte caret kept on grapheme-cluster
//! boundaries (UAX #29 through Door Text), and the inline IME composition.
//! A field is `(text, caret)` owned by its model; every helper clamps a
//! caret that the model moved or invalidated (`text.clear()`).
extern crate alloc;
use alloc::string::String;
use door_graphics_text::{grapheme_floor, next_grapheme, prev_grapheme};
/// The IME composition shown inline at the caret (`text_input::Record::Preedit`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// Byte cursor inside `text` (None: hidden, painted at its end).
    pub cursor: Option<usize>,
}
/// Caret clamped into `text` on a grapheme boundary.
pub fn caret(text: &str, caret: usize) -> usize {
    grapheme_floor(text, caret.min(text.len()))
}
/// Insert `s` (control characters dropped) at the caret while `fits` accepts
/// the resulting text; returns whether anything was inserted.
pub fn insert(text: &mut String, at: &mut usize, s: &str, fits: impl Fn(&str) -> bool) -> bool {
    let mut pos = caret(text, *at);
    let mut changed = false;
    for ch in s.chars().filter(|c| !c.is_control()) {
        let mut next = String::with_capacity(text.len() + ch.len_utf8());
        next.push_str(&text[..pos]);
        next.push(ch);
        next.push_str(&text[pos..]);
        if !fits(&next) {
            break;
        }
        *text = next;
        pos += ch.len_utf8();
        changed = true;
    }
    // A combining mark joins the previous cluster: keep the caret after it.
    *at = pos;
    changed
}
/// Backspace: remove the grapheme cluster before the caret.
pub fn backspace(text: &mut String, at: &mut usize) -> bool {
    let end = caret(text, *at);
    if end == 0 {
        *at = 0;
        return false;
    }
    let start = prev_grapheme(text, end);
    text.replace_range(start..end, "");
    *at = start;
    true
}
/// Delete: remove the grapheme cluster after the caret.
pub fn delete(text: &mut String, at: &mut usize) -> bool {
    let start = caret(text, *at);
    *at = start;
    if start >= text.len() {
        return false;
    }
    let end = next_grapheme(text, start);
    text.replace_range(start..end, "");
    true
}
/// Caret keys (scan codes): Left, Right, Home, End. Returns true when the
/// key is a caret key (handled even when the caret could not move).
pub fn move_caret(text: &str, at: &mut usize, scan: u32) -> bool {
    let pos = caret(text, *at);
    *at = match scan {
        0x14b => prev_grapheme(text, pos),
        0x14d => next_grapheme(text, pos).min(text.len()),
        0x147 => 0,
        0x14f => text.len(),
        _ => return false,
    };
    true
}
/// IME delete-surrounding: `before`/`after` UTF-8 bytes around the caret,
/// widened to whole characters.
pub fn delete_around(text: &mut String, at: &mut usize, before: usize, after: usize) -> bool {
    let pos = caret(text, *at);
    let mut start = pos.saturating_sub(before);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (pos + after).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    if start == end {
        return false;
    }
    text.replace_range(start..end, "");
    *at = start;
    true
}
/// What a field paints: the text with the composition spliced in at the
/// caret, the composition's byte range in it, and the painted caret.
pub fn display<'a>(
    text: &'a str,
    at: usize,
    preedit: &str,
    preedit_cursor: Option<usize>,
) -> (alloc::borrow::Cow<'a, str>, core::ops::Range<usize>, usize) {
    let pos = caret(text, at);
    if preedit.is_empty() {
        return (alloc::borrow::Cow::Borrowed(text), pos..pos, pos);
    }
    let mut s = String::with_capacity(text.len() + preedit.len());
    s.push_str(&text[..pos]);
    s.push_str(preedit);
    s.push_str(&text[pos..]);
    let cursor = preedit_cursor
        .filter(|c| *c <= preedit.len() && preedit.is_char_boundary(*c))
        .unwrap_or(preedit.len());
    (
        alloc::borrow::Cow::Owned(s),
        pos..pos + preedit.len(),
        pos + cursor,
    )
}
/// A field to paint: its text, caret and composition.
pub struct Field<'a> {
    pub text: &'a str,
    pub caret: usize,
    pub preedit: &'a Preedit,
}
/// Horizontal scroll that keeps the painted caret inside `width`, and the
/// prefix width function of the displayed text.
fn scroll(
    fonts: &door_graphics_text::FontAtlas,
    shown: &str,
    painted: usize,
    width: u32,
    style: door_graphics_text::TextStyle,
) -> i32 {
    let caret_x = if painted == 0 {
        0
    } else {
        crate::paint::measure(fonts, &shown[..painted], style) as i32 + 1
    };
    (caret_x + 2 - width as i32).max(0)
}
/// Logical x of the composition start (the field caret) as painted in
/// `area`: where the IME candidate window anchors.
pub fn caret_x(
    fonts: &door_graphics_text::FontAtlas,
    area: door_graphics_core::Rect,
    field: &Field<'_>,
    style: door_graphics_text::TextStyle,
) -> i32 {
    let (shown, range, painted) = display(
        field.text,
        field.caret,
        &field.preedit.text,
        field.preedit.cursor,
    );
    let offset = scroll(fonts, &shown, painted, area.width(), style);
    let start = if range.start == 0 {
        0
    } else {
        crate::paint::measure(fonts, &shown[..range.start], style) as i32 + 1
    };
    area.x() - offset + start
}
/// Paint a field's text in the logical row `area` (clipped, scrolled so the
/// caret stays visible), the composition's dotted underline (the TSF input
/// attribute) and, when `caret` is `(top, height)`, the 1 px caret.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut door_graphics_core::Canvas<'_>,
    fonts: &door_graphics_text::FontAtlas,
    area: door_graphics_core::Rect,
    field: &Field<'_>,
    style: door_graphics_text::TextStyle,
    color: [u8; 4],
    caret: Option<(i32, u32)>,
    lcd: bool,
) {
    use crate::paint;
    let (shown, range, painted) = display(
        field.text,
        field.caret,
        &field.preedit.text,
        field.preedit.cursor,
    );
    let prefix = |end: usize| {
        if end == 0 {
            0
        } else {
            paint::measure(fonts, &shown[..end], style) as i32 + 1
        }
    };
    let caret_x = prefix(painted);
    let w = area.width() as i32;
    let offset = scroll(fonts, &shown, painted, area.width(), style);
    let x = area.x() - offset;
    let Some(saved) = c.clip_rect() else { return };
    let Some(clip) = saved.intersection(paint::physical(area, c.scale())) else {
        return;
    };
    c.set_clip(clip);
    let max = (w + offset) as u32;
    if lcd {
        paint::lcd::text_row(
            c,
            fonts,
            x,
            area.y(),
            area.height(),
            &shown,
            style,
            color,
            max,
        );
    } else {
        paint::text_row(
            c,
            fonts,
            x,
            area.y(),
            area.height(),
            &shown,
            style,
            color,
            max,
        );
    }
    let (top, height) = caret.unwrap_or((area.y() + area.height() as i32 / 2 - 9, 18));
    if !range.is_empty() {
        let (x0, x1) = (x + prefix(range.start), x + prefix(range.end));
        let mut dx = x0;
        while dx < x1 {
            paint::rect(c, dx, top + height as i32, 1, 1, color);
            dx += 2;
        }
    }
    if caret.is_some() {
        paint::rect(c, x + caret_x, top, 1, height, color);
    }
    c.set_clip(saved);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backspace_and_delete_remove_whole_grapheme_clusters() {
        // e + combining acute, a ZWJ family, a flag, CJK.
        let mut text = String::from("e\u{301}👨\u{200d}👩\u{200d}👧🇮🇹日");
        let mut at = text.len();
        assert!(backspace(&mut text, &mut at));
        assert!(text.ends_with("🇮🇹"));
        assert!(backspace(&mut text, &mut at));
        assert!(text.ends_with('👧'));
        assert!(backspace(&mut text, &mut at));
        assert_eq!(text, "e\u{301}");
        assert!(backspace(&mut text, &mut at));
        assert_eq!((text.as_str(), at), ("", 0));
        assert!(!backspace(&mut text, &mut at));
        let mut text = String::from("a🇮🇹b");
        let mut at = 1;
        assert!(delete(&mut text, &mut at));
        assert_eq!((text.as_str(), at), ("ab", 1));
    }
    #[test]
    fn caret_moves_by_cluster_and_inserts_unicode_in_place() {
        let mut text = String::from("ab");
        let mut at = 1;
        assert!(insert(&mut text, &mut at, "拼音\n😀", |t| t.len() <= 64));
        assert_eq!((text.as_str(), at), ("a拼音😀b", "a拼音😀".len()));
        assert!(move_caret(&text, &mut at, 0x14b));
        assert_eq!(at, "a拼音".len());
        assert!(move_caret(&text, &mut at, 0x147));
        assert_eq!(at, 0);
        assert!(move_caret(&text, &mut at, 0x14d));
        assert_eq!(at, 1);
        assert!(move_caret(&text, &mut at, 0x14f));
        assert_eq!(at, text.len());
        assert!(!move_caret(&text, &mut at, 0x148));
        // The bound stops insertion at the last character that fits.
        let mut text = String::new();
        let mut at = 0;
        assert!(insert(&mut text, &mut at, "日日日", |t| t.len() <= 7));
        assert_eq!(text, "日日");
        // A caret the model invalidated is clamped onto a boundary.
        text.clear();
        assert_eq!(caret(&text, at), 0);
        let s = "e\u{301}x";
        assert_eq!(caret(s, 2), 0, "inside a cluster snaps to its start");
    }
    #[test]
    fn ime_deletes_and_composition_splice_at_the_caret() {
        let mut text = String::from("你好世界");
        let mut at = "你好".len();
        assert!(delete_around(&mut text, &mut at, 4, 1));
        assert_eq!((text.as_str(), at), ("界", 0));
        let (shown, range, painted) = display("ab", 1, "ni", Some(1));
        assert_eq!((&*shown, range, painted), ("anib", 1..3, 2));
        let (shown, range, painted) = display("ab", 2, "", None);
        assert_eq!((&*shown, range, painted), ("ab", 2..2, 2));
    }
}
