//! Bounded terminal state. Foreign profiles never pass through the native parser.
//!
//! Scrollback keeps logical lines (hard line breaks), each at most
//! `MAX_LINE` bytes; a longer line continues in the next entry
//! (`Line::continued`). Rows wrap at the console width only when drawn, so a
//! width change re-wraps every line (soft-wrapped rows rejoin, hard breaks
//! stay) and `Tab::reflow` keeps the viewport on the same logical content.
//! Program output reaches the scrollback through `Ingest`, a bounded queue
//! drained a bounded amount per frame (see its policy).
extern crate alloc;
use crate::line_edit::LineEdit;
use crate::terminal_selection::Selection;
use alloc::{collections::VecDeque, string::String, vec, vec::Vec};
pub const MAX_LINES: usize = 512;
pub const MAX_LINE: usize = 2048;
/// Wrap width a tab assumes until its window reports one.
pub const DEFAULT_COLUMNS: usize = 80;
/// The dark console scheme's text area: Windows Terminal Campbell `#0C0C0C`.
pub const DARK_CONSOLE: [u8; 4] = door_design_tokens::color(
    door_design_tokens::ColorToken::SurfaceTerminal,
    door_design_tokens::Theme::Dark,
);
/// Windows Terminal's scrollbar: 16 px wide when shown, hidden at rest.
pub const SCROLLBAR: u32 = 16;
/// The new-tab split button ("+" and the profile drop-down): two 32 x 24
/// halves at y 8 in the 40 px tab row (64 x 24 together).
pub const NEW_TAB_Y: i32 = 8;
pub const NEW_TAB_HALF: u32 = 32;
pub const NEW_TAB_H: u32 = 24;
/// Program output waiting between the console mirror and the scrollback.
pub const PENDING_LIMIT: usize = 16 * 1024;
/// Program output moved into the scrollback per frame.
pub const INGEST_BYTES: usize = 8 * 1024;
/// PowerShell 7's program, where its installer puts it.
pub const PWSH: &str = "/compat/Program Files/PowerShell/7/pwsh.exe";
/// The same program as Windows names it (its command line's first word).
pub const PWSH_WINDOWS: &str = "C:\\Program Files\\PowerShell\\7\\pwsh.exe";
static POWERSHELL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Native,
    Cmd,
    PowerShell,
    Dsl,
}
impl Profile {
    pub const ALL: [Self; 4] = [Self::Native, Self::Cmd, Self::PowerShell, Self::Dsl];
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "Doorsh",
            Self::Cmd => "Command Prompt",
            Self::PowerShell => "PowerShell",
            Self::Dsl => "DSL",
        }
    }
    pub fn available(self) -> bool {
        match self {
            Self::Native | Self::Cmd => true,
            Self::PowerShell => POWERSHELL.load(core::sync::atomic::Ordering::Relaxed),
            Self::Dsl => false,
        }
    }
    /// PowerShell 7 is installed (`PWSH`): Terminal checks when it starts
    /// and before it opens a PowerShell tab, as Windows Terminal generates
    /// its PowerShell profile from an installed pwsh.exe.
    pub fn set_powershell_installed(installed: bool) {
        POWERSHELL.store(installed, core::sync::atomic::Ordering::Relaxed);
    }
    /// The tab hosts one program (pwsh.exe) and closes when it ends, as a
    /// Windows Terminal profile's tab does.
    pub fn hosted(self) -> bool {
        self == Self::PowerShell
    }
    pub fn status(self) -> &'static str {
        match self {
            Self::Native | Self::Cmd => "Available",
            Self::PowerShell if self.available() => "Available",
            Self::PowerShell => "Not installed",
            Self::Dsl => "Work in progress",
        }
    }
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Native => {
                "Doorsh, the Doors shell: pipelines, scripts (.dsh), structured commands and tools, with your own permissions only."
            }
            Self::Cmd => {
                "The Windows command interpreter (cmd.exe): batch files (.bat/.cmd), every internal command and the System32 console tools."
            }
            Self::PowerShell if self.available() => {
                "PowerShell 7 (pwsh.exe) with its .NET runtime: PSReadLine editing and history, tab completion, modules and .ps1 scripts."
            }
            Self::PowerShell => {
                "PowerShell 7 is not installed. Install it to C:\\Program Files\\PowerShell\\7 (Microsoft's PowerShell 7 package) and this profile runs it."
            }
            Self::Dsl => {
                "Doors Subsystem for Linux: work in progress. It will run the Linux distribution you install; a Linux execution environment and Linux system-call support are required first."
            }
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Page {
    /// The active tab's session (console or a profile's requirements).
    Console,
    /// Home: start a session, switch to an open one, scripts and keys.
    Home,
    Settings,
    About,
    Profiles,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Preferences {
    pub light_console: bool,
    pub font: u8,
    pub history: bool,
    pub opacity: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            light_console: false,
            font: 1,
            history: true,
            opacity: 85,
        }
    }
}
impl Preferences {
    pub fn encode(self) -> [u8; 8] {
        [
            b'D',
            b'T',
            b'R',
            b'M',
            2,
            self.light_console as u8,
            self.font,
            self.history as u8 | (((self.opacity.clamp(50, 100) - 50) / 5) << 1),
        ]
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 8 || &b[..4] != b"DTRM" || b[5] > 1 || b[6] > 2 {
            return None;
        }
        let (history, opacity) = match b[4] {
            1 if b[7] <= 1 => (b[7] == 1, 95),
            2 if b[7] >> 1 <= 10 => (b[7] & 1 != 0, 50 + (b[7] >> 1) * 5),
            _ => return None,
        };
        Some(Self {
            light_console: b[5] == 1,
            font: b[6],
            history,
            opacity,
        })
    }
    /// The opaque scheme background (BGRA): Windows Terminal's Campbell
    /// `#0C0C0C` (`color.surface.terminal`), or the light console scheme.
    pub fn scheme(self) -> [u8; 4] {
        if self.light_console {
            [252, 250, 247, 255]
        } else {
            DARK_CONSOLE
        }
    }
    /// One premultiplied layer; the compositor supplies the cached backdrop.
    pub fn background(self) -> [u8; 4] {
        let alpha = (u16::from(self.opacity.clamp(50, 100)) * 255 / 100) as u8;
        let rgb = self.scheme();
        let premultiply = |c: u8| ((u16::from(c) * u16::from(alpha) + 127) / 255) as u8;
        [
            premultiply(rgb[0]),
            premultiply(rgb[1]),
            premultiply(rgb[2]),
            alpha,
        ]
    }
    pub fn cell(self) -> (u32, u32) {
        match self.font {
            0 => (8, 18),
            2 => (12, 28),
            _ => (9, 21),
        }
    }
}
/// One scrollback entry: a logical line, or a `MAX_LINE` part of a longer
/// one (`continued`: the next entry follows it without a line break).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Line {
    text: String,
    cells: usize,
    continued: bool,
}
impl Line {
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Console cells: one per character, as the console grid paints them.
    pub fn cells(&self) -> usize {
        self.cells
    }
    pub fn continued(&self) -> bool {
        self.continued
    }
    /// Rows this entry occupies wrapped at `columns` (an empty line has one).
    pub fn rows(&self, columns: usize) -> usize {
        self.cells.max(1).div_ceil(columns.max(1))
    }
    /// Byte offset of `cell`, clamped to the end of the text.
    pub fn byte(&self, cell: usize) -> usize {
        if self.text.len() == self.cells {
            return cell.min(self.text.len());
        }
        self.text
            .char_indices()
            .nth(cell)
            .map_or(self.text.len(), |(at, _)| at)
    }
    /// The characters of cells `start..end`, clamped to the text.
    pub fn slice(&self, start: usize, end: usize) -> &str {
        if start >= end {
            return "";
        }
        let from = self.byte(start);
        &self.text[from..self.byte(end).max(from)]
    }
}
/// Tab identities (engines and programs follow a tab across reordering).
static NEXT_TAB: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
pub struct Tab {
    pub id: u64,
    /// The engine's prompt (cmd.exe's PROMPT); None: the directory and "> ".
    pub prompt: Option<String>,
    /// The engine is busy or waits for a key: no input line is shown.
    pub hide_input: bool,
    pub profile: Profile,
    /// The tab's title (top left): the profile's name until the shell sets
    /// its own (cmd.exe's TITLE, Doorsh's title); the window shows the
    /// active tab's title, as Windows Terminal does.
    pub title: String,
    /// An administrator session (User Account Control approved it).
    pub elevated: bool,
    pub cwd: String,
    pub input: LineEdit,
    pub lines: VecDeque<Line>,
    /// Rows scrolled back from the bottom, counted at the tab's wrap width.
    pub scroll: usize,
    /// The mouse selection (`terminal_selection`).
    pub selection: Option<Selection>,
    /// The console grid of the program running in this tab (`Console`).
    pub console: Option<alloc::boxed::Box<Console>>,
    /// The tab runs one program (pwsh.exe, or a console program Explorer
    /// handed over) and closes when it ends, as Windows Terminal closes a
    /// tab whose program exits; set when the program starts.
    pub hosted: bool,
    /// Serial number of `lines[0]`: selections address lines by serial, so
    /// they stay on their text while old lines leave the scrollback.
    first: u64,
    /// The last line is unterminated: streamed program output continues it.
    open: bool,
    /// Wrap width `scroll` is measured at (`reflow`).
    columns: usize,
    /// `(line serial, cell, scroll)`: the text `reflow` last kept at the
    /// bottom of a scrolled-back view, while `scroll` is unchanged, so
    /// repeated resizes return to the same view.
    anchor: Option<(u64, usize, usize)>,
    history: VecDeque<String>,
    history_at: usize,
    draft: String,
}
impl Tab {
    /// An administrator session: titled "Administrator: <profile>", as
    /// Windows does.
    pub fn elevate(&mut self) {
        if !self.elevated {
            self.elevated = true;
            self.title = alloc::format!("Administrator: {}", self.profile.name());
        }
    }
    pub fn new(profile: Profile, cwd: String) -> Self {
        let mut tab = Self {
            id: NEXT_TAB.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
            prompt: None,
            hide_input: false,
            profile,
            title: String::from(profile.name()),
            elevated: false,
            cwd,
            input: LineEdit::default(),
            lines: VecDeque::new(),
            scroll: 0,
            selection: None,
            console: None,
            hosted: false,
            first: 0,
            open: false,
            columns: DEFAULT_COLUMNS,
            anchor: None,
            history: VecDeque::new(),
            history_at: 0,
            draft: String::new(),
        };
        match profile {
            Profile::Native => {
                tab.output("Doorsh, the Doors shell\nType 'help' for available commands.\n")
            }
            Profile::Cmd => {
                let (a, b, c, d) = door_shells::WINDOWS_VERSION;
                tab.output(&alloc::format!(
                    "{} [Version {a}.{b}.{c}.{d}]\n(c) {}. All rights reserved.\n",
                    door_shells::PRODUCT,
                    door_shells::COMPANY
                ));
            }
            _ => (),
        }
        tab
    }
    /// Complete lines (prompts, command results): every `\n`-separated
    /// segment becomes a line, after ending an unterminated program line.
    /// Rows stay bounded even for one huge line, and control bytes are
    /// sanitized: escape sequences cannot operate the UI or inject input.
    pub fn output(&mut self, text: &str) {
        self.open = false;
        for line in text.split('\n') {
            self.new_line();
            self.append(line);
        }
        self.settle();
    }
    /// Program output: text continues the unterminated last line, and a
    /// final segment without `\n` stays open for the next chunk.
    pub fn stream(&mut self, text: &str) {
        for (index, segment) in text.split('\n').enumerate() {
            if index > 0 {
                if !self.open {
                    // A line break at a line start is an empty line.
                    self.new_line();
                }
                self.open = false;
            }
            if segment.chars().any(|c| c != '\r') {
                if !self.open {
                    self.new_line();
                    self.open = true;
                }
                self.append(segment);
            }
        }
        self.settle();
    }
    /// The program finished: its last line no longer continues.
    pub fn end_stream(&mut self) {
        self.open = false;
    }
    /// Clear the scrollback (Clear output, Ctrl+L, `clear`).
    pub fn clear(&mut self) {
        self.first += self.lines.len() as u64;
        self.lines.clear();
        self.scroll = 0;
        self.open = false;
        self.selection = None;
    }
    fn new_line(&mut self) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
            self.first += 1;
            if let Some(selection) = self.selection.as_mut()
                && !selection.retain_from(self.first)
            {
                self.selection = None;
            }
        }
        self.lines.push_back(Line::default());
        // Scrolled back, the view stays on its text as rows arrive below
        // (Windows Terminal does not snap to output, only to input).
        if self.scroll > 0 {
            self.scroll += 1;
        }
    }
    /// Append `text` to the last line, which must exist.
    fn append(&mut self, text: &str) {
        let columns = self.columns.max(1);
        for c in text.chars() {
            let c = match c {
                '\r' => continue,
                '\t' => ' ',
                c if c.is_control() => '?',
                c => c,
            };
            if self
                .lines
                .back()
                .is_some_and(|l| l.text.len() + c.len_utf8() > MAX_LINE)
            {
                if let Some(last) = self.lines.back_mut() {
                    last.continued = true;
                }
                self.new_line();
            }
            let Some(last) = self.lines.back_mut() else {
                return;
            };
            if self.scroll > 0 && last.cells > 0 && last.cells.is_multiple_of(columns) {
                self.scroll += 1;
            }
            last.text.push(c);
            last.cells += 1;
        }
    }
    /// Keep a scrolled-back view inside the (bounded) scrollback.
    fn settle(&mut self) {
        if self.scroll > 0 {
            let last = self.total_rows(self.columns).saturating_sub(1);
            self.scroll = self.scroll.min(last);
        }
    }
    /// Serial number of `lines[index]`.
    pub fn serial(&self, index: usize) -> u64 {
        self.first + index as u64
    }
    /// Index in `lines` of the line with `serial`, while it is retained.
    pub fn index_of(&self, serial: u64) -> Option<usize> {
        let index = usize::try_from(serial.checked_sub(self.first)?).ok()?;
        (index < self.lines.len()).then_some(index)
    }
    /// The wrap width `scroll` is measured at.
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn total_rows(&self, columns: usize) -> usize {
        self.lines.iter().map(|l| l.rows(columns)).sum()
    }
    /// `(line index, wrapped row within it)` of buffer row `row`.
    pub fn locate_row(&self, row: usize, columns: usize) -> Option<(usize, usize)> {
        let mut at = 0;
        for (index, line) in self.lines.iter().enumerate() {
            let rows = line.rows(columns);
            if row < at + rows {
                return Some((index, row - at));
            }
            at += rows;
        }
        None
    }
    /// Buffer row holding `cell` of `lines[line]` (cells past the text sit
    /// on the line's last row).
    pub fn row_index(&self, line: usize, cell: usize, columns: usize) -> usize {
        let columns = columns.max(1);
        let before: usize = self.lines.iter().take(line).map(|l| l.rows(columns)).sum();
        let rows = self.lines.get(line).map_or(1, |l| l.rows(columns));
        before + (cell / columns).min(rows - 1)
    }
    /// The text of wrapped row `wrap` of `lines[line]`.
    pub fn row_text(&self, line: usize, wrap: usize, columns: usize) -> &str {
        let columns = columns.max(1);
        self.lines
            .get(line)
            .map_or("", |l| l.slice(wrap * columns, (wrap + 1) * columns))
    }
    /// Buffer rows a console `rows` tall shows, `scroll` rows from the bottom.
    pub fn viewport(&self, columns: usize, rows: usize) -> core::ops::Range<usize> {
        let total = self.total_rows(columns);
        let end = total.saturating_sub(self.scroll.min(total.saturating_sub(1)));
        end.saturating_sub(rows)..end
    }
    /// `(line index, wrapped row)` of each buffer row in `range`.
    pub fn rows_in(&self, range: core::ops::Range<usize>, columns: usize) -> Vec<(usize, usize)> {
        let mut rows = Vec::with_capacity(range.len());
        let Some((mut line, mut wrap)) = self.locate_row(range.start, columns) else {
            return rows;
        };
        for _ in range {
            if line >= self.lines.len() {
                break;
            }
            rows.push((line, wrap));
            wrap += 1;
            if wrap >= self.lines[line].rows(columns) {
                line += 1;
                wrap = 0;
            }
        }
        rows
    }
    /// Re-wrap at `columns` (window resize or text size change). Soft-wrapped
    /// rows rejoin and hard line breaks stay because wrapping happens only
    /// when rows are drawn; the bottom visible row keeps showing the same
    /// text (the bottom stays the bottom). Selections address text, so they
    /// need no change.
    pub fn reflow(&mut self, columns: usize) {
        let columns = columns.max(1);
        if columns == self.columns {
            return;
        }
        let old = core::mem::replace(&mut self.columns, columns);
        if self.scroll == 0 {
            self.anchor = None;
            return;
        }
        let kept = self
            .anchor
            .filter(|&(_, _, scroll)| scroll == self.scroll)
            .and_then(|(serial, cell, _)| Some((self.index_of(serial)?, cell)));
        let (line, cell) = match kept {
            Some(anchor) => anchor,
            None => {
                let total = self.total_rows(old);
                let bottom = total.saturating_sub(1 + self.scroll.min(total.saturating_sub(1)));
                let Some((line, wrap)) = self.locate_row(bottom, old) else {
                    self.scroll = 0;
                    return;
                };
                // The last cell of that row: a row that now wraps in two
                // keeps all of its text in view.
                (
                    line,
                    ((wrap + 1) * old).min(self.lines[line].cells.max(1)) - 1,
                )
            }
        };
        let row = self.row_index(line, cell, columns);
        self.scroll = self.total_rows(columns).saturating_sub(1 + row);
        self.anchor = Some((self.serial(line), cell, self.scroll));
    }
    pub fn submit(&mut self, remember: bool) -> String {
        let line = String::from(self.input.text());
        self.input.set(String::new(), false);
        if remember && !line.trim().is_empty() && self.history.back() != Some(&line) {
            if self.history.len() == 64 {
                self.history.pop_front();
            }
            self.history.push_back(line.clone());
        }
        self.history_at = self.history.len();
        self.draft.clear();
        self.scroll = 0;
        line
    }
    pub fn history(&mut self, previous: bool) {
        if previous {
            if self.history_at == self.history.len() {
                self.draft = self.input.text().into();
            }
            self.history_at = self.history_at.saturating_sub(1);
        } else {
            self.history_at = (self.history_at + 1).min(self.history.len());
        }
        self.input.set(
            self.history
                .get(self.history_at)
                .unwrap_or(&self.draft)
                .clone(),
            false,
        );
    }
    /// Nothing was typed in this tab yet.
    pub fn history_is_empty(&self) -> bool {
        self.history.is_empty()
    }
    pub fn forget(&mut self) {
        self.history.clear();
        self.history_at = 0;
        self.draft.clear();
    }
    pub fn search(&mut self, query: &str) -> bool {
        self.search_columns(query, MAX_LINE)
    }
    pub fn visual_lines(&self, columns: usize) -> Vec<String> {
        let columns = columns.max(1);
        self.lines
            .iter()
            .flat_map(|line| {
                // Every script is kept; rows wrap at `columns` characters
                // (one cell each), never inside a character.
                (0..line.rows(columns))
                    .map(move |wrap| String::from(line.slice(wrap * columns, (wrap + 1) * columns)))
            })
            .collect()
    }
    pub fn scroll_rows(&mut self, previous: bool, amount: usize, columns: usize) {
        self.reflow(columns);
        let last = self.total_rows(columns).saturating_sub(1);
        self.scroll = self.scroll.min(last);
        self.scroll = if previous {
            self.scroll.saturating_add(amount).min(last)
        } else {
            self.scroll.saturating_sub(amount)
        };
    }
    pub fn search_columns(&mut self, query: &str, columns: usize) -> bool {
        let columns = columns.max(1);
        self.reflow(columns);
        if query.is_empty() {
            self.scroll = 0;
            return false;
        }
        if let Some(index) = self.lines.iter().rposition(|l| l.text.contains(query)) {
            let total = self.total_rows(columns);
            let before: usize = self.lines.iter().take(index).map(|l| l.rows(columns)).sum();
            let text = &self.lines[index].text;
            let matched_byte = text.find(query).unwrap_or(0);
            let matched_row = text[..matched_byte + query.len()]
                .chars()
                .count()
                .saturating_sub(1)
                / columns;
            self.scroll = total.saturating_sub(before + matched_row + 1);
            true
        } else {
            false
        }
    }
}

// ---- the console grid while a program runs ---------------------------------------------

/// A console program's screen while it runs (`door_vt`): its output drives
/// the grid as Windows Terminal's does. The grid starts with the
/// scrollback's last lines (taken out of the scrollback) and the cursor
/// below them; rows leaving the grid's top return to the scrollback as
/// lines, and the screen's rows follow them when the program ends.
pub struct Console {
    pub screen: door_vt::Screen,
    pub parser: door_vt::Parser,
}
impl Tab {
    /// A console program starts in this tab with a grid of this size;
    /// returns the row its cursor starts on.
    pub fn open_console(&mut self, columns: usize, rows: usize) -> usize {
        let (columns, rows) = (columns.max(1), rows.max(1));
        self.open = false;
        self.scroll = 0;
        self.selection = None;
        let mut screen = door_vt::Screen::new(columns, rows);
        // Whole lines from the end, as many as fit above the cursor's row.
        let (mut taken, mut used) = (0, 0);
        while taken < self.lines.len() {
            let n = self.lines[self.lines.len() - 1 - taken].rows(columns);
            if used + n > rows - 1 {
                break;
            }
            used += n;
            taken += 1;
        }
        let mut y = 0;
        for line in self.lines.range(self.lines.len() - taken..) {
            for wrap in 0..line.rows(columns) {
                screen.set_row(y, line.slice(wrap * columns, (wrap + 1) * columns));
                y += 1;
            }
        }
        for _ in 0..taken {
            self.lines.pop_back();
        }
        screen.place(0, y);
        self.console = Some(alloc::boxed::Box::new(Console {
            screen,
            parser: door_vt::Parser::new(),
        }));
        y
    }
    /// Program output: through the console's parser while one is open,
    /// else the text stream.
    pub fn feed(&mut self, bytes: &[u8]) {
        let Some(console) = self.console.as_mut() else {
            self.stream(&String::from_utf8_lossy(bytes));
            return;
        };
        console.parser.feed(&mut console.screen, bytes);
        // The console service answers reports (it follows the same output).
        console.screen.replies.clear();
        if let Some(title) = console.screen.title.take() {
            self.title = if self.elevated {
                alloc::format!("Administrator: {title}")
            } else {
                title
            };
        }
        self.take_scrolled();
    }
    /// Rows the grid scrolled out: lines of the scrollback.
    fn take_scrolled(&mut self) {
        let Some(console) = self.console.as_mut() else {
            return;
        };
        let rows = core::mem::take(&mut console.screen.scrolled);
        for row in rows {
            let end = row.iter().rposition(|c| c.ch != ' ').map_or(0, |i| i + 1);
            let text: String = row[..end].iter().map(|c| c.ch).collect();
            self.new_line();
            self.append(&text);
        }
        self.settle();
    }
    /// The grid follows the tab's size; true when it changed.
    pub fn resize_console(&mut self, columns: usize, rows: usize) -> bool {
        let Some(console) = self.console.as_mut() else {
            return false;
        };
        let screen = &mut console.screen;
        if screen.columns() == columns.max(1) && screen.rows() == rows.max(1) {
            return false;
        }
        screen.resize(columns, rows);
        self.take_scrolled();
        true
    }
    /// The program ended: the screen's rows up to the last one written (or
    /// the cursor's) join the scrollback, and the text stream resumes.
    pub fn close_console(&mut self) {
        let Some(console) = self.console.take() else {
            return;
        };
        let screen = &console.screen;
        let (_, cursor_row) = screen.cursor();
        let last_written = (0..screen.rows())
            .rev()
            .find(|&y| !screen.row_text(y).is_empty());
        // Rows through the last written one, or above the cursor's (an
        // empty row the cursor sits on after a final line break is the
        // break, not content).
        let end = match last_written {
            Some(y) => y + 1,
            None => cursor_row,
        };
        for y in 0..end {
            let text = screen.row_text(y);
            self.new_line();
            self.append(&text);
        }
        self.open = false;
        self.settle();
    }
}
/// Program output between the console mirror and the scrollback.
///
/// Backpressure policy: at most `PENDING_LIMIT` bytes wait here. Terminal
/// drains the mirror only while a whole mirror chunk fits (`room`), so a
/// flood stops at the console service, whose bounded mirror ring then gives
/// way oldest first (the kernel's console records keep the full
/// transcript); nothing here grows without bound. Each frame moves at most
/// `INGEST_BYTES` into the scrollback (`drain_into`), so input and painting
/// never wait behind output, and the scrollback keeps `MAX_LINES` entries.
#[derive(Default)]
pub struct Ingest {
    pending: Vec<u8>,
}
impl Ingest {
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
    /// Bytes that can still be queued.
    pub fn room(&self) -> usize {
        PENDING_LIMIT.saturating_sub(self.pending.len())
    }
    /// Queue mirrored bytes; refuses (false, nothing queued) what does not fit.
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.room() {
            return false;
        }
        self.pending.extend_from_slice(bytes);
        true
    }
    /// Stream at most `budget` bytes into `tab`, never splitting a UTF-8
    /// sequence unless `all` (the writer finished). Returns the bytes moved.
    pub fn drain_into(&mut self, tab: &mut Tab, budget: usize, all: bool) -> usize {
        let cut = utf8_cut(&self.pending, budget, all);
        if cut == 0 {
            return 0;
        }
        let bytes: Vec<u8> = self.pending.drain(..cut).collect();
        tab.feed(&bytes);
        cut
    }
}
/// Length of the longest prefix of `bytes` within `budget` that does not end
/// inside a UTF-8 sequence; with `all`, a whole buffer is taken as it is.
pub fn utf8_cut(bytes: &[u8], budget: usize, all: bool) -> usize {
    let cut = budget.min(bytes.len());
    if all && cut == bytes.len() {
        return cut;
    }
    let mut back = 0;
    while back < 3 && back < cut && bytes[cut - 1 - back] & 0xc0 == 0x80 {
        back += 1;
    }
    if back == cut {
        return cut;
    }
    let length = match bytes[cut - 1 - back] {
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => 1,
    };
    if length > back + 1 {
        cut - 1 - back
    } else {
        cut
    }
}
pub struct TerminalState {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub page: Page,
    pub preferences: Preferences,
    pub home: String,
    /// Console width in cells; every tab wraps at it (`set_columns`).
    columns: usize,
    /// The window runs with the full administrator token (Run as
    /// administrator): every tab is elevated and says so in its title.
    pub elevated: bool,
}
impl TerminalState {
    pub fn new(home: String) -> Self {
        Self {
            tabs: vec![Tab::new(Profile::Native, home.clone())],
            active: 0,
            page: Page::Console,
            preferences: Preferences::default(),
            home,
            columns: DEFAULT_COLUMNS,
            elevated: false,
        }
    }
    /// An administrator window: its tabs (the first one included) are
    /// elevated.
    pub fn elevate(&mut self) {
        self.elevated = true;
        for tab in &mut self.tabs {
            tab.elevate();
        }
    }
    /// The console width changed: reflow every tab.
    pub fn set_columns(&mut self, columns: usize) {
        self.columns = columns.max(1);
        for tab in &mut self.tabs {
            tab.reflow(self.columns);
        }
    }
    fn tab_for(&self, profile: Profile) -> Tab {
        let mut tab = Tab::new(profile, self.home.clone());
        tab.reflow(self.columns);
        if self.elevated {
            tab.elevate();
        }
        tab
    }
    /// A new tab of `profile`, made active (any number: the strip scrolls,
    /// as Windows Terminal's does). True once added.
    pub fn new_tab(&mut self, profile: Profile) -> bool {
        let tab = self.tab_for(profile);
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.page = Page::Console;
        true
    }
    pub fn close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            let tab = self.tab_for(Profile::Native);
            self.tabs.push(tab);
        }
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
    }
    pub fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    /// Take tab `index` out for another window (a tab dragged out). The
    /// window keeps a fresh tab when it gave its last one (it closes).
    pub fn take(&mut self, index: usize) -> Option<Tab> {
        if index >= self.tabs.len() {
            return None;
        }
        let tab = self.tabs.remove(index);
        if self.tabs.is_empty() {
            let fresh = self.tab_for(Profile::Native);
            self.tabs.push(fresh);
        }
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        Some(tab)
    }
    /// A tab another window gave, made active; `pristine` replaces this
    /// window's untouched startup tabs (a window made for the tab).
    pub fn adopt(&mut self, tab: Tab, pristine: bool) {
        if pristine {
            self.tabs.clear();
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.page = Page::Console;
    }
    /// Move tab `from` to position `to` (a tab dragged along the strip); the
    /// active tab stays active. False when nothing moved.
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tabs.len() || to >= self.tabs.len() || from == to {
            return false;
        }
        let active = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = self.tabs.iter().position(|t| t.id == active).unwrap_or(0);
        true
    }
}
/// Same geometry drives drawing and hit testing, including compact windows.
#[derive(Clone, Copy)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    /// The navigation pane is hidden (the console's pane button): the
    /// console takes the whole width.
    pub rail_hidden: bool,
}
/// Caption (tab row) height: File Explorer's caption.
pub const CAPTION: u32 = crate::caption_tabs::HEIGHT;
/// Top of the console and page panels: directly under the tab row.
pub const CONTENT_TOP: i32 = CAPTION as i32;
/// Mouse-wheel step in console rows per detent (Windows Terminal default).
pub const WHEEL_ROWS: usize = 3;
impl Layout {
    pub fn rail(self) -> u32 {
        if self.rail_hidden {
            0
        } else if self.width < 660 {
            52
        } else {
            208
        }
    }
    /// Explorer's caption tab strip, starting at the window's left edge.
    pub fn strip(self, count: usize, active: usize) -> crate::document_tabs::Strip {
        crate::caption_tabs::strip(self.width, count, active)
    }
    pub fn tab_at(self, x: i32, y: i32, count: usize, active: usize) -> Option<usize> {
        let strip = self.strip(count, active);
        strip
            .range()
            .find(|&i| strip.tab(i).is_some_and(|r| r.contains(x, y)))
    }
    pub fn close_at(self, x: i32, y: i32, count: usize, active: usize) -> Option<usize> {
        let strip = self.strip(count, active);
        strip
            .range()
            .find(|&i| strip.close(i).is_some_and(|r| r.contains(x, y)))
    }
    pub fn plus(self, count: usize, active: usize) -> Option<crate::explorer_layout::Bounds> {
        self.strip(count, active).plus()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn texts(t: &Tab) -> Vec<&str> {
        t.lines.iter().map(Line::text).collect()
    }
    #[test]
    fn wrapped_rows_are_individually_scrollable_and_search_reveals_the_match() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.lines.clear();
        t.output(&alloc::format!("needle{}tail", "x".repeat(1900)));
        let columns = 12;
        let rows = 8;
        assert!(t.search_columns("needle", columns));
        let visual = t.visual_lines(columns);
        let end = visual.len() - t.scroll;
        assert!(
            visual[end.saturating_sub(rows)..end]
                .iter()
                .any(|l| l.contains("needle"))
        );
        t.scroll = 0;
        t.scroll_rows(true, 12, columns);
        assert_eq!(t.scroll, 12);
        t.scroll_rows(true, 10000, columns);
        assert_eq!(t.scroll, visual.len() - 1);
        t.scroll_rows(false, 10000, columns);
        assert_eq!(t.scroll, 0);
        assert!(t.search_columns("tail", columns));
        assert_eq!(t.scroll, 0);
        t.scroll_rows(true, 10000, 1);
        t.scroll_rows(false, 0, 200);
        assert!(t.scroll < t.visual_lines(200).len());
    }
    #[test]
    fn foreign_engines_never_claim_available() {
        // Doorsh and cmd.exe (door-shells) run; PowerShell and WSL need
        // their real runtimes.
        assert!(Profile::Native.available());
        assert!(Profile::Cmd.available());
        for p in [Profile::PowerShell, Profile::Dsl] {
            assert!(!p.available());
        }
        assert_eq!(Profile::Dsl.status(), "Work in progress");
    }
    #[test]
    fn tabs_keep_input_directory_history_and_scroll_isolated() {
        let mut s = TerminalState::new("/users/1001".into());
        s.tab().input.set("echo first".into(), false);
        s.tab().submit(true);
        s.tab().cwd = "/users/1001/notes".into();
        s.new_tab(Profile::Native);
        s.tab().input.set("draft".into(), false);
        s.active = 0;
        s.tab().history(true);
        assert_eq!(s.tab().input.text(), "echo first");
        assert_eq!(s.tab().cwd, "/users/1001/notes");
        s.close(0);
        assert_eq!(s.tab().input.text(), "draft");
        s.close(0);
        assert_eq!(s.active, 0);
        // No tab limit: the strip scrolls.
        for _ in 1..40 {
            assert!(s.new_tab(Profile::Cmd));
        }
        assert_eq!(s.tabs.len(), 40);
    }
    #[test]
    fn tabs_move_between_windows_and_along_the_strip() {
        let mut a = TerminalState::new("/users/1001".into());
        a.new_tab(Profile::Cmd);
        a.new_tab(Profile::PowerShell);
        let ids: Vec<u64> = a.tabs.iter().map(|t| t.id).collect();
        // Reordering keeps the active tab active.
        assert_eq!(a.active, 2);
        assert!(a.reorder(2, 0));
        assert_eq!(a.tabs[0].id, ids[2]);
        assert_eq!(a.active, 0);
        assert!(!a.reorder(0, 0) && !a.reorder(0, 9));
        // A tab dragged out joins another window, replacing its untouched
        // startup tab.
        let mut b = TerminalState::new("/users/1001".into());
        let moved = a.take(1).unwrap();
        assert_eq!(moved.id, ids[0]);
        b.adopt(moved, true);
        assert_eq!((b.tabs.len(), b.tabs[0].id, b.active), (1, ids[0], 0));
        assert_eq!(a.tabs.len(), 2);
        // The last tab out leaves a fresh one (the window closes).
        a.take(0).unwrap();
        let last = a.take(0).unwrap();
        assert_eq!(a.tabs.len(), 1);
        assert!(!ids.contains(&a.tabs[0].id));
        b.adopt(last, false);
        assert_eq!((b.tabs.len(), b.active), (2, 1));
        assert!(a.take(5).is_none());
    }
    #[test]
    fn output_history_and_control_input_are_bounded() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.output(&"x".repeat(MAX_LINE * 2 + 3));
        assert!(t.lines.iter().all(|l| l.text().len() <= MAX_LINE));
        // The parts of one long line continue each other; the line ends.
        let n = t.lines.len();
        assert!(t.lines[n - 3].continued() && t.lines[n - 2].continued());
        assert!(!t.lines[n - 1].continued());
        for _ in 0..MAX_LINES * 2 {
            t.output("\x1b[0m\0");
        }
        assert_eq!(t.lines.len(), MAX_LINES);
        assert!(
            t.lines
                .iter()
                .all(|l| !l.text().contains('\x1b') && !l.text().contains('\0'))
        );
        t.input.set("unfinished".into(), false);
        t.history(true);
        assert_eq!(t.input.text(), "unfinished");
        t.input.set("echo saved".into(), false);
        t.submit(true);
        t.input.set("draft".into(), false);
        t.history(true);
        t.history(false);
        assert_eq!(t.input.text(), "draft");
        t.forget();
        t.history(true);
        assert_eq!(t.input.text(), "draft");
    }
    #[test]
    fn streamed_output_continues_open_lines_and_keeps_line_breaks() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        t.stream("F1_START\r\nF1_TRACE a=1\r\nF1_PA");
        assert_eq!(texts(&t), ["F1_START", "F1_TRACE a=1", "F1_PA"]);
        t.stream("SS\n\nnext");
        assert_eq!(
            texts(&t),
            ["F1_START", "F1_TRACE a=1", "F1_PASS", "", "next"]
        );
        t.end_stream();
        t.stream("after");
        assert_eq!(t.lines.back().unwrap().text(), "after");
        // Command output starts its own line.
        t.output("done");
        assert_eq!(texts(&t)[5..], ["after", "done"]);
        // A line break alone ends the open line; it adds no empty row.
        t.clear();
        t.stream("a");
        t.stream("\n");
        t.stream("b\n");
        assert_eq!(texts(&t), ["a", "b"]);
    }
    #[test]
    fn scrolled_back_view_stays_on_its_text_while_output_arrives() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        t.reflow(10);
        for i in 0..40 {
            t.output(&alloc::format!("row {i}"));
        }
        t.scroll_rows(true, 5, 10);
        let rows = t.viewport(10, 4);
        let before: Vec<_> = t
            .rows_in(rows, 10)
            .iter()
            .map(|&(l, w)| (t.serial(l), w))
            .collect();
        t.output("one more\nand a long line that wraps over several rows");
        t.stream("partial");
        t.stream(" output that wraps too");
        let rows = t.viewport(10, 4);
        let after: Vec<_> = t
            .rows_in(rows, 10)
            .iter()
            .map(|&(l, w)| (t.serial(l), w))
            .collect();
        assert_eq!(before, after);
        // At the bottom the view follows output.
        t.scroll = 0;
        t.output("tail");
        assert_eq!(t.scroll, 0);
        // Eviction never leaves the view past the scrollback.
        t.scroll_rows(true, 10_000, 10);
        for _ in 0..MAX_LINES {
            t.output("flood");
        }
        assert!(t.scroll < t.total_rows(10));
    }
    #[test]
    fn reflow_rewraps_soft_rows_and_keeps_the_viewport_anchored() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        t.reflow(10);
        t.output("short\n");
        t.output(&"abcdefghij".repeat(5));
        t.output("end");
        // Hard breaks stay; soft-wrapped rows rejoin at any width.
        for columns in [3, 7, 10, 49, 50, 51, 200] {
            t.reflow(columns);
            let joined: String = t.visual_lines(columns).concat();
            assert_eq!(joined, alloc::format!("short{}end", "abcdefghij".repeat(5)));
            let rows = 5usize.div_ceil(columns) + 1 + 50usize.div_ceil(columns) + 1;
            assert_eq!(t.total_rows(columns), rows);
        }
        // Wrap narrow and wide and back: the text at the bottom of the
        // scrolled-back view stays there, and the round trip is exact.
        t.reflow(10);
        t.scroll_rows(true, 3, 10);
        let bottom = |t: &Tab, columns: usize| {
            let view = t.viewport(columns, 1);
            let (line, wrap) = t.locate_row(view.start, columns).unwrap();
            (t.serial(line), wrap * columns..(wrap + 1) * columns)
        };
        let (line, cells) = bottom(&t, 10);
        let anchor = cells.end - 1;
        for columns in [4, 7, 33, 3, 10] {
            t.reflow(columns);
            let (l, c) = bottom(&t, columns);
            assert_eq!(l, line, "columns={columns}");
            assert!(c.contains(&anchor), "columns={columns}");
        }
        assert_eq!(t.scroll, 3);
        // The bottom stays the bottom.
        t.scroll = 0;
        t.reflow(3);
        assert_eq!(t.scroll, 0);
        t.reflow(100);
        assert_eq!(t.scroll, 0);
    }
    #[test]
    fn a_program_runs_in_a_console_grid_that_returns_to_the_scrollback() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        t.output("first");
        t.output("C:\\> pwsh");
        // The grid starts with the scrollback's last lines, the cursor below.
        let row = t.open_console(20, 4);
        assert_eq!(row, 2);
        assert!(t.lines.is_empty());
        t.feed(b"\x1b]0;PowerShell\x07\x1b[93mPS\x1b[0m C:\\> ");
        assert_eq!(t.title, "PowerShell");
        let screen = &t.console.as_ref().unwrap().screen;
        assert_eq!(screen.row_text(1), "C:\\> pwsh");
        assert_eq!(screen.row_text(2), "PS C:\\>");
        assert_eq!(screen.row(2)[0].fg, door_vt::Color::Indexed(11));
        assert_eq!(screen.cursor(), (8, 2));
        // Rows scrolled off the grid's top join the scrollback.
        t.feed(b"\r\n1\r\n2\r\n3\r\n");
        assert_eq!(t.lines.len(), 3);
        assert_eq!(t.lines[0].text(), "first");
        assert_eq!(t.lines[2].text(), "PS C:\\>");
        // The program ends: its rows follow, the empty cursor row does not.
        t.close_console();
        assert!(t.console.is_none());
        let text: Vec<&str> = t.lines.iter().map(Line::text).collect();
        assert_eq!(text, ["first", "C:\\> pwsh", "PS C:\\>", "1", "2", "3"]);
        // Without a grid, output streams as before.
        t.feed(b"plain\n");
        assert_eq!(t.lines.back().unwrap().text(), "plain");
    }
    #[test]
    fn ingest_bounds_pending_output_and_frame_work() {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        let mut ingest = Ingest::default();
        let chunk = [b'y'; 192];
        let mut queued = 0;
        while ingest.room() >= chunk.len() {
            assert!(ingest.push(&chunk));
            queued += chunk.len();
        }
        assert!(queued <= PENDING_LIMIT && ingest.len() == queued);
        assert!(!ingest.push(&[b'z'; PENDING_LIMIT]), "a full queue refuses");
        assert_eq!(ingest.len(), queued);
        let moved = ingest.drain_into(&mut t, INGEST_BYTES, false);
        assert_eq!(moved, INGEST_BYTES.min(queued));
        assert_eq!(ingest.len(), queued - moved);
        while ingest.drain_into(&mut t, INGEST_BYTES, true) > 0 {}
        assert!(ingest.is_empty());
        let cells: usize = t.lines.iter().map(Line::cells).sum();
        assert_eq!(cells, queued);
        assert!(t.lines.iter().all(|l| l.text().len() <= MAX_LINE));
        // UTF-8 sequences are never split between frames.
        let mut ingest = Ingest::default();
        t.clear();
        assert!(ingest.push("a文".as_bytes()));
        assert_eq!(ingest.drain_into(&mut t, 3, false), 1);
        assert_eq!(ingest.drain_into(&mut t, 2, false), 0);
        assert_eq!(ingest.drain_into(&mut t, 8, false), 3);
        assert_eq!(t.lines[0].text(), "a文");
        assert!(ingest.push(&"文".as_bytes()[..2]));
        assert_eq!(
            ingest.drain_into(&mut t, 8, false),
            0,
            "incomplete tail waits"
        );
        assert_eq!(
            ingest.drain_into(&mut t, 8, true),
            2,
            "the finished writer flushes"
        );
        assert_eq!(utf8_cut(b"abc", 10, false), 3);
        assert_eq!(utf8_cut(&[0xe6, 0x96, 0x87, b'x'], 4, false), 4);
        assert_eq!(utf8_cut(&[0x80, 0x80, 0x80, 0x80], 4, false), 4);
    }
    #[test]
    fn preference_wire_format_refuses_malformed_or_future_data() {
        for font in 0..=2 {
            for light in [false, true] {
                let p = Preferences {
                    font,
                    light_console: light,
                    history: false,
                    opacity: 85,
                };
                assert_eq!(Preferences::decode(&p.encode()), Some(p));
            }
        }
        let b = Preferences::default().encode();
        for n in 0..8 {
            assert_eq!(Preferences::decode(&b[..n]), None);
        }
        for (index, value) in [(4, 3), (5, 2), (6, 3), (7, 22), (7, 255)] {
            let mut bad = b;
            bad[index] = value;
            assert_eq!(Preferences::decode(&bad), None);
        }
    }
    #[test]
    fn opacity_is_bounded_premultiplied_and_legacy_settings_migrate() {
        let old = Preferences::decode(b"DTRM\x01\x01\x02\x01").unwrap();
        assert!(old.history && old.light_console);
        assert_eq!((old.font, old.opacity), (2, 95));
        for opacity in (50..=100).step_by(5) {
            for light_console in [false, true] {
                let p = Preferences {
                    opacity,
                    light_console,
                    ..Preferences::default()
                };
                assert_eq!(Preferences::decode(&p.encode()), Some(p));
                let c = p.background();
                assert!(c[..3].iter().all(|v| *v <= c[3]));
                assert_eq!(c[3], (opacity as u16 * 255 / 100) as u8);
            }
        }
    }
    #[test]
    fn compact_tab_geometry_and_search_remain_reachable() {
        for width in [320, 490, 660, 1000, 1920] {
            let l = Layout {
                width,
                height: 530,
                rail_hidden: false,
            };
            for count in 1..=24 {
                for active in 0..count {
                    let strip = l.strip(count, active);
                    // The active tab is always visible; tabs start at the left edge.
                    assert!(strip.range().contains(&active));
                    for i in strip.range() {
                        let r = strip.tab(i).unwrap();
                        assert!(r.x >= 8 && r.x + r.width as i32 <= width as i32 - 138);
                        assert_eq!(l.tab_at(r.x + 4, 20, count, active), Some(i));
                        let c = strip.close(i).unwrap();
                        assert_eq!(l.close_at(c.x + 4, c.y + 4, count, active), Some(i));
                    }
                    if let Some(p) = l.plus(count, active) {
                        assert_eq!(l.tab_at(p.x + 4, 20, count, active), None);
                    }
                }
            }
        }
        let mut t = Tab::new(Profile::Native, "/".into());
        t.output("first\nneedle\nlast");
        assert!(t.search("needle"));
        assert_eq!(t.scroll, 1);
        assert!(!t.search("absent"));
    }
}
