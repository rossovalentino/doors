//! Windows 11's "Select an app to open this .ext file" picker (File
//! Explorer's Open with > Choose another app, and opening a type no app is
//! set for). The model, geometry, pointer and keyboard behaviour live here
//! so painting and hit testing share one layout; the app paints it with its
//! Fluent dialog surface and acts on the [`Outcome`].
//!
//! Layout: the title, the app list (the current default first and
//! selected, 44 px rows, scrolling past six), the "Always use this app to
//! open .ext files" checkbox, then the footer with "Just once" and
//! "Always". The checkbox picks the default button (Enter): Always when
//! checked, Just once otherwise; each button does what it says. Escape and
//! the close button cancel. Apps that cannot open a file are listed
//! disabled with their reason.
extern crate alloc;
use crate::default_apps::Apps;
use crate::explorer_layout::Bounds;
use alloc::{format, string::String, vec::Vec};
use door_graphics_text::TextStyle;

/// Panel width (logical px) and list geometry.
pub const WIDTH: u32 = 456;
pub const PAD: i32 = 24;
pub const ROW: i32 = 44;
pub const VISIBLE_ROWS: usize = 6;
pub const BUTTON_HEIGHT: i32 = 32;
pub const CLOSE: u32 = 32;
/// Shown when nothing on this PC can open the file.
pub const NO_APPS: &str = "No app on this PC can open this file.";

/// One app in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub name: String,
    /// `shell::icon` id.
    pub icon: u16,
    /// The current default for this type.
    pub default: bool,
    /// Why the app cannot open the file (the row is disabled), if so.
    pub reason: Option<&'static str>,
}
/// Keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Check,
    JustOnce,
    Always,
    Close,
}
const ORDER: [Focus; 5] = [
    Focus::List,
    Focus::Check,
    Focus::JustOnce,
    Focus::Always,
    Focus::Close,
];
/// What a pointer position is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    Check,
    JustOnce,
    Always,
    Close,
    /// Inside the panel, on no control.
    Panel,
    Outside,
}
/// What the dialog asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    /// Open the file with `key`; `always` also makes it the type's default.
    Open {
        key: String,
        always: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub panel: Bounds,
    pub close: Bounds,
    /// Title block (two lines reserved; the painter wraps the title).
    pub title: Bounds,
    pub list: Bounds,
    /// Row plate per entry; `None` for rows scrolled out of the list.
    pub rows: Vec<Option<Bounds>>,
    pub check: Bounds,
    pub footer: Bounds,
    pub just_once: Bounds,
    pub always: Bounds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    /// The file being opened (Door path) and its type (`.ext`).
    pub path: String,
    pub ext: String,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    /// "Always use this app to open .ext files".
    pub always: bool,
    pub focus: Focus,
    /// First visible row.
    pub scroll: usize,
    /// Focus is shown (keyboard use) rather than hidden (pointer use).
    pub keyboard: bool,
}
impl Picker {
    /// The picker for the file at `path` of type `ext`: `apps.choices` with
    /// the current `default` first and selected. `always` is the checkbox's
    /// initial state (checked when the type has no app yet, as Windows does
    /// when it asks how to open an unknown type).
    pub fn new(path: &str, ext: &str, apps: &Apps, default: Option<&str>, always: bool) -> Self {
        let entries: Vec<Entry> = apps
            .choices(ext, default)
            .into_iter()
            .map(|a| Entry {
                key: a.key.clone(),
                name: a.name.clone(),
                icon: a.icon,
                default: default.is_some_and(|d| apps.get(d).is_some_and(|x| x.key == a.key)),
                reason: a.unavailable(),
            })
            .collect();
        let selected = entries.iter().position(|e| e.reason.is_none());
        Self {
            path: path.into(),
            ext: ext.into(),
            entries,
            selected,
            always,
            focus: Focus::List,
            scroll: 0,
            keyboard: true,
        }
    }
    /// "Select an app to open this .txt file".
    pub fn title(&self) -> String {
        format!("Select an app to open this {} file", self.ext)
    }
    /// "Always use this app to open .txt files".
    pub fn check_label(&self) -> String {
        format!("Always use this app to open {} files", self.ext)
    }
    /// A selectable app is chosen, so the buttons are enabled.
    pub fn can_open(&self) -> bool {
        self.selected
            .and_then(|i| self.entries.get(i))
            .is_some_and(|e| e.reason.is_none())
    }
    /// The button Enter activates (drawn as the accent button).
    pub fn default_button(&self) -> Focus {
        if self.always {
            Focus::Always
        } else {
            Focus::JustOnce
        }
    }
    fn row() -> i32 {
        crate::palette::text_grow_i(ROW, TextStyle::Regular14)
    }
    pub fn layout(&self, width: u32, height: u32) -> Layout {
        let row = Self::row();
        let title_h = crate::palette::text_grow_i(64, TextStyle::Semibold20);
        let check_h = crate::palette::text_grow_i(32, TextStyle::Regular14);
        let button_h = crate::palette::text_grow_i(BUTTON_HEIGHT, TextStyle::Regular14);
        let footer_h = button_h + PAD * 2;
        let w = width.saturating_sub(32).clamp(1, WIDTH);
        let fixed = PAD + title_h + 12 + 12 + check_h + 16 + footer_h;
        // As many rows as fit (at most six, at least one).
        let room = (height as i32 - 32 - fixed).max(row);
        let shown = self
            .entries
            .len()
            .clamp(1, VISIBLE_ROWS)
            .min((room / row).max(1) as usize);
        let list_h = shown as i32 * row;
        let h = (fixed + list_h).min(height.max(1) as i32).max(1);
        let panel = Bounds {
            x: (width as i32 - w as i32) / 2,
            y: ((height as i32 - h) / 2).max(0),
            width: w,
            height: h as u32,
        };
        let inner = w.saturating_sub(PAD as u32 * 2);
        let x = panel.x + PAD;
        let close = Bounds {
            x: panel.x + w as i32 - CLOSE as i32 - 8,
            y: panel.y + 8,
            width: CLOSE,
            height: CLOSE,
        };
        let title = Bounds {
            x,
            y: panel.y + PAD,
            width: inner.saturating_sub(CLOSE),
            height: title_h as u32,
        };
        let list = Bounds {
            x: panel.x + PAD / 2,
            y: title.y + title_h + 12,
            width: w.saturating_sub(PAD as u32),
            height: list_h as u32,
        };
        let scroll = self.scroll.min(self.entries.len().saturating_sub(shown));
        let rows = (0..self.entries.len())
            .map(|i| {
                (scroll..scroll + shown).contains(&i).then(|| Bounds {
                    x: list.x,
                    y: list.y + (i - scroll) as i32 * row,
                    width: list.width,
                    height: row as u32,
                })
            })
            .collect();
        let check = Bounds {
            x,
            y: list.y + list_h + 12,
            width: inner,
            height: check_h as u32,
        };
        let footer = Bounds {
            x: panel.x,
            y: panel.y + h - footer_h,
            width: w,
            height: footer_h as u32,
        };
        let gap = 8;
        let bw = inner.saturating_sub(gap) / 2;
        let by = footer.y + PAD;
        Layout {
            panel,
            close,
            title,
            list,
            rows,
            check,
            footer,
            just_once: Bounds {
                x,
                y: by,
                width: bw,
                height: button_h as u32,
            },
            always: Bounds {
                x: x + (bw + gap) as i32,
                y: by,
                width: bw,
                height: button_h as u32,
            },
        }
    }
    pub fn hit(&self, l: &Layout, x: i32, y: i32) -> Hit {
        if !l.panel.contains(x, y) {
            return Hit::Outside;
        }
        if l.close.contains(x, y) {
            return Hit::Close;
        }
        if let Some(i) = l
            .rows
            .iter()
            .position(|r| r.is_some_and(|r| r.contains(x, y)))
        {
            return Hit::Row(i);
        }
        if l.check.contains(x, y) {
            Hit::Check
        } else if l.just_once.contains(x, y) {
            Hit::JustOnce
        } else if l.always.contains(x, y) {
            Hit::Always
        } else {
            Hit::Panel
        }
    }
    fn open(&self, always: bool) -> Outcome {
        match self.selected.and_then(|i| self.entries.get(i)) {
            Some(e) if e.reason.is_none() => Outcome::Open {
                key: e.key.clone(),
                always,
            },
            _ => Outcome::Stay,
        }
    }
    /// A click (`double`: the second click of a double click on a row opens
    /// with the default button, as Windows does).
    pub fn click(&mut self, hit: Hit, double: bool) -> Outcome {
        self.keyboard = false;
        match hit {
            Hit::Row(i) => {
                if self.entries.get(i).is_some_and(|e| e.reason.is_none()) {
                    self.selected = Some(i);
                    self.focus = Focus::List;
                    if double {
                        return self.open(self.always);
                    }
                }
                Outcome::Stay
            }
            Hit::Check => {
                self.always = !self.always;
                self.focus = Focus::Check;
                Outcome::Stay
            }
            Hit::JustOnce => self.open(false),
            Hit::Always => self.open(true),
            Hit::Close => Outcome::Cancel,
            Hit::Panel | Hit::Outside => Outcome::Stay,
        }
    }
    /// Move the selection by `delta` rows among enabled entries.
    fn step(&mut self, delta: isize) {
        let enabled: Vec<usize> = (0..self.entries.len())
            .filter(|i| self.entries[*i].reason.is_none())
            .collect();
        if enabled.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|s| enabled.iter().position(|i| *i == s))
            .unwrap_or(0) as isize;
        let next = (at + delta).clamp(0, enabled.len() as isize - 1) as usize;
        self.selected = Some(enabled[next]);
        self.reveal();
    }
    fn reveal(&mut self) {
        if let Some(s) = self.selected {
            if s < self.scroll {
                self.scroll = s;
            } else if s >= self.scroll + VISIBLE_ROWS {
                self.scroll = s + 1 - VISIBLE_ROWS;
            }
        }
    }
    /// Keyboard: Tab / Shift+Tab move focus; Up, Down, Home, End, Page Up
    /// and Page Down move the selection; Space toggles the checkbox or
    /// presses the focused button; Enter presses the focused button, or the
    /// default button from the list and the checkbox; Alt+A toggles the
    /// checkbox; Escape cancels.
    pub fn key(&mut self, scan: u32, shift: bool, alt: bool) -> Outcome {
        self.keyboard = true;
        match (alt, scan) {
            (_, 1) => return Outcome::Cancel,
            (true, 30) => {
                self.always = !self.always;
                return Outcome::Stay;
            }
            (true, _) => return Outcome::Stay,
            _ => (),
        }
        match scan {
            15 => {
                let at = ORDER.iter().position(|f| *f == self.focus).unwrap_or(0);
                let n = ORDER.len();
                self.focus = ORDER[(at + if shift { n - 1 } else { 1 }) % n];
                Outcome::Stay
            }
            0x148 if self.focus == Focus::List => {
                self.step(-1);
                Outcome::Stay
            }
            0x150 if self.focus == Focus::List => {
                self.step(1);
                Outcome::Stay
            }
            0x149 if self.focus == Focus::List => {
                self.step(-(VISIBLE_ROWS as isize));
                Outcome::Stay
            }
            0x151 if self.focus == Focus::List => {
                self.step(VISIBLE_ROWS as isize);
                Outcome::Stay
            }
            0x147 if self.focus == Focus::List => {
                self.step(-(self.entries.len() as isize));
                Outcome::Stay
            }
            0x14f if self.focus == Focus::List => {
                self.step(self.entries.len() as isize);
                Outcome::Stay
            }
            57 if self.focus == Focus::Check => {
                self.always = !self.always;
                Outcome::Stay
            }
            28 | 0x11c | 57 => match (self.focus, scan) {
                (Focus::JustOnce, _) => self.open(false),
                (Focus::Always, _) => self.open(true),
                (Focus::Close, _) => Outcome::Cancel,
                (_, 57) => Outcome::Stay,
                _ => self.open(self.always),
            },
            _ => Outcome::Stay,
        }
    }
    /// Mouse wheel over the list (`notches` > 0 scrolls up).
    pub fn wheel(&mut self, notches: i32) {
        let max = self.entries.len().saturating_sub(VISIBLE_ROWS);
        self.scroll = if notches > 0 {
            self.scroll.saturating_sub(notches as usize)
        } else {
            (self.scroll + notches.unsigned_abs() as usize).min(max)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_apps::tests;
    /// The table's apps with Terminal made unable to take documents: a
    /// disabled row for the picker's skip and reason tests.
    fn apps() -> Apps {
        let mut a = tests::apps();
        for app in &mut a.list {
            if app.key == "native:9"
                && let crate::default_apps::Target::Native { documents, .. } = &mut app.target
            {
                *documents = false;
            }
        }
        a
    }
    #[test]
    fn the_picker_lists_choices_with_the_default_selected_and_terminal_disabled() {
        let p = Picker::new("/u/run.bat", ".bat", &apps(), Some("native:2"), false);
        assert_eq!(p.title(), "Select an app to open this .bat file");
        assert_eq!(p.check_label(), "Always use this app to open .bat files");
        let names: Vec<&str> = p.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Notes", "Terminal", "Preview", "wingui-editor", "SETUP"]
        );
        assert!(p.entries[0].default && !p.entries[1].default);
        assert!(p.entries[1].reason.is_some());
        assert_eq!(p.selected, Some(0));
        assert!(p.can_open());
        assert_eq!(p.default_button(), Focus::JustOnce);
        // Unknown type: nothing is the default; the first app is selected and
        // the checkbox starts checked.
        let p = Picker::new("/u/a.xyz", ".xyz", &apps(), None, true);
        assert!(p.entries.iter().all(|e| !e.default));
        assert_eq!(p.selected, Some(0));
        assert_eq!(p.default_button(), Focus::Always);
        let none = Picker::new("/u/a.xyz", ".xyz", &Apps::default(), None, true);
        assert!(!none.can_open());
        assert_eq!(none.clone().key(28, false, false), Outcome::Stay);
    }
    #[test]
    fn keyboard_moves_selection_skips_disabled_rows_and_chooses_buttons() {
        let mut p = Picker::new("/u/run.bat", ".bat", &apps(), Some("native:2"), false);
        assert_eq!(p.key(0x150, false, false), Outcome::Stay);
        assert_eq!(p.selected, Some(2), "Terminal is skipped");
        p.key(0x148, false, false);
        assert_eq!(p.selected, Some(0));
        p.key(0x14f, false, false);
        assert_eq!(p.selected, Some(4));
        p.key(0x147, false, false);
        assert_eq!(p.selected, Some(0));
        // Enter from the list uses the default button (Just once).
        assert_eq!(
            p.key(28, false, false),
            Outcome::Open {
                key: "native:2".into(),
                always: false
            }
        );
        // Alt+A checks "Always use"; Enter now means Always.
        p.key(30, false, true);
        assert!(p.always);
        p.key(0x150, false, false);
        assert_eq!(
            p.key(0x11c, false, false),
            Outcome::Open {
                key: "native:10".into(),
                always: true
            }
        );
        // Tab order: list, checkbox, Just once, Always, close.
        p.key(15, false, false);
        assert_eq!(p.focus, Focus::Check);
        p.key(57, false, false);
        assert!(!p.always, "Space toggles the focused checkbox");
        p.key(15, false, false);
        assert_eq!(p.focus, Focus::JustOnce);
        assert_eq!(
            p.key(57, false, false),
            Outcome::Open {
                key: "native:10".into(),
                always: false
            }
        );
        p.key(15, false, false);
        assert_eq!(
            p.key(28, false, false),
            Outcome::Open {
                key: "native:10".into(),
                always: true
            },
            "Enter on Always always persists"
        );
        p.key(15, false, false);
        assert_eq!(p.focus, Focus::Close);
        assert_eq!(p.key(28, false, false), Outcome::Cancel);
        p.key(15, true, false);
        assert_eq!(p.focus, Focus::Always);
        assert_eq!(p.key(1, false, false), Outcome::Cancel, "Escape");
    }
    #[test]
    fn pointer_hits_share_the_layout_and_buttons_do_what_they_say() {
        let mut p = Picker::new("/u/a.txt", ".txt", &apps(), Some("native:2"), false);
        let l = p.layout(1280, 800);
        assert_eq!(l.panel.width, WIDTH);
        assert!(l.rows.iter().all(Option::is_some));
        let row = |l: &Layout, i: usize| l.rows[i].unwrap();
        let center = |b: Bounds| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
        let (x, y) = center(row(&l, 2));
        assert_eq!(p.hit(&l, x, y), Hit::Row(2));
        assert_eq!(p.click(Hit::Row(2), false), Outcome::Stay);
        assert_eq!(p.entries[2].name, "wingui-editor");
        assert_eq!(p.selected, Some(2));
        // Terminal is not offered for .txt; a disabled row would not select.
        let (x, y) = center(l.check);
        assert_eq!(p.hit(&l, x, y), Hit::Check);
        p.click(Hit::Check, false);
        assert!(p.always);
        let (x, y) = center(l.just_once);
        assert_eq!(p.hit(&l, x, y), Hit::JustOnce);
        assert_eq!(
            p.click(Hit::JustOnce, false),
            Outcome::Open {
                key: "exe:/system/bin/wingui-editor.exe".into(),
                always: false
            },
            "Just once never persists, even with the box checked"
        );
        let (x, y) = center(l.always);
        assert_eq!(p.hit(&l, x, y), Hit::Always);
        let (x, y) = center(l.close);
        assert_eq!(p.hit(&l, x, y), Hit::Close);
        assert_eq!(p.click(Hit::Close, false), Outcome::Cancel);
        assert_eq!(p.hit(&l, 2, 2), Hit::Outside);
        assert_eq!(
            p.click(Hit::Row(0), true),
            Outcome::Open {
                key: "native:2".into(),
                always: true
            },
            "double-click opens with the default button"
        );
        let mut b = Picker::new("/u/run.bat", ".bat", &apps(), Some("native:2"), false);
        assert_eq!(b.click(Hit::Row(1), true), Outcome::Stay);
        assert_eq!(b.selected, Some(0), "disabled Terminal row");
    }
    #[test]
    fn long_lists_scroll_and_small_windows_keep_controls_inside() {
        let mut apps = apps();
        let names: Vec<String> = (0..10).map(|i| format!("tool{i}.exe")).collect();
        apps.discover(names.iter().map(String::as_str));
        let mut p = Picker::new("/u/a.txt", ".txt", &apps, Some("native:2"), false);
        assert!(p.entries.len() > VISIBLE_ROWS);
        let l = p.layout(1280, 800);
        assert_eq!(l.rows.iter().filter(|r| r.is_some()).count(), VISIBLE_ROWS);
        p.key(0x14f, false, false);
        assert_eq!(p.scroll, p.entries.len() - VISIBLE_ROWS);
        let l = p.layout(1280, 800);
        assert!(l.rows.last().unwrap().is_some());
        p.wheel(3);
        assert_eq!(p.scroll, p.entries.len() - VISIBLE_ROWS - 3);
        p.wheel(-20);
        assert_eq!(p.scroll, p.entries.len() - VISIBLE_ROWS);
        for (w, h) in [(320, 240), (480, 360), (3840, 2160), (100, 80)] {
            let l = p.layout(w, h);
            for b in [l.close, l.just_once, l.always, l.check] {
                assert!(
                    b.x >= 0 && b.x as u32 + b.width <= w.max(1),
                    "{w}x{h} {b:?}"
                );
            }
            assert!(!l.just_once.contains(l.always.x, l.always.y));
            assert!(l.rows.iter().flatten().count() >= 1);
        }
    }
}
