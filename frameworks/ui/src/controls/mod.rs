//! The Door control catalog (Windows 11 / WinUI behaviour and metrics).
//! Every control owns a state machine driven by routed events, paints from
//! theme tokens only, describes itself to assistive technology (role, name,
//! value, states, actions) and accepts the matching accessibility actions.
use crate::event::{Button as MouseButton, Event, Key};
use crate::theme::{Color, Theme};
use crate::units::{LRect, Lu};
use crate::widget::{EventCx, PaintCx, WidgetState};

mod basic;
mod combo;
mod containers;
mod dialog;
pub mod document;
mod list;
mod menu;
mod nav;
mod range;
mod text_box;
pub use basic::*;
pub use combo::*;
pub use containers::*;
pub use dialog::*;
pub use list::*;
pub use menu::*;
pub use nav::*;
pub use range::*;
pub use text_box::*;

/// Button activation (WinUI `ClickMode.Release`): pointer press captures,
/// release inside invokes, lost capture cancels; Space arms on key down and
/// invokes on key up; Enter invokes on key down. Disabled controls never
/// receive events, so they never invoke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Press {
    /// Space is held (pressed visual without the pointer).
    pub key_armed: bool,
}
impl Press {
    /// Feed one event; returns true when the control is invoked.
    pub fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) -> bool {
        if cx.phase != crate::event::Phase::Target {
            return false;
        }
        match e {
            Event::PointerDown(p) if p.button == MouseButton::Primary => {
                cx.capture_pointer();
                cx.invalidate();
                cx.set_handled();
                false
            }
            Event::PointerUp(p) if p.button == MouseButton::Primary => {
                let invoked = cx.state.captured && cx.rect.contains(p.pos);
                cx.release_pointer();
                cx.invalidate();
                cx.set_handled();
                invoked
            }
            Event::CaptureLost | Event::PointerEnter | Event::PointerLeave => {
                cx.invalidate();
                false
            }
            Event::KeyDown(k) if k.key == Key::Space && !k.modifiers.ctrl && !k.modifiers.alt => {
                self.key_armed = true;
                cx.invalidate();
                cx.set_handled();
                false
            }
            Event::KeyUp(k) if k.key == Key::Space && self.key_armed => {
                self.key_armed = false;
                cx.invalidate();
                cx.set_handled();
                true
            }
            Event::KeyDown(k) if k.key == Key::Enter && k.modifiers.none() => {
                cx.set_handled();
                true
            }
            Event::KeyDown(k) if k.key == Key::Escape && self.key_armed => {
                self.key_armed = false;
                cx.invalidate();
                cx.set_handled();
                false
            }
            Event::FocusOut => {
                self.key_armed = false;
                false
            }
            _ => false,
        }
    }
    pub fn pressed(&self, s: &WidgetState) -> bool {
        s.pressed || self.key_armed
    }
}

/// Visual state of a pressable surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    Rest,
    Hover,
    Pressed,
    Disabled,
}
impl Visual {
    pub fn of(s: &WidgetState, pressed: bool) -> Self {
        if !s.enabled {
            Self::Disabled
        } else if pressed {
            Self::Pressed
        } else if s.hovered {
            Self::Hover
        } else {
            Self::Rest
        }
    }
}
/// (fill, text) of a standard or accent button surface.
pub fn button_colors(t: &Theme, v: Visual, accent: bool) -> (Color, Color) {
    let c = &t.colors;
    if t.is_high_contrast() {
        // WinUI high-contrast resources: pointer over inverts (Highlight
        // text on a HighlightText fill), pressed is a Highlight fill with
        // HighlightText, disabled uses GrayText.
        return match (v, accent) {
            (Visual::Rest, false) => (c.control, c.text),
            (Visual::Rest, true) => (c.accent, c.text_on_accent),
            (Visual::Hover, _) => (c.text_on_accent, c.accent),
            (Visual::Pressed, _) => (c.accent, c.text_on_accent),
            (Visual::Disabled, _) => (c.control, c.text_disabled),
        };
    }
    if accent {
        match v {
            Visual::Rest => (c.accent, c.text_on_accent),
            Visual::Hover => (c.accent_hover, c.text_on_accent),
            Visual::Pressed => (c.accent_pressed, crate::theme::fade(c.text_on_accent, 200)),
            Visual::Disabled => (c.accent_disabled, c.text_on_accent_disabled),
        }
    } else {
        match v {
            Visual::Rest => (c.control, c.text),
            Visual::Hover => (c.control_hover, c.control_text_hover),
            Visual::Pressed => (c.control_pressed, c.text_secondary),
            Visual::Disabled => (c.control_disabled, c.text_disabled),
        }
    }
}
/// The elevated control surface: 1-DIP border whose bottom edge is darker
/// (WinUI ControlElevationBorder), rounded by the control radius.
pub fn surface(cx: &mut PaintCx<'_>, r: LRect, fill: Color, accent: bool) {
    let t = cx.theme;
    let radius = t.metrics.control_radius;
    let hc = t.is_high_contrast();
    let (stroke, bottom) = if hc {
        // Highlight while pointer over or pressed, GrayText when disabled.
        let s = if !cx.state.enabled {
            t.colors.strong_disabled
        } else if cx.state.hovered || cx.state.pressed || accent {
            t.colors.accent
        } else {
            t.colors.control_stroke
        };
        (s, s)
    } else if accent {
        (t.colors.accent_hover, t.colors.accent_pressed)
    } else {
        (t.colors.control_stroke, t.colors.control_stroke_bottom)
    };
    if !cx.state.enabled && !hc {
        cx.round(r, radius, t.colors.control_stroke);
    } else {
        cx.round(r, radius, bottom);
        cx.round(LRect::new(r.x, r.y, r.w, r.h - Lu::ONE), radius, stroke);
    }
    cx.round(r.inflate(-Lu::ONE), radius - 1, fill);
}

/// Common accessibility-node set-up.
pub(crate) fn describe(
    node: &mut door_accessibility::Node,
    role: door_accessibility::Role,
    name: &str,
) {
    node.role = role;
    if node.name.is_empty() {
        node.name = name.into();
    }
}
