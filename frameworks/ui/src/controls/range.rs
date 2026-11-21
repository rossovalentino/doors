//! Slider, ProgressBar and ProgressRing.
use super::basic::{control_height, range_node};
use super::{Visual, describe};
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Measured, Style};
use crate::theme::TextRole;
use crate::units::{LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{format, string::String};
use door_accessibility::{Action, Actions, Node, Role, States};

/// Horizontal Slider over integers `min..=max` in `step`s.
pub struct Slider {
    pub header: String,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub large_change: i64,
    pub value: i64,
    dragging: bool,
}
impl Slider {
    pub fn new(header: &str, min: i64, max: i64, value: i64) -> Self {
        let max = max.max(min);
        Self {
            header: header.into(),
            min,
            max,
            step: 1,
            large_change: ((max - min) / 10).max(1),
            value: value.clamp(min, max),
            dragging: false,
        }
    }
    pub fn step(mut self, step: i64) -> Self {
        self.step = step.max(1);
        self
    }
    fn header_h(&self, t: &dyn crate::text::TextSystem) -> Lu {
        if self.header.is_empty() {
            Lu::ZERO
        } else {
            t.line_height(TextRole::Body) + Lu::px(4)
        }
    }
    fn track(&self, r: LRect, hh: Lu) -> LRect {
        LRect::new(
            r.x + Lu::px(10),
            r.y + hh + (r.h - hh - Lu::px(4)) / 2,
            r.w - Lu::px(20),
            Lu::px(4),
        )
    }
    fn snap(&self, v: i64) -> i64 {
        let s = self.step.max(1);
        let v = v.clamp(self.min, self.max);
        (self.min + (v - self.min + s / 2) / s * s).clamp(self.min, self.max)
    }
    /// Set the value (snapped and clamped); returns whether it changed.
    pub fn set(&mut self, v: i64) -> bool {
        let v = self.snap(v);
        let changed = v != self.value;
        self.value = v;
        changed
    }
    fn change(&mut self, cx: &mut EventCx<'_>, v: i64) {
        if self.set(v) {
            cx.notify(Notify::ValueChanged(self.value * 1000));
        }
        cx.invalidate();
    }
    fn value_at(&self, track: LRect, x: Lu) -> i64 {
        let span = (self.max - self.min).max(1);
        let w = track.w.0.max(1) as i64;
        let rel = i64::from((x - track.x).0).clamp(0, w);
        self.min + (rel * span + w / 2) / w
    }
}
impl Widget for Slider {
    fn type_name(&self) -> &'static str {
        "Slider"
    }
    fn style(&self) -> Style {
        Style::default().min_size(120, 0)
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let w = if a.w.is_infinite() {
            Lu::px(200)
        } else {
            a.w.min(Lu::px(320))
        };
        Measured::new(LSize::new(
            w.max(cx.text.measure(&self.header, TextRole::Body).w),
            self.header_h(cx.text) + control_height(cx),
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let r = cx.rect;
        let hh = self.header_h(cx.text);
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
        let track = self.track(r, hh);
        let v = Visual::of(&cx.state, cx.state.captured);
        let span = (self.max - self.min).max(1);
        let pos = track.x + Lu(((i64::from(track.w.0) * (self.value - self.min)) / span) as i32);
        cx.round(
            track,
            2,
            if v == Visual::Disabled {
                c.strong_disabled
            } else {
                c.strong
            },
        );
        let filled = LRect::new(track.x, track.y, pos - track.x, track.h);
        cx.round(
            filled,
            2,
            if v == Visual::Disabled {
                c.accent_disabled
            } else {
                c.accent
            },
        );
        let thumb = LRect::new(
            pos - Lu::px(10),
            track.y + Lu::px(2) - Lu::px(10),
            Lu::px(20),
            Lu::px(20),
        );
        cx.ellipse(thumb, c.control_stroke_bottom);
        cx.ellipse(
            thumb.inflate(-Lu::ONE),
            if cx.theme.is_dark() {
                c.control
            } else {
                [255, 255, 255, 255]
            },
        );
        let inner = Lu::px(match v {
            Visual::Hover => 14,
            Visual::Pressed => 10,
            _ => 12,
        });
        let fill = match v {
            Visual::Disabled => c.accent_disabled,
            Visual::Hover => c.accent_hover,
            Visual::Pressed => c.accent_pressed,
            Visual::Rest => c.accent,
        };
        cx.ellipse(
            LRect::new(
                thumb.x + (thumb.w - inner) / 2,
                thumb.y + (thumb.h - inner) / 2,
                inner,
                inner,
            ),
            fill,
        );
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let hh = self.header_h(cx.text);
        let r = cx.rect;
        let row = LRect::new(r.x, r.y + hh, r.w, r.h - hh);
        cx.focus_if_visible(row, cx.theme.metrics.control_radius);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        let track = self.track(cx.rect, self.header_h(cx.text));
        match e {
            Event::PointerDown(p) if p.button == MouseButton::Primary => {
                self.dragging = true;
                cx.capture_pointer();
                let v = self.value_at(track, p.pos.x);
                self.change(cx, v);
                cx.set_handled();
            }
            Event::PointerMove(p) if self.dragging && cx.state.captured => {
                let v = self.value_at(track, p.pos.x);
                self.change(cx, v);
            }
            Event::PointerUp(_) | Event::CaptureLost => {
                self.dragging = false;
                cx.invalidate();
            }
            Event::PointerEnter | Event::PointerLeave => cx.invalidate(),
            Event::KeyDown(k) if k.modifiers.none() => {
                let v = match k.key {
                    Key::Left | Key::Down => Some(self.value - self.step),
                    Key::Right | Key::Up => Some(self.value + self.step),
                    Key::PageDown => Some(self.value - self.large_change),
                    Key::PageUp => Some(self.value + self.large_change),
                    Key::Home => Some(self.min),
                    Key::End => Some(self.max),
                    _ => None,
                };
                if let Some(v) = v {
                    self.change(cx, v);
                    cx.set_handled();
                }
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Slider, &self.header);
        range_node(node, self.min, self.max, self.value, self.step);
        node.value = format!("{}", self.value);
        node.states |= States::HORIZONTAL;
        node.actions |= Actions::INCREMENT | Actions::DECREMENT | Actions::SET_RANGE;
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        match a {
            Action::Increment => self.change(cx, self.value + self.step),
            Action::Decrement => self.change(cx, self.value - self.step),
            Action::SetRange(m) => self.change(cx, m.div_euclid(1000)),
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Progress {
    #[default]
    Running,
    Paused,
    Error,
}
/// Determinate or indeterminate ProgressBar (0..=100).
pub struct ProgressBar {
    pub name: String,
    pub value: u32,
    pub indeterminate: bool,
    pub status: Progress,
}
impl ProgressBar {
    pub fn new(name: &str, value: u32) -> Self {
        Self {
            name: name.into(),
            value: value.min(100),
            indeterminate: false,
            status: Progress::Running,
        }
    }
    pub fn indeterminate(mut self) -> Self {
        self.indeterminate = true;
        self
    }
    pub fn status(mut self, s: Progress) -> Self {
        self.status = s;
        self
    }
}
fn progress_color(t: &crate::theme::Theme, s: Progress, enabled: bool) -> crate::theme::Color {
    match (s, enabled) {
        (_, false) => t.colors.accent_disabled,
        (Progress::Running, _) => t.colors.accent,
        (Progress::Paused, _) => t.colors.caution,
        (Progress::Error, _) => t.colors.critical,
    }
}
impl Widget for ProgressBar {
    fn type_name(&self) -> &'static str {
        "ProgressBar"
    }
    fn animating(&self) -> bool {
        self.is_animating()
    }
    fn style(&self) -> Style {
        Style::default().min_size(60, 0)
    }
    fn measure(&self, _cx: &LayoutCx<'_>, a: LSize) -> Measured {
        Measured::new(LSize::new(
            if a.w.is_infinite() { Lu::px(200) } else { a.w },
            Lu::px(16),
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        let c = cx.theme.colors;
        let mid = r.y + (r.h - Lu::px(3)) / 2;
        let track = LRect::new(r.x, mid + Lu::ONE, r.w, Lu::ONE);
        cx.fill(track, c.strong);
        let fill = progress_color(cx.theme, self.status, cx.state.enabled);
        if self.indeterminate && self.status == Progress::Running {
            let (x, w) = if cx.theme.reduced_motion {
                (r.w / 3, r.w / 3)
            } else {
                // One 40% segment sweeping across every 2 s.
                let period = 2000;
                let t = (cx.now % period) as i32;
                let w = r.w * 2 / 5;
                (
                    Lu((i64::from((r.w + w).0) * i64::from(t) / i64::from(period as i32)) as i32)
                        - w,
                    w,
                )
            };
            let x0 = (r.x + x).max(r.x);
            let x1 = (r.x + x + w).min(r.right());
            cx.push_clip(r);
            cx.round(LRect::new(x0, mid, x1 - x0, Lu::px(3)), 2, fill);
            cx.pop_clip();
        } else {
            let w = r.w.mul_div(self.value as i32, 100);
            cx.round(LRect::new(r.x, mid, w, Lu::px(3)), 2, fill);
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ProgressBar, &self.name);
        node.states |= States::READ_ONLY;
        if self.indeterminate {
            node.states |= States::BUSY;
        } else {
            range_node(node, 0, 100, i64::from(self.value), 1);
        }
        node.value = match self.status {
            Progress::Paused => "Paused".into(),
            Progress::Error => "Error".into(),
            Progress::Running if self.indeterminate => String::new(),
            Progress::Running => format!("{}%", self.value),
        };
        true
    }
}
impl ProgressBar {
    pub fn is_animating(&self) -> bool {
        self.indeterminate && self.status == Progress::Running
    }
}

/// ProgressRing (32 DIPs by default, 4-DIP stroke).
pub struct ProgressRing {
    pub name: String,
    /// None: indeterminate.
    pub value: Option<u32>,
    pub size: i32,
}
impl ProgressRing {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            value: None,
            size: 32,
        }
    }
    pub fn value(mut self, v: u32) -> Self {
        self.value = Some(v.min(100));
        self
    }
    pub fn size(mut self, s: i32) -> Self {
        self.size = s.max(16);
        self
    }
}
impl Widget for ProgressRing {
    fn type_name(&self) -> &'static str {
        "ProgressRing"
    }
    fn animating(&self) -> bool {
        self.is_animating()
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Start)
    }
    fn measure(&self, _cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        Measured::new(LSize::px(self.size, self.size))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        let s = Lu::px(self.size).min(r.w).min(r.h);
        let ring = LRect::new(r.x + (r.w - s) / 2, r.y + (r.h - s) / 2, s, s);
        let width = Lu::px((self.size / 8).max(2));
        let c = cx.theme.colors;
        let color = if cx.state.enabled {
            c.accent
        } else {
            c.accent_disabled
        };
        match self.value {
            Some(v) => {
                cx.push(crate::display::Op::Arc {
                    rect: ring,
                    width,
                    start: 0,
                    sweep: 360,
                    color: c.control_stroke,
                });
                if v > 0 {
                    cx.push(crate::display::Op::Arc {
                        rect: ring,
                        width,
                        start: 0,
                        sweep: (v * 360 / 100) as i32,
                        color,
                    });
                }
            }
            None => {
                let (start, sweep) = if cx.theme.reduced_motion {
                    (0, 270)
                } else {
                    let t = (cx.now % 2000) as i32;
                    (t * 360 / 1000 % 360, 60 + (t - 1000).abs() * 200 / 1000)
                };
                cx.push(crate::display::Op::Arc {
                    rect: ring,
                    width,
                    start,
                    sweep,
                    color,
                });
            }
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ProgressBar, &self.name);
        node.states |= States::READ_ONLY;
        match self.value {
            Some(v) => {
                range_node(node, 0, 100, i64::from(v), 1);
                node.value = format!("{v}%");
            }
            None => node.states |= States::BUSY,
        }
        true
    }
}
impl ProgressRing {
    pub fn is_animating(&self) -> bool {
        self.value.is_none()
    }
}
