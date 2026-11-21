//! TextBox (single and multi-line), PasswordBox and NumberBox over the
//! `TextDocument` model. Caret stops, hit testing and wrapping come from the
//! `TextSystem` (Door Text shaping when attached), so the control is
//! unchanged when the text stack changes.
use super::describe;
use super::document::{Affinity, TextDocument, floor};
use crate::display::Icon;
use crate::event::{Button as MouseButton, Composition, Event, Key, Phase};
use crate::layout::{Measured, Style};
use crate::text::{TextSystem, hit_stop, stop_x, wrap};
use crate::theme::TextRole;
use crate::units::{LPoint, LRect, LSize, Lu};
use crate::widget::{Cursor, EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{format, string::String, vec::Vec};
use core::ops::Range;
use door_accessibility::{Action, Actions, Node, RangeValue, Role, States};

/// NumberBox value domain in milli-units (value x 1000).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberSpec {
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub decimals: u8,
    pub value: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    /// Masked; no copy/cut, no value or text ranges published.
    Password,
    Number(NumberSpec),
}

/// Parse "[-+]digits[.digits]" to milli-units (rounded), without floats.
pub fn parse_milli(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let mut v: i64 = 0;
    for b in int.bytes() {
        v = v.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
    }
    v = v.checked_mul(1000)?;
    let mut scale = 100;
    let mut round = 0;
    for (i, b) in frac.bytes().enumerate() {
        let d = i64::from(b - b'0');
        if i < 3 {
            v = v.checked_add(d * scale)?;
            scale /= 10;
        } else if i == 3 && d >= 5 {
            round = 1;
        }
    }
    v = v.checked_add(round)?;
    Some(if neg { -v } else { v })
}
pub fn format_milli(v: i64, decimals: u8) -> String {
    let neg = v < 0;
    let a = v.unsigned_abs();
    let d = u64::from(decimals.min(3));
    let unit = 10u64.pow(3 - d as u32);
    let rounded = (a + unit / 2) / unit;
    let div = 10u64.pow(d as u32);
    let (whole, frac) = (rounded / div, rounded % div);
    let sign = if neg && rounded != 0 { "-" } else { "" };
    if d == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac:0width$}", width = d as usize)
    }
}

pub struct TextBox {
    pub header: String,
    pub placeholder: String,
    pub doc: TextDocument,
    pub mode: Mode,
    pub read_only: bool,
    pub invalid: bool,
    /// Visible rows of a multi-line box.
    pub rows: u16,
    scroll: LPoint,
    goal_x: Option<Lu>,
    selecting: bool,
    last_down: Option<(u64, LPoint)>,
    button_pressed: Option<u8>,
}
const CLEAR: u8 = 1;
const SPIN_UP: u8 = 2;
const SPIN_DOWN: u8 = 3;
impl TextBox {
    pub fn new(header: &str) -> Self {
        Self {
            header: header.into(),
            placeholder: String::new(),
            doc: TextDocument::new("", false, 32 * 1024),
            mode: Mode::Text,
            read_only: false,
            invalid: false,
            rows: 1,
            scroll: LPoint::default(),
            goal_x: None,
            selecting: false,
            last_down: None,
            button_pressed: None,
        }
    }
    /// Multi-line box (documents up to 1 MiB) showing `rows` lines.
    pub fn multiline(header: &str, rows: u16) -> Self {
        Self {
            doc: TextDocument::new("", true, 1 << 20),
            rows: rows.max(2),
            ..Self::new(header)
        }
    }
    pub fn password(header: &str) -> Self {
        Self {
            mode: Mode::Password,
            doc: TextDocument::new("", false, 256),
            ..Self::new(header)
        }
    }
    pub fn number(header: &str, min: i64, max: i64, value: i64) -> Self {
        let spec = NumberSpec {
            min: min * 1000,
            max: max.max(min) * 1000,
            step: 1000,
            decimals: 0,
            value: value.clamp(min, max.max(min)) * 1000,
        };
        let mut t = Self {
            mode: Mode::Number(spec),
            doc: TextDocument::new("", false, 64),
            ..Self::new(header)
        };
        t.doc.set_text(&format_milli(spec.value, 0));
        t
    }
    pub fn text(mut self, text: &str) -> Self {
        self.doc.set_text(text);
        self
    }
    pub fn placeholder(mut self, p: &str) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
    pub fn value(&self) -> &str {
        self.doc.text()
    }
    pub fn number_value(&self) -> Option<i64> {
        match self.mode {
            Mode::Number(s) => Some(s.value),
            _ => None,
        }
    }
    fn multiline_mode(&self) -> bool {
        self.doc.multiline()
    }
    fn header_h(&self, t: &dyn TextSystem) -> Lu {
        if self.header.is_empty() {
            Lu::ZERO
        } else {
            t.line_height(TextRole::Body) + Lu::px(8)
        }
    }
    fn field(&self, r: LRect, t: &dyn TextSystem) -> LRect {
        let h = self.header_h(t);
        LRect::new(r.x, r.y + h, r.w, r.h - h)
    }
    fn buttons(&self, focused: bool) -> Lu {
        match self.mode {
            Mode::Number(_) => Lu::px(64),
            _ if focused
                && !self.doc.text().is_empty()
                && !self.read_only
                && !self.multiline_mode() =>
            {
                Lu::px(30)
            }
            _ => Lu::ZERO,
        }
    }
    fn area(&self, r: LRect, t: &dyn TextSystem, focused: bool) -> LRect {
        let f = self.field(r, t);
        LRect::new(
            f.x + Lu::px(10),
            f.y + Lu::px(5),
            f.w - Lu::px(16) - self.buttons(focused),
            f.h - Lu::px(11),
        )
    }
    fn button_rect(&self, r: LRect, t: &dyn TextSystem, which: u8) -> LRect {
        let f = self.field(r, t);
        let h = Lu::px(24);
        let y = f.y + (f.h.min(Lu::px(32)) - h) / 2;
        match which {
            CLEAR => LRect::new(f.right() - Lu::px(34), y, Lu::px(30), h),
            SPIN_UP => LRect::new(f.right() - Lu::px(64), y, Lu::px(30), h),
            _ => LRect::new(f.right() - Lu::px(34), y, Lu::px(30), h),
        }
    }
    /// Displayed text and the original offset of each shown char boundary.
    fn shown(&self, t: &dyn TextSystem) -> (String, Vec<(usize, usize)>) {
        let text = self.doc.text();
        if self.mode != Mode::Password {
            return (text.into(), Vec::new());
        }
        let bullet = if t.covers('\u{25cf}') {
            '\u{25cf}'
        } else {
            '*'
        };
        let mut s = String::new();
        let mut map = Vec::new();
        for (i, _) in text.char_indices() {
            map.push((s.len(), i));
            s.push(bullet);
        }
        map.push((s.len(), text.len()));
        (s, map)
    }
    fn to_shown(map: &[(usize, usize)], original: usize) -> usize {
        if map.is_empty() {
            return original;
        }
        map.iter()
            .take_while(|m| m.1 <= original)
            .last()
            .map_or(0, |m| m.0)
    }
    fn to_original(map: &[(usize, usize)], shown: usize) -> usize {
        if map.is_empty() {
            return shown;
        }
        map.iter()
            .take_while(|m| m.0 <= shown)
            .last()
            .map_or(0, |m| m.1)
    }
    fn lines(&self, t: &dyn TextSystem, shown: &str, width: Lu) -> Vec<Range<usize>> {
        if self.multiline_mode() {
            wrap(t, shown, TextRole::Body, width)
        } else {
            alloc::vec![0..shown.len()]
        }
    }
    /// Visual line index and x of a shown offset.
    fn locate(
        &self,
        t: &dyn TextSystem,
        shown: &str,
        lines: &[Range<usize>],
        at: usize,
    ) -> (usize, Lu) {
        let mut line = lines.len().saturating_sub(1);
        for (i, l) in lines.iter().enumerate() {
            let next_starts_here = lines.get(i + 1).is_some_and(|n| n.start == at);
            if at >= l.start
                && (at < l.end
                    || at == l.end
                        && !(next_starts_here && self.doc.affinity == Affinity::Downstream))
            {
                line = i;
                break;
            }
        }
        let l = lines.get(line).cloned().unwrap_or(0..0);
        let stops = t.caret_stops(&shown[l.clone()], TextRole::Body);
        (line, stop_x(&stops, at.saturating_sub(l.start)))
    }
    fn offset_at(&self, t: &dyn TextSystem, area: LRect, p: LPoint) -> usize {
        let (shown, map) = self.shown(t);
        let lines = self.lines(t, &shown, area.w);
        let lh = t.line_height(TextRole::Body);
        let y = p.y - area.y + self.scroll.y;
        let i = if lines.is_empty() {
            0
        } else {
            ((y.0.max(0) / lh.0.max(1)) as usize).min(lines.len() - 1)
        };
        let l = lines.get(i).cloned().unwrap_or(0..0);
        let stops = t.caret_stops(&shown[l.clone()], TextRole::Body);
        let at = l.start + hit_stop(&stops, p.x - area.x + self.scroll.x);
        Self::to_original(&map, at)
    }
    fn reveal(&mut self, cx: &EventCx<'_>) {
        let area = self.area(cx.rect, cx.text, true);
        let (shown, map) = self.shown(cx.text);
        let lines = self.lines(cx.text, &shown, area.w);
        let (line, x) = self.locate(
            cx.text,
            &shown,
            &lines,
            Self::to_shown(&map, self.doc.caret()),
        );
        let lh = cx.text.line_height(TextRole::Body);
        if self.multiline_mode() {
            let top = lh * line as i32;
            if top < self.scroll.y {
                self.scroll.y = top;
            } else if top + lh > self.scroll.y + area.h {
                self.scroll.y = top + lh - area.h;
            }
            self.scroll.x = Lu::ZERO;
        } else {
            if x < self.scroll.x {
                self.scroll.x = x;
            } else if x > self.scroll.x + area.w - Lu::ONE {
                self.scroll.x = x - area.w + Lu::ONE;
            }
            let total = cx.text.measure(&shown, TextRole::Body).w;
            self.scroll.x = self
                .scroll
                .x
                .clamp(Lu::ZERO, (total - area.w + Lu::ONE).max(Lu::ZERO));
        }
    }
    fn changed(&mut self, cx: &mut EventCx<'_>) {
        self.goal_x = None;
        self.reveal(cx);
        cx.invalidate();
        cx.notify(Notify::TextChanged);
    }
    fn commit_number(&mut self, cx: &mut EventCx<'_>) {
        if let Mode::Number(mut spec) = self.mode {
            match parse_milli(self.doc.text()) {
                Some(v) => {
                    let v = v.clamp(spec.min, spec.max);
                    self.invalid = false;
                    if v != spec.value {
                        spec.value = v;
                        cx.notify(Notify::ValueChanged(v));
                    }
                }
                // Invalid input is overwritten with the last valid value.
                None => self.invalid = false,
            }
            self.mode = Mode::Number(spec);
            self.doc.set_text(&format_milli(spec.value, spec.decimals));
            cx.invalidate();
        }
    }
    fn spin(&mut self, cx: &mut EventCx<'_>, up: bool) {
        if let Mode::Number(mut spec) = self.mode {
            let base = parse_milli(self.doc.text()).unwrap_or(spec.value);
            let v = if up {
                base.saturating_add(spec.step)
            } else {
                base.saturating_sub(spec.step)
            }
            .clamp(spec.min, spec.max);
            if v != spec.value {
                spec.value = v;
                cx.notify(Notify::ValueChanged(v));
            }
            self.mode = Mode::Number(spec);
            self.doc.set_text(&format_milli(v, spec.decimals));
            self.doc.select_all();
            cx.invalidate();
        }
    }
    fn vertical(&mut self, cx: &mut EventCx<'_>, delta: i32, extend: bool) {
        let area = self.area(cx.rect, cx.text, true);
        let (shown, map) = self.shown(cx.text);
        let lines = self.lines(cx.text, &shown, area.w);
        let (line, x) = self.locate(
            cx.text,
            &shown,
            &lines,
            Self::to_shown(&map, self.doc.caret()),
        );
        let goal = *self.goal_x.get_or_insert(x);
        let target = (line as i32 + delta).clamp(0, lines.len() as i32 - 1) as usize;
        let at = if (line as i32 + delta) < 0 {
            0
        } else if line as i32 + delta >= lines.len() as i32 {
            shown.len()
        } else {
            let l = lines[target].clone();
            let stops = cx.text.caret_stops(&shown[l.clone()], TextRole::Body);
            l.start + hit_stop(&stops, goal)
        };
        self.doc.place(Self::to_original(&map, at), extend);
        self.goal_x = Some(goal);
        self.reveal(cx);
        cx.invalidate();
    }
    fn key(&mut self, cx: &mut EventCx<'_>, k: &crate::event::KeyEvent) -> bool {
        let m = k.modifiers;
        let editable = !self.read_only;
        let password = self.mode == Mode::Password;
        match k.key {
            Key::Left | Key::Right => {
                self.doc
                    .horizontal(k.key == Key::Right, m.shift, m.ctrl && !password);
                self.goal_x = None;
                self.reveal(cx);
                cx.invalidate();
            }
            Key::Home | Key::End => {
                let home = k.key == Key::Home;
                let at = if m.ctrl || !self.multiline_mode() {
                    if home { 0 } else { self.doc.text().len() }
                } else if home {
                    self.doc.line_start(self.doc.caret())
                } else {
                    self.doc.line_end(self.doc.caret())
                };
                self.doc.place(at, m.shift);
                self.goal_x = None;
                self.reveal(cx);
                cx.invalidate();
            }
            Key::Up | Key::Down if self.multiline_mode() => {
                self.vertical(cx, if k.key == Key::Up { -1 } else { 1 }, m.shift)
            }
            Key::Up | Key::Down if matches!(self.mode, Mode::Number(_)) && editable => {
                self.spin(cx, k.key == Key::Up)
            }
            Key::PageUp | Key::PageDown if self.multiline_mode() => {
                let rows = i32::from(self.rows).max(1);
                self.vertical(cx, if k.key == Key::PageUp { -rows } else { rows }, m.shift);
            }
            Key::Backspace if editable => {
                if self.doc.backspace(m.ctrl && !password) {
                    self.changed(cx);
                }
            }
            Key::Delete if editable => {
                if self.doc.delete(m.ctrl && !password) {
                    self.changed(cx);
                }
            }
            Key::Enter if self.multiline_mode() && editable && !m.ctrl && !m.alt => {
                if self.doc.insert("\n") {
                    self.changed(cx);
                }
            }
            Key::Enter if m.none() => {
                self.commit_number(cx);
                cx.notify(Notify::Submitted);
            }
            Key::Escape if self.doc.preedit().is_some() => {
                self.doc.cancel_preedit();
                cx.invalidate();
            }
            Key::Char('a') if m.ctrl => {
                self.doc.select_all();
                cx.invalidate();
            }
            Key::Char('c') | Key::Char('x') if m.ctrl => {
                // PasswordBox never copies (master specification 113).
                if password || self.doc.selection().is_empty() {
                    return true;
                }
                let s = String::from(self.doc.selected());
                cx.clipboard().set_text(&s);
                if k.key == Key::Char('x') && editable && self.doc.backspace(false) {
                    self.changed(cx);
                }
            }
            Key::Char('v') if m.ctrl && editable => {
                if let Some(s) = cx.clipboard().text()
                    && self.doc.paste(&s)
                {
                    self.changed(cx);
                }
            }
            Key::Char('z') if m.ctrl && editable => {
                let done = if m.shift {
                    self.doc.redo()
                } else {
                    self.doc.undo()
                };
                if done {
                    self.changed(cx);
                }
            }
            Key::Char('y') if m.ctrl && editable => {
                if self.doc.redo() {
                    self.changed(cx);
                }
            }
            _ => {
                let Some(c) = k.text.filter(|_| !m.ctrl && !m.alt) else {
                    return false;
                };
                if !editable {
                    return true;
                }
                let mut b = [0; 4];
                if self.doc.insert(c.encode_utf8(&mut b)) {
                    self.changed(cx);
                }
            }
        }
        true
    }
}
impl Widget for TextBox {
    fn type_name(&self) -> &'static str {
        match self.mode {
            Mode::Text => "TextBox",
            Mode::Password => "PasswordBox",
            Mode::Number(_) => "NumberBox",
        }
    }
    fn style(&self) -> Style {
        Style::default().min_size(64, 0)
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let lh = cx.text.line_height(TextRole::Body);
        let h = if self.multiline_mode() {
            lh * i32::from(self.rows) + Lu::px(11)
        } else {
            Lu::px(32).max(lh + Lu::px(12))
        };
        let w = if a.w.is_infinite() { Lu::px(240) } else { a.w };
        let hh = self.header_h(cx.text);
        let (top, ink) = cx.text.ink(TextRole::Body);
        let base = hh
            + Lu::px(5)
            + if self.multiline_mode() {
                cx.text.baseline(TextRole::Body)
            } else {
                (h - Lu::px(11) - ink) / 2 - top + cx.text.baseline(TextRole::Body)
            };
        Measured::with_baseline(LSize::new(w, hh + h), base)
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        let focused = cx.state.focused && cx.state.window_active;
        if !self.header.is_empty() {
            cx.text_at(
                &self.header,
                r.origin(),
                TextRole::Body,
                if cx.state.enabled {
                    c.text
                } else {
                    c.text_disabled
                },
            );
        }
        let f = self.field(r, cx.text);
        let fill = if !cx.state.enabled {
            c.control_disabled
        } else if focused {
            c.field_focused
        } else if cx.state.hovered {
            c.control_hover
        } else {
            c.control
        };
        let radius = t.metrics.control_radius;
        // High contrast draws a disabled field in GrayText.
        let gray = !cx.state.enabled && t.is_high_contrast();
        cx.round(
            f,
            radius,
            if gray {
                c.strong_disabled
            } else if focused {
                c.accent
            } else if self.invalid {
                c.critical
            } else {
                c.strong
            },
        );
        let inner_bottom = if focused { Lu::px(2) } else { Lu::ONE };
        cx.round(
            LRect::new(f.x, f.y, f.w, f.h - inner_bottom),
            radius,
            if gray {
                c.strong_disabled
            } else {
                c.control_stroke
            },
        );
        cx.round(
            LRect::new(
                f.x + Lu::ONE,
                f.y + Lu::ONE,
                f.w - Lu::px(2),
                f.h - inner_bottom - Lu::ONE,
            ),
            radius - 1,
            fill,
        );
        let area = self.area(r, cx.text, focused);
        let (shown, map) = self.shown(cx.text);
        let lh = cx.text.line_height(TextRole::Body);
        let fg = if cx.state.enabled {
            c.text
        } else {
            c.text_disabled
        };
        cx.push_clip(area);
        let origin = LPoint::new(area.x - self.scroll.x, area.y - self.scroll.y);
        let single_top = |cx: &PaintCx<'_>| {
            let (top, ink) = cx.text.ink(TextRole::Body);
            area.y + (area.h - ink) / 2 - top
        };
        if shown.is_empty() && self.doc.preedit().is_none() {
            if !self.placeholder.is_empty() {
                let y = if self.multiline_mode() {
                    area.y
                } else {
                    single_top(cx)
                };
                cx.text_at(
                    &self.placeholder,
                    LPoint::new(area.x, y),
                    TextRole::Body,
                    c.text_secondary,
                );
            }
        } else {
            let lines = self.lines(cx.text, &shown, area.w);
            let sel = self.doc.selection();
            let (s0, s1) = (
                Self::to_shown(&map, sel.start),
                Self::to_shown(&map, sel.end),
            );
            for (i, l) in lines.iter().enumerate() {
                let y = if self.multiline_mode() {
                    origin.y + lh * i as i32
                } else {
                    single_top(cx) - self.scroll.y
                };
                if y + lh < area.y || y > area.bottom() {
                    continue;
                }
                let line = &shown[l.clone()];
                let stops = cx.text.caret_stops(line, TextRole::Body);
                let (a, b) = (s0.max(l.start), s1.min(l.end));
                let sel_rect = (focused && a < b).then(|| {
                    let x0 = stop_x(&stops, a - l.start);
                    let x1 = stop_x(&stops, b - l.start);
                    LRect::new(origin.x + x0, y, x1 - x0, lh)
                });
                if let Some(sr) = sel_rect {
                    cx.fill(sr, c.accent);
                }
                cx.text_at(line, LPoint::new(origin.x, y), TextRole::Body, fg);
                if let Some(sr) = sel_rect {
                    cx.push_clip(sr);
                    cx.text_at(
                        line,
                        LPoint::new(origin.x, y),
                        TextRole::Body,
                        c.text_on_accent,
                    );
                    cx.pop_clip();
                }
            }
            if focused {
                let caret = Self::to_shown(&map, self.doc.caret());
                let (line, x) = self.locate(cx.text, &shown, &lines, caret);
                let y = if self.multiline_mode() {
                    origin.y + lh * line as i32
                } else {
                    single_top(cx) - self.scroll.y
                };
                if let Some((pre, _)) = self.doc.preedit() {
                    // Preedit: drawn at the caret, underlined, not committed.
                    let w = cx.text.measure(pre, TextRole::Body).w;
                    cx.fill(LRect::new(origin.x + x, y, w, lh), c.field_focused);
                    cx.text_at(pre, LPoint::new(origin.x + x, y), TextRole::Body, fg);
                    cx.fill(LRect::new(origin.x + x, y + lh - Lu::px(2), w, Lu::ONE), fg);
                } else if self.doc.selection().is_empty() {
                    let (top, ink) = cx.text.ink(TextRole::Body);
                    cx.fill(
                        LRect::new(origin.x + x, y + top - Lu::ONE, Lu::ONE, ink + Lu::px(2)),
                        c.text,
                    );
                }
            }
        }
        cx.pop_clip();
        let mut glyphs = Vec::new();
        if matches!(self.mode, Mode::Number(_)) {
            glyphs.push((SPIN_UP, Icon::ChevronUp));
            glyphs.push((SPIN_DOWN, Icon::ChevronDown));
        } else if self.buttons(focused) > Lu::ZERO {
            glyphs.push((CLEAR, Icon::Close));
        }
        for (which, icon) in glyphs {
            let b = self.button_rect(r, cx.text, which);
            if self.button_pressed == Some(which) && cx.state.captured {
                cx.round(b, 4, c.subtle_pressed);
            }
            cx.icon_centered(
                icon,
                b,
                12,
                if cx.state.enabled {
                    c.text_secondary
                } else {
                    c.text_disabled
                },
            );
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        let focused = cx.state.focused;
        match e {
            Event::PointerDown(p) if p.button == MouseButton::Primary => {
                cx.set_handled();
                let mut buttons = Vec::new();
                if matches!(self.mode, Mode::Number(_)) {
                    buttons.extend([SPIN_UP, SPIN_DOWN]);
                } else if self.buttons(true) > Lu::ZERO {
                    buttons.push(CLEAR);
                }
                for b in buttons {
                    if self.button_rect(cx.rect, cx.text, b).contains(p.pos) {
                        self.button_pressed = Some(b);
                        cx.capture_pointer();
                        cx.invalidate();
                        return;
                    }
                }
                let area = self.area(cx.rect, cx.text, true);
                let at = self.offset_at(cx.text, area, p.pos);
                let double = self.last_down.is_some_and(|(t, q)| {
                    cx.now.saturating_sub(t) <= 500
                        && (q.x - p.pos.x).0.abs() <= 256
                        && (q.y - p.pos.y).0.abs() <= 256
                });
                self.last_down = Some((cx.now, p.pos));
                if double && self.mode != Mode::Password {
                    let text = self.doc.text();
                    let start = door_graphics_text::prev_word(
                        text,
                        door_graphics_text::next_grapheme(text, at).min(text.len()),
                    );
                    let end = door_graphics_text::next_word(text, start);
                    self.doc.set_selection(start, end);
                    self.last_down = None;
                } else {
                    self.doc.place(at, p.modifiers.shift);
                    self.selecting = true;
                    cx.capture_pointer();
                }
                self.goal_x = None;
                cx.invalidate();
            }
            Event::PointerMove(p) if self.selecting && cx.state.captured => {
                let area = self.area(cx.rect, cx.text, true);
                let at = self.offset_at(cx.text, area, p.pos);
                if at != self.doc.caret() {
                    self.doc.place(at, true);
                    self.reveal(cx);
                    cx.invalidate();
                }
            }
            Event::PointerUp(p) => {
                self.selecting = false;
                if let Some(b) = self.button_pressed.take() {
                    if cx.state.captured && self.button_rect(cx.rect, cx.text, b).contains(p.pos) {
                        match b {
                            CLEAR => {
                                self.doc.select_all();
                                if self.doc.backspace(false) {
                                    self.changed(cx);
                                }
                            }
                            SPIN_UP => self.spin(cx, true),
                            _ => self.spin(cx, false),
                        }
                    }
                    cx.invalidate();
                }
            }
            Event::CaptureLost => {
                self.selecting = false;
                self.button_pressed = None;
                cx.invalidate();
            }
            Event::PointerEnter | Event::PointerLeave | Event::FocusIn => cx.invalidate(),
            Event::FocusOut => {
                self.doc.cancel_preedit();
                self.commit_number(cx);
                cx.invalidate();
            }
            Event::KeyDown(k) if focused => {
                if self.key(cx, k) {
                    cx.set_handled();
                }
            }
            Event::Wheel { delta, .. } if self.multiline_mode() => {
                let lh = cx.text.line_height(TextRole::Body);
                let area = self.area(cx.rect, cx.text, true);
                let (shown, _) = self.shown(cx.text);
                let total = lh * self.lines(cx.text, &shown, area.w).len() as i32;
                let max = (total - area.h).max(Lu::ZERO);
                let next = (self.scroll.y - lh * 3 * *delta).clamp(Lu::ZERO, max);
                if next != self.scroll.y {
                    self.scroll.y = next;
                    cx.invalidate();
                    cx.set_handled();
                }
            }
            Event::Composition(c) if !self.read_only => {
                match c {
                    Composition::Update { text, caret } => self.doc.set_preedit(text, *caret),
                    Composition::Commit(text) => {
                        if self.doc.commit_preedit(text) {
                            self.changed(cx);
                        }
                    }
                    Composition::Cancel => self.doc.cancel_preedit(),
                }
                cx.invalidate();
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn cursor(&self, _p: LPoint) -> Cursor {
        Cursor::Text
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        let role = if matches!(self.mode, Mode::Number(_)) {
            Role::Spinner
        } else {
            Role::Edit
        };
        describe(node, role, &self.header);
        node.description = self.placeholder.clone();
        if self.read_only {
            node.states |= States::READ_ONLY;
        } else {
            node.states |= States::EDITABLE;
            node.actions |= Actions::SET_VALUE;
        }
        if self.multiline_mode() {
            node.states |= States::MULTILINE;
        }
        if self.invalid {
            node.states |= States::INVALID;
        }
        match self.mode {
            Mode::Password => node.states |= States::PROTECTED,
            Mode::Text | Mode::Number(_) => {
                node.value = self.doc.text().into();
                let sel = (self.doc.anchor() as u32, self.doc.caret() as u32);
                if self.doc.text().len() <= door_accessibility::wire::MAX_VALUE_BYTES {
                    node.selection = Some(sel);
                    node.states |= States::HAS_SELECTION;
                    node.actions |= Actions::SET_SELECTION;
                }
            }
        }
        if let Mode::Number(s) = self.mode {
            node.range = Some(RangeValue {
                min: s.min,
                max: s.max,
                now: s.value,
                step: s.step,
            });
            node.states |= States::HAS_RANGE;
            if !self.read_only {
                node.actions |= Actions::INCREMENT | Actions::DECREMENT | Actions::SET_RANGE;
            }
        }
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        if self.read_only {
            return false;
        }
        match a {
            Action::SetValue(v) => {
                self.doc.select_all();
                self.doc.paste(v);
                if let Mode::Number(_) = self.mode {
                    self.commit_number(cx);
                }
                self.changed(cx);
                true
            }
            Action::SetSelection(an, ca) if self.mode != Mode::Password => {
                let ok = self.doc.set_selection(
                    floor(self.doc.text(), *an as usize),
                    floor(self.doc.text(), *ca as usize),
                );
                cx.invalidate();
                ok
            }
            Action::Increment | Action::Decrement if matches!(self.mode, Mode::Number(_)) => {
                self.spin(cx, *a == Action::Increment);
                true
            }
            Action::SetRange(v) => {
                if let Mode::Number(spec) = self.mode {
                    self.doc.set_text(&format_milli(*v, spec.decimals));
                    self.commit_number(cx);
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_parse_and_format_exactly() {
        assert_eq!(parse_milli("12"), Some(12_000));
        assert_eq!(parse_milli("-1.5"), Some(-1_500));
        assert_eq!(parse_milli("+0.0005"), Some(1));
        assert_eq!(parse_milli(".25"), Some(250));
        assert_eq!(parse_milli("1e3"), None);
        assert_eq!(parse_milli(""), None);
        assert_eq!(parse_milli("99999999999999999999"), None);
        assert_eq!(format_milli(1_500, 1), "1.5");
        assert_eq!(format_milli(-1_499, 0), "-1");
        assert_eq!(format_milli(-400, 0), "0");
        assert_eq!(format_milli(2_000, 2), "2.00");
    }
}
