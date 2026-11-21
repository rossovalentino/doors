//! Windows 11 26H2's modern Run dialog (Win+R, the WinUI 3 rebuild): one
//! rounded acrylic bar above the Start corner with the Run icon, a wide
//! input ("Type the name of a command to run") and a Run button; recent
//! commands and matching suggestions (with icons) list above it. There is
//! no Browse button. It opens Doors apps and inbox tools by their Windows
//! command names, folders and paths in File Explorer (`~\`, `C:\...`,
//! `%USERPROFILE%`, `shell:` folders, profile-relative names) and every
//! legacy Control Panel / `*.cpl` / `*.msc` / `ms-settings:` entry point in
//! the unified Settings (`door_settings_map`).
extern crate alloc;
use crate::apps::{App, Shell, Tool};
use crate::paint::{self, Palette};
#[cfg(test)]
use crate::search::page;
use crate::taskbar;
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    App(App),
    Tool(Tool),
    Settings(u32),
    /// File Explorer at the typed folder or path (`Run::accepted`).
    Path,
    /// The Snipping overlay (`ms-screenclip:`).
    Snip,
    /// A program Windows ships that Doors does not (`run_programs`); Run
    /// says so and stays open.
    Missing,
    /// A web address: Doors has no browser yet; Run says so.
    Web,
    /// A shell (cmd, powershell, wsl, doorsh): Terminal on a tab of that
    /// profile, as Windows 11 opens cmd.exe in Windows Terminal.
    Profile(Shell),
}
/// Windows command names, executables and URI schemes of what Doors ships.
/// Legacy Settings entry points (Control Panel, `*.cpl`, `*.msc`,
/// `regedit`, `msconfig`, `ms-settings:` URIs, ...) resolve through the
/// shared route table `door_settings_map` instead.
pub const ALIASES: &[(&str, Action)] = &[
    ("explorer", Action::App(App::Files)),
    ("explorer.exe", Action::App(App::Files)),
    ("file explorer", Action::App(App::Files)),
    ("notes", Action::App(App::Notes)),
    ("notepad", Action::App(App::Notes)),
    ("notepad.exe", Action::App(App::Notes)),
    ("write", Action::App(App::Notes)),
    ("wordpad", Action::App(App::Notes)),
    ("stikynot", Action::App(App::Notes)),
    ("sticky notes", Action::App(App::Notes)),
    ("wmplayer", Action::App(App::Preview)),
    ("windows media player", Action::App(App::Preview)),
    ("xpsrchvw", Action::App(App::Preview)),
    ("psr", Action::Tool(Tool::SnippingTool)),
    ("iexpress", Action::App(App::Files)),
    ("wscript", Action::Profile(Shell::CommandPrompt)),
    ("cscript", Action::Profile(Shell::CommandPrompt)),
    ("settings", Action::App(App::Settings)),
    ("terminal", Action::App(App::Terminal)),
    ("wt", Action::App(App::Terminal)),
    ("wt.exe", Action::App(App::Terminal)),
    ("cmd", Action::Profile(Shell::CommandPrompt)),
    ("cmd.exe", Action::Profile(Shell::CommandPrompt)),
    ("command prompt", Action::Profile(Shell::CommandPrompt)),
    ("doorsh", Action::Profile(Shell::Doorsh)),
    ("dsh", Action::Profile(Shell::Doorsh)),
    ("powershell", Action::Profile(Shell::PowerShell)),
    ("powershell.exe", Action::Profile(Shell::PowerShell)),
    ("powershell_ise", Action::Profile(Shell::PowerShell)),
    ("pwsh", Action::Profile(Shell::PowerShell)),
    ("pwsh.exe", Action::Profile(Shell::PowerShell)),
    ("dsl", Action::Profile(Shell::Dsl)),
    ("wsl.exe", Action::Profile(Shell::Dsl)),
    ("bash.exe", Action::Profile(Shell::Dsl)),
    ("preview", Action::App(App::Preview)),
    ("photos", Action::App(App::Preview)),
    ("ms-photos:", Action::App(App::Preview)),
    ("monitor", Action::App(App::Monitor)),
    ("taskmgr", Action::App(App::Monitor)),
    ("taskmgr.exe", Action::App(App::Monitor)),
    ("resmon", Action::App(App::Monitor)),
    ("calc", Action::Tool(Tool::Calculator)),
    ("calc.exe", Action::Tool(Tool::Calculator)),
    ("calculator", Action::Tool(Tool::Calculator)),
    ("ms-calculator:", Action::Tool(Tool::Calculator)),
    ("ms-clock:", Action::Tool(Tool::Clock)),
    ("mspaint", Action::Tool(Tool::Paint)),
    ("mspaint.exe", Action::Tool(Tool::Paint)),
    ("pbrush", Action::Tool(Tool::Paint)),
    ("pbrush.exe", Action::Tool(Tool::Paint)),
    ("ms-paint:", Action::Tool(Tool::Paint)),
    ("snippingtool", Action::Tool(Tool::SnippingTool)),
    ("snippingtool.exe", Action::Tool(Tool::SnippingTool)),
    ("ms-screensketch:", Action::Tool(Tool::SnippingTool)),
    ("ms-screenclip:", Action::Snip),
    ("msnweather:", Action::Tool(Tool::Weather)),
    ("bingweather:", Action::Tool(Tool::Weather)),
    ("magnify", Action::Tool(Tool::Magnifier)),
    ("magnify.exe", Action::Tool(Tool::Magnifier)),
    ("narrator", Action::Tool(Tool::Narrator)),
    ("narrator.exe", Action::Tool(Tool::Narrator)),
    ("osk", Action::Tool(Tool::OnScreenKeyboard)),
    ("osk.exe", Action::Tool(Tool::OnScreenKeyboard)),
    ("voiceaccess", Action::Tool(Tool::VoiceAccess)),
    ("voiceaccess.exe", Action::Tool(Tool::VoiceAccess)),
    ("livecaptions", Action::Tool(Tool::Captions)),
    ("livecaptions.exe", Action::Tool(Tool::Captions)),
    ("systemsettings", Action::App(App::Settings)),
    ("ms-settings", Action::App(App::Settings)),
    ("conhost", Action::App(App::Terminal)),
    ("windowsterminal", Action::App(App::Terminal)),
    ("bash", Action::Profile(Shell::Dsl)),
    ("wsl", Action::Profile(Shell::Dsl)),
    ("perfmon", Action::App(App::Monitor)),
    ("photoviewer", Action::App(App::Preview)),
    ("ms-screensketch", Action::Tool(Tool::SnippingTool)),
    ("screenclippinghost", Action::Snip),
];
/// Profile folders Run opens by name: it resolves relative names against
/// the user's profile, as Windows' Run does.
const PROFILE_FOLDERS: [&str; 6] = [
    "Desktop",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Videos",
];
fn eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}
fn prefix(text: &str, query: &str) -> bool {
    text.len() >= query.len()
        && text
            .bytes()
            .zip(query.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}
/// A folder or path File Explorer opens: `~` and `~\...`, a drive (`C:`,
/// `C:\...`), `%USERPROFILE%` / `%HOMEPATH%`, `.` and `..`, rooted or
/// relative paths with a separator, and `shell:` folders.
fn path_like(text: &str) -> bool {
    let b = text.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    text.starts_with('~')
        || drive
        || text.starts_with('%')
        || text == "."
        || text == ".."
        || text.contains(['\\', '/'])
        || prefix(text, "shell:")
        || PROFILE_FOLDERS.iter().any(|f| eq(f, text))
}
fn unquote(text: &str) -> &str {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text[1..text.len() - 1].trim()
    } else {
        text
    }
}
/// A command's program and its arguments (a quoted program may hold
/// spaces).
fn split_command(command: &str) -> (&str, &str) {
    if let Some(rest) = command.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        return (&rest[..end], rest[end + 1..].trim());
    }
    match command.split_once(char::is_whitespace) {
        Some((program, args)) => (program, args.trim()),
        None => (command, ""),
    }
}
fn strip_exe(name: &str) -> &str {
    match name.len().checked_sub(4) {
        Some(n) if name.is_char_boundary(n) && eq(&name[n..], ".exe") => &name[..n],
        _ => name,
    }
}
/// An alias, or an app or tool display name, exactly.
fn named(command: &str) -> Option<Action> {
    ALIASES
        .iter()
        .find(|(alias, _)| eq(alias, command))
        .map(|v| v.1)
        .or_else(|| {
            App::ALL
                .into_iter()
                .find(|app| eq(app.label(), command))
                .map(Action::App)
        })
        .or_else(|| {
            Tool::ALL
                .into_iter()
                .find(|tool| *tool != Tool::Gallery && eq(tool.label(), command))
                .map(Action::Tool)
        })
}
fn web(command: &str) -> bool {
    ["http://", "https://", "www.", "microsoft-edge:", "ftp://"]
        .iter()
        .any(|p| prefix(command, p))
}
/// What `explorer [path]` (or a bare path) opens in File Explorer: the
/// path itself.
pub fn path_text(command: &str) -> &str {
    let command = unquote(command);
    let (program, args) = split_command(command);
    if eq(strip_exe(program), "explorer") && !args.is_empty() {
        unquote(args)
    } else {
        command
    }
}
/// Resolve a typed command as Windows' Run does (case-insensitive,
/// surrounding spaces and quotes ignored): Doors apps and tools by their
/// Windows names (with or without `.exe` and arguments), legacy Settings
/// entry points, folders and paths, `explorer <path>`; web addresses and
/// the Windows programs Doors does not ship are recognised as such.
pub fn resolve(command: &str) -> Option<Action> {
    let command = unquote(command);
    if command.is_empty() {
        return None;
    }
    if let Some(action) = named(command).or_else(|| settings(command)) {
        return Some(action);
    }
    if web(command) {
        return Some(Action::Web);
    }
    // A program with arguments or ".exe" (its arguments are not passed on:
    // Doors apps open as they start), the Windows programs Doors does not
    // ship, then folders and paths.
    let (program, args) = split_command(command);
    let base = strip_exe(program);
    if eq(base, "explorer") && !args.is_empty() {
        return Some(Action::Path);
    }
    let windows = |name: &str| {
        let mut lower = String::from(name);
        lower.make_ascii_lowercase();
        crate::run_programs::subsystem(&lower).map(|_| Action::Missing)
    };
    named(base)
        .or_else(|| named(program))
        .or_else(|| settings(base))
        .or_else(|| windows(base))
        .or_else(|| path_like(command).then_some(Action::Path))
}
/// A legacy Settings entry point (`control`, `sysdm.cpl`, `devmgmt.msc`,
/// `ms-settings:display`, ...): Settings at that page and section, as the
/// SETTINGS_NAVIGATE argument.
pub fn settings(command: &str) -> Option<Action> {
    door_settings_map::resolve(command)
        .map(|(page, section)| Action::Settings(door_settings_map::navigate_arg(page, section)))
}
/// Prefix suggestions (text, action): apps and tools first, then command
/// names, profile folders and Settings entry points, without duplicates.
pub fn suggestions(text: &str) -> Vec<(&'static str, Action)> {
    let text = text.trim();
    let mut out: Vec<(&'static str, Action)> = Vec::new();
    if text.is_empty() {
        return out;
    }
    let mut push = |label: &'static str, action: Action| {
        if !out.iter().any(|(a, _)| eq(a, label)) {
            out.push((label, action));
        }
    };
    for app in App::ALL {
        if prefix(app.label(), text) {
            push(app.label(), Action::App(app));
        }
    }
    for tool in Tool::ALL {
        if tool != Tool::Gallery && prefix(tool.label(), text) {
            push(tool.label(), Action::Tool(tool));
        }
    }
    for (alias, action) in ALIASES {
        if prefix(alias, text) {
            push(alias, *action);
        }
    }
    for folder in PROFILE_FOLDERS {
        if prefix(folder, text) {
            push(folder, Action::Path);
        }
    }
    for r in door_settings_map::REDIRECTS {
        if prefix(r.key, text) {
            push(
                r.key,
                Action::Settings(door_settings_map::navigate_arg(r.page, r.section)),
            );
        }
    }
    out
}
pub fn action_label(action: Action) -> &'static str {
    match action {
        Action::App(app) => app.label(),
        Action::Tool(tool) => tool.label(),
        Action::Settings(arg) => match door_settings_map::decode_navigate(arg) {
            Some((_, Some(section))) => section.title,
            Some((page, None)) => page.nav_title(),
            None => "Settings",
        },
        Action::Path => "File Explorer",
        Action::Snip => "Snipping Tool",
        Action::Missing => "Windows program",
        Action::Web => "Web address",
        Action::Profile(shell) => shell.label(),
    }
}
/// Run input bound in UTF-8 bytes.
pub const MAX_TEXT: usize = 120;
/// Geometry (logical px), measured from Microsoft's published 100% capture:
/// a 500 x 62 bar 24 px from the work area's left edge and 16 px above the
/// taskbar, the 24 px icon 21 px in, the text 61 px in, and a 74 x 30 Run
/// button 16 px from the right.
const WIDTH: u32 = 500;
const BAR: i32 = 62;
const ROW: i32 = 40;
const LIST_PAD: i32 = 6;
const TEXT_X: i32 = 61;
const BUTTON: (u32, u32) = (74, 30);
const VISIBLE: usize = 4;
const EXPANDED: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Row(usize),
    Input,
    /// The Run button.
    Ok,
    Panel,
}
#[derive(Default)]
pub struct Run {
    pub open: bool,
    pub text: String,
    /// Byte caret in `text` (grapheme boundary).
    pub caret: usize,
    /// IME composition at the caret.
    pub preedit: crate::text_edit::Preedit,
    /// Session history, most recent first.
    pub history: Vec<String>,
    pub expanded: bool,
    pub selected: Option<usize>,
    pub error: Option<String>,
    /// The command the last `accept` ran (an `Action::Path` opens it).
    pub accepted: String,
}
/// List entry: a recent command or a suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    pub action: Option<Action>,
    /// From the history (drawn with the recent glyph).
    pub recent: bool,
}
impl Run {
    pub fn show(&mut self) {
        self.open = true;
        self.text.clear();
        self.caret = 0;
        self.preedit = Default::default();
        self.expanded = false;
        self.selected = None;
        self.error = None;
    }
    /// Recent commands while the input is empty, else matching suggestions.
    pub fn entries(&self) -> Vec<Entry> {
        let limit = if self.expanded { EXPANDED } else { VISIBLE };
        if self.text.trim().is_empty() {
            self.history
                .iter()
                .take(limit)
                .map(|h| Entry {
                    text: h.clone(),
                    action: resolve(h),
                    recent: true,
                })
                .collect()
        } else {
            suggestions(&self.text)
                .into_iter()
                .take(limit)
                .map(|(text, action)| Entry {
                    text: String::from(text),
                    action: Some(action),
                    recent: false,
                })
                .collect()
        }
    }
    /// List rows above the bar: the error (if any), then the entries.
    fn error_rows(&self) -> usize {
        usize::from(self.error.is_some())
    }
    pub fn rows(&self) -> usize {
        self.error_rows() + self.entries().len()
    }
    fn list_height(&self) -> i32 {
        match self.rows() {
            0 => 0,
            n => 2 * LIST_PAD + n as i32 * ROW + 1,
        }
    }
    pub fn panel(&self, layout: taskbar::Layout) -> Rect {
        let area = layout.work_area();
        let w = WIDTH.min(area.width().saturating_sub(48)).max(280);
        let h = (self.list_height() + BAR) as u32;
        let h = h.min(area.height().saturating_sub(32)).max(BAR as u32);
        let x = area.x() + 24;
        let y = if layout.controls().edge() == door_service_protocol::desktop::Edge::Top {
            area.y() + 16
        } else {
            area.bottom() - 16 - h as i32
        };
        Rect::new(x, y, w, h).unwrap()
    }
    /// List row `index` (0 is the error row when there is one).
    fn slot(&self, layout: taskbar::Layout, index: usize) -> Rect {
        let p = self.panel(layout);
        Rect::new(
            p.x() + LIST_PAD,
            p.y() + LIST_PAD + index as i32 * ROW,
            p.width().saturating_sub(2 * LIST_PAD as u32).max(1),
            ROW as u32,
        )
        .unwrap()
    }
    /// Entry `index`'s row.
    pub fn row(&self, layout: taskbar::Layout, index: usize) -> Rect {
        self.slot(layout, self.error_rows() + index)
    }
    /// The input bar (icon, text and Run button).
    pub fn input(&self, layout: taskbar::Layout) -> Rect {
        let p = self.panel(layout);
        Rect::new(p.x(), p.bottom() - BAR, p.width(), BAR as u32).unwrap()
    }
    pub fn button(&self, layout: taskbar::Layout, target: Target) -> Option<Rect> {
        if target != Target::Ok {
            return None;
        }
        let bar = self.input(layout);
        Rect::new(
            bar.right() - 16 - BUTTON.0 as i32,
            bar.y() + (BAR - BUTTON.1 as i32) / 2,
            BUTTON.0,
            BUTTON.1,
        )
        .ok()
    }
    pub fn hit(&self, layout: taskbar::Layout, x: i32, y: i32) -> Option<Target> {
        let p = self.panel(layout);
        if !self.open || !p.contains(x, y) {
            return None;
        }
        if self
            .button(layout, Target::Ok)
            .is_some_and(|r| r.contains(x, y))
        {
            return Some(Target::Ok);
        }
        if self.input(layout).contains(x, y) {
            return Some(Target::Input);
        }
        let n = self.entries().len();
        (0..n)
            .find(|i| self.row(layout, *i).contains(x, y))
            .map(Target::Row)
            .or(Some(Target::Panel))
    }
    /// Whether Run would run something (its button is enabled).
    pub fn ready(&self) -> bool {
        !self.text.trim().is_empty() || self.selected.is_some()
    }
    pub fn type_char(&mut self, ch: char) -> bool {
        let mut b = [0u8; 4];
        self.type_text(ch.encode_utf8(&mut b))
    }
    /// Key text or an IME commit at the caret (controls dropped), bounded
    /// in bytes.
    pub fn type_text(&mut self, text: &str) -> bool {
        if !crate::text_edit::insert(&mut self.text, &mut self.caret, text, |t| {
            t.len() <= MAX_TEXT
        }) {
            return false;
        }
        self.selected = None;
        self.error = None;
        true
    }
    /// Backspace: the grapheme cluster before the caret.
    pub fn backspace(&mut self) -> bool {
        self.selected = None;
        self.error = None;
        crate::text_edit::backspace(&mut self.text, &mut self.caret)
    }
    /// IME delete-surrounding around the caret (UTF-8 bytes).
    pub fn ime_delete(&mut self, before: usize, after: usize) -> bool {
        self.selected = None;
        crate::text_edit::delete_around(&mut self.text, &mut self.caret, before, after)
    }
    /// Left/Right/Home/End move the caret by cluster; Delete removes the
    /// cluster after it. False for any other key.
    pub fn edit_key(&mut self, scan: u32) -> bool {
        if scan == 0x153 {
            crate::text_edit::delete(&mut self.text, &mut self.caret);
            self.selected = None;
            return true;
        }
        crate::text_edit::move_caret(&self.text, &mut self.caret, scan)
    }
    /// Up/Down through the list (None returns to the input).
    pub fn step(&mut self, down: bool) {
        let n = self.entries().len();
        if n == 0 {
            self.selected = None;
            return;
        }
        // The list sits above the input: Up enters it from the bottom.
        self.selected = match (self.selected, down) {
            (None, false) => Some(n - 1),
            (None, true) => None,
            (Some(0), false) => Some(0),
            (Some(i), false) => Some(i - 1),
            (Some(i), true) if i + 1 < n => Some(i + 1),
            (Some(_), true) => None,
        };
    }
    /// Resolve what Enter or Run would run; records history and reports an
    /// error for unknown commands (the dialog stays open, as in Windows).
    pub fn accept(&mut self) -> Option<Action> {
        let text = match self.selected.and_then(|i| self.entries().get(i).cloned()) {
            Some(entry) => entry.text,
            None => String::from(self.text.trim()),
        };
        if text.is_empty() {
            return None;
        }
        match resolve(&text) {
            Some(Action::Web) => {
                self.error = Some(String::from(
                    "Doors has no web browser yet, so web addresses can't be opened.",
                ));
                None
            }
            Some(Action::Missing) => {
                let (program, _) = split_command(unquote(&text));
                let console = {
                    let mut lower = String::from(strip_exe(program));
                    lower.make_ascii_lowercase();
                    crate::run_programs::subsystem(&lower) == Some(3)
                };
                self.error = Some(if console {
                    alloc::format!(
                        "'{program}' is a Windows command-line tool that Doors doesn't include. Doors Terminal has its own commands."
                    )
                } else {
                    alloc::format!(
                        "'{program}' is a Windows program that Doors doesn't include yet."
                    )
                });
                None
            }
            Some(action) => {
                self.history.retain(|h| !eq(h, &text));
                self.history.insert(0, text.clone());
                self.history.truncate(EXPANDED);
                self.accepted = String::from(path_text(&text));
                self.open = false;
                Some(action)
            }
            None => {
                self.error = Some(alloc::format!(
                    "Doors can't find '{text}'. Make sure you typed the name correctly, and then try again."
                ));
                None
            }
        }
    }
}
/// A list row's leading icon (16 px).
fn entry_icon(c: &mut Canvas<'_>, x: i32, y: i32, entry: &Entry, palette: &Palette) {
    match entry.action {
        Some(Action::App(app)) => crate::apps::draw_icon(c, x, y, app, 16),
        Some(Action::Tool(tool)) => crate::apps::draw_tool_icon(c, x, y, tool, 16),
        Some(Action::Snip) => crate::apps::draw_tool_icon(c, x, y, Tool::SnippingTool, 16),
        Some(Action::Settings(_)) => crate::apps::draw_icon(c, x, y, App::Settings, 16),
        Some(Action::Path) => crate::apps::draw_icon(c, x, y, App::Files, 16),
        Some(Action::Missing | Action::Web) => crate::apps::draw_run_icon(c, x, y, 16),
        Some(Action::Profile(shell)) => crate::apps::draw_shell_icon(c, x, y, shell, 16),
        None => paint::glyph(
            c,
            x,
            y,
            16,
            crate::shell_mask!("shell-history", 16),
            palette.text_secondary,
        ),
    }
}
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    run: &Run,
    layout: taskbar::Layout,
    dark: bool,
    pointer: (i32, i32),
) {
    let palette = Palette::new(dark);
    let p = run.panel(layout);
    paint::shadow(c, p, 8, dark);
    // The acrylic flyout surface (opaque tint), as Start and the flyouts.
    paint::framed(c, p, 8, palette.surface_border, palette.surface);
    let hover = run.hit(layout, pointer.0, pointer.1);
    // Recent commands / suggestions above the bar, under the error.
    if let Some(error) = &run.error {
        let r = run.slot(layout, 0);
        paint::lcd::text_row(
            c,
            fonts,
            r.x() + 12,
            r.y(),
            r.height(),
            error,
            TextStyle::Regular12,
            if palette.contrast {
                palette.text
            } else if dark {
                paint::rgb(255, 153, 164)
            } else {
                paint::rgb(196, 43, 28)
            },
            r.width() - 24,
        );
    }
    let entries = run.entries();
    for (i, entry) in entries.iter().enumerate() {
        let r = run.row(layout, i);
        if run.selected == Some(i) || hover == Some(Target::Row(i)) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        if run.selected == Some(i) {
            paint::rounded(
                c,
                Rect::new(r.x(), r.y() + 12, 3, 16).unwrap(),
                1,
                palette.accent,
            );
        }
        entry_icon(c, r.x() + 12, r.y() + 12, entry, &palette);
        paint::lcd::text_row(
            c,
            fonts,
            r.x() + 40,
            r.y(),
            r.height(),
            &entry.text,
            TextStyle::Regular14,
            palette.text,
            r.width() / 2,
        );
        if entry.recent {
            paint::glyph(
                c,
                r.right() - 28,
                r.y() + 12,
                16,
                crate::shell_mask!("shell-history", 16),
                palette.text_secondary,
            );
        } else if let Some(action) = entry.action
            && !eq(action_label(action), &entry.text)
        {
            paint::lcd::text_right(
                c,
                fonts,
                r.right() - 12,
                r.y(),
                r.height(),
                action_label(action),
                TextStyle::Regular12,
                palette.text_secondary,
            );
        }
    }
    if run.rows() > 0 {
        let bar = run.input(layout);
        paint::rect(
            c,
            p.x() + 1,
            bar.y() - 1,
            p.width() - 2,
            1,
            palette.surface_divider,
        );
    }
    // The bar: Run icon, input and the Run button.
    let bar = run.input(layout);
    crate::apps::draw_run_icon(c, bar.x() + 21, bar.y() + (BAR - 24) / 2, 24);
    let text_x = bar.x() + TEXT_X;
    let button = run.button(layout, Target::Ok).unwrap();
    if run.text.is_empty() && run.preedit.text.is_empty() {
        paint::lcd::text_row(
            c,
            fonts,
            text_x,
            bar.y(),
            bar.height(),
            "Type the name of a command to run",
            TextStyle::Regular14,
            palette.text_secondary,
            (button.x() - 12 - text_x).max(1) as u32,
        );
    }
    // Unicode text with its caret and inline IME composition, scrolled to
    // keep the caret visible left of the button.
    let area = Rect::new(
        text_x,
        bar.y(),
        (button.x() - 12 - text_x).max(1) as u32,
        bar.height(),
    )
    .unwrap();
    let field = crate::text_edit::Field {
        text: &run.text,
        caret: run.caret,
        preedit: &run.preedit,
    };
    crate::text_edit::draw(
        c,
        fonts,
        area,
        &field,
        TextStyle::Regular14,
        palette.text,
        Some((bar.y() + (BAR - 20) / 2, 20)),
        true,
    );
    // Run (accent once there is something to run; disabled otherwise).
    let ready = run.ready();
    let (fill, fg) = if ready {
        let fill = if hover == Some(Target::Ok) && !palette.contrast {
            paint::rgb(
                palette.accent[2].saturating_add(16),
                palette.accent[1].saturating_add(16),
                palette.accent[0].saturating_add(16),
            )
        } else {
            palette.accent
        };
        (fill, palette.accent_text)
    } else {
        (palette.subtle_hover, palette.text_disabled)
    };
    paint::rounded(c, button, 4, fill);
    paint::lcd::text_row(
        c,
        fonts,
        button.x() + 17,
        button.y(),
        button.height(),
        "Run",
        TextStyle::Regular14,
        fg,
        40,
    );
    paint::glyph(
        c,
        button.right() - 13 - 16,
        button.y() + (BUTTON.1 as i32 - 16) / 2,
        16,
        crate::shell_mask!("shell-enter", 16),
        fg,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_accepts_any_key_text_but_controls_within_the_byte_bound() {
        let mut run = Run::default();
        run.show();
        for ch in ['é', '日', 'ß', ' ', '😀'] {
            assert!(run.type_char(ch), "{ch}");
        }
        assert_eq!(run.text, "é日ß 😀");
        assert!(!run.type_char('\n') && !run.type_char('\u{7f}'));
        while run.type_char('日') {}
        assert!(run.text.len() <= 120);
        assert!(run.backspace());
        assert!(
            run.text.ends_with('日'),
            "backspace removes a whole character"
        );
        // IME commits insert at the caret; Backspace takes whole clusters.
        run.show();
        assert!(run.type_text("ab"));
        assert!(run.edit_key(0x14b));
        assert!(run.type_text("拼音"));
        assert!(run.type_text("e\u{301}"));
        assert_eq!(run.text, "a拼音e\u{301}b");
        assert!(run.backspace());
        assert_eq!(run.text, "a拼音b");
        assert!(run.edit_key(0x147) && run.edit_key(0x153));
        assert_eq!((run.text.as_str(), run.caret), ("拼音b", 0));
        assert!(
            !run.edit_key(0x148),
            "Up is list navigation, not a caret key"
        );
    }
    #[test]
    fn commands_resolve_to_real_targets_and_unknown_ones_report_errors() {
        assert_eq!(resolve("notepad"), Some(Action::App(App::Notes)));
        assert_eq!(
            resolve("  CMD "),
            Some(Action::Profile(Shell::CommandPrompt))
        );
        assert_eq!(
            resolve("pwsh.exe -NoLogo"),
            Some(Action::Profile(Shell::PowerShell))
        );
        assert_eq!(resolve("wsl"), Some(Action::Profile(Shell::Dsl)));
        assert_eq!(resolve("Doorsh"), Some(Action::Profile(Shell::Doorsh)));
        assert_eq!(resolve("taskmgr"), Some(Action::App(App::Monitor)));
        assert_eq!(resolve("File Explorer"), Some(Action::App(App::Files)));
        assert_eq!(resolve("calc"), Some(Action::Tool(Tool::Calculator)));
        assert_eq!(resolve("\"mspaint.exe\""), Some(Action::Tool(Tool::Paint)));
        assert_eq!(resolve("osk"), Some(Action::Tool(Tool::OnScreenKeyboard)));
        assert_eq!(
            resolve("Snipping Tool"),
            Some(Action::Tool(Tool::SnippingTool))
        );
        assert_eq!(resolve("ms-screenclip:"), Some(Action::Snip));
        assert_eq!(resolve("Doors UI Gallery"), None, "not a Windows app");
        let page_of = |command: &str| match resolve(command) {
            Some(Action::Settings(arg)) => Some(arg & 0xffff),
            _ => None,
        };
        assert_eq!(page_of("ms-settings:display"), Some(page::DISPLAY));
        assert_eq!(page_of("devmgmt.msc"), Some(page::DEVICES_DRIVERS));
        assert_eq!(page_of("control"), Some(page::HOME));
        assert_eq!(
            page_of("Control Panel"),
            Some(page::HOME),
            "control, ignoring its argument"
        );
        assert!(page_of("sysdm.cpl").is_some());
        for command in [
            "regedit",
            "msconfig",
            "services.msc",
            "SystemPropertiesAdvanced",
            "appwiz.cpl",
            "ncpa.cpl",
            "winver",
        ] {
            assert!(page_of(command).is_some(), "{command}");
        }
        // Folders and paths open in File Explorer.
        for path in [
            "~",
            "~\\Documents",
            "C:\\",
            "c:/Users",
            "%USERPROFILE%\\Downloads",
            ".",
            "..",
            "Documents",
            "shell:Downloads",
            "Projects\\notes",
        ] {
            assert_eq!(resolve(path), Some(Action::Path), "{path}");
        }
        assert_eq!(resolve("frobnicate"), None);
        assert_eq!(resolve(""), None);
        for (_, action) in suggestions("ms-settings:") {
            if let Action::Settings(arg) = action {
                assert!(door_settings_map::decode_navigate(arg).is_some());
            }
        }
        let mut run = Run::default();
        run.show();
        assert!(!run.ready(), "Run is disabled while empty");
        for ch in "frobnicate".chars() {
            run.type_char(ch);
        }
        assert!(run.ready());
        assert_eq!(run.accept(), None);
        assert!(run.open && run.error.is_some());
        run.text.clear();
        run.error = None;
        for ch in "note".chars() {
            run.type_char(ch);
        }
        let s = run.entries();
        assert_eq!(s[0].action, Some(Action::App(App::Notes)));
        run.step(false);
        assert_eq!(run.selected, Some(s.len() - 1));
        run.selected = Some(0);
        assert_eq!(run.accept(), Some(Action::App(App::Notes)));
        assert!(!run.open);
        assert_eq!(run.history, ["Notes"]);
        run.show();
        let recent = run.entries();
        assert!(
            recent[0].text == "Notes" && recent[0].recent,
            "recent commands list"
        );
        run.text = "~\\Downloads".into();
        assert_eq!(run.accept(), Some(Action::Path));
        assert_eq!(run.accepted, "~\\Downloads");
    }
    #[test]
    fn every_windows_command_is_run_redirected_or_explained() {
        // Programs with ".exe", arguments or quotes.
        assert_eq!(
            resolve("NOTEPAD.EXE C:\\notes.txt"),
            Some(Action::App(App::Notes))
        );
        assert_eq!(
            resolve("\"calc.exe\""),
            Some(Action::Tool(Tool::Calculator))
        );
        assert_eq!(
            resolve("SystemSettings.exe"),
            Some(Action::App(App::Settings))
        );
        assert_eq!(
            resolve("cmd /k dir"),
            Some(Action::Profile(Shell::CommandPrompt))
        );
        assert_eq!(
            resolve("snippingtool /clip"),
            Some(Action::Tool(Tool::SnippingTool))
        );
        // Explorer with a path opens that path.
        assert_eq!(resolve("explorer C:\\Users"), Some(Action::Path));
        assert_eq!(
            path_text("explorer.exe \"C:\\Program Files\""),
            "C:\\Program Files"
        );
        assert_eq!(path_text("%temp%"), "%temp%");
        for path in [
            "%TEMP%",
            "%appdata%\\Microsoft",
            "shell:startup",
            "shell:sendto",
        ] {
            assert_eq!(resolve(path), Some(Action::Path), "{path}");
        }
        // Legacy entry points, with arguments, go to Settings.
        for command in [
            "control /name Microsoft.System",
            "control.exe",
            "rundll32 sysdm.cpl,EditEnvironmentVariables",
            "tpm.msc",
            "irprops.cpl",
            "comexp.msc",
        ] {
            assert!(
                matches!(resolve(command), Some(Action::Settings(_))),
                "{command}"
            );
        }
        // Every .msc and .cpl Windows ships resolves.
        for name in [
            "devmoderunasuserconfig.msc",
            "firewall.cpl",
            "tabletpc.cpl",
            "wf.msc",
            "wmimgmt.msc",
            "appwiz.cpl",
            "azman.msc",
            "bthprops.cpl",
            "certlm.msc",
            "certmgr.msc",
            "compmgmt.msc",
            "desk.cpl",
            "devmgmt.msc",
            "diskmgmt.msc",
            "eventvwr.msc",
            "fsmgmt.msc",
            "gpedit.msc",
            "hdwwiz.cpl",
            "inetcpl.cpl",
            "intl.cpl",
            "joy.cpl",
            "lusrmgr.msc",
            "main.cpl",
            "mmsys.cpl",
            "ncpa.cpl",
            "perfmon.msc",
            "powercfg.cpl",
            "printmanagement.msc",
            "rsop.msc",
            "secpol.msc",
            "services.msc",
            "sysdm.cpl",
            "taskschd.msc",
            "telephon.cpl",
            "timedate.cpl",
            "wscui.cpl",
        ] {
            assert!(resolve(name).is_some(), "{name}");
        }
        // Windows programs Doors lacks, and web addresses, are explained.
        assert!(
            matches!(resolve("charmap"), Some(Action::Settings(_))),
            "Character Map opens Fonts"
        );
        assert_eq!(resolve("ipconfig /all"), Some(Action::Missing));
        assert_eq!(resolve("https://example.com"), Some(Action::Web));
        let mut run = Run::default();
        run.show();
        run.text = "ipconfig".into();
        assert_eq!(run.accept(), None);
        assert!(
            run.open
                && run
                    .error
                    .as_deref()
                    .is_some_and(|e| e.contains("command-line"))
        );
        run.text = "bootsect.exe".into();
        assert_eq!(run.accept(), None);
        assert!(
            run.error
                .as_deref()
                .is_some_and(|e| e.contains("doesn't include") || e.contains("command-line")),
            "{:?}",
            run.error
        );
        run.text = "explorer shell:Downloads".into();
        assert_eq!(run.accept(), Some(Action::Path));
        assert_eq!(run.accepted, "shell:Downloads");
        assert_eq!(resolve("frobnicate"), None);
    }
    /// Writes the Run dialog (light, dark, typing) to DUMP_DIR as BMPs (run
    /// with --ignored).
    #[test]
    #[ignore]
    fn render_run() {
        extern crate std;
        let dir = std::env::var("DUMP_DIR").unwrap();
        let layout = taskbar::Layout::new(1280, 720);
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        for (name, dark, text) in [
            ("run-light", false, ""),
            ("run-dark", true, ""),
            ("run-typing", true, "c"),
            ("run-typing-light", false, "ms-se"),
        ] {
            let (w, h) = (640u32, 720u32);
            let mut px = alloc::vec![0u8; (w * h * 4) as usize];
            for p in px.chunks_exact_mut(4) {
                p.copy_from_slice(&if dark {
                    [60, 40, 30, 255]
                } else {
                    [230, 200, 170, 255]
                });
            }
            let mut c = Canvas::new(&mut px, w, h, w as usize * 4).unwrap();
            let mut run = Run::default();
            run.show();
            run.history = alloc::vec!["shell:startup".into(), "calc".into()];
            if !text.is_empty() {
                run.text = text.into();
                run.caret = text.len();
                run.selected = Some(0);
            } else if name == "run-light" {
                run.history.clear();
            }
            draw(&mut c, &fonts, &run, layout, dark, (0, 0));
            let mut out = std::vec::Vec::new();
            out.extend_from_slice(b"BM");
            out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&54u32.to_le_bytes());
            out.extend_from_slice(&40u32.to_le_bytes());
            out.extend_from_slice(&(w as i32).to_le_bytes());
            out.extend_from_slice(&(-(h as i32)).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&[0; 24]);
            out.extend_from_slice(&px);
            std::fs::write(std::format!("{dir}/{name}.bmp"), out).unwrap();
        }
    }
    #[test]
    fn run_bar_sits_above_the_start_corner_and_grows_upward() {
        let layout = taskbar::Layout::new(1280, 720);
        let mut run = Run::default();
        run.show();
        let bar = run.panel(layout);
        let area = layout.work_area();
        assert_eq!(area.intersection(bar), Some(bar));
        assert_eq!(
            (bar.x(), bar.width(), bar.height()),
            (area.x() + 24, 500, 62)
        );
        assert_eq!(bar.bottom(), area.bottom() - 16);
        let r = run.button(layout, Target::Ok).unwrap();
        assert_eq!(
            (r.width(), r.height(), r.right()),
            (74, 30, bar.right() - 16)
        );
        assert_eq!(run.hit(layout, r.x() + 2, r.y() + 2), Some(Target::Ok));
        assert_eq!(
            run.button(layout, Target::Panel),
            None,
            "no Cancel or Browse"
        );
        let input = run.input(layout);
        assert_eq!(
            run.hit(layout, input.x() + 80, input.y() + 2),
            Some(Target::Input)
        );
        assert_eq!(run.hit(layout, 1000, 10), None);
        // Suggestions list above the bar; the bar keeps its place.
        run.text = "c".into();
        let grown = run.panel(layout);
        assert!(grown.height() > bar.height());
        assert_eq!(
            grown.bottom(),
            bar.bottom(),
            "grows upward from the taskbar"
        );
        assert_eq!(run.input(layout), input);
        let first = run.row(layout, 0);
        assert_eq!(
            run.hit(layout, first.x() + 4, first.y() + 4),
            Some(Target::Row(0))
        );
    }
}
#[cfg(test)]
mod legacy_coverage {
    use super::*;
    /// Windows programs a user starts by name (Run, search, a shortcut):
    /// each opens the Doors app, tool, shell tab or Settings page replacing it.
    pub const USER_FACING: &[&str] = &[
        "notepad",
        "write",
        "wordpad",
        "mspaint",
        "calc",
        "snippingtool",
        "explorer",
        "control",
        "taskmgr",
        "resmon",
        "perfmon",
        "cmd",
        "powershell",
        "pwsh",
        "wsl",
        "bash",
        "wt",
        "regedit",
        "regedt32",
        "msconfig",
        "msinfo32",
        "winver",
        "dxdiag",
        "charmap",
        "mstsc",
        "msra",
        "eventvwr",
        "mmc",
        "cleanmgr",
        "dfrgui",
        "sndvol",
        "mblctr",
        "displayswitch",
        "magnify",
        "narrator",
        "osk",
        "netplwiz",
        "useraccountcontrolsettings",
        "optionalfeatures",
        "systempropertiesadvanced",
        "systempropertiescomputername",
        "systempropertiesperformance",
        "systempropertiesprotection",
        "systempropertiesremote",
        "systempropertieshardware",
        "rstrui",
        "recoverydrive",
        "mdsched",
        "msdt",
        "credwiz",
        "colorcpl",
        "cttune",
        "dpiscaling",
        "fsquirt",
        "sigverif",
        "verifier",
        "iexpress",
        "odbcad32",
        "eudcedit",
        "psr",
        "tabcal",
        "wmplayer",
        "xpsrchvw",
        "stikynot",
        "devicepairingwizard",
        "lpksetup",
        "printui",
        "rekeywiz",
        "wf",
        "wfs",
        "dialer",
        "fxscover",
        "msedge",
        "iexplore",
        "quickassist",
        "windowsupdate",
        "ms-settings:",
        "slui",
        "changepk",
        "computerdefaults",
        "fodhelper",
        "presentationsettings",
        "tpminit",
        "bitlockerwizard",
        "shrpubw",
        "wscript",
        "cscript",
        "mrt",
        "sdclt",
        "control printers",
        "control admintools",
        "gpedit.msc",
        "services.msc",
        "secpol.msc",
        "compmgmt.msc",
        "diskmgmt.msc",
        "devmgmt.msc",
        "lusrmgr.msc",
        "taskschd.msc",
        "certmgr.msc",
        "certlm.msc",
        "wf.msc",
        "firewall.cpl",
        "appwiz.cpl",
        "ncpa.cpl",
        "sysdm.cpl",
        "inetcpl.cpl",
        "powercfg.cpl",
        "timedate.cpl",
        "mmsys.cpl",
    ];
    #[test]
    fn user_facing_windows_programs_open_a_doors_destination() {
        let missing: Vec<&str> = USER_FACING
            .iter()
            .copied()
            .filter(|name| {
                !matches!(
                    resolve(name),
                    Some(
                        Action::App(_)
                            | Action::Tool(_)
                            | Action::Settings(_)
                            | Action::Profile(_)
                            | Action::Snip
                    )
                )
            })
            .collect();
        assert!(missing.is_empty(), "no Doors destination: {missing:?}");
    }
}
