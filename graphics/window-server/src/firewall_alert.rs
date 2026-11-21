//! "Doors Security Alert" (Windows Security Alert): door.network blocked a
//! program from accepting connections and no firewall rule names it. As on
//! Windows 11 the dialog names the program (file name, publisher, path),
//! offers the network types to allow it on (the active one, public,
//! checked) and has "Allow access" (shield: administrator approval) and
//! Cancel, which blocks the program on those networks. It is an ordinary
//! topmost dialog, not the secure desktop: presses outside pass, and it
//! has the keyboard while open (Alt+A allows, Esc cancels, Tab and Space
//! move and toggle).
use crate::paint::{self, Palette};
use alloc::string::String;
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};

/// Profile bits (`door_firewall::profile`).
pub const PRIVATE: u8 = 0x2;
pub const PUBLIC: u8 = 0x4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Allow on these profiles (after User Account Control).
    Allow(u8),
    /// Block on these profiles.
    Cancel(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Private,
    Public,
    Allow,
    Cancel,
    Close,
    Panel,
}
/// An open alert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub pid: u64,
    /// The program's Windows path, and its name (the file name).
    pub path: String,
    pub name: String,
    pub private: bool,
    pub public: bool,
    pub focus: Target,
    pressed: Option<Target>,
}
impl Alert {
    pub fn new(pid: u64, path: &str) -> Self {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        Self {
            pid,
            path: String::from(path),
            name: String::from(name),
            private: false,
            // The network Doors is on: public.
            public: true,
            focus: Target::Allow,
            pressed: None,
        }
    }
    fn profiles(&self) -> u8 {
        (if self.private { PRIVATE } else { 0 }) | if self.public { PUBLIC } else { 0 }
    }
    fn press(&mut self, target: Target) -> Option<Answer> {
        match target {
            Target::Private => self.private = !self.private,
            Target::Public => self.public = !self.public,
            // Allow access needs a network to allow it on.
            Target::Allow if self.profiles() != 0 => return Some(Answer::Allow(self.profiles())),
            Target::Cancel | Target::Close => {
                return Some(Answer::Cancel(self.profiles().max(PUBLIC)));
            }
            _ => (),
        }
        None
    }
    /// A key press: Alt+A allows, Esc (and Alt+F4) cancels, Enter the
    /// focused button, Space toggles the focused box, Tab moves the focus.
    pub fn key(&mut self, scan: u32, alt: bool) -> Option<Answer> {
        const ORDER: [Target; 4] = [
            Target::Private,
            Target::Public,
            Target::Allow,
            Target::Cancel,
        ];
        match scan {
            30 if alt => self.press(Target::Allow),
            62 if alt => self.press(Target::Cancel),
            1 => self.press(Target::Cancel),
            28 | 0x11c => self.press(match self.focus {
                Target::Private | Target::Public => Target::Allow,
                t => t,
            }),
            57 => self.press(self.focus),
            15 | 0x14d | 0x150 => {
                let at = ORDER.iter().position(|t| *t == self.focus).unwrap_or(0);
                self.focus = ORDER[(at + 1) % ORDER.len()];
                None
            }
            0x14b | 0x148 => {
                let at = ORDER.iter().position(|t| *t == self.focus).unwrap_or(0);
                self.focus = ORDER[(at + ORDER.len() - 1) % ORDER.len()];
                None
            }
            _ => None,
        }
    }
    /// A primary press or release at (`x`, `y`): (consumed, answer).
    pub fn button(&mut self, area: Rect, x: i32, y: i32, down: bool) -> (bool, Option<Answer>) {
        let target = hit(area, x, y);
        if down {
            self.pressed = target;
            return (target.is_some(), None);
        }
        let pressed = self.pressed.take();
        let Some(t) = target.filter(|t| Some(*t) == pressed) else {
            return (target.is_some() || pressed.is_some(), None);
        };
        if t != Target::Panel && t != Target::Close {
            self.focus = t;
        }
        (true, self.press(t))
    }
}
pub struct Geometry {
    pub panel: Rect,
    pub close: Rect,
    pub private: Rect,
    pub public: Rect,
    pub allow: Rect,
    pub cancel: Rect,
}
const WIDTH: u32 = 520;
const HEIGHT: u32 = 404;
const FOOTER: u32 = 64;
const TITLE: u32 = 32;
/// Centred on `area` (the work area).
pub fn geometry(area: Rect) -> Geometry {
    let w = WIDTH.min(area.width().max(1));
    let h = HEIGHT.min(area.height().max(1));
    let x = area.x() + (area.width().saturating_sub(w) / 2) as i32;
    let y = area.y() + (area.height().saturating_sub(h) / 2) as i32;
    let panel = Rect::new(x, y, w, h).unwrap_or(area);
    let r = |x: i32, y: i32, w: u32, h: u32| Rect::new(x, y, w, h).unwrap_or(panel);
    let by = panel.bottom() - (FOOTER as i32 + 32) / 2;
    Geometry {
        panel,
        close: r(panel.right() - 46, y + 1, 45, TITLE as i32 as u32 - 1),
        private: r(x + 40, y + 252, w - 64, 22),
        public: r(x + 40, y + 280, w - 64, 38),
        allow: r(panel.right() - 24 - 120 - 8 - 132, by, 132, 32),
        cancel: r(panel.right() - 24 - 120, by, 120, 32),
    }
}
pub fn hit(area: Rect, x: i32, y: i32) -> Option<Target> {
    let g = geometry(area);
    [
        (g.close, Target::Close),
        (g.private, Target::Private),
        (g.public, Target::Public),
        (g.allow, Target::Allow),
        (g.cancel, Target::Cancel),
        (g.panel, Target::Panel),
    ]
    .into_iter()
    .find(|(r, _)| r.contains(x, y))
    .map(|(_, t)| t)
}
#[allow(clippy::too_many_arguments)]
fn row(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    y: i32,
    text: &str,
    style: TextStyle,
    ink: [u8; 4],
    width: u32,
) {
    let text = paint::ellipsize(fonts, text, style, width);
    paint::lcd::text_row(c, fonts, x, y, 20, &text, style, ink, width);
}
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    alert: &Alert,
    area: Rect,
    dark: bool,
    pointer: (i32, i32),
) {
    let palette = Palette::new(dark);
    let g = geometry(area);
    let p = g.panel;
    paint::shadow(c, p, 8, dark);
    let (base, ink, secondary, band) = if dark {
        (
            paint::rgb(43, 43, 43),
            paint::rgb(255, 255, 255),
            paint::rgb(200, 200, 200),
            paint::rgb(32, 32, 32),
        )
    } else {
        (
            paint::rgb(255, 255, 255),
            paint::rgb(26, 26, 26),
            paint::rgb(96, 96, 96),
            paint::rgb(243, 243, 243),
        )
    };
    paint::framed(c, p, 8, palette.surface_border, base);
    // Title bar and footer.
    if let Ok(title) = Rect::new(p.x() + 1, p.y() + 1, p.width() - 2, TITLE) {
        paint::fill(c, title, band);
    }
    if let Ok(footer) = Rect::new(
        p.x() + 1,
        p.bottom() - FOOTER as i32,
        p.width() - 2,
        FOOTER - 1,
    ) {
        paint::fill(c, footer, band);
    }
    let hover = hit(area, pointer.0, pointer.1);
    paint::glyph(
        c,
        p.x() + 12,
        p.y() + 8,
        16,
        crate::shell_mask!("shell-shield", 16),
        paint::rgb(0, 103, 192),
    );
    row(
        c,
        fonts,
        p.x() + 36,
        p.y() + 6,
        "Doors Security Alert",
        TextStyle::Regular12,
        ink,
        300,
    );
    if hover == Some(Target::Close) {
        paint::fill(c, g.close, paint::rgb(196, 43, 28));
    }
    paint::glyph(
        c,
        g.close.x() + 16,
        g.close.y() + 9,
        12,
        crate::shell_mask!("explorer-close", 12),
        if hover == Some(Target::Close) {
            paint::rgb(255, 255, 255)
        } else {
            ink
        },
    );
    let width = p.width().saturating_sub(48);
    let x = p.x() + 24;
    let mut y = p.y() + TITLE as i32 + 16;
    let heading = if dark {
        paint::rgb(153, 204, 255)
    } else {
        paint::rgb(0, 51, 153)
    };
    row(
        c,
        fonts,
        x,
        y,
        "Doors Defender Firewall has blocked some features of this app",
        TextStyle::Semibold14,
        heading,
        width,
    );
    y += 34;
    let blocked = alloc::format!(
        "Doors Defender Firewall has blocked some features of {} on all public and private networks.",
        alert.name
    );
    let mut lines = 0;
    let mut line = String::new();
    for word in blocked.split(' ') {
        let candidate = if line.is_empty() {
            String::from(word)
        } else {
            alloc::format!("{line} {word}")
        };
        if !line.is_empty() && paint::measure(fonts, &candidate, TextStyle::Regular12) > width {
            row(
                c,
                fonts,
                x,
                y + lines * 20,
                &line,
                TextStyle::Regular12,
                ink,
                width,
            );
            lines += 1;
            line = String::from(word);
        } else {
            line = candidate;
        }
    }
    row(
        c,
        fonts,
        x,
        y + lines * 20,
        &line,
        TextStyle::Regular12,
        ink,
        width,
    );
    y += (lines + 1) * 20 + 14;
    for (label, value) in [
        ("Name:", alert.name.as_str()),
        ("Publisher:", "Unknown"),
        ("Path:", alert.path.as_str()),
    ] {
        row(
            c,
            fonts,
            x + 16,
            y,
            label,
            TextStyle::Regular12,
            secondary,
            80,
        );
        row(
            c,
            fonts,
            x + 100,
            y,
            value,
            TextStyle::Regular12,
            ink,
            width - 100,
        );
        y += 22;
    }
    let question = alloc::format!("Allow {} to communicate on these networks:", alert.name);
    row(
        c,
        fonts,
        x,
        p.y() + 226,
        &question,
        TextStyle::Regular12,
        ink,
        width,
    );
    for (target, r, on, label, second) in [
        (
            Target::Private,
            g.private,
            alert.private,
            "Private networks, such as my home or work network",
            "",
        ),
        (
            Target::Public,
            g.public,
            alert.public,
            "Public networks, such as those in airports and coffee shops (not",
            "recommended because these networks often have little or no security)",
        ),
    ] {
        let Ok(boxed) = Rect::new(r.x(), r.y() + 2, 16, 16) else {
            continue;
        };
        if on {
            paint::rounded(c, boxed, 3, paint::rgb(0, 103, 192));
            paint::glyph(
                c,
                boxed.x(),
                boxed.y(),
                16,
                crate::shell_mask!("shell-check", 16),
                paint::rgb(255, 255, 255),
            );
        } else {
            paint::framed(c, boxed, 3, secondary, base);
        }
        if alert.focus == target {
            paint::focus_ring(c, boxed, &palette);
        }
        row(
            c,
            fonts,
            r.x() + 26,
            r.y(),
            label,
            TextStyle::Regular12,
            ink,
            r.width() - 26,
        );
        if !second.is_empty() {
            row(
                c,
                fonts,
                r.x() + 26,
                r.y() + 18,
                second,
                TextStyle::Regular12,
                ink,
                r.width() - 26,
            );
        }
    }
    row(
        c,
        fonts,
        x,
        p.y() + 328,
        "What are the risks of allowing an app through a firewall?",
        TextStyle::Regular12,
        paint::rgb(0, 103, 192),
        width,
    );
    for (target, r, label) in [
        (Target::Allow, g.allow, "Allow access"),
        (Target::Cancel, g.cancel, "Cancel"),
    ] {
        let hot = hover == Some(target);
        let enabled = target != Target::Allow || alert.profiles() != 0;
        let (fill, border, text) = if target == Target::Allow && enabled {
            let fill = if hot {
                paint::rgb(25, 117, 197)
            } else {
                paint::rgb(0, 103, 192)
            };
            (fill, fill, paint::rgb(255, 255, 255))
        } else {
            (
                if hot {
                    paint::rgb(246, 246, 246)
                } else {
                    paint::rgb(251, 251, 251)
                },
                paint::rgb(229, 229, 229),
                if enabled {
                    paint::rgb(26, 26, 26)
                } else {
                    paint::rgb(160, 160, 160)
                },
            )
        };
        paint::framed(c, r, 4, border, fill);
        if alert.focus == target {
            paint::focus_ring(c, r, &palette);
        }
        if target == Target::Allow {
            paint::glyph(
                c,
                r.x() + 12,
                r.y() + 8,
                16,
                crate::shell_mask!("shell-shield", 16),
                text,
            );
            if let Ok(label_box) = Rect::new(r.x() + 20, r.y(), r.width() - 20, r.height()) {
                paint::lcd::text_in(c, fonts, label_box, label, TextStyle::Regular14, text);
            }
        } else {
            paint::lcd::text_in(c, fonts, r, label, TextStyle::Regular14, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> Rect {
        Rect::new(0, 0, 1280, 752).unwrap()
    }
    #[test]
    fn answers() {
        let mut a = Alert::new(7, r"C:\Users\1001\server.exe");
        assert_eq!(a.name, "server.exe");
        assert_eq!(a.key(30, true), Some(Answer::Allow(PUBLIC)));
        let g = geometry(area());
        // The private box, then Allow access by pointer.
        let (x, y) = (g.private.x() + 4, g.private.y() + 4);
        assert_eq!(a.button(area(), x, y, true), (true, None));
        assert_eq!(a.button(area(), x, y, false), (true, None));
        assert!(a.private);
        let (x, y) = (g.allow.x() + 4, g.allow.y() + 4);
        a.button(area(), x, y, true);
        assert_eq!(
            a.button(area(), x, y, false),
            (true, Some(Answer::Allow(PRIVATE | PUBLIC)))
        );
        // Nothing checked: Allow access does nothing; Cancel blocks on public.
        let mut b = Alert::new(7, "C:\\x.exe");
        b.public = false;
        assert_eq!(b.key(30, true), None);
        assert_eq!(b.key(1, false), Some(Answer::Cancel(PUBLIC)));
        // Outside the dialog: not consumed.
        assert_eq!(b.button(area(), 2, 2, true), (false, None));
        // Tab order and Space.
        let mut c = Alert::new(1, "C:\\y.exe");
        assert_eq!(c.focus, Target::Allow);
        c.key(15, false);
        assert_eq!(c.focus, Target::Cancel);
        c.key(15, false);
        c.key(57, false);
        assert!(c.private);
    }
}
