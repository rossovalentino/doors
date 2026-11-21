//! Windows' "Open File - Security Warning" (the Attachment Manager's
//! prompt), which File Explorer shows before it runs a program that came
//! from the Internet: the file's `Zone.Identifier` stream names zone 3
//! (Internet) or 4 (Restricted sites). An unsigned program reads "The
//! publisher could not be verified. Are you sure you want to run this
//! software?" with Publisher "Unknown Publisher"; a verified one asks "Do you
//! want to run this file?" and names its publisher. Run starts it; Cancel,
//! Escape and the close button don't. Clearing "Always ask before opening
//! this file" removes the mark, so it runs without asking next time.
//!
//! The model, geometry, pointer and keyboard behaviour live here so painting
//! and hit testing share one layout; File Explorer paints it.
extern crate alloc;
use crate::explorer_layout::Bounds;
use alloc::{format, string::String};

/// Dialog size (logical px), as the classic dialog draws at 100%.
pub const WIDTH: u32 = 470;
pub const HEIGHT: u32 = 330;
pub const TITLE_BAR: u32 = 32;
pub const BUTTON: (u32, u32) = (88, 26);
pub const CAPTION: &str = "Open File - Security Warning";
pub const UNSIGNED: &str =
    "The publisher could not be verified. Are you sure you want to run this software?";
pub const SIGNED: &str = "Do you want to run this file?";
pub const UNKNOWN_PUBLISHER: &str = "Unknown Publisher";
pub const CHECK: &str = "Always ask before opening this file";
pub const NOTE_UNSIGNED: &str = "This file does not have a valid digital signature that verifies its publisher. You should only run software from publishers you trust.";
pub const NOTE_SIGNED: &str = "While files from the Internet can be useful, this file type can potentially harm your computer. Only run software from publishers you trust.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Run,
    Cancel,
    Check,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Run,
    Cancel,
    Check,
    Close,
    Panel,
    Outside,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    /// Run the program; `always_ask` false: remove its mark of the web.
    Run {
        always_ask: bool,
    },
}
pub struct Layout {
    pub panel: Bounds,
    pub caption: Bounds,
    pub close: Bounds,
    pub icon: Bounds,
    pub heading: Bounds,
    /// Name, Publisher, Type, From: (label, value).
    pub rows: [(Bounds, Bounds); 4],
    pub run: Bounds,
    pub cancel: Bounds,
    pub check: Bounds,
    pub rule: Bounds,
    pub note_icon: Bounds,
    pub note: Bounds,
}
pub struct Warning {
    /// The program to run (storage path).
    pub path: String,
    pub name: String,
    /// The verified Authenticode publisher; None: Unknown Publisher.
    pub publisher: Option<String>,
    pub kind: String,
    /// The folder, as Windows shows it (`C:\Users\...\Downloads`).
    pub from: String,
    pub always_ask: bool,
    pub focus: Focus,
    /// Keyboard focus is shown (after a key press), as in Windows.
    pub keyboard: bool,
}
impl Warning {
    pub fn new(path: &str, publisher: Option<String>, kind: &str, from: &str) -> Self {
        let name = path.rsplit('/').next().unwrap_or(path);
        Self {
            path: String::from(path),
            name: String::from(name),
            publisher,
            kind: String::from(kind),
            from: String::from(from),
            always_ask: true,
            focus: Focus::Run,
            keyboard: false,
        }
    }
    pub fn heading(&self) -> &'static str {
        if self.publisher.is_some() {
            SIGNED
        } else {
            UNSIGNED
        }
    }
    pub fn note(&self) -> &'static str {
        if self.publisher.is_some() {
            NOTE_SIGNED
        } else {
            NOTE_UNSIGNED
        }
    }
    /// (label, value) rows.
    pub fn rows(&self) -> [(&'static str, String); 4] {
        [
            ("Name:", self.display_name()),
            (
                "Publisher:",
                self.publisher
                    .clone()
                    .unwrap_or_else(|| String::from(UNKNOWN_PUBLISHER)),
            ),
            ("Type:", self.kind.clone()),
            ("From:", self.from.clone()),
        ]
    }
    fn display_name(&self) -> String {
        if self.from.is_empty() {
            self.name.clone()
        } else {
            format!("{}\\{}", self.from.trim_end_matches('\\'), self.name)
        }
    }
    /// Centered in a `width` x `height` window.
    pub fn layout(&self, width: u32, height: u32) -> Layout {
        let w = WIDTH.min(width.saturating_sub(16)).max(320);
        let h = HEIGHT;
        let x = (width as i32 - w as i32) / 2;
        let y = ((height as i32 - h as i32) / 2).max(0);
        let b = |dx: i32, dy: i32, bw: u32, bh: u32| Bounds {
            x: x + dx,
            y: y + dy,
            width: bw,
            height: bh,
        };
        let inner = w.saturating_sub(84);
        let rows = core::array::from_fn(|i| {
            let ry = 112 + i as i32 * 22;
            (b(64, ry, 72, 20), b(140, ry, w.saturating_sub(160), 20))
        });
        let cancel = b(w as i32 - 20 - BUTTON.0 as i32, 212, BUTTON.0, BUTTON.1);
        let run = b(cancel.x - x - 8 - BUTTON.0 as i32, 212, BUTTON.0, BUTTON.1);
        Layout {
            panel: b(0, 0, w, h),
            caption: b(12, 0, w.saturating_sub(70), TITLE_BAR),
            close: b(w as i32 - 46, 0, 46, TITLE_BAR),
            icon: b(20, 50, 32, 32),
            heading: b(64, 48, inner, 48),
            rows,
            run,
            cancel,
            check: b(20, 252, w.saturating_sub(40), 20),
            rule: b(12, 276, w.saturating_sub(24), 1),
            note_icon: b(20, 286, 24, 24),
            note: b(52, 284, w.saturating_sub(72), 40),
        }
    }
    pub fn hit(&self, l: &Layout, x: i32, y: i32) -> Hit {
        if l.run.contains(x, y) {
            Hit::Run
        } else if l.cancel.contains(x, y) {
            Hit::Cancel
        } else if l.close.contains(x, y) {
            Hit::Close
        } else if l.check.contains(x, y) {
            Hit::Check
        } else if l.panel.contains(x, y) {
            Hit::Panel
        } else {
            Hit::Outside
        }
    }
    pub fn click(&mut self, hit: Hit) -> Outcome {
        self.keyboard = false;
        match hit {
            Hit::Run => Outcome::Run {
                always_ask: self.always_ask,
            },
            Hit::Cancel | Hit::Close => Outcome::Cancel,
            Hit::Check => {
                self.always_ask = !self.always_ask;
                self.focus = Focus::Check;
                Outcome::Stay
            }
            Hit::Panel | Hit::Outside => Outcome::Stay,
        }
    }
    /// Tab/Shift+Tab move between Run, Cancel and the checkbox; Enter
    /// presses the focused button (Run on the checkbox); Space toggles the
    /// checkbox or presses a button; Alt+R runs, Escape cancels.
    pub fn key(&mut self, scan: u32, shift: bool, alt: bool) -> Outcome {
        const ESCAPE: u32 = 1;
        const TAB: u32 = 15;
        const ENTER: u32 = 28;
        const SPACE: u32 = 57;
        const R: u32 = 19;
        const A: u32 = 30;
        let order = [Focus::Run, Focus::Cancel, Focus::Check];
        match scan {
            ESCAPE => Outcome::Cancel,
            R if alt => Outcome::Run {
                always_ask: self.always_ask,
            },
            A if alt => {
                self.always_ask = !self.always_ask;
                self.focus = Focus::Check;
                self.keyboard = true;
                Outcome::Stay
            }
            TAB => {
                let at = order.iter().position(|f| *f == self.focus).unwrap_or(0);
                let next = if shift { at + order.len() - 1 } else { at + 1 } % order.len();
                self.focus = order[next];
                self.keyboard = true;
                Outcome::Stay
            }
            ENTER => match self.focus {
                Focus::Cancel => Outcome::Cancel,
                _ => Outcome::Run {
                    always_ask: self.always_ask,
                },
            },
            SPACE => match self.focus {
                Focus::Check => {
                    self.always_ask = !self.always_ask;
                    self.keyboard = true;
                    Outcome::Stay
                }
                Focus::Cancel => Outcome::Cancel,
                Focus::Run => Outcome::Run {
                    always_ask: self.always_ask,
                },
            },
            _ => Outcome::Stay,
        }
    }
}
/// Types the Attachment Manager asks about before running (programs,
/// installers, scripts, shortcuts to them).
pub fn guarded(name: &str) -> bool {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    [
        "exe", "com", "scr", "pif", "msi", "msp", "bat", "cmd", "vbs", "vbe", "js", "jse", "wsf",
        "ps1", "cpl", "hta", "lnk", "reg",
    ]
    .iter()
    .any(|e| ext.eq_ignore_ascii_case(e))
}
/// The zone a file's `Zone.Identifier` stream names, read through
/// `storage` for user `uid`; None when it has no mark.
pub fn zone(storage: &mut door_service_client::Storage, uid: u32, path: &str) -> Option<u8> {
    use door_service_client::{Disposition, READ, SHARE_ALL};
    use door_service_protocol::streams;
    let object = storage.metadata(path).ok()?.object;
    let mut buffer = [0u8; door_service_protocol::CHUNK_BYTES];
    let n = streams::path(uid, object, streams::ZONE_IDENTIFIER, &mut buffer)?;
    let stream = core::str::from_utf8(&buffer[..n]).ok()?;
    let file = storage
        .open(stream, Disposition::OpenExisting, READ, SHARE_ALL)
        .ok()?;
    let mut text = [0u8; 512];
    let read = storage.read_full(&file, 0, &mut text);
    let _ = storage.close(file);
    streams::zone(&text[..read.ok()?])
}
/// Remove a file's mark of the web (Unblock, or "Always ask" cleared).
pub fn unblock(storage: &mut door_service_client::Storage, uid: u32, path: &str) -> bool {
    use door_service_protocol::streams;
    let Ok(info) = storage.metadata(path) else {
        return false;
    };
    let mut buffer = [0u8; door_service_protocol::CHUNK_BYTES];
    let Some(n) = streams::path(uid, info.object, streams::ZONE_IDENTIFIER, &mut buffer) else {
        return false;
    };
    core::str::from_utf8(&buffer[..n])
        .ok()
        .is_some_and(|p| storage.remove(p).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_warning_asks_runs_cancels_and_clears_the_mark_as_windows_does() {
        let mut w = Warning::new(
            "/users/1001/Downloads/setup.exe",
            None,
            "Application",
            "C:\\Users\\1001\\Downloads",
        );
        assert_eq!(w.heading(), UNSIGNED);
        assert_eq!(w.rows()[1].1, UNKNOWN_PUBLISHER);
        assert_eq!(w.rows()[0].1, "C:\\Users\\1001\\Downloads\\setup.exe");
        let l = w.layout(1000, 700);
        assert!(l.panel.contains(l.run.x + 1, l.run.y + 1));
        assert_eq!(w.hit(&l, l.run.x + 4, l.run.y + 4), Hit::Run);
        assert_eq!(w.hit(&l, 0, 0), Hit::Outside);
        assert_eq!(w.click(Hit::Check), Outcome::Stay);
        assert!(!w.always_ask);
        assert_eq!(w.click(Hit::Run), Outcome::Run { always_ask: false });
        assert_eq!(w.key(1, false, false), Outcome::Cancel);
        // Tab order: Run, Cancel, the checkbox; Enter on Cancel cancels.
        let mut w = Warning::new("/a.exe", Some(String::from("Contoso")), "Application", "");
        assert_eq!(w.heading(), SIGNED);
        assert_eq!(w.key(15, false, false), Outcome::Stay);
        assert_eq!(w.focus, Focus::Cancel);
        assert_eq!(w.key(28, false, false), Outcome::Cancel);
        assert_eq!(w.key(15, true, false), Outcome::Stay);
        assert_eq!(w.focus, Focus::Run);
        assert_eq!(w.key(19, false, true), Outcome::Run { always_ask: true });
        assert!(guarded("Setup.EXE") && guarded("install.msi") && !guarded("notes.txt"));
    }
}
