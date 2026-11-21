//! "Open File - Security Warning" (`door_workspace::security_warning`):
//! before File Explorer runs a program, installer or script whose
//! `Zone.Identifier` stream marks it as downloaded from the Internet, it
//! asks, naming the verified publisher (or Unknown Publisher). Run starts
//! it as File Explorer always does; clearing "Always ask before opening this
//! file" removes the mark first.
use super::*;
use door_workspace::security_warning::{self as sw, Focus, Hit, Outcome, Warning};
/// Terminal's desktop command (`application::DESKTOP_COMMANDS`).
const TERMINAL_APP: u32 = door_service_protocol::application::DESKTOP_COMMANDS[3] as u32;

/// Hit-test ids of the dialog.
const RUN: u32 = 5200;
const CANCEL: u32 = RUN + 1;
const CHECK: u32 = RUN + 2;
const CLOSE: u32 = RUN + 3;
const PANEL: u32 = RUN + 4;

impl Files {
    /// This user's number (`/users/<uid>` is the home folder).
    fn own_uid(&self) -> u32 {
        self.home
            .strip_prefix("/users/")
            .and_then(|rest| rest.split('/').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    }
    /// Open a program: through the security warning when it came from the
    /// Internet, else at once.
    pub(super) fn run_checked(&mut self, storage: &mut Storage, path: &str) -> Result<(), Error> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let zone = if sw::guarded(name) {
            sw::zone(storage, self.own_uid(), path)
        } else {
            None
        };
        if zone.is_none_or(|z| z < door_service_protocol::streams::INTERNET) {
            return self.launch(storage, path);
        }
        let publisher = self.verified_publisher(storage, path);
        let folder = path
            .rsplit_once('/')
            .map_or("/", |(f, _)| if f.is_empty() { "/" } else { f });
        let warning = Warning::new(
            path,
            publisher,
            Self::program_kind(name),
            &door_workspace::explorer_path::display(folder),
        );
        let l = warning.layout(self.layout.width, self.layout.height);
        let rect = |r: Bounds| format!("{},{},{},{}", r.x, r.y, r.width, r.height);
        door_rt::diagnostic(format_args!(
            "DOOR_P16 FILES SECURITY WARNING path={path} zone={} publisher={} run={} cancel={} check={}\n",
            zone.unwrap_or(0),
            warning.publisher.as_deref().unwrap_or("Unknown"),
            rect(l.run),
            rect(l.cancel),
            rect(l.check)
        ));
        self.security = Some(warning);
        Ok(())
    }
    /// Start a program as Windows' shell does: one whose manifest asks for
    /// administrator rights (requireAdministrator, or highestAvailable for
    /// an administrator) goes through User Account Control; everything else
    /// starts at once.
    pub(super) fn launch(&mut self, storage: &mut Storage, path: &str) -> Result<(), Error> {
        let name = path.rsplit('/').next().unwrap_or(path);
        if super::is_program(name) && self.requires_elevation(storage, path) {
            door_rt::diagnostic(format_args!(
                "DOOR_P16 FILES ELEVATION REQUIRED path={path}\n"
            ));
            self.run_as_administrator(path, storage);
            return Ok(());
        }
        if super::is_program(name) && Self::console_program(storage, path) {
            // A console program opens in Terminal, as Windows 11 hands
            // console programs to Windows Terminal: a tab of its own.
            self.apps.document = Some((TERMINAL_APP, format!("run:{path}").into_bytes()));
            self.record_open(path, false);
            door_rt::diagnostic(format_args!("DOOR_P18 FILES CONSOLE path={path}\n"));
            return Ok(());
        }
        self.run_program(path)
    }
    /// A console program (PE subsystem 3, the Windows character subsystem).
    fn console_program(storage: &mut Storage, path: &str) -> bool {
        let Ok(file) = storage.open(path, Disposition::OpenExisting, READ, SHARE_ALL) else {
            return false;
        };
        let mut header = [0u8; 1024];
        let n = storage.read_full(&file, 0, &mut header).unwrap_or(0);
        let _ = storage.close(file);
        let header = &header[..n.min(header.len())];
        let word = |at: usize| {
            header
                .get(at..at + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        let Some(pe) = header
            .get(0x3c..0x40)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        else {
            return false;
        };
        // The optional header follows the 24-byte PE header; its Subsystem
        // field is at offset 68 in both PE32 and PE32+.
        header.get(pe..pe + 4) == Some(b"PE\0\0") && word(pe + 24 + 68) == Some(3)
    }
    fn requires_elevation(&self, storage: &mut Storage, path: &str) -> bool {
        let Ok(file) = storage.open(path, Disposition::OpenExisting, READ, SHARE_ALL) else {
            return false;
        };
        let bytes = (|| {
            let info = storage.info(&file).ok()?;
            if info.bytes == 0 || info.bytes > door_abi::platform::EXECUTABLE_MAX_BYTES as u64 {
                return None;
            }
            let mut data = alloc::vec![0u8; info.bytes as usize];
            let n = storage.read_full(&file, 0, &mut data).ok()?;
            (n == data.len()).then_some(data)
        })();
        let _ = storage.close(file);
        let Some(bytes) = bytes else {
            return false;
        };
        let Some(manifest) = door_image_format::PeImage::parse_with_delay_imports(&bytes)
            .ok()
            .and_then(|image| image.manifest())
        else {
            return false;
        };
        matches!(
            door_image_format::execution_level(manifest),
            door_image_format::ExecutionLevel::RequireAdministrator
                | door_image_format::ExecutionLevel::HighestAvailable
        )
    }
    fn program_kind(name: &str) -> &'static str {
        let ext = name
            .rsplit_once('.')
            .map_or("", |(_, e)| e)
            .to_ascii_lowercase();
        match ext.as_str() {
            "msi" => "Windows Installer Package",
            "msp" => "Windows Installer Patch",
            "bat" => "Windows Batch File",
            "cmd" => "Windows Command Script",
            "ps1" => "Windows PowerShell Script",
            "psm1" => "Windows PowerShell Script Module",
            "psd1" => "Windows PowerShell Data File",
            "ps1xml" => "Windows PowerShell XML Document",
            "vbs" | "vbe" | "js" | "jse" | "wsf" => "Script File",
            "reg" => "Registration Entries",
            "lnk" => "Shortcut",
            _ => "Application",
        }
    }
    /// The program's Authenticode publisher, when its signature verifies.
    fn verified_publisher(&self, storage: &mut Storage, path: &str) -> Option<String> {
        if !door_workspace::file_metadata::signable(path) {
            return None;
        }
        let file = storage
            .open(path, Disposition::OpenExisting, READ, SHARE_ALL)
            .ok()?;
        let bytes = (|| {
            let info = storage.info(&file).ok()?;
            if info.bytes == 0 || info.bytes > door_abi::platform::EXECUTABLE_MAX_BYTES as u64 {
                return None;
            }
            let mut data = alloc::vec![0u8; info.bytes as usize];
            let n = storage.read_full(&file, 0, &mut data).ok()?;
            (n == data.len()).then_some(data)
        })();
        let _ = storage.close(file);
        let now = door_sys::wall_time().map_or(0, |t| (t / 10_000_000) as i64 - 11_644_473_600);
        let store = door_workspace::file_metadata::root_store(storage);
        let anchors = door_workspace::file_metadata::anchors(&store);
        door_authenticode::verify(&bytes?, now, &anchors)
            .ok()
            .map(|s| s.publisher)
    }
    pub(super) fn security_open(&self) -> bool {
        self.security.is_some()
    }
    fn security_layout(&self) -> Option<sw::Layout> {
        let w = self.security.as_ref()?;
        Some(w.layout(self.layout.width, self.layout.height))
    }
    pub(super) fn security_target(&self, x: i32, y: i32) -> Option<u32> {
        let (w, l) = (self.security.as_ref()?, self.security_layout()?);
        match w.hit(&l, x, y) {
            Hit::Run => Some(RUN),
            Hit::Cancel => Some(CANCEL),
            Hit::Check => Some(CHECK),
            Hit::Close => Some(CLOSE),
            Hit::Panel => Some(PANEL),
            Hit::Outside => None,
        }
    }
    pub(super) fn security_click(&mut self, storage: &mut Storage, x: i32, y: i32) {
        let Some(l) = self.security_layout() else {
            return;
        };
        let Some(w) = self.security.as_mut() else {
            return;
        };
        let outcome = w.click(w.hit(&l, x, y));
        self.finish_security(storage, outcome);
    }
    pub(super) fn security_key(&mut self, storage: &mut Storage, code: u32) {
        let scan = code & 511;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let alt = (code >> 9) & door_abi::graphics::ALT != 0;
        let Some(w) = self.security.as_mut() else {
            return;
        };
        let outcome = w.key(scan, shift, alt);
        self.finish_security(storage, outcome);
    }
    fn finish_security(&mut self, storage: &mut Storage, outcome: Outcome) {
        match outcome {
            Outcome::Stay => (),
            Outcome::Cancel => {
                if let Some(w) = self.security.take() {
                    door_rt::diagnostic(format_args!(
                        "DOOR_P16 FILES SECURITY CANCEL path={}\n",
                        w.path
                    ));
                }
            }
            Outcome::Run { always_ask } => {
                let Some(w) = self.security.take() else {
                    return;
                };
                let unblocked = !always_ask && sw::unblock(storage, self.own_uid(), &w.path);
                door_rt::diagnostic(format_args!(
                    "DOOR_P16 FILES SECURITY RUN path={} unblocked={}\n",
                    w.path,
                    u8::from(unblocked)
                ));
                if let Err(error) = self.launch(storage, &w.path) {
                    self.message =
                        format!("Couldn't open {}: {}", w.name, Self::location_error(&error));
                }
            }
        }
    }

    // ---- Painting ----

    /// The shield with an exclamation mark the dialog shows.
    fn shield(v: &mut ui::View<'_>, x: i32, y: i32, size: u32) {
        let s = size as i32;
        let amber = [16, 160, 242, 255]; // BGRA #F2A010
        // A shield: square shoulders narrowing to a point.
        for row in 0..s {
            let inset = if row < s / 2 {
                0
            } else {
                (row - s / 2) * 9 / 10
            };
            let width = (s - 2 * inset).max(1) as u32;
            v.rect(x + inset, y + row, width, 1, amber);
        }
        let mark = size / 8;
        let cx = x + s / 2 - mark as i32 / 2;
        v.rect(cx, y + s / 5, mark, size * 2 / 5, [24, 24, 24, 255]);
        v.rect(cx, y + s * 13 / 20, mark, mark, [24, 24, 24, 255]);
    }
    pub(super) fn draw_security(&self, v: &mut ui::View<'_>) {
        use super::modern::paint::{ACCENT, MUTED, TEXT};
        let (Some(w), Some(l)) = (self.security.as_ref(), self.security_layout()) else {
            return;
        };
        let hover = v.hover;
        let over = |r: Bounds| hover.is_some_and(|(x, y)| r.contains(x, y));
        let focus = w.keyboard.then_some(w.focus);
        v.rounded_asset(0, 0, v.width, v.height, 0, [0, 0, 0, 80]);
        let b = l.panel;
        v.elevation(b.x, b.y + 8, b.width, b.height, 90);
        v.rect(b.x, b.y, b.width, b.height, [160, 160, 160, 255]);
        v.rect(b.x + 1, b.y + 1, b.width - 2, b.height - 2, [255; 4]);
        // Caption bar.
        v.text_clip_centered(
            l.caption.x,
            (l.caption.y, l.caption.height),
            l.caption.width,
            sw::CAPTION,
            TextStyle::Regular12,
            TEXT,
        );
        if over(l.close) {
            v.rect(
                l.close.x,
                l.close.y + 1,
                l.close.width - 1,
                l.close.height - 1,
                [35, 17, 196, 255],
            );
        }
        v.tiny_icon(
            l.close.x + (l.close.width as i32 - 12) / 2,
            l.close.y + (l.close.height as i32 - 12) / 2,
            Icon::Close,
            if over(l.close) { [255; 4] } else { TEXT },
        );
        Self::shield(v, l.icon.x, l.icon.y, l.icon.width);
        // Heading, wrapped.
        let style = TextStyle::Semibold14;
        let heading = w.heading();
        let lines =
            door_workspace::dialog::lines(heading, l.heading.width, |s| v.measure_style(s, style));
        for (i, line) in lines.iter().take(2).enumerate() {
            v.text_clip(
                l.heading.x,
                l.heading.y + i as i32 * 22,
                l.heading.width,
                &heading[line.clone()],
                style,
                [153, 51, 0, 255], // #003399, the dialog's heading blue
            );
        }
        for ((label, value), (lr, vr)) in w.rows().iter().zip(&l.rows) {
            v.text_clip(lr.x, lr.y, lr.width, label, TextStyle::Regular12, TEXT);
            v.text_clip(vr.x, vr.y, vr.width, value, TextStyle::Regular12, TEXT);
        }
        // Run (default) and Cancel.
        for (r, text, default, f) in [
            (l.run, "Run", true, Focus::Run),
            (l.cancel, "Cancel", false, Focus::Cancel),
        ] {
            let edge = if default {
                v.palette.foreground(ACCENT)
            } else {
                [173, 173, 173, 255]
            };
            v.rect(r.x, r.y, r.width, r.height, edge);
            let fill = if over(r) {
                [251, 241, 229, 255]
            } else {
                [225, 225, 225, 255]
            };
            v.rect(r.x + 1, r.y + 1, r.width - 2, r.height - 2, fill);
            if focus == Some(f) {
                v.rect(r.x + 3, r.y + 3, r.width - 6, 1, TEXT);
                v.rect(r.x + 3, r.y + r.height as i32 - 4, r.width - 6, 1, TEXT);
            }
            v.text_clip_centered(
                r.x + (r.width as i32 - v.measure_style(text, TextStyle::Regular12) as i32) / 2,
                (r.y, r.height),
                r.width,
                text,
                TextStyle::Regular12,
                TEXT,
            );
        }
        // Always ask.
        let c = l.check;
        let (bx, by) = (c.x, c.y + (c.height as i32 - 13) / 2);
        v.rect(bx, by, 13, 13, [51, 51, 51, 255]);
        v.rect(bx + 1, by + 1, 11, 11, [255; 4]);
        if w.always_ask {
            v.compact_icon(bx - 1, by - 1, Icon::Check, TEXT);
        }
        v.text_clip_centered(
            c.x + 20,
            (c.y, c.height),
            c.width.saturating_sub(20),
            sw::CHECK,
            TextStyle::Regular12,
            TEXT,
        );
        if focus == Some(Focus::Check) {
            let width = v.measure_style(sw::CHECK, TextStyle::Regular12);
            v.rect(c.x + 18, c.y + c.height as i32 - 2, width + 4, 1, TEXT);
        }
        // The note under its rule.
        v.rect(l.rule.x, l.rule.y, l.rule.width, 1, [223, 223, 223, 255]);
        Self::shield(v, l.note_icon.x, l.note_icon.y, 16);
        let note = w.note();
        let lines = door_workspace::dialog::lines(note, l.note.width, |s| {
            v.measure_style(s, TextStyle::Regular12)
        });
        for (i, line) in lines.iter().take(3).enumerate() {
            v.text_clip(
                l.note.x,
                l.note.y + i as i32 * 16,
                l.note.width,
                &note[line.clone()],
                TextStyle::Regular12,
                MUTED,
            );
        }
    }
    pub(super) fn security_semantics(&self, s: &mut door_workspace::semantics::Semantics) {
        use door_workspace::semantics::{Role, States};
        let (Some(w), Some(l)) = (self.security.as_ref(), self.security_layout()) else {
            return;
        };
        let b = |r: Bounds| (r.x, r.y, r.width, r.height);
        s.begin(Role::Dialog, sw::CAPTION, b(l.panel))
            .states(States::MODAL);
        s.control(Role::Button, "Run", b(l.run), Some(RUN))
            .focused(w.focus == Focus::Run);
        s.control(Role::Button, "Cancel", b(l.cancel), Some(CANCEL))
            .focused(w.focus == Focus::Cancel);
        s.control(Role::CheckBox, sw::CHECK, b(l.check), Some(CHECK))
            .state(States::CHECKED, w.always_ask)
            .focused(w.focus == Focus::Check);
        s.control(Role::Button, "Close", b(l.close), Some(CLOSE));
        s.end();
    }
}
