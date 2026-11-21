//! Settings' ContentDialogs (Windows 11 "Rename this PC" style): a smoke
//! layer over the window, a centred card with a title, labelled text boxes,
//! an optional type combo, help and error lines, and a footer of equal-width
//! buttons with the primary one in the accent colour. Geometry comes from
//! `settings_catalog::dialog_geometry`, shared with hit testing and the
//! guest scenarios. The dialog is modal: nothing behind it reacts.
use super::theme::Theme;
use super::*;
use door_workspace::line_edit::LineEdit;
use door_workspace::registry_browser as registry;
use door_workspace::registry_value as value;
use door_workspace::settings_catalog::{DialogGeometry, DialogKind, Rect as R};

/// Hit-test identities inside a dialog.
pub const FIELD: u32 = 6000;
pub const TYPE: u32 = 6010;
pub const BUTTON: u32 = 6020;
pub const BACKGROUND: u32 = 6099;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Field(usize),
    Type,
    Button(usize),
}
/// What Save writes to.
pub(super) enum Target {
    None,
    /// A registry value in the key the snapshot was read from; the exact
    /// name of the value being edited, or None for a new one.
    Registry(registry::Snapshot, Option<Vec<u16>>),
    /// A user variable; its name when editing.
    Variable(Option<String>),
}
pub(super) struct Dialog {
    pub kind: DialogKind,
    pub title: &'static str,
    pub fields: [LineEdit; 2],
    /// Registry value type (`registry_value::KINDS` entry).
    pub value_kind: u8,
    pub focus: Focus,
    pub error: String,
    pub target: Target,
    /// The page item that opened the dialog (focus returns there).
    pub opener: Option<usize>,
    /// The Name field is read-only (editing an existing value or variable).
    pub fixed_name: bool,
}
impl Dialog {
    pub fn new(
        kind: DialogKind,
        title: &'static str,
        target: Target,
        opener: Option<usize>,
    ) -> Self {
        Self {
            kind,
            title,
            fields: [LineEdit::default(), LineEdit::default()],
            value_kind: 1,
            focus: Focus::Field(0),
            error: String::new(),
            target,
            opener,
            fixed_name: false,
        }
    }
    /// Start editing an existing entry: the name is fixed, Data has focus
    /// with its text selected.
    pub fn editing(mut self, name: String, data: String) -> Self {
        self.fields[0].set(name, false);
        self.fields[1].set(data, true);
        self.fixed_name = true;
        self.focus = Focus::Field(1);
        self
    }
    pub fn geometry(&self, width: u32, height: u32) -> DialogGeometry {
        catalog::dialog_geometry(self.kind, width, height)
    }
    /// Keyboard order: editable fields, the type combo, then the buttons.
    pub fn order(&self) -> Vec<Focus> {
        let mut order = Vec::new();
        for i in 0..self.kind.fields().len() {
            if i == 0 && self.fixed_name {
                continue;
            }
            if i == 1 && self.kind.has_type() {
                order.push(Focus::Type);
            }
            order.push(Focus::Field(i));
        }
        order.extend((0..self.kind.buttons().len()).map(Focus::Button));
        order
    }
    pub fn tab(&mut self, back: bool) {
        let order = self.order();
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        self.focus = order[if back { (i + n - 1) % n } else { (i + 1) % n }];
        if let Focus::Field(f) = self.focus {
            self.fields[f].select_all();
        }
    }
    /// Target id under (x, y) (window coordinates).
    pub fn hit(&self, width: u32, height: u32, x: i32, y: i32) -> u32 {
        let g = self.geometry(width, height);
        for i in 0..self.kind.fields().len() {
            if g.fields[i].contains(x, y) && !(i == 0 && self.fixed_name) {
                return FIELD + i as u32;
            }
        }
        if self.kind.has_type() && g.kind.contains(x, y) {
            return TYPE;
        }
        for i in 0..self.kind.buttons().len() {
            if g.buttons[i].contains(x, y) {
                return BUTTON + i as u32;
            }
        }
        BACKGROUND
    }
    /// The focused text box, if any.
    pub fn field(&mut self) -> Option<&mut LineEdit> {
        match self.focus {
            Focus::Field(i) if !(i == 0 && self.fixed_name) => Some(&mut self.fields[i]),
            _ => None,
        }
    }
    /// Text editing keys for the focused box; true when consumed.
    pub fn edit(&mut self, code: u32) -> bool {
        let scan = code & 511;
        let flags = code >> 9;
        let control = flags & door_abi::graphics::CONTROL != 0;
        let shift = flags & door_abi::graphics::SHIFT != 0;
        // PIN boxes take digits only (the AddPin password box takes any).
        let digits = matches!(
            (self.kind, self.focus),
            (DialogKind::Pin, _) | (DialogKind::AddPin, Focus::Field(1))
        );
        let Some(field) = self.field() else {
            return false;
        };
        match scan {
            30 if control => field.select_all(),
            14 => field.backspace(),
            0x153 => field.delete(),
            0x14b | 0x14d => field.horizontal(scan == 0x14d, shift, control),
            0x147 => field.place(0, shift),
            0x14f => field.place(field.text().len(), shift),
            _ => {
                let ch = (code >> 16) as u8;
                if !control
                    && (32..=126).contains(&ch)
                    && (!digits || ch.is_ascii_digit())
                    && field.text().len() < 120
                {
                    field.insert(ch as char);
                } else {
                    return false;
                }
            }
        }
        self.error.clear();
        true
    }
    /// The registry type list's labels.
    pub fn type_labels() -> Vec<&'static str> {
        value::KINDS.iter().map(|k| value::label(*k)).collect()
    }
    pub fn type_index(&self) -> usize {
        value::KINDS
            .iter()
            .position(|k| *k == self.value_kind)
            .unwrap_or(0)
    }
    pub fn draw(&self, v: &mut ui::View<'_>, t: &Theme, keyboard: bool, popup_open: bool) {
        let g = self.geometry(v.width, v.height);
        // Smoke layer (WinUI SmokeFillColorDefault, 30% black).
        v.rounded_asset(0, 0, v.width, v.height, 0, [0, 0, 0, 77]);
        let r = g.rect;
        v.elevation(r.x, r.y + 8, r.w, r.h, if t.dark { 70 } else { 50 });
        v.rounded(r.x, r.y, r.w, r.h, 8, t.card_border);
        v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 7, t.card);
        // Footer band (the dialog's base layer).
        let f = g.footer;
        v.rounded(f.x + 1, f.y, f.w - 2, f.h - 1, 7, t.bg);
        v.rect(f.x + 1, f.y, f.w - 2, 8, t.bg);
        v.rect(f.x + 1, f.y, f.w - 2, 1, t.divider);
        let inner = r.w.saturating_sub(48);
        v.text_clip(
            r.x + 24,
            r.y + 24,
            inner,
            self.title,
            TextStyle::Semibold20,
            t.text,
        );
        for (i, label) in self.kind.fields().iter().enumerate() {
            if i == 1 && self.kind.has_type() {
                let k = g.kind;
                v.text_clip(k.x, k.y - 24, inner, "Type", TextStyle::Regular14, t.text);
                let hot = v.hover.is_some_and(|(x, y)| k.contains(x, y)) && !popup_open;
                combo(v, t, k, value::label(self.value_kind), hot || popup_open);
                if keyboard && self.focus == Focus::Type && !popup_open {
                    focus_ring(v, t, k);
                }
            }
            let b = g.fields[i];
            v.text_clip(b.x, b.y - 24, inner, label, TextStyle::Regular14, t.text);
            let focused = self.focus == Focus::Field(i) && !popup_open;
            let masked;
            let edit = if self.kind.secret_field(i) {
                masked = self.fields[i].masked();
                &masked
            } else {
                &self.fields[i]
            };
            text_box(v, t, b, edit, focused, !(i == 0 && self.fixed_name));
        }
        for (i, line) in self.kind.help().iter().enumerate() {
            v.text_clip(
                r.x + 24,
                g.help + i as i32 * 18,
                inner,
                line,
                TextStyle::Regular12,
                t.secondary,
            );
        }
        if !self.error.is_empty() {
            let error = if t.dark {
                [153, 153, 255, 255]
            } else {
                [28, 43, 196, 255]
            };
            v.text_clip(
                r.x + 24,
                g.error,
                inner,
                &super::pages::fitted(v, &self.error, inner, TextStyle::Regular14),
                TextStyle::Regular14,
                error,
            );
        }
        for (i, label) in self.kind.buttons().iter().enumerate() {
            let b = g.buttons[i];
            let hot = v.hover.is_some_and(|(x, y)| b.contains(x, y)) && !popup_open;
            let down = hot && v.pressed.is_some_and(|(x, y)| b.contains(x, y));
            if i == 0 {
                let fill = if down || hot {
                    t.accent_hover
                } else {
                    t.accent
                };
                v.rounded(b.x, b.y, b.w, b.h, 4, fill);
                v.text_in_box(
                    (b.x, b.y, b.w, b.h),
                    label,
                    TextStyle::Regular14,
                    t.on_accent,
                );
            } else {
                super::pages::button(v, t, b, label, hot, down);
            }
            if keyboard && self.focus == Focus::Button(i) && !popup_open {
                focus_ring(v, t, b);
            }
        }
    }
}
/// WinUI TextBox: control fill, 1 px stroke with a darker bottom edge; when
/// focused, a white (card) fill and a 2 px accent bottom edge.
fn text_box(v: &mut ui::View<'_>, t: &Theme, r: R, edit: &LineEdit, focused: bool, enabled: bool) {
    v.rounded(r.x, r.y, r.w, r.h, 4, t.control_border);
    v.rounded(
        r.x,
        r.y + 1,
        r.w,
        r.h - 1,
        4,
        if focused { t.accent } else { t.control_bottom },
    );
    let fill = if !enabled {
        t.control_disabled
    } else if focused {
        if t.dark {
            [30, 27, 26, 255]
        } else {
            [255, 255, 255, 255]
        }
    } else {
        t.control
    };
    v.rounded(
        r.x + 1,
        r.y + 1,
        r.w - 2,
        r.h - if focused { 4 } else { 3 },
        3,
        fill,
    );
    if enabled {
        v.line_editor(
            r.x + 11,
            r.y + (r.h as i32 - 20) / 2,
            r.w.saturating_sub(22),
            edit,
            focused,
        );
    } else {
        v.text_clip_centered(
            r.x + 11,
            (r.y, r.h),
            r.w.saturating_sub(22),
            &ui::display_ascii(edit.text()),
            TextStyle::Regular14,
            t.disabled,
        );
    }
}
fn combo(v: &mut ui::View<'_>, t: &Theme, r: R, label: &str, hot: bool) {
    v.rounded(r.x, r.y, r.w, r.h, 4, t.control_border);
    v.rounded(r.x, r.y + 1, r.w, r.h - 1, 4, t.control_bottom);
    v.rounded(
        r.x + 1,
        r.y + 1,
        r.w - 2,
        r.h - 3,
        3,
        if hot { t.control_hover } else { t.control },
    );
    v.text_clip_centered(
        r.x + 11,
        (r.y, r.h),
        r.w.saturating_sub(44),
        label,
        TextStyle::Regular14,
        t.text,
    );
    v.mask_pixels(
        r.right() - 28,
        r.center_y() - 6,
        12,
        Icon::Chevron.mask_scaled(12, v.scale),
        t.secondary,
        None,
    );
}
fn focus_ring(v: &mut ui::View<'_>, t: &Theme, r: R) {
    super::pages::focus_ring(v, t, r);
}
