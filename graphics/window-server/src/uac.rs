//! User Account Control consent prompt, drawn by the window server itself
//! over a dimmed "secure desktop": while it is open it owns every key and
//! press, so no application can answer it, cover it or read what it shows.
//! Geometry and colours follow Microsoft Learn's Windows 11 captures ("How
//! User Account Control works", 100%): a 456 px dialog with a 114 px header
//! band (pale yellow, 140 px and a longer question for an unknown
//! publisher), the app's icon and name, its publisher and file origin, a
//! "Show more details" link, and Yes / No in an 80 px footer with No the
//! accent default.
//!
//! An administrator's session approves with consent (Yes / No). A standard
//! user's session gets the credential prompt instead: "To continue, enter
//! an admin user name and password.", an administrator account's tile and
//! a password box; Yes checks that password with the session service
//! before anything runs (the elevated program still runs as the signed-in
//! user, with the Administrators group: Windows runs it as the
//! administrator's account).
extern crate alloc;
use crate::apps::{App, Shell};
use crate::paint::{self, Palette};
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};

/// The verified publisher of Doors components (signed with the system).
pub const PUBLISHER: &str = "Macrohard Doors";
/// What approval runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Then {
    /// Settings at this route (`door_settings_map::navigate_arg`).
    Settings(u32),
    /// Terminal on an administrator tab of this shell.
    Terminal(Shell),
    /// File Explorer's Run as administrator on a program: Init starts
    /// `path` in `cwd` (`directory`, a handle the server holds) as the
    /// session user, if its bytes still hash to `image`.
    Program {
        path: String,
        cwd: String,
        directory: u64,
        image: [u8; 32],
        /// One argument after the program's own name, or none (empty):
        /// what the elevated program is told the user approved
        /// (Windows Installer's consent token).
        arguments: String,
    },
    /// Run as administrator on a batch file: an administrator Terminal tab
    /// runs it.
    Script(String),
    /// A firewall change Init stores (`door_service_protocol::firewall`
    /// CHANGE body): the Security Alert's Allow access, the Settings
    /// firewall page.
    Firewall(Vec<u8>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    App(App),
    Shell(Shell),
    /// A program's file-type icon (Run as administrator from File Explorer).
    File(crate::apps::FileIcon),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Button(Answer),
    Details,
    Close,
    Panel,
}
/// What the credential prompt says under the password box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialNotice {
    None,
    /// The password is being checked.
    Checking,
    Incorrect,
    /// Too many wrong passwords: try again in this many seconds.
    Throttled(u64),
}
/// A standard user's prompt: the administrator whose password approves.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    pub uid: u32,
    pub name: String,
    secret: alloc::vec::Vec<u8>,
    pub notice: CredentialNotice,
}
impl core::fmt::Debug for Credential {
    // Never the password.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Credential")
            .field("uid", &self.uid)
            .field("name", &self.name)
            .field("typed", &self.secret.len())
            .field("notice", &self.notice)
            .finish()
    }
}
impl Drop for Credential {
    fn drop(&mut self) {
        self.clear();
    }
}
impl Credential {
    pub fn new(uid: u32, name: &str) -> Self {
        Self {
            uid,
            name: String::from(name),
            secret: alloc::vec::Vec::new(),
            notice: CredentialNotice::None,
        }
    }
    pub fn secret(&self) -> &[u8] {
        &self.secret
    }
    pub fn typed(&self) -> usize {
        self.secret.len()
    }
    pub fn clear(&mut self) {
        self.secret.fill(0);
        core::hint::black_box(&mut self.secret);
        self.secret.clear();
    }
}
/// An open consent prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consent {
    /// The program's name as its file description gives it.
    pub program: String,
    /// Verified publisher (Authenticode or a Doors component), or None.
    pub publisher: Option<String>,
    /// Program location shown under "Show more details".
    pub location: String,
    /// "Downloaded from the Internet" instead of "Hard drive on this computer".
    pub downloaded: bool,
    pub icon: Icon,
    /// The program's own icon (Run as administrator), drawn instead of `icon`.
    pub image: Option<door_pe_icon::Image>,
    pub then: Then,
    pub details: bool,
    pub focus: Answer,
    pressed: Option<Target>,
    /// A standard user's session: an administrator's password.
    pub credential: Option<Credential>,
    translator: door_keyboard_layouts::Translator,
}
impl Consent {
    pub fn new(
        program: &str,
        publisher: Option<&str>,
        location: &str,
        icon: Icon,
        then: Then,
    ) -> Self {
        Self {
            program: String::from(program),
            publisher: publisher.map(String::from),
            location: String::from(location),
            downloaded: false,
            icon,
            image: None,
            then,
            details: false,
            // No is the default: Enter alone never approves.
            focus: Answer::No,
            pressed: None,
            credential: None,
            translator: door_keyboard_layouts::Translator::new(),
        }
    }
    /// The credential prompt for a standard user's session; Yes is the
    /// default once a password is typed.
    pub fn with_credential(mut self, uid: u32, name: &str) -> Self {
        self.credential = Some(Credential::new(uid, name));
        self.focus = Answer::Yes;
        self
    }
    /// Whether Yes may be pressed: consent always, credentials once a
    /// password is typed and not while it is checked.
    pub fn approvable(&self) -> bool {
        self.credential
            .as_ref()
            .is_none_or(|c| c.typed() > 0 && !matches!(c.notice, CredentialNotice::Checking))
    }
    pub fn question(&self) -> &'static str {
        if self.publisher.is_some() {
            "Do you want to allow this app to make changes to your device?"
        } else {
            "Do you want to allow this app from an unknown publisher to make changes to your device?"
        }
    }
    pub fn origin(&self) -> &'static str {
        if self.downloaded {
            "File origin: Downloaded from the Internet"
        } else {
            "File origin: Hard drive on this computer"
        }
    }
    /// A key event (`flags` the display flags; Alt held: shortcuts).
    /// The credential prompt's password box takes typed text (US layout,
    /// as the sign-in screen) and Backspace; Enter there approves.
    pub fn key_event(&mut self, scan: u32, flags: u32) -> Option<Answer> {
        let down = flags & door_abi::graphics::DOWN != 0;
        let translation =
            self.translator
                .translate(&door_keyboard_layouts::layouts::US, scan, flags);
        if !down {
            return None;
        }
        let alt = flags & door_abi::graphics::ALT != 0;
        if let Some(c) = self.credential.as_mut()
            && !alt
            && !matches!(c.notice, CredentialNotice::Checking)
        {
            match scan {
                0x0e => {
                    c.secret.pop();
                    return None;
                }
                0x1c | 0x11c => {
                    return (c.typed() > 0).then_some(Answer::Yes);
                }
                0x01 | 0x0f | 0x14b | 0x14d | 0x148 | 0x150 | 0x39 => (),
                _ => {
                    for ch in translation.text.chars() {
                        let mut b = [0u8; 4];
                        let s = ch.encode_utf8(&mut b);
                        if !ch.is_control()
                            && c.secret.len() + s.len()
                                <= door_service_protocol::credential::SECRET_MAX
                        {
                            c.secret.extend_from_slice(s.as_bytes());
                            if c.notice == CredentialNotice::Incorrect {
                                c.notice = CredentialNotice::None;
                            }
                        }
                    }
                    return None;
                }
            }
        }
        self.key(scan, alt)
    }
    /// A key (`scan`, with Alt held): Alt+Y approves, Alt+N, Esc and the
    /// close shortcut decline, Enter or Space press the focused button,
    /// Tab and the arrows move the focus. Every key is the prompt's.
    pub fn key(&mut self, scan: u32, alt: bool) -> Option<Answer> {
        let answer = self.key_inner(scan, alt);
        if answer == Some(Answer::Yes) && !self.approvable() {
            return None;
        }
        answer
    }
    fn key_inner(&mut self, scan: u32, alt: bool) -> Option<Answer> {
        match scan {
            21 if alt => Some(Answer::Yes),
            49 if alt => Some(Answer::No),
            62 if alt => Some(Answer::No),
            1 => Some(Answer::No),
            28 | 57 | 0x11c => Some(self.focus),
            15 | 0x14b | 0x14d | 0x148 | 0x150 => {
                self.focus = if self.focus == Answer::Yes {
                    Answer::No
                } else {
                    Answer::Yes
                };
                None
            }
            _ => None,
        }
    }
    /// A left press or release at (`x`, `y`): (consumed, answer). Buttons
    /// answer on release over the button pressed; "Show more details"
    /// toggles. Presses outside the dialog are swallowed (secure desktop).
    pub fn button(&mut self, area: Rect, x: i32, y: i32, down: bool) -> Option<Answer> {
        let hit = hit(area, self, x, y);
        if down {
            self.pressed = hit;
            return None;
        }
        let pressed = self.pressed.take();
        if hit != pressed {
            return None;
        }
        match hit {
            Some(Target::Button(Answer::Yes)) if !self.approvable() => None,
            Some(Target::Button(a)) => {
                self.focus = a;
                Some(a)
            }
            Some(Target::Close) => Some(Answer::No),
            Some(Target::Details) => {
                self.details = !self.details;
                None
            }
            _ => None,
        }
    }
    fn header(&self) -> u32 {
        if self.publisher.is_some() { 114 } else { 140 }
    }
    fn body(&self) -> u32 {
        178 + if self.details { 44 } else { 0 }
            + if self.credential.is_some() {
                CREDENTIAL
            } else {
                0
            }
    }
}
/// Logical layout, centred on the whole output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub panel: Rect,
    pub close: Rect,
    pub details: Rect,
    pub yes: Rect,
    pub no: Rect,
}
const WIDTH: u32 = 456;
const FOOTER: u32 = 80;
/// The credential prompt's band: its line, the account tile, the password
/// box and its notice.
const CREDENTIAL: u32 = 150;
const BUTTON: (u32, u32) = (196, 32);
pub fn geometry(area: Rect, consent: &Consent) -> Geometry {
    let h = consent.header() + consent.body() + FOOTER;
    let w = WIDTH.min(area.width().max(1));
    let x = area.x() + (area.width().saturating_sub(w) / 2) as i32;
    let y = area.y() + (area.height().saturating_sub(h) / 2) as i32;
    let panel = Rect::new(x, y, w, h).unwrap_or(area);
    let body = y + consent.header() as i32;
    let link = body + 122 + if consent.details { 44 } else { 0 };
    let by = panel.bottom() - (FOOTER as i32 + BUTTON.1 as i32) / 2;
    let r = |x: i32, y: i32, w: u32, h: u32| Rect::new(x, y, w, h).unwrap_or(panel);
    Geometry {
        panel,
        close: r(panel.right() - 46, y + 1, 45, 31),
        details: r(x + 24, link, 140, 20),
        yes: r(x + 24, by, BUTTON.0, BUTTON.1),
        no: r(panel.right() - 24 - BUTTON.0 as i32, by, BUTTON.0, BUTTON.1),
    }
}
pub fn hit(area: Rect, consent: &Consent, x: i32, y: i32) -> Option<Target> {
    let g = geometry(area, consent);
    if g.yes.contains(x, y) {
        Some(Target::Button(Answer::Yes))
    } else if g.no.contains(x, y) {
        Some(Target::Button(Answer::No))
    } else if g.close.contains(x, y) {
        Some(Target::Close)
    } else if g.details.contains(x, y) {
        Some(Target::Details)
    } else if g.panel.contains(x, y) {
        Some(Target::Panel)
    } else {
        None
    }
}
/// Greedy word wrap into lines no wider than `width`.
fn wrap(fonts: &FontAtlas, text: &str, style: TextStyle, width: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() {
            String::from(word)
        } else {
            alloc::format!("{line} {word}")
        };
        if !line.is_empty() && paint::measure(fonts, &candidate, style) > width {
            lines.push(core::mem::replace(&mut line, String::from(word)));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
/// The secure desktop (the screen dimmed) and the dialog.
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    consent: &Consent,
    area: Rect,
    pointer: (i32, i32),
) {
    let palette = Palette::new(false);
    paint::fill(c, area, [0, 0, 0, 150]);
    let g = geometry(area, consent);
    let p = g.panel;
    paint::shadow(c, p, 8, false);
    let header_color = if consent.publisher.is_some() {
        paint::rgb(231, 231, 231)
    } else {
        paint::rgb(255, 244, 206)
    };
    let text = paint::rgb(26, 26, 26);
    let secondary = paint::rgb(96, 96, 96);
    paint::rounded(c, p, 8, paint::rgb(228, 228, 228));
    let Some(inner) = paint::inset(p, 1, 1) else {
        return;
    };
    paint::rounded(c, inner, 7, paint::rgb(243, 243, 243));
    let header = consent.header() as i32;
    if let Ok(top) = Rect::new(inner.x(), inner.y(), inner.width(), header as u32 + 7) {
        paint::rounded(c, top, 7, header_color);
    }
    let body_top = p.y() + header;
    if let Ok(body) = Rect::new(inner.x(), body_top, inner.width(), consent.body()) {
        paint::fill(c, body, paint::rgb(255, 255, 255));
    }
    let width = p.width().saturating_sub(48);
    paint::lcd::text_row(
        c,
        fonts,
        p.x() + 14,
        p.y() + 6,
        20,
        "User Account Control",
        TextStyle::Regular12,
        text,
        width,
    );
    let hover = hit(area, consent, pointer.0, pointer.1);
    if hover == Some(Target::Close) {
        paint::fill(c, g.close, paint::rgb(196, 43, 28));
    }
    let close_ink = if hover == Some(Target::Close) {
        paint::rgb(255, 255, 255)
    } else {
        text
    };
    paint::glyph(
        c,
        g.close.x() + 16,
        g.close.y() + 9,
        12,
        crate::shell_mask!("explorer-close", 12),
        close_ink,
    );
    for (i, line) in wrap(fonts, consent.question(), TextStyle::Regular20, width)
        .iter()
        .enumerate()
        .take(3)
    {
        paint::lcd::text_row(
            c,
            fonts,
            p.x() + 24,
            p.y() + 38 + i as i32 * 28,
            28,
            line,
            TextStyle::Regular20,
            text,
            width,
        );
    }
    // The program: its icon, name, publisher and origin.
    let (ix, iy) = (p.x() + 24, body_top + 20);
    match (&consent.image, consent.icon) {
        (Some(image), _) => crate::apps::draw_image_icon(c, ix, iy, 32, image),
        (None, Icon::App(app)) => crate::apps::draw_icon(c, ix, iy, app, 32),
        (None, Icon::Shell(shell)) => crate::apps::draw_shell_icon(c, ix, iy, shell, 32),
        (None, Icon::File(file)) => crate::apps::draw_file_icon(c, ix, iy, file, 32),
    }
    let name = paint::ellipsize(fonts, &consent.program, TextStyle::Regular16, width - 44);
    paint::lcd::text_row(
        c,
        fonts,
        ix + 44,
        iy,
        32,
        &name,
        TextStyle::Regular16,
        text,
        width - 44,
    );
    let publisher = match &consent.publisher {
        Some(name) => alloc::format!("Verified publisher: {name}"),
        None => String::from("Publisher: Unknown"),
    };
    let mut y = body_top + 66;
    for line in [publisher.as_str(), consent.origin()] {
        let line = paint::ellipsize(fonts, line, TextStyle::Regular12, width);
        paint::lcd::text_row(
            c,
            fonts,
            p.x() + 24,
            y,
            20,
            &line,
            TextStyle::Regular12,
            text,
            width,
        );
        y += 22;
    }
    if consent.details {
        let location = alloc::format!("Program location: \"{}\"", consent.location);
        for line in wrap(fonts, &location, TextStyle::Regular12, width)
            .iter()
            .take(2)
        {
            paint::lcd::text_row(
                c,
                fonts,
                p.x() + 24,
                y,
                20,
                line,
                TextStyle::Regular12,
                secondary,
                width,
            );
            y += 22;
        }
    }
    let link = if consent.details {
        "Hide details"
    } else {
        "Show more details"
    };
    let link_color = if hover == Some(Target::Details) {
        paint::rgb(0, 62, 146)
    } else {
        paint::rgb(0, 103, 192)
    };
    paint::lcd::text_row(
        c,
        fonts,
        g.details.x(),
        g.details.y(),
        20,
        link,
        TextStyle::Regular12,
        link_color,
        160,
    );
    // A standard user: an administrator's tile and password box.
    if let Some(cred) = &consent.credential {
        let top = g.details.bottom() + 16;
        paint::lcd::text_row(
            c,
            fonts,
            p.x() + 24,
            top,
            20,
            "To continue, enter an admin user name and password.",
            TextStyle::Regular12,
            text,
            width,
        );
        let tile = top + 28;
        if let Ok(avatar) = Rect::new(p.x() + 24, tile, 40, 40) {
            paint::rounded(c, avatar, 20, paint::rgb(204, 204, 204));
        }
        paint::lcd::text_row(
            c,
            fonts,
            p.x() + 76,
            tile + 10,
            20,
            &cred.name,
            TextStyle::Semibold14,
            text,
            width - 52,
        );
        if let Ok(field) = Rect::new(p.x() + 24, tile + 52, width, 32) {
            paint::framed(
                c,
                field,
                4,
                paint::rgb(134, 134, 134),
                paint::rgb(255, 255, 255),
            );
            let shown: String = if cred.typed() == 0 {
                String::from("Password")
            } else {
                core::iter::repeat_n('\u{25cf}', cred.typed().min(40)).collect()
            };
            let ink = if cred.typed() == 0 { secondary } else { text };
            paint::lcd::text_row(
                c,
                fonts,
                field.x() + 10,
                field.y() + 6,
                20,
                &shown,
                TextStyle::Regular14,
                ink,
                field.width().saturating_sub(20),
            );
        }
        let notice = match cred.notice {
            CredentialNotice::None => None,
            CredentialNotice::Checking => Some(String::from("Checking...")),
            CredentialNotice::Incorrect => {
                Some(String::from("The user name or password is incorrect."))
            }
            CredentialNotice::Throttled(s) => Some(alloc::format!(
                "Too many attempts. Try again in {s} seconds."
            )),
        };
        if let Some(notice) = notice {
            paint::lcd::text_row(
                c,
                fonts,
                p.x() + 24,
                tile + 90,
                20,
                &notice,
                TextStyle::Regular12,
                paint::rgb(196, 43, 28),
                width,
            );
        }
    }
    // Yes / No; No is the accent default (Yes for the credential prompt).
    let accent = if consent.credential.is_some() {
        Answer::Yes
    } else {
        Answer::No
    };
    for (answer, r, label) in [(Answer::Yes, g.yes, "Yes"), (Answer::No, g.no, "No")] {
        let hot = hover == Some(Target::Button(answer));
        let (fill, border, ink) = if answer == accent {
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
                text,
            )
        };
        paint::framed(c, r, 4, border, fill);
        if consent.focus == answer {
            paint::focus_ring(c, r, &palette);
        }
        paint::lcd::text_in(c, fonts, r, label, TextStyle::Regular14, ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prompt() -> Consent {
        Consent::new(
            "Registry Editor",
            Some("Macrohard Doors"),
            "C:\\Windows\\regedit.exe",
            Icon::App(App::Settings),
            Then::Settings(7),
        )
    }
    #[test]
    fn a_standard_user_types_an_administrators_password_before_yes() {
        use door_abi::graphics::{ALT, DOWN};
        let area = Rect::new(0, 0, 1280, 720).unwrap();
        let mut c = prompt().with_credential(1001, "Owner");
        assert_eq!(c.focus, Answer::Yes);
        assert!(!c.approvable());
        // Enter and Alt+Y do nothing until a password is typed.
        assert_eq!(c.key_event(0x1c, DOWN), None);
        assert_eq!(c.key_event(21, DOWN | ALT), None);
        let g = geometry(area, &c);
        assert_eq!(g.panel.height(), 114 + 178 + CREDENTIAL + 80);
        c.button(area, g.yes.x() + 4, g.yes.y() + 4, true);
        assert_eq!(c.button(area, g.yes.x() + 4, g.yes.y() + 4, false), None);
        // "pa" then Backspace then "q": "pq".
        c.key_event(0x19, DOWN);
        c.key_event(0x1e, DOWN);
        c.key_event(0x0e, DOWN);
        c.key_event(0x10, DOWN);
        assert_eq!(c.credential.as_ref().unwrap().secret(), b"pq");
        assert!(c.approvable());
        assert_eq!(c.key_event(0x1c, DOWN), Some(Answer::Yes));
        // While the password is checked nothing approves twice.
        c.credential.as_mut().unwrap().notice = CredentialNotice::Checking;
        assert!(!c.approvable());
        assert_eq!(c.key_event(0x1c, DOWN), None);
        // Esc still declines; Debug never shows the password.
        c.credential.as_mut().unwrap().notice = CredentialNotice::Incorrect;
        assert_eq!(c.key_event(0x01, DOWN), Some(Answer::No));
        assert!(!alloc::format!("{:?}", c).contains("pq"));
    }
    #[test]
    fn no_is_the_default_and_keys_answer_or_move_the_focus() {
        let mut c = prompt();
        assert_eq!(c.key(28, false), Some(Answer::No), "Enter alone declines");
        assert_eq!(c.key(15, false), None);
        assert_eq!(c.focus, Answer::Yes);
        assert_eq!(c.key(28, false), Some(Answer::Yes));
        assert_eq!(c.key(21, true), Some(Answer::Yes), "Alt+Y");
        assert_eq!(c.key(49, true), Some(Answer::No), "Alt+N");
        assert_eq!(c.key(1, false), Some(Answer::No), "Esc");
        assert_eq!(c.key(30, false), None, "other keys do nothing");
    }
    #[test]
    fn buttons_answer_on_release_over_the_pressed_button_and_details_expand() {
        let area = Rect::new(0, 0, 1280, 720).unwrap();
        let mut c = prompt();
        let g = geometry(area, &c);
        assert_eq!(g.panel.width(), 456);
        assert_eq!(g.panel.height(), 114 + 178 + 80);
        let (yx, yy) = (g.yes.x() + 10, g.yes.y() + 10);
        assert_eq!(c.button(area, yx, yy, true), None);
        // Released elsewhere: nothing.
        assert_eq!(c.button(area, g.no.x() + 5, g.no.y() + 5, false), None);
        c.button(area, yx, yy, true);
        assert_eq!(c.button(area, yx, yy, false), Some(Answer::Yes));
        // Outside the dialog: swallowed.
        c.button(area, 5, 5, true);
        assert_eq!(c.button(area, 5, 5, false), None);
        let (dx, dy) = (g.details.x() + 4, g.details.y() + 4);
        c.button(area, dx, dy, true);
        c.button(area, dx, dy, false);
        assert!(c.details);
        assert_eq!(geometry(area, &c).panel.height(), 114 + 178 + 44 + 80);
        let close = geometry(area, &c).close;
        c.button(area, close.x() + 4, close.y() + 4, true);
        assert_eq!(
            c.button(area, close.x() + 4, close.y() + 4, false),
            Some(Answer::No)
        );
        // An unknown publisher asks the longer question in a taller header.
        let unknown = Consent::new(
            "setup",
            None,
            "C:\\setup.exe",
            Icon::App(App::Files),
            Then::Settings(0),
        );
        assert!(unknown.question().contains("unknown publisher"));
        assert_eq!(geometry(area, &unknown).panel.height(), 140 + 178 + 80);
    }
}
