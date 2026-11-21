//! File Explorer's Open (default app), "Open with" and "Choose another app"
//! on the shared Default apps model (`door_workspace::default_apps`) and
//! picker (`door_workspace::open_with`). Opening a file hands it to its
//! default app: Notes and Preview through the window server
//! (OPEN_DOCUMENT, sent from `update`), a Windows program through Init with
//! the file as its argument. A type no app is set for opens the picker;
//! "Always" stores the choice in the user's registry, so Settings > Default
//! apps and every Explorer window see it at once.
use super::*;
use door_workspace::context_menu as cm;
use door_workspace::default_apps::{self as apps, Apps, Backend, Launch};
use door_workspace::open_with::{self as picker, Focus, Hit, Outcome, Picker};

/// Hit-test ids of the picker (rows, then the controls).
pub(super) const PICKER: u32 = 5000;
const PICKER_CHECK: u32 = PICKER + 100;
const PICKER_JUST_ONCE: u32 = PICKER + 101;
const PICKER_ALWAYS: u32 = PICKER + 102;
const PICKER_CLOSE: u32 = PICKER + 103;
const PICKER_PANEL: u32 = PICKER + 104;

pub(super) struct State {
    /// Apps that open files: the built-in apps, plus the installed programs
    /// once a storage connection lists them (window creation, refresh).
    pub apps: Apps,
    /// The user's stored choices (`.ext`, registration key), re-read on
    /// refresh like the time zone and region, and updated by every Open and
    /// "Always" here. Context menus are built from it (no storage there).
    pub choices: Vec<(String, String)>,
    /// Registration keys of the open "Open with" submenu, by row.
    pub menu: Vec<String>,
    pub picker: Option<Picker>,
    /// OPEN_DOCUMENT for the window server (app command, Door path).
    pub document: Option<(u32, Vec<u8>)>,
    /// Picker row last clicked and when (double-click opens).
    pub last_row: Option<(usize, u64)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            apps: Apps::builtin(),
            choices: Vec::new(),
            menu: Vec::new(),
            picker: None,
            document: None,
            last_row: None,
        }
    }
}

impl Files {
    /// A click's request the app loop has not yet sent (a document, a
    /// Settings page or an app to open). A close queued behind the click
    /// waits for it, as Windows handles the click before WM_CLOSE.
    pub fn has_pending_request(&self) -> bool {
        self.apps.document.is_some()
            || self.pending_settings.is_some()
            || self.pending_launch.is_some()
    }
    /// The document the window server should open next (`update`).
    pub fn take_document(&mut self) -> Option<(u32, Vec<u8>)> {
        self.apps.document.take()
    }
    /// The window server refused the OPEN_DOCUMENT.
    pub fn document_failed(&mut self, code: Code) {
        self.message = match code {
            Code::Closing => "Doors is shutting down. The file wasn't opened.".into(),
            _ => "The app couldn't open this file.".into(),
        };
    }
    /// The installed programs (window creation and F5; `refresh` re-reads
    /// the stored choices).
    pub(super) fn load_associations(&mut self, storage: &mut Storage) {
        self.apps.apps = Apps::load(storage);
    }
    /// F5 / Refresh: re-list the installed programs, then refresh.
    pub(super) fn user_refresh(&mut self, storage: &mut Storage) -> Result<(), Error> {
        self.load_associations(storage);
        self.refresh(storage)
    }
    pub(super) fn load_choices<B: Backend>(&mut self, b: &mut B) {
        // Unreadable choices leave the built-in defaults (and the last list).
        if let Ok(choices) = apps::user_choices(b) {
            self.apps.choices = choices;
        }
    }
    fn app_list(&self) -> &Apps {
        &self.apps.apps
    }
    fn remember_choice(&mut self, ext: &str, key: Option<String>) {
        self.apps.choices.retain(|(e, _)| e != ext);
        if let Some(key) = key {
            self.apps.choices.push((ext.into(), key));
        }
    }
    /// The current default of `ext` from the kept choices (menus).
    fn cached_default(&self, ext: &str) -> Option<String> {
        let user = self
            .apps
            .choices
            .iter()
            .find(|(e, _)| e == ext)
            .map(|(_, k)| k.as_str());
        apps::resolve(self.app_list(), ext, user).map(|d| d.0)
    }
    /// The current default of `ext`, read from the registry now (keeping
    /// the cache in step); the kept choice when the registry is unreadable.
    fn current_default<B: Backend>(&mut self, b: &mut B, ext: &str) -> Option<String> {
        match apps::user_choice(b, ext) {
            Ok(choice) => {
                self.remember_choice(ext, choice.clone());
                apps::resolve(self.app_list(), ext, choice.as_deref()).map(|d| d.0)
            }
            Err(_) => self.cached_default(ext),
        }
    }
    /// Double-click / Enter / Open on a file: its default app, or the
    /// picker for a type no app is set for.
    pub(super) fn open_file(&mut self, storage: &mut Storage, path: &str) {
        self.open_file_in(storage, path);
    }
    pub(super) fn open_file_in<B: Backend>(&mut self, b: &mut B, path: &str) {
        let Some(ext) = apps::extension_for(path) else {
            // No type: Windows asks which app to use, without "Always".
            self.show_picker(path, "", None);
            return;
        };
        match self.current_default(b, &ext) {
            Some(key) => {
                self.open_with_key(&key, path);
            }
            None => self.show_picker(path, &ext, None),
        }
    }
    /// Open `path` with the app `key` (no change to defaults). False when
    /// it could not start (the status bar says why).
    pub(super) fn open_with_key(&mut self, key: &str, path: &str) -> bool {
        let Some(app) = self.app_list().get(key).cloned() else {
            self.message = "That app isn't installed anymore.".into();
            return false;
        };
        match apps::launch(&app, path) {
            Ok(Launch::Document {
                app: command,
                payload,
            }) => {
                self.apps.document = Some((command, payload));
                self.record_open(path, false);
                door_rt::diagnostic(format_args!(
                    "DOOR_P14 FILES OPEN app={command} path={path}\n"
                ));
                true
            }
            Ok(Launch::Program {
                path: program,
                cwd,
                arguments,
            }) => match self.execute(&program, &cwd, &arguments) {
                Ok(()) => {
                    self.record_open(path, false);
                    door_rt::diagnostic(format_args!(
                        "DOOR_P14 FILES OPEN program={program} path={path}\n"
                    ));
                    true
                }
                Err(error) => {
                    self.message = format!(
                        "{} couldn't open the file. {}",
                        app.name,
                        Self::location_error(&error)
                    );
                    false
                }
            },
            Err(reason) => {
                self.message = reason.into();
                false
            }
        }
    }
    /// Init EXECUTE_DETACHED, as Explorer starts programs.
    fn execute(&mut self, program: &str, cwd: &str, arguments: &[u8]) -> Result<(), Error> {
        if self.registry == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        let mut applications = door_service_client::Applications::connect(self.registry)?;
        let child = applications.execute_detached(program, cwd, arguments, self.directory)?;
        let _ = door_sys::close(child);
        Ok(())
    }
    /// The "Open with" submenu of the file at `path` (its apps, the
    /// default first, then "Choose another app"), from the kept choices.
    /// The row keys are kept for `Action::OpenWith`.
    pub(super) fn open_with_entry(&mut self, path: &str) -> cm::Entry {
        let ext = apps::extension_for(path);
        let default = ext.as_deref().and_then(|e| self.cached_default(e));
        let list = ext.as_deref().map_or_else(Vec::new, |e| {
            self.app_list().open_with_menu(e, default.as_deref())
        });
        let rows: Vec<cm::OpenWithApp> = list
            .iter()
            .map(|a| cm::OpenWithApp {
                name: a.name.clone(),
                icon: a.icon,
                unavailable: a.unavailable(),
            })
            .collect();
        self.apps.menu = list.iter().map(|a| a.key.clone()).collect();
        cm::open_with(&rows)
    }
    /// A single file's menu gets "Open with" right after "Open" (a program
    /// runs on Open and has none; Recycle Bin items have no Open).
    pub(super) fn add_open_with(
        &mut self,
        menu: &mut cm::Menu,
        subject: cm::Subject,
        paths: &[String],
    ) {
        if subject != cm::Subject::File || self.recycle_view() {
            return;
        }
        let Some(path) = paths.first() else {
            return;
        };
        if is_program(path) {
            return;
        }
        let entry = self.open_with_entry(path);
        cm::insert_open_with(menu, entry);
        // 26H2: "Open" shows the app it opens with (its colour icon).
        let ext = apps::extension_for(path);
        let default = ext.as_deref().and_then(|e| self.cached_default(e));
        let icon = ext.as_deref().and_then(|e| {
            let list = self.app_list().open_with(e, default.as_deref());
            list.iter()
                .find(|a| default.as_deref() == Some(a.key.as_str()))
                .or(list.first())
                .map(|a| a.icon)
        });
        if let Some(icon) = icon
            && let Some(open) = menu
                .entries
                .iter_mut()
                .find(|e| e.action == cm::Action::Open)
        {
            open.glyph = cm::Glyph::App(icon);
        }
    }
    /// "Open with" > an app.
    pub(super) fn open_with_row(&mut self, row: u8, path: &str) {
        if let Some(key) = self.apps.menu.get(usize::from(row)).cloned() {
            self.open_with_key(&key, path);
        }
    }
    /// "Open with" > "Choose another app".
    pub(super) fn choose_app(&mut self, storage: &mut Storage, path: &str) {
        self.choose_app_in(storage, path);
    }
    pub(super) fn choose_app_in<B: Backend>(&mut self, b: &mut B, path: &str) {
        let ext = apps::extension_for(path).unwrap_or_default();
        let default = if ext.is_empty() {
            None
        } else {
            self.current_default(b, &ext)
        };
        self.show_picker(path, &ext, default);
    }
    fn show_picker(&mut self, path: &str, ext: &str, default: Option<String>) {
        self.dismiss_menu();
        // A type with no app yet starts with "Always use this app" checked.
        let always = default.is_none() && !ext.is_empty();
        let mut p = Picker::new(path, ext, self.app_list(), default.as_deref(), always);
        if ext.is_empty() {
            p.always = false;
        }
        self.apps.picker = Some(p);
        self.apps.last_row = None;
        door_rt::diagnostic(format_args!("DOOR_P14 FILES OPEN_WITH ext={ext}\n"));
    }
    pub(super) fn picker_open(&self) -> bool {
        self.apps.picker.is_some()
    }
    fn picker_layout(&self) -> Option<picker::Layout> {
        let p = self.apps.picker.as_ref()?;
        Some(p.layout(self.layout.width, self.layout.height))
    }
    pub(super) fn picker_target(&self, x: i32, y: i32) -> Option<u32> {
        let (p, l) = (self.apps.picker.as_ref()?, self.picker_layout()?);
        match p.hit(&l, x, y) {
            Hit::Row(i) => Some(PICKER + i.min(99) as u32),
            Hit::Check => Some(PICKER_CHECK),
            Hit::JustOnce => Some(PICKER_JUST_ONCE),
            Hit::Always => Some(PICKER_ALWAYS),
            Hit::Close => Some(PICKER_CLOSE),
            Hit::Panel => Some(PICKER_PANEL),
            Hit::Outside => None,
        }
    }
    pub(super) fn picker_click<B: Backend>(&mut self, b: &mut B, x: i32, y: i32) {
        let Some(l) = self.picker_layout() else {
            return;
        };
        let Some(p) = self.apps.picker.as_mut() else {
            return;
        };
        let hit = p.hit(&l, x, y);
        // Checking "Always use" needs a type to associate.
        if hit == Hit::Check && p.ext.is_empty() {
            return;
        }
        if hit == Hit::Always && p.ext.is_empty() {
            return;
        }
        let now = door_sys::time().unwrap_or(0);
        let double = match hit {
            Hit::Row(i) => {
                let twice = self
                    .apps
                    .last_row
                    .is_some_and(|(row, at)| row == i && now.saturating_sub(at) <= 50);
                self.apps.last_row = Some((i, now));
                twice
            }
            _ => false,
        };
        let outcome = p.click(hit, double);
        self.finish_picker(b, outcome);
    }
    pub(super) fn picker_key<B: Backend>(&mut self, b: &mut B, code: u32) {
        let scan = code & 511;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let alt = (code >> 9) & door_abi::graphics::ALT != 0;
        let Some(p) = self.apps.picker.as_mut() else {
            return;
        };
        let associable = !p.ext.is_empty();
        let mut outcome = p.key(scan, shift, alt);
        if !associable {
            // No type: the checkbox and Always stay off.
            p.always = false;
            if matches!(outcome, Outcome::Open { always: true, .. }) {
                outcome = Outcome::Stay;
            }
        }
        self.finish_picker(b, outcome);
    }
    pub(super) fn picker_wheel(&mut self, notches: i32) -> bool {
        match self.apps.picker.as_mut() {
            Some(p) => {
                let before = p.scroll;
                p.wheel(notches);
                p.scroll != before
            }
            None => false,
        }
    }
    fn finish_picker<B: Backend>(&mut self, b: &mut B, outcome: Outcome) {
        match outcome {
            Outcome::Stay => (),
            Outcome::Cancel => {
                self.apps.picker = None;
            }
            Outcome::Open { key, always } => {
                let Some(p) = self.apps.picker.take() else {
                    return;
                };
                if always && !p.ext.is_empty() {
                    if let Err(error) = apps::set_default(b, self.app_list(), &p.ext, &key) {
                        door_rt::diagnostic(format_args!(
                            "DOOR_P14 FILES DEFAULT_APP ext={} saved=0 error={error:?}\n",
                            p.ext
                        ));
                        self.open_with_key(&key, &p.path);
                        self.message = format!(
                            "The file opened, but your default app for {} files couldn't be saved.",
                            p.ext
                        );
                        return;
                    }
                    door_rt::diagnostic(format_args!(
                        "DOOR_P14 FILES DEFAULT_APP ext={} app={key} saved=1\n",
                        p.ext
                    ));
                    self.remember_choice(&p.ext, Some(key.clone()));
                }
                self.open_with_key(&key, &p.path);
            }
        }
    }

    // ---- Painting ----

    /// An app's icon (`shell::icon` id) at 16 or 32 logical px.
    pub(super) fn app_icon(v: &mut ui::View<'_>, x: i32, y: i32, size: u32, id: u16) {
        use door_service_protocol::shell::icon;
        let large = size >= 32;
        let scale = v.scale.clamp(1, 3);
        let bgra: Option<&[u8]> = match (id, large, scale) {
            (icon::TERMINAL, false, 1) => Some(door_workspace::asset!(
                "../../../../assets/icons/windows/terminal-current.16.bgra"
            )),
            (icon::TERMINAL, false, 2) | (icon::TERMINAL, true, 1) => Some(door_workspace::asset!(
                "../../../../assets/icons/windows/terminal-current.32.bgra"
            )),
            (icon::TERMINAL, false, _) => Some(door_workspace::asset!(
                "../../../../assets/icons/windows/terminal-current.48.bgra"
            )),
            (icon::TERMINAL, true, 2) => Some(door_workspace::asset!(
                "../../../../assets/icons/windows/terminal-current.64.bgra"
            )),
            (icon::TERMINAL, true, _) => Some(door_workspace::asset!(
                "../../../../assets/icons/windows/terminal-current.96.bgra"
            )),
            (icon::PREVIEW, false, 1) => Some(door_workspace::asset!(
                "../../../../assets/branding/preview.16.bgra"
            )),
            (icon::PREVIEW, false, 2) | (icon::PREVIEW, true, 1) => Some(door_workspace::asset!(
                "../../../../assets/branding/preview.32.bgra"
            )),
            (icon::PREVIEW, false, _) => Some(door_workspace::asset!(
                "../../../../assets/branding/preview.48.bgra"
            )),
            (icon::PREVIEW, true, 2) => Some(door_workspace::asset!(
                "../../../../assets/branding/preview.64.bgra"
            )),
            (icon::PREVIEW, true, _) => Some(door_workspace::asset!(
                "../../../../assets/branding/preview.96.bgra"
            )),
            _ => None,
        };
        let size = if large { 32 } else { 16 };
        if let Some(pixels) = bgra {
            v.color_pixels(x, y, size, pixels);
            return;
        }
        match id {
            icon::NOTES => v.sized_icon(x, y, size as u32, Icon::Notepad),
            icon::FILES => v.sized_icon(x, y, size as u32, Icon::Explorer),
            icon::SETTINGS => v.sized_icon(x, y, size as u32, Icon::Settings),
            // A program without bundled artwork: the generic window glyph.
            _ => {
                let at = (size as i32 - 16) / 2;
                v.mask_pixels(
                    x + at,
                    y + at,
                    16,
                    Icon::NewWindow.mask_scaled(16, v.scale),
                    super::modern::paint::TEXT,
                    None,
                );
            }
        }
    }
    fn picker_focus_ring(v: &mut ui::View<'_>, r: Bounds, radius: u32) {
        let c = if v.palette.dark {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        };
        v.rounded_asset(r.x - 3, r.y - 3, r.width + 6, r.height + 6, radius + 3, c);
        v.rounded(
            r.x - 1,
            r.y - 1,
            r.width + 2,
            r.height + 2,
            radius + 1,
            [255; 4],
        );
    }
    /// Windows 11's app picker over a dimmed window.
    pub(super) fn draw_picker(&self, v: &mut ui::View<'_>) {
        use super::modern::paint::{ACCENT, HOVER, MUTED, TEXT, label};
        let (Some(p), Some(l)) = (self.apps.picker.as_ref(), self.picker_layout()) else {
            return;
        };
        let associable = !p.ext.is_empty();
        let can_open = p.can_open();
        let focus = p.keyboard.then_some(p.focus);
        let hover = v.hover;
        let over = |r: Bounds| hover.is_some_and(|(x, y)| r.contains(x, y));
        v.rounded_asset(0, 0, v.width, v.height, 0, [0, 0, 0, 80]);
        let b = l.panel;
        v.elevation(b.x, b.y + 8, b.width, b.height, 90);
        v.rounded(b.x, b.y, b.width, b.height, 8, [200, 200, 200, 255]);
        v.rounded(
            b.x + 1,
            b.y + 1,
            b.width.saturating_sub(2),
            b.height.saturating_sub(2),
            7,
            [255; 4],
        );
        // Footer band with its top rule, as a ContentDialog draws it.
        let f = l.footer;
        v.rounded(
            f.x + 1,
            f.y,
            f.width.saturating_sub(2),
            f.height.saturating_sub(1),
            7,
            [243, 243, 243, 255],
        );
        v.rounded(
            f.x + 1,
            f.y,
            f.width.saturating_sub(2),
            8,
            0,
            [243, 243, 243, 255],
        );
        v.rect(
            f.x + 1,
            f.y,
            f.width.saturating_sub(2),
            1,
            [229, 229, 229, 255],
        );
        // Close button.
        if over(l.close) {
            v.rounded(
                l.close.x,
                l.close.y,
                l.close.width,
                l.close.height,
                4,
                HOVER,
            );
        }
        if focus == Some(Focus::Close) {
            Self::picker_focus_ring(v, l.close, 4);
        }
        v.tiny_icon(
            l.close.x + (l.close.width as i32 - 12) / 2,
            l.close.y + (l.close.height as i32 - 12) / 2,
            Icon::Close,
            TEXT,
        );
        // Title, wrapped into its block.
        let title = ui::display_ascii(&if associable {
            p.title()
        } else {
            String::from("Select an app to open this file")
        });
        let style = TextStyle::Semibold20;
        let lines =
            door_workspace::dialog::lines(&title, l.title.width, |s| v.measure_style(s, style));
        let line_h = (l.title.height / 2).max(1) as i32;
        for (i, line) in lines.iter().take(2).enumerate() {
            v.text_clip_centered(
                l.title.x,
                (l.title.y + i as i32 * line_h, line_h as u32),
                l.title.width,
                &title[line.clone()],
                style,
                TEXT,
            );
        }
        // App list.
        if p.entries.is_empty() {
            label(
                v,
                l.list.x + 12,
                l.list.y,
                l.list.width.saturating_sub(24),
                l.list.height,
                picker::NO_APPS,
                MUTED,
            );
        }
        for (i, (e, r)) in p.entries.iter().zip(&l.rows).enumerate() {
            let Some(r) = *r else { continue };
            let selected = p.selected == Some(i);
            let enabled = e.reason.is_none();
            if selected {
                v.rounded(r.x, r.y + 2, r.width, r.height - 4, 4, [237, 237, 237, 255]);
                v.rounded_asset(
                    r.x,
                    r.y + (r.height as i32 - 16) / 2,
                    3,
                    16,
                    2,
                    v.palette.foreground(ACCENT),
                );
            } else if enabled && over(r) {
                v.rounded(r.x, r.y + 2, r.width, r.height - 4, 4, [245, 245, 245, 255]);
            }
            if selected && focus == Some(Focus::List) {
                Self::picker_focus_ring(
                    v,
                    Bounds {
                        x: r.x,
                        y: r.y + 2,
                        width: r.width,
                        height: r.height - 4,
                    },
                    4,
                );
                v.rounded(r.x, r.y + 2, r.width, r.height - 4, 4, [237, 237, 237, 255]);
                v.rounded_asset(
                    r.x,
                    r.y + (r.height as i32 - 16) / 2,
                    3,
                    16,
                    2,
                    v.palette.foreground(ACCENT),
                );
            }
            Self::app_icon(v, r.x + 14, r.y + (r.height as i32 - 32) / 2, 32, e.icon);
            let color = if enabled { TEXT } else { [160, 160, 160, 255] };
            let text_x = r.x + 60;
            let text_w = r.width.saturating_sub(72);
            let name = ui::display_ascii(&e.name);
            if e.default {
                let half = r.height / 2;
                label(v, text_x, r.y + 3, text_w, half - 2, &name, color);
                v.text_clip_centered(
                    text_x,
                    (r.y + half as i32 - 1, half - 2),
                    text_w,
                    "Current default",
                    TextStyle::Regular12,
                    MUTED,
                );
            } else {
                label(v, text_x, r.y, text_w, r.height, &name, color);
            }
        }
        // "Always use this app to open .ext files".
        let c = l.check;
        let (bx, by) = (c.x, c.y + (c.height as i32 - 20) / 2);
        let box_on = p.always && associable;
        if box_on {
            v.rounded_asset(bx, by, 20, 20, 4, v.palette.foreground(ACCENT));
            v.compact_icon(bx + 2, by + 2, Icon::Check, [255; 4]);
        } else {
            let edge = if associable {
                [138, 138, 138, 255]
            } else {
                [200, 200, 200, 255]
            };
            v.rounded(bx, by, 20, 20, 4, edge);
            v.rounded(bx + 1, by + 1, 18, 18, 3, [250, 250, 250, 255]);
        }
        if focus == Some(Focus::Check) {
            Self::picker_focus_ring(
                v,
                Bounds {
                    x: bx,
                    y: by,
                    width: 20,
                    height: 20,
                },
                4,
            );
            if box_on {
                v.rounded_asset(bx, by, 20, 20, 4, v.palette.foreground(ACCENT));
                v.compact_icon(bx + 2, by + 2, Icon::Check, [255; 4]);
            } else {
                v.rounded(bx + 1, by + 1, 18, 18, 3, [250, 250, 250, 255]);
            }
        }
        let check_text = ui::display_ascii(&if associable {
            p.check_label()
        } else {
            String::from("Always use this app to open files of this type")
        });
        label(
            v,
            bx + 30,
            c.y,
            c.width.saturating_sub(30),
            c.height,
            &check_text,
            if associable {
                TEXT
            } else {
                [160, 160, 160, 255]
            },
        );
        // Footer buttons: the default button is the accent one.
        let default_button = p.default_button();
        for (r, text, which, enabled) in [
            (l.just_once, "Just once", Focus::JustOnce, can_open),
            (l.always, "Always", Focus::Always, can_open && associable),
        ] {
            let accent = which == default_button && enabled;
            let down = v.pressed.is_some_and(|(x, y)| r.contains(x, y));
            if accent {
                let fill = if down {
                    [160, 80, 0, 255]
                } else if over(r) {
                    [180, 94, 0, 255]
                } else {
                    ACCENT
                };
                v.rounded_asset(r.x, r.y, r.width, r.height, 4, v.palette.foreground(fill));
            } else {
                v.rounded(r.x, r.y, r.width, r.height, 4, [210, 210, 210, 255]);
                let fill = if !enabled {
                    [245, 245, 245, 255]
                } else if down {
                    [240, 240, 240, 255]
                } else if over(r) {
                    [246, 246, 246, 255]
                } else {
                    [251, 251, 251, 255]
                };
                v.rounded(
                    r.x + 1,
                    r.y + 1,
                    r.width.saturating_sub(2),
                    r.height.saturating_sub(2),
                    3,
                    fill,
                );
            }
            if focus == Some(which) && enabled {
                Self::picker_focus_ring(v, r, 4);
                if accent {
                    v.rounded_asset(r.x, r.y, r.width, r.height, 4, v.palette.foreground(ACCENT));
                } else {
                    v.rounded(r.x, r.y, r.width, r.height, 4, [251, 251, 251, 255]);
                }
            }
            let ink = if accent {
                [255; 4]
            } else if enabled {
                TEXT
            } else {
                [160, 160, 160, 255]
            };
            v.text_in_box(
                (r.x + 4, r.y, r.width.saturating_sub(8), r.height),
                text,
                TextStyle::Regular14,
                ink,
            );
        }
        // A disabled app explains itself under the pointer.
        if let Some((x, y)) = hover
            && let Some((e, r)) = p
                .entries
                .iter()
                .zip(&l.rows)
                .find(|(_, r)| r.is_some_and(|r| r.contains(x, y)))
            && let (Some(reason), Some(r)) = (e.reason, *r)
        {
            self.draw_tooltip(v, r, reason);
        }
    }
    /// door.accessibility: the picker as a modal dialog.
    pub(super) fn picker_semantics(&self, s: &mut door_workspace::semantics::Semantics) {
        use door_workspace::semantics::{Role, States};
        let (Some(p), Some(l)) = (self.apps.picker.as_ref(), self.picker_layout()) else {
            return;
        };
        let b = |r: Bounds| (r.x, r.y, r.width, r.height);
        let associable = !p.ext.is_empty();
        s.begin(Role::Dialog, &p.title(), b(l.panel))
            .states(States::MODAL);
        s.begin(Role::List, "Apps", b(l.list));
        for (i, (e, r)) in p.entries.iter().zip(&l.rows).enumerate() {
            let Some(r) = r else { continue };
            s.control(
                Role::ListItem,
                &e.name,
                b(*r),
                Some(PICKER + i.min(99) as u32),
            )
            .state(States::SELECTED, p.selected == Some(i))
            .state(States::DISABLED, e.reason.is_some())
            .focused(p.focus == Focus::List && p.selected == Some(i));
        }
        s.end();
        s.control(
            Role::CheckBox,
            &p.check_label(),
            b(l.check),
            Some(PICKER_CHECK),
        )
        .state(States::CHECKED, p.always && associable)
        .state(States::DISABLED, !associable)
        .focused(p.focus == Focus::Check);
        s.control(
            Role::Button,
            "Just once",
            b(l.just_once),
            Some(PICKER_JUST_ONCE),
        )
        .state(States::DISABLED, !p.can_open())
        .focused(p.focus == Focus::JustOnce);
        s.control(Role::Button, "Always", b(l.always), Some(PICKER_ALWAYS))
            .state(States::DISABLED, !p.can_open() || !associable)
            .focused(p.focus == Focus::Always);
        s.control(Role::Button, "Close", b(l.close), Some(PICKER_CLOSE))
            .focused(p.focus == Focus::Close);
        s.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use door_service_client::RegistryValue;
    /// The per-user hive in memory (lower-case key paths).
    #[derive(Default)]
    struct Hive(BTreeMap<String, BTreeMap<String, RegistryValue>>);
    impl Backend for Hive {
        type Key = String;
        fn open(&mut self, path: &str, _write: bool, create: bool) -> Result<String, Error> {
            let path = path.to_ascii_lowercase();
            if !self.0.contains_key(&path) && !create {
                return Err(Error::Service(Code::NotFound));
            }
            // Parents are created with the key, as the registry does.
            for (i, _) in path.match_indices('\\') {
                self.0.entry(path[..i].into()).or_default();
            }
            self.0.entry(path.clone()).or_default();
            Ok(path)
        }
        fn query(&mut self, key: &String, name: &str) -> Result<RegistryValue, Error> {
            self.0[key]
                .get(&name.to_ascii_lowercase())
                .copied()
                .ok_or(Error::Service(Code::NotFound))
        }
        fn set(&mut self, key: &String, name: &str, value: RegistryValue) -> Result<(), Error> {
            self.0
                .get_mut(key)
                .unwrap()
                .insert(name.to_ascii_lowercase(), value);
            Ok(())
        }
        fn children(&mut self, key: &String, max: u32) -> Result<Vec<String>, Error> {
            let prefix = format!("{key}\\");
            Ok(self
                .0
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter(|rest| !rest.contains('\\'))
                .take(max as usize)
                .map(String::from)
                .collect())
        }
        fn flush(&mut self) -> Result<(), Error> {
            Ok(())
        }
        fn close(&mut self, _key: String) -> Result<(), Error> {
            Ok(())
        }
    }
    fn explorer() -> Files {
        let mut f = Files::empty("/users/owner".into(), Controls::default());
        f.configure(1460, 900);
        let mut list = Apps::builtin();
        list.discover(["SETUP.EXE"].into_iter());
        f.apps.apps = list;
        f
    }
    const DOC: &str = "/users/owner/Projects/notes.txt";
    #[test]
    fn open_hands_a_file_to_its_default_app_or_asks_for_unknown_types() {
        let mut f = explorer();
        let mut hive = Hive::default();
        f.open_file_in(&mut hive, DOC);
        assert_eq!(f.take_document(), Some((2, DOC.as_bytes().to_vec())));
        assert!(!f.modal());
        f.open_file_in(&mut hive, "/users/owner/a.PNG");
        assert_eq!(f.take_document().map(|d| d.0), Some(10));
        // A batch file runs in Terminal (Windows runs it in cmd.exe).
        f.open_file_in(&mut hive, "/users/owner/run.bat");
        assert_eq!(f.take_document().map(|d| d.0), Some(9));
        // A PowerShell script runs in Terminal's PowerShell profile.
        f.open_file_in(&mut hive, "/users/owner/run.ps1");
        assert_eq!(f.take_document().map(|d| d.0), Some(9));
        // Other scripts open in Notes.
        f.open_file_in(&mut hive, "/users/owner/run.sh");
        assert_eq!(f.take_document().map(|d| d.0), Some(2));
        // An unknown type opens "Select an app to open this .xyz file".
        f.open_file_in(&mut hive, "/users/owner/data.xyz");
        assert!(f.take_document().is_none());
        assert!(f.modal() && f.picker_open());
        let p = f.apps.picker.as_ref().unwrap();
        assert_eq!(p.title(), "Select an app to open this .xyz file");
        assert!(p.always, "Always use starts checked for a type with no app");
        // Escape cancels.
        f.picker_key(&mut hive, 1);
        assert!(!f.picker_open());
    }
    #[test]
    fn always_persists_the_choice_and_explorer_uses_it_at_once() {
        let mut f = explorer();
        let mut hive = Hive::default();
        f.open_file_in(&mut hive, "/users/owner/data.xyz");
        // Enter: the first app (Notes), Always (the box is checked).
        f.picker_key(&mut hive, 28);
        assert!(!f.picker_open());
        assert_eq!(
            f.take_document(),
            Some((2, b"/users/owner/data.xyz".to_vec()))
        );
        assert_eq!(
            apps::user_choice(&mut hive, ".xyz").unwrap().as_deref(),
            Some("native:2")
        );
        // The next open goes straight to Notes.
        f.open_file_in(&mut hive, "/users/owner/other.XYZ");
        assert!(!f.picker_open());
        assert_eq!(f.take_document().map(|d| d.0), Some(2));
        // Choose another app > Preview, Just once: opens, changes nothing.
        f.choose_app_in(&mut hive, DOC);
        let p = f.apps.picker.as_ref().unwrap();
        assert!(!p.always);
        assert_eq!(p.entries[0].name, "Notes");
        assert!(p.entries[0].default);
        f.picker_key(&mut hive, 0x150);
        f.picker_key(&mut hive, 28);
        assert_eq!(f.take_document().map(|d| d.0), Some(10));
        assert_eq!(apps::user_choice(&mut hive, "txt").unwrap(), None);
        // Settings resets a type: Explorer follows the built-in default.
        apps::set_default(&mut hive, f.app_list(), "txt", "native:10").unwrap();
        f.open_file_in(&mut hive, DOC);
        assert_eq!(f.take_document().map(|d| d.0), Some(10));
        apps::reset_default(&mut hive, "txt").unwrap();
        f.open_file_in(&mut hive, DOC);
        assert_eq!(f.take_document().map(|d| d.0), Some(2));
    }
    #[test]
    fn programs_start_through_init_and_failures_explain_themselves() {
        let mut f = explorer();
        let mut hive = Hive::default();
        apps::set_default(&mut hive, f.app_list(), "txt", "exe:/system/bin/SETUP.EXE").unwrap();
        f.open_file_in(&mut hive, DOC);
        // Host windows have no launch handles: nothing starts, the status
        // bar says why, and no document is sent instead.
        assert!(f.take_document().is_none());
        assert!(
            f.message.starts_with("SETUP couldn't open the file."),
            "{}",
            f.message
        );
        // Terminal runs batch files: the file is handed to it.
        assert!(f.open_with_key("native:9", "/users/owner/run.bat"));
        assert_eq!(f.take_document().map(|d| d.0), Some(9));
        assert!(!f.open_with_key("exe:/system/bin/gone.exe", DOC));
        assert_eq!(f.message, "That app isn't installed anymore.");
    }
    #[test]
    fn the_open_with_submenu_lists_apps_default_first_and_opens_the_picked_one() {
        let mut f = explorer();
        let mut hive = Hive::default();
        let entry = f.open_with_entry("/users/owner/run.sh");
        let rows: Vec<(&str, bool)> = entry
            .children
            .iter()
            .filter(|e| !e.separator)
            .map(|e| (e.label.as_str(), e.enabled))
            .collect();
        assert_eq!(
            rows,
            [
                ("Preview", true),
                ("Notes", true),
                ("Choose another app", true)
            ]
        );
        // Preview leads; the default (Notes) follows.
        assert_eq!(f.apps.menu, ["native:10", "native:2"]);
        f.open_with_row(1, "/users/owner/run.sh");
        assert_eq!(f.take_document().map(|d| d.0), Some(2));
        // A user default moves to the top.
        apps::set_default(&mut hive, f.app_list(), "png", "exe:/system/bin/SETUP.EXE").unwrap();
        f.load_choices(&mut hive);
        f.open_with_entry("/users/owner/a.png");
        // The menu still leads with Preview; the user default follows it.
        assert_eq!(f.apps.menu, ["native:10", "exe:/system/bin/SETUP.EXE"]);
        // No type: only "Choose another app".
        let entry = f.open_with_entry("/users/owner/Makefile");
        assert_eq!(entry.children.len(), 1);
        assert_eq!(entry.children[0].action, cm::Action::ChooseApp);
    }
    #[test]
    fn the_picker_takes_pointer_clicks_and_a_file_without_a_type_never_associates() {
        let mut f = explorer();
        let mut hive = Hive::default();
        f.choose_app_in(&mut hive, DOC);
        let l = f.picker_layout().unwrap();
        let center = |b: Bounds| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
        let (x, y) = center(l.rows[1].unwrap());
        assert_eq!(f.picker_target(x, y), Some(PICKER + 1));
        f.picker_click(&mut hive, x, y);
        let (x, y) = center(l.always);
        assert_eq!(f.picker_target(x, y), Some(PICKER_ALWAYS));
        f.picker_click(&mut hive, x, y);
        assert_eq!(f.take_document().map(|d| d.0), Some(10));
        assert_eq!(
            apps::user_choice(&mut hive, "txt").unwrap().as_deref(),
            Some("native:10")
        );
        assert_eq!(f.picker_target(-5, -5), None);
        // A file without an extension: Just once only.
        f.open_file_in(&mut hive, "/users/owner/Makefile");
        assert!(f.picker_open());
        assert!(!f.apps.picker.as_ref().unwrap().always);
        let l = f.picker_layout().unwrap();
        let (x, y) = center(l.check);
        f.picker_click(&mut hive, x, y);
        assert!(
            !f.apps.picker.as_ref().unwrap().always,
            "nothing to associate"
        );
        let (x, y) = center(l.always);
        f.picker_click(&mut hive, x, y);
        assert!(f.picker_open(), "Always is disabled");
        f.picker_key(&mut hive, 30 | (door_abi::graphics::ALT << 9));
        f.picker_key(&mut hive, 28);
        assert_eq!(
            f.take_document(),
            Some((2, b"/users/owner/Makefile".to_vec()))
        );
        let (x, y) = center(l.close);
        f.open_file_in(&mut hive, "/users/owner/Makefile");
        f.picker_click(&mut hive, x, y);
        assert!(!f.picker_open());
    }
}
