//! Text measurement facade. Layout and caret placement go through
//! `TextSystem`, so the Door Text shaping stack (stream T: Unicode, bidi,
//! fallback, fractional sizes) replaces `AtlasText` without touching
//! controls. `AtlasText` adapts today's hinted ASCII atlas: characters it
//! cannot draw are shown as '?' one-for-one, so byte offsets into the
//! original string still map to caret stops exactly.
use crate::theme::TextRole;
use crate::units::{LSize, Lu};
use alloc::{borrow::Cow, string::String, vec::Vec};
use core::ops::Range;
use door_graphics_text::FontAtlas;

pub trait TextSystem {
    /// Advance width of the widest line and total height of `text` (lines
    /// split at '\n'), at the current text scale.
    fn measure(&self, text: &str, role: TextRole) -> LSize;
    fn line_height(&self, role: TextRole) -> Lu;
    /// Baseline offset from the top of a line box.
    fn baseline(&self, role: TextRole) -> Lu;
    /// The stable ink band (capital top to descender bottom) inside a line
    /// box: (top, height). Labels centre on this, as Segoe UI does.
    fn ink(&self, role: TextRole) -> (Lu, Lu);
    /// Caret stops of one line: (byte offset, x) for every character
    /// boundary, including 0 and `text.len()`, in increasing order.
    fn caret_stops(&self, text: &str, role: TextRole) -> Vec<(usize, Lu)>;
    /// Accessibility > Text size in percent.
    fn text_scale(&self) -> u32;
    /// Apply a new text size (theme change).
    fn set_text_scale(&mut self, _percent: u32) {}
    /// Whether `c` renders as itself (not a replacement glyph).
    fn covers(&self, c: char) -> bool {
        c.is_ascii() && !c.is_control()
    }
}

/// The characters today's atlas draws: printable ASCII; every other
/// character becomes one '?' so offsets stay one-to-one per character.
pub fn display(text: &str) -> Cow<'_, str> {
    if text
        .bytes()
        .all(|b| (32..=126).contains(&b) || b == b'\n' || b == b'\t')
    {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| {
                if c.is_ascii() && (c == '\n' || c == '\t' || !c.is_control()) {
                    c
                } else {
                    '?'
                }
            })
            .collect(),
    )
}

/// `TextSystem` over the 100% hinted atlas (layout metrics are taken at 96
/// DPI and scaled, so layout is identical at every display scale). When a
/// Door Text engine is attached to the atlas (`FontAtlas::attach_unicode`),
/// non-ASCII text is shaped by it and carets come from its cluster maps;
/// otherwise unsupported characters show as one '?' each.
pub struct AtlasText {
    atlas: FontAtlas,
    scale: u32,
}
impl AtlasText {
    pub fn new(atlas: FontAtlas, text_scale: u32) -> Self {
        Self {
            atlas,
            scale: text_scale.clamp(100, 225),
        }
    }
    /// The Windows reference (Segoe UI) hinted atlas built into the host
    /// tools (guest platforms pass the asset-pack atlas to `new`).
    #[cfg(not(target_os = "none"))]
    pub fn windows(text_scale: u32) -> Option<Self> {
        FontAtlas::windows_reference_scaled(1)
            .ok()
            .map(|a| Self::new(a, text_scale))
    }
    pub fn atlas(&self) -> &FontAtlas {
        &self.atlas
    }
    fn scaled(&self, px: u32) -> Lu {
        Lu::px(px as i32).mul_div(self.scale as i32, 100)
    }
    fn shown<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.atlas.unicode().is_some() {
            Cow::Borrowed(text)
        } else {
            display(text)
        }
    }
}
impl TextSystem for AtlasText {
    fn measure(&self, text: &str, role: TextRole) -> LSize {
        let shown = self.shown(text);
        let style = role.style();
        let mut w = 0;
        let mut lines = 0;
        for line in shown.split('\n') {
            let mut end = line.len().min(door_graphics_text::MAX_TEXT_BYTES);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            w = w.max(
                self.atlas
                    .measure(&line[..end], style)
                    .map_or(0, |m| m.advance_width),
            );
            lines += 1;
        }
        LSize::new(
            self.scaled(w),
            self.scaled(style.line_height() * lines.max(1)),
        )
    }
    fn line_height(&self, role: TextRole) -> Lu {
        self.scaled(role.style().line_height())
    }
    fn baseline(&self, role: TextRole) -> Lu {
        self.scaled(self.atlas.metrics(role.style()).baseline)
    }
    fn ink(&self, role: TextRole) -> (Lu, Lu) {
        let ink = self.atlas.ink_metrics(role.style());
        (
            Lu::px(ink.top).mul_div(self.scale as i32, 100),
            self.scaled(ink.height),
        )
    }
    fn caret_stops(&self, text: &str, role: TextRole) -> Vec<(usize, Lu)> {
        let line = &text[..text.find('\n').unwrap_or(text.len())];
        let bounds = door_graphics_text::grapheme_boundaries(line);
        let mut stops = Vec::with_capacity(bounds.len());
        // Shaped carets from the attached engine (bidi, ligatures, marks).
        if let (Some(system), false) = (self.atlas.unicode(), line.is_ascii()) {
            let format = door_graphics_text::TextFormat::from_style(role.style(), 1.0);
            if let Ok(layout) = system.layout_line(line, &format) {
                for b in bounds {
                    let x = layout.caret(b, door_graphics_text::Affinity::Downstream).x;
                    stops.push((
                        b,
                        Lu((x * 64.0 + 0.5) as i32).mul_div(self.scale as i32, 100),
                    ));
                }
                return stops;
            }
        }
        let shown = display(line);
        let mut previous = Lu::ZERO;
        for b in bounds {
            // `display` maps each character to one byte.
            let k = line[..b].chars().count().min(shown.len());
            let prefix = &shown[..k.min(door_graphics_text::MAX_TEXT_BYTES)];
            let w = if prefix.is_empty() {
                Lu::ZERO
            } else {
                self.scaled(
                    self.atlas
                        .measure(prefix, role.style())
                        .map_or(0, |m| m.advance_width),
                )
            };
            // Kerning can pull a prefix back; stops never move left.
            previous = w.max(previous);
            stops.push((b, previous));
        }
        stops
    }
    fn text_scale(&self) -> u32 {
        self.scale
    }
    fn set_text_scale(&mut self, percent: u32) {
        self.scale = percent.clamp(100, 225);
    }
    fn covers(&self, c: char) -> bool {
        match self.atlas.unicode() {
            Some(s) => (c.is_ascii() && !c.is_control()) || s.fonts().covers(c),
            None => c.is_ascii() && !c.is_control(),
        }
    }
}

/// Byte offset of the caret stop nearest to `x` in a line.
pub fn hit_stop(stops: &[(usize, Lu)], x: Lu) -> usize {
    let mut best = (0usize, i32::MAX);
    for &(i, sx) in stops {
        let d = (sx - x).0.abs();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}
/// x of the stop at byte offset `at` (or of the last stop before it).
pub fn stop_x(stops: &[(usize, Lu)], at: usize) -> Lu {
    stops
        .iter()
        .take_while(|s| s.0 <= at)
        .last()
        .map_or(Lu::ZERO, |s| s.1)
}

/// Greedy word wrap of `text` into line ranges no wider than `width`
/// (a single long word is broken at character boundaries). Ranges index
/// the original text; explicit newlines always break.
pub fn wrap(text: &dyn TextSystem, s: &str, role: TextRole, width: Lu) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut offset = 0;
    for paragraph in s.split('\n') {
        let end = offset + paragraph.len();
        let mut start = offset;
        if paragraph.is_empty() {
            out.push(start..start);
        }
        while start < end {
            let mut fit = start;
            let mut space = None;
            for (i, ch) in s[start..end].char_indices() {
                let next = start + i + ch.len_utf8();
                if fit > start && text.measure(&s[start..next], role).w > width {
                    if ch == ' ' {
                        space = Some(fit);
                    }
                    break;
                }
                fit = next;
                if ch == ' ' && i > 0 {
                    space = Some(start + i);
                }
            }
            let stop = if fit < end { space.unwrap_or(fit) } else { fit };
            out.push(start..stop);
            start = stop;
            while start < end && s.as_bytes()[start] == b' ' {
                start += 1;
            }
        }
        offset = end + 1;
    }
    out
}
/// `text` shortened with a trailing "..." to fit `width`.
pub fn ellipsize(text: &dyn TextSystem, s: &str, role: TextRole, width: Lu) -> String {
    if text.measure(s, role).w <= width {
        return s.into();
    }
    let mut end = s.len();
    while end > 0 {
        end -= 1;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut candidate = String::from(s[..end].trim_end());
        candidate.push_str("...");
        if text.measure(&candidate, role).w <= width {
            return candidate;
        }
    }
    String::from("...")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atlas_measurement_scales_with_text_size_and_maps_unicode_one_to_one() {
        let t = AtlasText::windows(100).unwrap();
        let a = t.measure("Hello", TextRole::Body);
        assert_eq!(a.h, Lu::px(20));
        assert!(a.w > Lu::px(20) && a.w < Lu::px(60));
        let big = AtlasText::windows(200).unwrap();
        let b = big.measure("Hello", TextRole::Body);
        assert_eq!(b.h, Lu::px(40));
        assert_eq!(b.w, a.w * 2);
        assert_eq!(display("caf\u{e9} \u{1f600}"), "caf? ?");
        let stops = t.caret_stops("a\u{e9}b", TextRole::Body);
        assert_eq!(stops.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 1, 3, 4]);
        assert!(stops.windows(2).all(|w| w[0].1 <= w[1].1));
        assert_eq!(hit_stop(&stops, Lu::px(1000)), 4);
        assert_eq!(hit_stop(&stops, Lu::ZERO), 0);
        let lines = wrap(&t, "The quick brown fox jumps", TextRole::Body, Lu::px(80));
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|r| {
            t.measure(&"The quick brown fox jumps"[r.clone()], TextRole::Body)
                .w
                <= Lu::px(80)
        }));
        let e = ellipsize(&t, "A rather long label", TextRole::Body, Lu::px(60));
        assert!(e.ends_with("...") && t.measure(&e, TextRole::Body).w <= Lu::px(60));
    }
}
