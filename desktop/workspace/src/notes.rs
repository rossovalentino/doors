//! Notes (Windows 11 Notepad): the P12 TEXT-01 editor. Unicode documents up to
//! `document::MAX_BYTES` with grapheme-aware editing, Door Text shaping, IME
//! composition through door.text-input, clipboard text, zoom, word wrap and
//! durable Storage save/reopen preserving line endings and BOM.
use super::*;
use crate::icons::Icon;
use alloc::vec::Vec;
use core::cell::Cell;
use door_graphics_text::TextStyle;
use door_service_client::window::WindowClient;
use door_service_client::{Disposition, READ, SHARE_ALL};
use door_service_protocol::{clipboard as cb, text_input as ti, window as w};
use door_workspace::dialog::Layout as DialogLayout;
use door_workspace::document::{Document, EditError, MAX_BYTES, normalize_newlines};
use door_workspace::document_format::{
    self as fmt, BAND_HEIGHT, BAND_TOP, Block, CAPTION, Chrome, Mark, SEPARATOR, STATUS,
    TAB_HEIGHT, TAB_TEXT, TAB_TOP, Tool,
};
use door_workspace::document_layout::{self as layout, Engine, STYLE, display};
use door_workspace::document_prompt::{Intent, Prompt};
use door_workspace::document_save::{Outcome, SaveState};
use door_workspace::document_view::{DocumentView, Geometry, Theme};
use door_workspace::text_entry::TextEvent;
#[cfg(test)]
#[path = "../tests/support/notes_raster.rs"]
mod raster_tests;
// door.accessibility tree (shared adapter `door_workspace::semantics`).
#[path = "notes_semantics.rs"]
mod semantics;
const MENU_LABELS: [&str; 3] = ["File", "Edit", "View"];
/// Regular14 advances of the menu labels in the reference UI font; `draw`
/// replaces them with the live measurement before any input arrives.
const MENU_WIDTHS: [u32; 3] = [21, 24, 30];
const LIMIT: &str = "Notes edits documents up to 4 MiB. Text is unchanged.";
const FORMAT_LIMIT: &str = "Formatting exceeds the document limit. Text is unchanged.";
/// Clipboard commands waiting for `window_update` (Ctrl+C, Ctrl+V, Cut).
const MAX_CLIPBOARD_COMMANDS: usize = 8;
/// Material alpha of the band, tab and status layer (LayerOnMicaBaseAlt
/// over the Mica caption, premultiplied by the compositor).
const LAYER_ALPHA: u8 = 236;
/// Windows' accent (BGRA); dark mode text paints it as Light2.
const ACCENT: [u8; 4] = [196, 103, 0, 255];
/// Drop-down menus of the formatting toolbar (menu kinds 3, 4 and 5).
const HEADINGS: [&str; 7] = [
    "Heading 1",
    "Heading 2",
    "Heading 3",
    "Heading 4",
    "Heading 5",
    "Heading 6",
    "Body text",
];
const LISTS: [&str; 2] = ["Bulleted list", "Numbered list"];
const TABLES: [&str; 4] = ["2 x 2 table", "3 x 3 table", "4 x 4 table", "5 x 5 table"];
/// Notepad 26H2 colours measured on the owner's dark screenshot (and the
/// WinUI light equivalents): the Mica caption, the layer shared by the
/// selected tab, the menu band and the status bar, the editor surface
/// (#272727 dark), strokes, and the secondary status text (#CDCDCD).
#[derive(Clone, Copy)]
struct Look {
    caption: [u8; 4],
    layer: [u8; 4],
    body: [u8; 4],
    stroke: [u8; 4],
    divider: [u8; 4],
    text: [u8; 4],
    secondary: [u8; 4],
    dim: [u8; 4],
    hover: [u8; 4],
    pressed: [u8; 4],
    link: [u8; 4],
}
impl Look {
    fn resolve(dark: bool, theme: Theme) -> Self {
        if dark {
            Self {
                caption: [32, 32, 32, 255],
                layer: [45, 45, 45, 255],
                body: [39, 39, 39, 255],
                stroke: [33, 33, 33, 255],
                divider: [62, 62, 62, 255],
                text: [255; 4],
                secondary: [205, 205, 205, 255],
                dim: [128, 128, 128, 255],
                hover: [58, 58, 58, 255],
                pressed: [53, 53, 53, 255],
                link: [255, 183, 96, 255],
            }
        } else {
            Self {
                caption: theme.caption,
                layer: theme.bar,
                body: theme.background,
                stroke: [229, 229, 229, 255],
                divider: [222, 222, 222, 255],
                text: theme.text,
                secondary: [96, 96, 96, 255],
                dim: [138, 138, 138, 255],
                hover: theme.hover,
                pressed: [233, 229, 224, 255],
                link: ACCENT,
            }
        }
    }
}
/// A colour as `View::material_rect` stores it: premultiplied by `alpha`
/// when the window is composited over a material, else opaque.
fn material(v: &ui::View<'_>, color: [u8; 4], alpha: u8) -> [u8; 4] {
    if !v.materials {
        return color;
    }
    let mut c = color;
    for value in &mut c[..3] {
        *value = ((u16::from(*value) * u16::from(alpha) + 127) / 255) as u8;
    }
    c[3] = alpha;
    c
}
/// A coverage mask `from` px square drawn as a `size` px glyph, resampled
/// here to the exact physical size (the View's resample cache is keyed by
/// buffer address, which per-frame masks would alias).
fn paint_mask(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    size: u32,
    mask: &[u8],
    from: usize,
    color: [u8; 4],
) {
    let side = v.zoom.len(size) as usize;
    let m = fmt::resample(mask, from, side);
    v.mask_pixels(x, y, size as usize, &m, color, None);
}
/// A 20 px shell glyph drawn at `size` px.
fn glyph(v: &mut ui::View<'_>, x: i32, y: i32, size: u32, mask: &[u8], color: [u8; 4]) {
    let from = 20 * v.scale.max(1) as usize;
    paint_mask(v, x, y, size, mask, from, color);
}
/// A stroked 16 px toolbar glyph (`document_format::stroke_glyph`).
fn stroked(v: &mut ui::View<'_>, x: i32, y: i32, mask: &[u8], color: [u8; 4]) {
    let from = 16 * v.scale.max(1) as usize;
    paint_mask(v, x, y, 16, mask, from, color);
}
/// A filled dot of `radius` px centred in a 16 px box at (x, y).
fn dot(v: &mut ui::View<'_>, x: i32, y: i32, radius: f32, color: [u8; 4]) {
    let m = fmt::stroke_glyph(v.scale, &[], &[(8.0, 8.0, radius)]);
    stroked(v, x, y, &m, color);
}
struct NoteDoc {
    path: String,
    document: Document,
    save_state: SaveState,
    message: String,
    view: DocumentView,
    prompt: Prompt,
    prompt_error: String,
}
/// Clipboard work done in `window_update`, where the window client is.
enum Clip {
    Copy(String),
    /// Removed only once the clipboard holds it, if the text is unchanged.
    Cut {
        text: String,
        range: core::ops::Range<usize>,
        revision: Option<u64>,
    },
    Paste,
}
/// door.text-input context of this window.
#[derive(Default)]
struct Ime {
    serial: Option<u32>,
    sent: Option<w::Rect>,
    /// A local edit or caret move finished the composition: cancel the IME's.
    reset: bool,
    /// Disabled by the server or refused: wait for the next focus change.
    blocked: bool,
}
pub struct Notes {
    menu: crate::menu::Menu,
    menu_kind: u8,
    current: NoteDoc,
    tabs: Vec<Option<NoteDoc>>,
    active: usize,
    size: (u32, u32),
    pending_window_close: bool,
    tab_close_pending: bool,
    closing_all: bool,
    /// Cancel was chosen in a whole-window close prompt (reported once).
    close_declined: bool,
    new_window: bool,
    needs_configure: bool,
    status_bar: bool,
    /// Formatted mode (Markdown shown formatted) or Plain text, toggled
    /// from the status bar; the file is plain text either way.
    markdown: bool,
    /// What's new was opened: its accent dot is gone.
    whats_new_seen: bool,
    /// Notepad zoom in percent (window-wide, 10..=500).
    zoom: u32,
    focused: bool,
    ime: Ime,
    /// Clipboard commands in key order, run by `window_update` (rapid
    /// Ctrl+C then Ctrl+V keeps both; at most `MAX_CLIPBOARD_COMMANDS`).
    clipboard: alloc::collections::VecDeque<Clip>,
    /// Copied text when the session has no clipboard broker.
    local_clipboard: String,
    /// Painted caret (logical x, row top, row height) and display percent,
    /// reported to the IME for candidate placement.
    caret_rect: Cell<Option<(i32, i32, u32)>>,
    percent: Cell<u32>,
    /// Selected-character count cache: (revision, selection, count).
    selected_count: Cell<(Option<u64>, usize, usize, usize)>,
    /// Menu-bar label advances, measured with the UI font by `draw` (layout
    /// uses the editor font) so hit testing matches the painted plates.
    menu_widths: core::cell::Cell<[u32; 3]>,
}
impl Notes {
    fn menu_items(&self) -> &'static [&'static str] {
        match self.menu_kind {
            1 => &[
                "Undo            Ctrl+Z",
                "Redo            Ctrl+Y",
                "Cut             Ctrl+X",
                "Copy            Ctrl+C",
                "Paste           Ctrl+V",
                "Select all      Ctrl+A",
            ],
            2 => &[
                "Word wrap",
                "Status bar",
                "Zoom in         Ctrl+Plus",
                "Zoom out        Ctrl+Minus",
                "Restore default zoom  Ctrl+0",
            ],
            3 => &HEADINGS,
            4 => &LISTS,
            5 => &TABLES,
            _ => &[
                "New tab         Ctrl+N",
                "New window      Ctrl+Shift+N",
                "Save            Ctrl+S",
                "Reopen saved file  Ctrl+O",
                "Close tab       Ctrl+W",
                "Close window    Alt+F4",
            ],
        }
    }
    fn menu_action(&self, index: usize) -> u32 {
        match self.menu_kind {
            1 => [20, 21, 23, 24, 25, 22].get(index).copied().unwrap_or(0),
            2 => [3, 30, 31, 32, 33].get(index).copied().unwrap_or(0),
            // Heading 1..6 then body text; bulleted, numbered; n x n tables.
            3 => [71, 72, 73, 74, 75, 76, 70]
                .get(index)
                .copied()
                .unwrap_or(0),
            4 => [80, 81].get(index).copied().unwrap_or(0),
            5 => [92, 93, 94, 95].get(index).copied().unwrap_or(0),
            _ => [10, 11, 1, 2, 12, 13].get(index).copied().unwrap_or(0),
        }
    }
    fn engine<'a>(&self, fonts: &'a FontAtlas) -> Engine<'a> {
        Engine::new(fonts, self.zoom)
    }
    /// The window's Notepad chrome (`document_format::Chrome`).
    fn chrome(&self) -> Chrome {
        Chrome {
            width: self.size.0,
            height: self.size.1,
            tabs: self.tabs.len(),
            active: self.active,
        }
    }
    pub fn configure(&mut self, width: u32, height: u32, fonts: &FontAtlas) {
        self.size = (width, height);
        self.needs_configure = false;
        self.percent.set(fonts.percent());
        let engine = self.engine(fonts);
        let body = self.chrome().body(self.status_bar);
        self.current.view.configure(
            &self.current.document,
            Geometry {
                x: body.x,
                y: body.y,
                width: body.width,
                height: body.height,
                line_height: engine.line_height(),
            },
            &engine,
        );
    }
    fn reveal(&mut self, fonts: &FontAtlas) {
        if self.needs_configure {
            self.configure(self.size.0, self.size.1, fonts);
        } else {
            self.current
                .view
                .refresh(&self.current.document, &self.engine(fonts));
        }
    }
    /// A caret move or edit during composition commits the preedit here and
    /// cancels the IME's composition.
    fn finish_composition(&mut self) {
        if self.current.document.finish_composition() {
            self.ime.reset = true;
        }
    }
    pub fn text_hit(&self, x: i32, y: i32) -> bool {
        !self.modal() && self.current.view.body_contains(x, y)
    }
    pub fn pointer_down(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        if self.modal() {
            let target = self
                .current
                .prompt
                .dialog
                .target(&self.dialog_layout(), x, y);
            self.current.prompt.dialog.pointer(target);
            return true;
        }
        if self.menu.open() || !self.current.view.body_contains(x, y) {
            return false;
        }
        self.finish_composition();
        let engine = self.engine(fonts);
        self.current
            .view
            .pointer_down(&mut self.current.document, x, y, &engine)
    }
    pub fn pointer_motion(&mut self, x: i32, y: i32, fonts: &FontAtlas) -> bool {
        if self.modal() {
            return false;
        }
        let engine = self.engine(fonts);
        self.current
            .view
            .pointer_motion(&mut self.current.document, x, y, &engine)
    }
    /// Mouse wheel (see `Application::wheel`): three text rows per detent; the
    /// viewport moves without moving the caret, as in Notepad.
    pub fn wheel(
        &mut self,
        _x: i32,
        _y: i32,
        notches: i32,
        fonts: &door_graphics_text::FontAtlas,
    ) -> bool {
        if notches == 0 || self.modal() || self.menu.open() || self.needs_configure {
            return false;
        }
        let rows = -(notches as isize) * 3;
        self.current
            .view
            .scroll_by(&self.current.document, rows, &self.engine(fonts))
    }
    pub fn pointer_up(&mut self) {
        self.current.view.cancel_drag();
    }
    pub fn menu_open(&self) -> bool {
        self.menu.open()
    }
    pub fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        !self.modal() && self.menu.hover(x, y, self.menu_items().len())
    }
    pub fn dismiss_menu(&mut self) {
        self.menu.dismiss();
    }
    pub fn context(&mut self, x: i32, y: i32, w: u32, h: u32) {
        if self.modal() {
            return;
        }
        self.finish_composition();
        self.current.view.cancel_drag();
        if self.menu_kind > 2 {
            self.menu_kind = 1;
        }
        self.menu.show(x, y, w, h, self.menu_items().len());
    }
    pub fn focus(&mut self, focused: bool) {
        self.focused = focused;
        if focused {
            self.ime.blocked = false;
        }
    }
    /// Windows 11 Notepad menu bar: each plate is its measured label plus
    /// 10 px padding per side, from x 4 with 8 px between plates, filling
    /// the 32 px band (`Chrome::menu`). Paint, hit testing and menu
    /// anchoring all read these rectangles.
    fn menu_bar(&self) -> [door_workspace::explorer_layout::Bounds; 3] {
        self.chrome().menu(self.menu_widths.get())
    }
    /// The bold, italic and strikethrough runs around the caret (the
    /// toolbar shows those toggles checked), in Formatted mode.
    fn caret_marks(&self) -> [bool; 3] {
        let d = &self.current.document;
        if !self.markdown || d.len() > MAX_BYTES {
            return [false; 3];
        }
        let line = d.line_of(d.caret());
        let start = d.line_start(line);
        let text = d.text(start..d.line_end(line));
        let at = d.caret() - start;
        let (_, spans) = fmt::parse_line(&text);
        [Mark::Bold, Mark::Italic, Mark::Strike].map(|mark| {
            spans
                .iter()
                .any(|s| s.mark == mark && s.range.start <= at && at <= s.range.end)
        })
    }
    /// Replaces `range` with `text` as one undo group, then selects
    /// `select` (relative to the replacement) when given. Formatting edits
    /// switch the window to Formatted mode, as Notepad's toolbar does.
    fn replace(
        &mut self,
        range: core::ops::Range<usize>,
        text: &str,
        select: Option<core::ops::Range<usize>>,
    ) {
        let doc = &mut self.current.document;
        if doc.text(range.clone()) == text {
            if let Some(s) = select {
                doc.select(range.start + s.start..range.start + s.end);
            }
            self.markdown = true;
            return;
        }
        doc.select(range.clone());
        match doc.insert(text) {
            Ok(_) => {
                if let Some(s) = select {
                    doc.select(range.start + s.start..range.start + s.end);
                }
                self.markdown = true;
                self.current.view.edited();
            }
            Err(_) => self.current.message = FORMAT_LIMIT.into(),
        }
    }
    /// The whole lines the selection touches (a selection ending at a
    /// line start leaves that line out), without the last line ending.
    fn selected_lines(&self) -> core::ops::Range<usize> {
        let d = &self.current.document;
        let sel = d.selection();
        let first = d.line_of(sel.start);
        let mut last = d.line_of(sel.end);
        if last > first && d.line_start(last) == sel.end {
            last -= 1;
        }
        d.line_start(first)..d.line_end(last)
    }
    /// Rewrites each selected line with `f(index, line)`. A caret keeps its
    /// place in its text; a selection covers the rewritten lines.
    fn edit_lines(&mut self, f: impl Fn(usize, &str) -> String) {
        self.finish_composition();
        let range = self.selected_lines();
        let d = &self.current.document;
        let (caret, collapsed) = (d.caret(), d.selection().is_empty());
        let old = d.text(range.clone()).into_owned();
        let mut out = String::with_capacity(old.len() + 16);
        for (i, piece) in old.split('\n').enumerate() {
            let (line, cr) = piece
                .strip_suffix('\r')
                .map_or((piece, ""), |line| (line, "\r"));
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&f(i, line));
            out.push_str(cr);
        }
        let select = if collapsed && !old.contains('\n') {
            let at = (caret - range.start + out.len()).saturating_sub(old.len());
            at.min(out.len())..at.min(out.len())
        } else {
            0..out.len()
        };
        self.replace(range, &out, Some(select));
    }
    /// One toolbar button (or its shortcut): Markdown source edits that
    /// Formatted mode shows formatted. Drop-down buttons apply their first
    /// item (Heading 1 toggles, a bulleted list toggles, a 3 x 3 table).
    fn apply_tool(&mut self, tool: Tool) {
        self.finish_composition();
        let d = &self.current.document;
        let sel = d.selection();
        match tool {
            Tool::Headings => self.edit_lines(|_, line| {
                let level = match fmt::parse_line(line).0 {
                    Block::Heading(1) => 0,
                    _ => 1,
                };
                fmt::set_heading(line, level)
            }),
            Tool::Lists => self.edit_lines(|_, line| fmt::toggle_list(line, false, 1)),
            Tool::Bold | Tool::Italic | Tool::Strike => {
                let mark = match tool {
                    Tool::Bold => "**",
                    Tool::Italic => "*",
                    _ => "~~",
                };
                let (text, inner) = fmt::toggle_wrap(&d.selected_text(), mark);
                self.replace(sel, &text, Some(inner));
            }
            Tool::Link => {
                let (text, address) = fmt::link(&d.selected_text());
                self.replace(sel, &text, Some(address));
            }
            Tool::Table => self.insert_table(3),
            Tool::Clear => {
                let range = if sel.is_empty() {
                    self.selected_lines()
                } else {
                    sel
                };
                let text = fmt::clear(&self.current.document.text(range.clone()));
                let all = 0..text.len();
                self.replace(range, &text, Some(all));
            }
        }
    }
    /// An `n` x `n` table on its own lines at the caret; its first header
    /// cell is selected to type over.
    fn insert_table(&mut self, n: usize) {
        self.finish_composition();
        let d = &self.current.document;
        let sel = d.selection();
        let eol = d.eol().as_str();
        let prefix = if sel.start == d.line_start(d.line_of(sel.start)) {
            ""
        } else {
            eol
        };
        let (table, first) = fmt::table(n, n, eol);
        let at = prefix.len();
        self.replace(
            sel,
            &format!("{prefix}{table}"),
            Some(at + first.start..at + first.end),
        );
    }
    fn set_zoom(&mut self, zoom: u32) {
        let zoom = zoom.clamp(layout::MIN_ZOOM, layout::MAX_ZOOM);
        if zoom != self.zoom {
            self.zoom = zoom;
            self.needs_configure = true;
        }
    }
    fn zoom_key(&mut self, scan: u32) -> bool {
        match scan {
            13 | 0x4e => self.set_zoom(self.zoom + layout::ZOOM_STEP),
            12 | 0x4a => self.set_zoom(self.zoom.saturating_sub(layout::ZOOM_STEP)),
            11 | 0x52 => self.set_zoom(100),
            _ => return false,
        }
        true
    }
    fn copy(&mut self, cut: bool) {
        let range = self.current.document.selection();
        if range.is_empty() {
            return;
        }
        let text = self.current.document.selected_text();
        self.queue_clipboard(if cut {
            Clip::Cut {
                text,
                range,
                revision: self.current.document.content_revision(),
            }
        } else {
            Clip::Copy(text)
        });
    }
    /// Queue a clipboard command after the ones not yet run (the window
    /// client is only available in `window_update`).
    fn queue_clipboard(&mut self, clip: Clip) {
        if self.clipboard.len() >= MAX_CLIPBOARD_COMMANDS {
            self.current.message = "The clipboard is busy. Try again.".into();
            return;
        }
        self.clipboard.push_back(clip);
    }
    fn command(&mut self, id: u32, storage: &mut Storage) -> Result<(), Error> {
        self.finish_composition();
        match id {
            50..=57 => self.apply_tool(Tool::ALL[(id - 50) as usize]),
            70..=76 => {
                let level = (id - 70) as u8;
                self.edit_lines(|_, line| fmt::set_heading(line, level));
            }
            80 => self.edit_lines(|_, line| fmt::toggle_list(line, false, 1)),
            81 => self.edit_lines(|i, line| fmt::toggle_list(line, true, i + 1)),
            92..=95 => self.insert_table((id - 90) as usize),
            10 => self.new_tab(),
            11 => self.new_window = true,
            12 => self.close_tab(self.active),
            13 => self.pending_window_close = true,
            20 => {
                self.current.document.undo();
            }
            21 => {
                self.current.document.redo();
            }
            22 => self.current.document.select_all(),
            23 => self.copy(true),
            24 => self.copy(false),
            25 => self.queue_clipboard(Clip::Paste),
            30 => {
                self.status_bar = !self.status_bar;
                self.needs_configure = true;
            }
            31 => self.set_zoom(self.zoom + layout::ZOOM_STEP),
            32 => self.set_zoom(self.zoom.saturating_sub(layout::ZOOM_STEP)),
            33 => self.set_zoom(100),
            1 => self.save(storage)?,
            2 => self.reopen(storage)?,
            3 => self.current.view.set_wrap(!self.current.view.wrap()),
            _ => (),
        }
        Ok(())
    }
    pub fn target(&self, x: i32, y: i32) -> Option<u32> {
        if self.modal() {
            return self
                .current
                .prompt
                .dialog
                .target(&self.dialog_layout(), x, y)
                .map(|i| 3000 + i as u32);
        }
        if self.menu.open() {
            return self.menu.target(x, y, self.menu_items().len());
        }
        let c = self.chrome();
        for i in c.range() {
            if c.close(i).is_some_and(|r| r.contains(x, y)) {
                return Some(600 + i as u32);
            }
            if c.tab(i).is_some_and(|r| r.contains(x, y)) {
                return Some(500 + i as u32);
            }
        }
        if c.plus().is_some_and(|r| r.contains(x, y)) {
            return Some(10);
        }
        if let Some(i) = Tool::ALL
            .iter()
            .position(|t| c.tool(*t).is_some_and(|r| r.contains(x, y)))
        {
            return Some(50 + i as u32);
        }
        if c.whats_new().contains(x, y) {
            return Some(42);
        }
        if c.settings().contains(x, y) {
            return Some(43);
        }
        if self.status_bar && c.format_toggle().is_some_and(|r| r.contains(x, y)) {
            return Some(44);
        }
        let [file, edit, view] = self.menu_bar();
        [(file, 1), (edit, 40), (view, 41)]
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, id)| id)
    }

    pub fn new(path: String, storage: &mut Storage) -> Result<Self, Error> {
        if let Some((parent, _)) = path.rsplit_once('/') {
            match storage.mkdir(parent) {
                Ok(()) | Err(Error::Service(Code::Exists)) => (),
                Err(e) => return Err(e),
            }
        }
        let mut notes = Self::empty(path);
        match notes.load(storage) {
            Ok(()) | Err(Error::Service(Code::NotFound)) => (),
            Err(error) => return Err(error),
        }
        Ok(notes)
    }
    pub(crate) fn empty(path: String) -> Self {
        Self {
            menu: Default::default(),
            menu_kind: 0,
            current: NoteDoc {
                path,
                document: Document::new("").expect("empty document"),
                save_state: SaveState::default(),
                message: "Ctrl+S to save - Ctrl+O to reopen".into(),
                view: DocumentView::default(),
                prompt: Prompt::default(),
                prompt_error: String::new(),
            },
            tabs: alloc::vec![None],
            active: 0,
            size: (490, 358),
            pending_window_close: false,
            tab_close_pending: false,
            closing_all: false,
            close_declined: false,
            new_window: false,
            needs_configure: true,
            status_bar: true,
            markdown: true,
            whats_new_seen: false,
            zoom: 100,
            focused: true,
            ime: Ime::default(),
            clipboard: alloc::collections::VecDeque::new(),
            local_clipboard: String::new(),
            caret_rect: Cell::new(None),
            percent: Cell::new(100),
            selected_count: Cell::new((None, 0, 0, 0)),
            menu_widths: core::cell::Cell::new(MENU_WIDTHS),
        }
    }
    pub fn file_name(&self) -> &str {
        self.current.path.rsplit('/').next().unwrap_or("Untitled")
    }
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
    /// The client-owned part of the title bar, from the first tab (x 44)
    /// through "+" (`notes_main` reports it with `caption`).
    pub fn caption_width(&self) -> u32 {
        self.chrome().caption_width()
    }
    fn switch_raw(&mut self, index: usize) {
        if index == self.active || index >= self.tabs.len() {
            return;
        }
        self.finish_composition();
        let next = self.tabs[index].take().expect("inactive document");
        let old = core::mem::replace(&mut self.current, next);
        self.tabs[self.active] = Some(old);
        self.active = index;
        self.needs_configure = true;
        self.menu.dismiss();
    }
    fn new_tab(&mut self) {
        let parent = self
            .current
            .path
            .rsplit_once('/')
            .map_or("/users/1001/notes", |p| p.0);
        let Some(path) =
            door_workspace::document_tabs::untitled_path(parent, door_sys::time().unwrap_or(0))
        else {
            self.current.message = "Document identity limit reached.".into();
            return;
        };
        let doc = Self::empty(path).current;
        self.tabs.push(Some(doc));
        self.switch_raw(self.tabs.len() - 1);
    }
    fn drop_tab(&mut self) {
        if self.tabs.len() == 1 {
            self.pending_window_close = true;
            return;
        }
        self.tabs.remove(self.active);
        self.active = self.active.min(self.tabs.len() - 1);
        self.current = self.tabs[self.active].take().expect("remaining document");
        self.needs_configure = true;
        self.tab_close_pending = false;
    }
    fn close_tab(&mut self, index: usize) {
        self.switch_raw(index);
        self.closing_all = false;
        if self
            .current
            .prompt
            .accepts_close(self.current.save_state.attention(&self.current.document))
        {
            self.drop_tab();
        } else {
            self.tab_close_pending = true;
            self.show_prompt(Intent::Close);
        }
    }
    /// Open `path` from Start search: a tab already showing it is selected,
    /// otherwise the file opens in a new tab (as Windows Notepad does). A
    /// file that cannot be read leaves the tabs as they were.
    pub fn open_document(&mut self, path: String, storage: &mut Storage) -> Result<(), Error> {
        if self.current.path == path {
            return Ok(());
        }
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| t.as_ref().is_some_and(|d| d.path == path))
        {
            self.switch_raw(i);
            return Ok(());
        }
        let previous = self.active;
        self.tabs.push(Some(Self::empty(path).current));
        let opened = self.tabs.len() - 1;
        self.switch_raw(opened);
        if let Err(error) = self.load(storage) {
            self.switch_raw(previous);
            self.tabs.remove(opened);
            self.current.message = match &error {
                Error::Service(Code::Resources) => {
                    "This file is larger than the 4 MiB Notes limit.".into()
                }
                Error::Service(Code::Unsupported) => "This file isn't UTF-8 text.".into(),
                _ => "The file could not be opened.".into(),
            };
            return Err(error);
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P12 NOTES OPENED_DOCUMENT tabs={}\n",
            self.tabs.len()
        ));
        Ok(())
    }
    pub fn take_new_window(&mut self) -> bool {
        core::mem::take(&mut self.new_window)
    }
    fn reopen(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.current.save_state.attention(&self.current.document) {
            self.show_prompt(Intent::Reload);
            door_rt::diagnostic(format_args!("DOOR_P12 NOTES REOPEN_BLOCKED\n"));
            return Ok(());
        }
        self.load(storage)
    }
    pub fn modal(&self) -> bool {
        self.current.prompt.blocked()
    }
    fn dialog_layout(&self) -> DialogLayout {
        DialogLayout::new(self.size.0, self.size.1, self.current.prompt.dialog.count())
    }
    fn show_prompt(&mut self, intent: Intent) {
        if self.modal() {
            return;
        }
        self.finish_composition();
        self.menu.dismiss();
        self.current.view.cancel_drag();
        self.current.prompt_error.clear();
        self.current.prompt.show(intent);
        door_rt::diagnostic(format_args!("DOOR_P12 NOTES PROMPT intent={intent:?}\n"));
    }
    /// Called only after all preceding input has been drained to the model.
    pub fn close_requested(&mut self) -> bool {
        self.closing_all = true;
        if !self
            .current
            .prompt
            .accepts_close(self.current.save_state.attention(&self.current.document))
        {
            self.show_prompt(Intent::Close);
            return false;
        }
        if let Some(i) = self.tabs.iter().position(|d| {
            d.as_ref()
                .is_some_and(|d| !d.prompt.accepts_close(d.save_state.attention(&d.document)))
        }) {
            self.switch_raw(i);
            self.show_prompt(Intent::Close);
            return false;
        }
        true
    }
    pub fn take_close_declined(&mut self) -> bool {
        core::mem::take(&mut self.close_declined)
    }
    pub fn take_close(&mut self) -> bool {
        if core::mem::take(&mut self.pending_window_close) {
            return true;
        }
        if !self.current.prompt.take_close() {
            return false;
        }
        if self.tab_close_pending && !self.closing_all {
            self.drop_tab();
            false
        } else {
            true
        }
    }
    pub fn close_failed(&mut self) {
        self.current.prompt.close_failed();
        self.current.prompt_error = "Could not close this window. Your document is kept.".into();
    }
    pub fn window_recreated(&mut self) {
        self.closing_all = false;
        self.ime = Ime::default();
        for d in self.tabs.iter_mut().flatten() {
            d.prompt = Prompt::default();
        }
        if self.current.prompt.window_recreated() {
            self.current.prompt_error = "The window connection was restored. Your document is kept; choose how to close it.".into();
        }
    }
    fn choose(&mut self, choice: usize, storage: &mut Storage) {
        let Some(intent) = self.current.prompt.intent() else {
            return;
        };
        if choice == 2 {
            // Cancel on a window close: the window stays, and a shutdown
            // waiting on this prompt is cancelled (Windows 11).
            self.close_declined |= intent == Intent::Close && self.closing_all;
            self.current.prompt = Prompt::default();
            self.tab_close_pending = false;
            self.closing_all = false;
            for d in self.tabs.iter_mut().flatten() {
                d.prompt = Prompt::default();
            }
            self.current.prompt_error.clear();
            door_rt::diagnostic(format_args!(
                "DOOR_P12 NOTES PROMPT_CANCEL intent={intent:?}\n"
            ));
            return;
        }
        if choice > 2 {
            return;
        }
        if choice == 0 && self.save(storage).is_err() {
            self.current.prompt_error = self.current.message.clone();
            return;
        }
        if intent == Intent::Reload && self.load(storage).is_err() {
            self.current.prompt_error =
                "Could not reopen the saved file. Your current document is kept.".into();
            return;
        }
        self.current.prompt.completed();
        self.current.prompt_error.clear();
        door_rt::diagnostic(format_args!(
            "DOOR_P12 NOTES PROMPT_DONE intent={intent:?} choice={choice}\n"
        ));
    }
    fn load(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let file = storage.open(
            &self.current.path,
            Disposition::OpenExisting,
            READ,
            SHARE_ALL,
        )?;
        let loaded = (|| {
            let before = storage.info(&file)?;
            let size = before.bytes;
            // The UTF-8 BOM is not counted against the document bound.
            if size > MAX_BYTES as u64 + 3 {
                return Err(Error::Service(Code::Resources));
            }
            let mut text = Vec::with_capacity(size as usize);
            let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
            loop {
                let n = storage.read(&file, text.len() as u64, &mut chunk)?;
                if n == 0 {
                    break;
                }
                if text.len() + n > MAX_BYTES + 3 {
                    return Err(Error::Service(Code::Resources));
                }
                text.extend_from_slice(&chunk[..n]);
            }
            if text.len() as u64 != size || storage.info(&file)? != before {
                return Err(Error::Service(Code::Changed));
            }
            let document = Document::from_file(&text).map_err(|error| {
                Error::Service(match error {
                    EditError::TooLong => Code::Resources,
                    EditError::Unsupported => Code::Unsupported,
                })
            })?;
            Ok((document, before))
        })();
        let closed = storage.close(file);
        let (document, identity) = loaded?;
        closed?;
        self.current.document = document;
        self.current.save_state.loaded(identity);
        self.current.view.reset();
        self.ime.reset |= self.ime.serial.is_some();
        self.current.message = "Reopened saved bytes".into();
        // File Explorer Home > Recent; before the marker, which marks completion.
        door_workspace::recent::note_opened(
            storage,
            &self.current.path,
            door_workspace::recent::Kind::File,
        );
        door_rt::diagnostic(format_args!("DOOR_P12 NOTES REOPENED\n"));
        Ok(())
    }
    pub fn save(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if !self.current.save_state.attention(&self.current.document) {
            return Ok(());
        }
        let result = self.save_inner(storage);
        if result.is_err() {
            self.current.message = match (self.current.save_state.outcome(), result.as_ref().err())
            {
                (Outcome::ClosePending, _) => {
                    "Bytes saved; close not confirmed. Save to retry".into()
                }
                (Outcome::Uncertain, _) => {
                    "Save outcome unknown; edits kept and close guarded".into()
                }
                (_, Some(Error::Service(Code::Changed))) => {
                    "File changed elsewhere; your edits are kept".into()
                }
                _ => "Save failed; your unsaved edits are kept".into(),
            };
        }
        result
    }
    fn save_inner(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let bytes = self.current.document.file_bytes();
        // One atomic storage transaction at every size (checked replacement
        // up to 4 KiB, staged whole-file replacement up to the per-file cap).
        door_workspace::document_save::save(
            storage,
            &self.current.path,
            &mut self.current.save_state,
            &mut self.current.document,
            &bytes,
        )?;
        self.current.save_state.closed();
        self.current.message = format!("Saved {} bytes to disk", bytes.len());
        // File Explorer Home > Recent; before the marker, which marks completion.
        door_workspace::recent::note_opened(
            storage,
            &self.current.path,
            door_workspace::recent::Kind::File,
        );
        let _ = door_sys::write(b"DOOR_P11 NOTES SAVED\n");
        Ok(())
    }
    fn type_text(&mut self, text: &str) -> bool {
        match self.current.document.type_text(text) {
            Ok(changed) => {
                self.current.view.edited();
                changed
            }
            Err(EditError::TooLong) => {
                self.current.message = LIMIT.into();
                true
            }
            Err(EditError::Unsupported) => false,
        }
    }
    pub fn key(
        &mut self,
        code: u32,
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        let scan = code & 511;
        if self.modal() {
            let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
            if let Some(choice) = self.current.prompt.dialog.key(scan, shift) {
                self.choose(choice, storage);
                self.reveal(fonts);
            }
            return Ok(true);
        }
        if self.menu.open() {
            if let Some(id) = self.menu.key(scan, self.menu_items().len()) {
                self.command(self.menu_action(id), storage)?;
            }
            self.reveal(fonts);
            return Ok(true);
        }
        // Modifier presses an IME forwards must not end its composition.
        if matches!(scan, 29 | 42 | 54 | 56 | 58 | 0x11d | 0x138 | 0x15b | 0x15c) {
            return Ok(false);
        }
        self.finish_composition();
        let control = (code >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let text = (code >> 16) as u8;
        let engine = self.engine(fonts);
        if !control && matches!(scan, 0x148 | 0x150 | 0x149 | 0x151) {
            let page = self.current.view.rows().saturating_sub(1).max(1) as isize;
            let delta = match scan {
                0x148 => -1,
                0x150 => 1,
                0x149 => -page,
                _ => page,
            };
            self.current
                .view
                .vertical(&mut self.current.document, delta, shift, &engine);
            return Ok(true);
        }
        if control {
            if scan == 49 {
                if shift {
                    self.new_window = true
                } else {
                    self.new_tab();
                    self.reveal(fonts);
                }
                return Ok(true);
            }
            if scan == 17 {
                self.close_tab(self.active);
                self.reveal(fonts);
                return Ok(true);
            }
            if scan == 15 && self.tabs.len() > 1 {
                let next = if shift {
                    (self.active + self.tabs.len() - 1) % self.tabs.len()
                } else {
                    (self.active + 1) % self.tabs.len()
                };
                self.switch_raw(next);
                self.reveal(fonts);
                return Ok(true);
            }
            if self.zoom_key(scan) {
                self.reveal(fonts);
                return Ok(true);
            }
            match scan {
                31 => self.save(storage)?,
                24 => self.reopen(storage)?,
                30 => self.current.document.select_all(),
                // Notepad's formatting shortcuts: Ctrl+B, Ctrl+I,
                // Ctrl+Shift+X, Ctrl+K and Ctrl+Space.
                48 => self.apply_tool(Tool::Bold),
                23 => self.apply_tool(Tool::Italic),
                45 if shift => self.apply_tool(Tool::Strike),
                37 => self.apply_tool(Tool::Link),
                57 => self.apply_tool(Tool::Clear),
                46 | 0x152 => self.copy(false),
                45 => self.copy(true),
                47 => self.queue_clipboard(Clip::Paste),
                44 if shift => {
                    self.current.document.redo();
                }
                44 => {
                    self.current.document.undo();
                }
                21 => {
                    self.current.document.redo();
                }
                0x14b => self.current.document.horizontal(false, shift, true),
                0x14d => self.current.document.horizontal(true, shift, true),
                0x147 => self.current.document.home_end(false, true, shift),
                0x14f => self.current.document.home_end(true, true, shift),
                14 => {
                    self.current.document.erase(true, true);
                }
                0x153 => {
                    self.current.document.erase(false, true);
                }
                _ => return Ok(false),
            }
            if scan != 31 {
                self.current.view.edited();
            }
            self.reveal(fonts);
            return Ok(true);
        }
        match scan {
            0x14b => {
                self.current
                    .view
                    .horizontal(&mut self.current.document, false, shift, &engine)
            }
            0x14d => self
                .current
                .view
                .horizontal(&mut self.current.document, true, shift, &engine),
            0x147 => self
                .current
                .view
                .edge(&mut self.current.document, false, shift, &engine),
            0x14f => self
                .current
                .view
                .edge(&mut self.current.document, true, shift, &engine),
            14 => {
                self.current.document.erase(true, false);
            }
            0x153 if shift => self.copy(true),
            0x153 => {
                self.current.document.erase(false, false);
            }
            0x152 if shift => self.queue_clipboard(Clip::Paste),
            28 | 0x11c => {
                let eol = self.current.document.eol().as_str();
                self.type_text(eol);
            }
            _ if text == b'\n' => {
                let eol = self.current.document.eol().as_str();
                self.type_text(eol);
            }
            _ if (32..=126).contains(&text) || text == b'\t' => {
                self.type_text(core::str::from_utf8(&[text]).unwrap_or(""));
            }
            _ => return Ok(false),
        }
        self.current.view.edited();
        self.reveal(fonts);
        Ok(true)
    }
    /// door.text-input text for this window (`runtime::Application::text_input`).
    pub fn text_input(&mut self, event: TextEvent, fonts: &FontAtlas) -> bool {
        let live = |serial: u32, ime: &Ime| ime.serial == Some(serial);
        let changed = match event {
            TextEvent::Typed(text) if !self.modal() && !self.menu.open() => {
                self.finish_composition();
                self.type_text(&text)
            }
            TextEvent::Preedit {
                serial,
                text,
                cursor,
            } if live(serial, &self.ime) && !self.modal() => {
                self.current.view.edited();
                self.current.document.set_composition(&text, cursor)
            }
            TextEvent::Commit { serial, text } if live(serial, &self.ime) => {
                let text = normalize_newlines(&text, self.current.document.eol());
                match self.current.document.commit_composition(&text) {
                    Ok(changed) => changed,
                    Err(_) => {
                        self.current.message = LIMIT.into();
                        true
                    }
                }
            }
            TextEvent::Delete {
                serial,
                before,
                after,
            } if live(serial, &self.ime) => self
                .current
                .document
                .delete_surrounding(before as usize, after as usize),
            TextEvent::Disabled { serial } if live(serial, &self.ime) => {
                self.ime = Ime {
                    blocked: true,
                    ..Ime::default()
                };
                self.current.document.cancel_composition()
            }
            _ => false,
        };
        if changed {
            self.current.view.edited();
            self.reveal(fonts);
        }
        changed
    }
    /// Physical, content-local caret rectangle for IME candidate placement.
    fn ime_rect(&self) -> w::Rect {
        let percent = self.percent.get().max(1) as i64;
        let physical = |v: i64| (v * percent + 50).div_euclid(100);
        let (x, y, h) = self.caret_rect.get().unwrap_or((20, 86, 20));
        w::Rect {
            x: physical(i64::from(x)) as i32,
            y: physical(i64::from(y)) as i32,
            width: physical(1).max(1) as u32,
            height: physical(i64::from(h)).max(1) as u32,
        }
    }
    /// Text-input context and clipboard work for this window (`update`).
    pub fn window_update(&mut self, client: &mut WindowClient, window: u64) -> Result<bool, Error> {
        let mut dirty = false;
        if client.supports_text_input() && !self.ime.blocked {
            let want = self.focused && !self.modal();
            let rect = self.ime_rect();
            match self.ime.serial {
                None if want => {
                    match client.text_input_enable(window, ti::NORMAL, ti::HINT_MULTILINE, rect) {
                        Ok(serial) => {
                            self.ime.serial = Some(serial);
                            self.ime.sent = Some(rect);
                            self.ime.reset = false;
                        }
                        Err(Error::Service(_) | Error::Malformed) => self.ime.blocked = true,
                        Err(e) => return Err(e),
                    }
                }
                Some(_) if !want => {
                    if self.current.document.finish_composition() {
                        dirty = true;
                    }
                    match client.text_input_disable(window) {
                        Ok(()) | Err(Error::Service(_)) => (),
                        Err(e) => return Err(e),
                    }
                    self.ime = Ime::default();
                }
                Some(serial) => {
                    if core::mem::take(&mut self.ime.reset) {
                        match client.text_input_reset(window, serial, ti::RESET_CANCEL) {
                            Ok(()) | Err(Error::Service(_)) => (),
                            Err(e) => return Err(e),
                        }
                    }
                    if self.ime.sent != Some(rect) {
                        match client.text_input_cursor(window, serial, rect) {
                            Ok(()) | Err(Error::Service(_)) => (),
                            Err(e) => return Err(e),
                        }
                        self.ime.sent = Some(rect);
                    }
                }
                None => (),
            }
        }
        while let Some(clip) = self.clipboard.pop_front() {
            dirty |= self.run_clipboard(clip, client, window)?;
        }
        Ok(dirty)
    }
    fn run_clipboard(
        &mut self,
        clip: Clip,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        match clip {
            Clip::Paste => {
                let text = if client.supports_clipboard() {
                    match client.clipboard_info() {
                        Ok(info) => match info.size(cb::TEXT) as usize {
                            0 => None,
                            size if size > MAX_BYTES => {
                                self.current.message =
                                    "The clipboard text is larger than the 4 MiB Notes limit."
                                        .into();
                                return Ok(true);
                            }
                            size => {
                                let mut buffer = alloc::vec![0; size];
                                match client.clipboard_read_into(&info, cb::TEXT, &mut buffer) {
                                    Ok(n) => String::from_utf8(buffer[..n].to_vec()).ok(),
                                    Err(Error::Service(_)) => None,
                                    Err(e) => return Err(e),
                                }
                            }
                        },
                        Err(Error::Service(_)) => None,
                        Err(e) => return Err(e),
                    }
                } else {
                    Some(self.local_clipboard.clone())
                };
                let Some(text) = text.filter(|t| !t.is_empty()) else {
                    return Ok(false);
                };
                self.finish_composition();
                match self.current.document.paste(&text) {
                    Ok(_) => self.current.view.edited(),
                    Err(_) => self.current.message = LIMIT.into(),
                }
                Ok(true)
            }
            Clip::Copy(text) => self.store_clipboard(text, client, window).map(|_| false),
            Clip::Cut {
                text,
                range,
                revision,
            } => {
                if !self.store_clipboard(text, client, window)? {
                    return Ok(true);
                }
                if self.current.document.content_revision() == revision {
                    self.finish_composition();
                    self.current.document.select(range);
                    self.current.document.erase(true, false);
                    self.current.view.edited();
                }
                Ok(true)
            }
        }
    }
    fn store_clipboard(
        &mut self,
        text: String,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        if client.supports_clipboard() {
            match client.clipboard_set(window, &[(cb::TEXT, text.as_bytes())]) {
                Ok(_) => (),
                Err(Error::Service(_)) => {
                    self.current.message = "The text couldn't be copied to the clipboard.".into();
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        self.local_clipboard = text;
        Ok(true)
    }
    pub fn click(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        if self.modal() {
            if let Some(choice) = self
                .current
                .prompt
                .dialog
                .target(&self.dialog_layout(), x, y)
            {
                self.choose(choice, storage);
                self.reveal(fonts);
            }
            return Ok(true);
        }
        if self.menu.open() {
            if let Some(id) = self.menu.click(x, y, self.menu_items().len()) {
                self.command(self.menu_action(id), storage)?;
            }
            self.reveal(fonts);
            return Ok(true);
        }
        let c = self.chrome();
        let menus = self.menu_bar();
        let popup = |notes: &mut Self, kind: u8, x: i32| -> Result<bool, Error> {
            notes.menu_kind = kind;
            let count = notes.menu_items().len();
            let below = BAND_TOP + BAND_HEIGHT as i32;
            notes.menu.show(x, below, width, height, count);
            Ok(true)
        };
        match self.target(x, y) {
            // Drop-downs: Headings, Lists and Table open their menus.
            Some(50) => popup(self, 3, c.tool(Tool::Headings).map_or(x, |r| r.x)),
            Some(51) => popup(self, 4, c.tool(Tool::Lists).map_or(x, |r| r.x)),
            Some(56) => popup(self, 5, c.tool(Tool::Table).map_or(x, |r| r.x)),
            Some(id) if (50..58).contains(&id) => {
                self.apply_tool(Tool::ALL[(id - 50) as usize]);
                self.reveal(fonts);
                Ok(true)
            }
            Some(id) if (500..508).contains(&id) => {
                self.switch_raw((id - 500) as usize);
                self.reveal(fonts);
                Ok(true)
            }
            Some(id) if (600..608).contains(&id) => {
                self.close_tab((id - 600) as usize);
                self.reveal(fonts);
                Ok(true)
            }
            Some(10) => {
                self.new_tab();
                self.reveal(fonts);
                Ok(true)
            }
            Some(1) => popup(self, 0, menus[0].x),
            Some(40) => popup(self, 1, menus[1].x),
            Some(41) => popup(self, 2, menus[2].x),
            // Settings: the view options, anchored under the gear.
            Some(43) => popup(self, 2, c.settings().x),
            Some(42) => {
                self.whats_new_seen = true;
                self.current.message =
                    "What's new: formatting with headings, lists, links and tables.".into();
                Ok(true)
            }
            Some(44) => {
                self.markdown = !self.markdown;
                Ok(true)
            }
            Some(2) => {
                self.current.view.set_wrap(!self.current.view.wrap());
                self.reveal(fonts);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// Characters selected (counted once per revision and selection).
    fn selected_chars(&self) -> usize {
        let d = &self.current.document;
        let range = d.selection();
        let key = (d.content_revision(), range.start, range.end);
        let cached = self.selected_count.get();
        if key.0.is_some() && (cached.0, cached.1, cached.2) == key {
            return cached.3;
        }
        let count = d.chars_in(range);
        self.selected_count.set((key.0, key.1, key.2, count));
        count
    }
    /// Formatted mode's overlay on one painted row of a block: markup
    /// dimmed, bold and headings emboldened, italic slanted, strikethrough
    /// struck, links in the link colour and underlined, bullets as dots and
    /// table rules dimmed. `repaint(v, clip, color)` paints the row's text
    /// again inside `clip`. Every character keeps its advance, so the caret,
    /// selection and hit testing are those of the plain text.
    #[allow(clippy::too_many_arguments)]
    fn format_row(
        v: &mut ui::View<'_>,
        engine: &Engine<'_>,
        block: &layout::Block,
        row: usize,
        (top, origin, line_height): (i32, i32, u32),
        body: door_graphics_core::Rect,
        (kind, spans): &(Block, Vec<fmt::Span>),
        look: &Look,
        repaint: &mut dyn FnMut(&mut ui::View<'_>, door_graphics_core::Rect, [u8; 4]),
    ) {
        let rects = |range: core::ops::Range<usize>| -> Vec<door_graphics_core::Rect> {
            block
                .text_spans(engine, row, range)
                .into_iter()
                .filter_map(|(a, b)| {
                    door_graphics_core::Rect::new(
                        origin + a,
                        top,
                        (b - a).max(1) as u32,
                        line_height,
                    )
                    .ok()?
                    .intersection(body)
                })
                .collect()
        };
        let mut recolor = |v: &mut ui::View<'_>, r: door_graphics_core::Rect, color| {
            v.rect(r.x(), r.y(), r.width(), r.height(), look.body);
            repaint(v, r, color);
        };
        if *kind == Block::TableRule {
            for r in rects(0..block.text().len()) {
                recolor(v, r, look.dim);
            }
            return;
        }
        let line = line_height as i32;
        for span in spans {
            for r in rects(span.range.clone()) {
                let (x, y, w, h) = (r.x(), r.y(), r.width(), r.height());
                match span.mark {
                    Mark::Marker | Mark::Url => recolor(v, r, look.dim),
                    Mark::Link => {
                        recolor(v, r, look.link);
                        v.text_rule(x, y + line - 4, w, 1, look.link);
                    }
                    Mark::Bold | Mark::Heading => v.embolden(x, y, w, h, look.body),
                    Mark::Italic => v.shear(x, y, w, h, 3, look.body),
                    Mark::Strike => v.text_rule(x, y + line / 2 + 1, w, 1, look.text),
                }
            }
        }
        if let Block::Bullet(at) = *kind {
            for r in rects(at..at + 1) {
                v.rect(r.x(), r.y(), r.width(), r.height(), look.body);
                let (cx, cy) = (r.x() + r.width() as i32 / 2, r.y() + line / 2);
                dot(v, cx - 8, cy - 8, 2.0, look.text);
            }
        }
    }
    fn draw_body(&self, v: &mut ui::View<'_>, theme: Theme, look: &Look) {
        let fonts = v.editor_fonts();
        let engine = self.engine(fonts);
        let doc = &self.current.document;
        let view = &self.current.view;
        view.refresh(doc, &engine);
        let g = view.geometry();
        let line_height = g.line_height;
        let bottom = if self.status_bar {
            self.chrome().status_top()
        } else {
            v.height as i32
        };
        let Ok(body) = door_graphics_core::Rect::new(
            if view.hscroll() > 0 { g.x - 2 } else { 0 },
            fmt::BODY_TOP,
            if view.hscroll() > 0 {
                g.width + 4
            } else {
                v.width
            },
            (bottom - fmt::BODY_TOP).max(1) as u32,
        ) else {
            return;
        };
        let right = g.x + g.width as i32;
        let origin = g.x - view.hscroll();
        let selected = doc.selection();
        v.viewport(body, 0);
        view.visible(doc, &engine, |block, block_top, rows| {
            let top_of = |r: usize| block_top + r as i32 * line_height as i32;
            let parsed = self.markdown.then(|| fmt::parse_line(block.text()));
            if let (Some(layout), Some(system)) = (block.layout(), engine.system()) {
                let mut marks = Vec::new();
                for r in rows.clone() {
                    let (spans, edge) = block.selection(&engine, r, selected.clone());
                    for &(a, b) in &spans {
                        marks.push((origin + a, top_of(r), (b - a).max(1)));
                    }
                    if edge {
                        let from = origin
                            + spans
                                .iter()
                                .map(|s| s.1)
                                .max()
                                .unwrap_or_else(|| block.row_width(&engine, r));
                        marks.push((from, top_of(r), (right - from).max(1)));
                    }
                }
                for &(x, y, width) in &marks {
                    v.rect(x, y, width as u32, line_height, theme.selection);
                }
                v.text_layout(origin, block_top, system, layout, look.text);
                if let Some(parsed) = &parsed {
                    for r in rows.clone() {
                        Self::format_row(
                            v,
                            &engine,
                            block,
                            r,
                            (top_of(r), origin, line_height),
                            body,
                            parsed,
                            look,
                            &mut |v, clip, color| {
                                v.viewport(clip, 0);
                                v.text_layout(origin, block_top, system, layout, color);
                                v.viewport(body, 0);
                            },
                        );
                    }
                    // Formatting repainted runs under the selection: lay it again.
                    for &(x, y, width) in &marks {
                        v.rect(x, y, width as u32, line_height, theme.selection);
                    }
                }
                for &(x, y, width) in &marks {
                    let clip = door_graphics_core::Rect::new(x, y, width as u32, line_height)
                        .ok()
                        .and_then(|r| r.intersection(body));
                    if let Some(clip) = clip {
                        v.viewport(clip, 0);
                        v.text_layout(origin, block_top, system, layout, theme.selected_text);
                        v.viewport(body, 0);
                    }
                }
            } else {
                for r in rows.clone() {
                    let top = top_of(r);
                    let (window, offset) = block.atlas_window(
                        &engine,
                        r,
                        view.hscroll() - 8,
                        view.hscroll() + g.width as i32 + 8,
                    );
                    let shown = display(&block.text()[window]);
                    let (spans, edge) = block.selection(&engine, r, selected.clone());
                    let highlight = (!spans.is_empty() || edge).then(|| {
                        let left = origin
                            + spans
                                .first()
                                .map_or_else(|| block.row_width(&engine, r), |s| s.0);
                        let end = if edge {
                            right
                        } else {
                            origin + spans.first().map_or(0, |s| s.1)
                        };
                        door_graphics_core::Rect::new(
                            left,
                            top,
                            end.saturating_sub(left).max(1) as u32,
                            line_height,
                        )
                        .ok()
                        .and_then(|r| r.intersection(body))
                    });
                    let text_top = v.line_text_top(top, line_height, STYLE);
                    let x = origin + offset;
                    v.text(x, text_top, &shown, STYLE, look.text);
                    if let Some(parsed) = &parsed {
                        Self::format_row(
                            v,
                            &engine,
                            block,
                            r,
                            (top, origin, line_height),
                            body,
                            parsed,
                            look,
                            &mut |v, clip, color| {
                                v.viewport(clip, 0);
                                v.text(x, text_top, &shown, STYLE, color);
                                v.viewport(body, 0);
                            },
                        );
                    }
                    if let Some(h) = highlight.flatten() {
                        v.rect(h.x(), h.y(), h.width(), h.height(), theme.selection);
                        v.viewport(h, 0);
                        v.text(x, text_top, &shown, STYLE, theme.selected_text);
                        v.viewport(body, 0);
                    }
                }
            }
            // IME preedit: Windows' dotted underline.
            if let Some(preedit) = block.preedit() {
                for r in rows {
                    for (a, b) in block.text_spans(&engine, r, preedit.clone()) {
                        let y = top_of(r) + line_height as i32 - 3;
                        let mut x = origin + a;
                        while x < origin + b {
                            v.text_rule(x, y, 1, 1, look.text);
                            x += 2;
                        }
                    }
                }
            }
        });
        let caret = view.caret_point(doc, &engine);
        if let Some((x, y)) = caret {
            if engine.zoom() == 100 {
                v.text_caret(
                    x,
                    v.line_text_top(y, line_height, STYLE),
                    STYLE,
                    theme.caret,
                );
            } else {
                v.rect(
                    x,
                    y + 2,
                    1,
                    line_height.saturating_sub(4).max(1),
                    theme.caret,
                );
            }
        }
        self.caret_rect.set(caret.map(|(x, y)| (x, y, line_height)));
        self.percent.set(v.zoom.percent());
        v.reset_viewport();
    }
    /// The status bar: position, character count and the Formatted / Plain
    /// text toggle at the left; zoom, line ending and encoding at the right,
    /// each after a 1 x 16 px separator (`Chrome::status_columns`).
    fn draw_status(&self, v: &mut ui::View<'_>, look: &Look, zoom: u32) {
        let c = self.chrome();
        let top = c.status_top();
        v.material_rect(0, top, v.width, STATUS, look.layer, LAYER_ALPHA);
        v.rect(0, top, v.width, 1, look.stroke);
        let row = (top, STATUS);
        let fg = look.secondary;
        let s = c.status_columns();
        let doc = &self.current.document;
        let separator = |v: &mut ui::View<'_>, x: i32| v.rect(x, top + 8, 1, 16, look.divider);
        let (line, col) = doc.line_column(doc.caret());
        let position = format!("Ln {line}, Col {col}");
        v.text_clip_centered(s.position, row, 72, &position, TextStyle::Regular12, fg);
        if let Some((x, before)) = s.characters {
            separator(v, before);
            let total = doc.char_count();
            let count = if doc.selection().is_empty() {
                format!("{total} characters")
            } else {
                format!("{} of {total} characters", self.selected_chars())
            };
            v.text_clip_centered(x, row, 196, &count, TextStyle::Regular12, fg);
        }
        if let (Some((icon, x, before)), Some(toggle)) = (s.format, c.format_toggle()) {
            separator(v, before);
            if v.hover.is_some_and(|(px, py)| toggle.contains(px, py)) {
                v.rounded(toggle.x, top + 4, toggle.width, 24, 4, look.hover);
            }
            let font = door_workspace::fluent::Glyph::Font.mask(v.scale);
            glyph(v, icon, top + 10, 12, font, fg);
            let label = if self.markdown {
                "Formatted"
            } else {
                "Plain text"
            };
            v.text_clip_centered(x, row, 88, label, TextStyle::Regular12, fg);
        }
        if let Some((x, before)) = s.zoom {
            separator(v, before);
            v.text_clip_centered(x, row, 64, &format!("{zoom}%"), TextStyle::Regular12, fg);
        }
        if let Some((x, before)) = s.eol {
            separator(v, before);
            v.text_clip_centered(x, row, 150, doc.eol().label(), TextStyle::Regular12, fg);
        }
        let (x, before) = s.encoding;
        separator(v, before);
        let encoding = if doc.bom() { "UTF-8 with BOM" } else { "UTF-8" };
        v.text_clip_centered(x, row, 128, encoding, TextStyle::Regular12, fg);
    }
    /// The tab row (the title bar): tabs and "+". The selected tab is a
    /// top-rounded plate of the band's layer, joined to the band with 3 px
    /// flares; an unsaved document shows a dot in place of the close button
    /// until the tab is hovered.
    fn draw_tabs(&self, v: &mut ui::View<'_>, look: &Look) {
        let c = self.chrome();
        let caption = material(v, look.caption, 220);
        let layer = material(v, look.layer, LAYER_ALPHA);
        let hover = |v: &ui::View<'_>, r: door_workspace::explorer_layout::Bounds| {
            v.hover.is_some_and(|(x, y)| r.contains(x, y))
        };
        for i in c.range() {
            let Some(r) = c.tab(i) else { continue };
            let doc = if i == self.active {
                &self.current
            } else {
                self.tabs[i].as_ref().expect("stored document")
            };
            let selected = i == self.active;
            let hovered = hover(v, r);
            let (px, py, pw) = (r.x + 4, TAB_TOP + 1, r.width.saturating_sub(8));
            if selected {
                v.rounded(px, py, pw, (BAND_TOP - py) as u32 + 8, 8, layer);
                for (fx, cx) in [(px - 3, px - 6), (px + pw as i32, px + pw as i32)] {
                    v.rect(fx, BAND_TOP - 3, 3, 3, layer);
                    v.rounded(cx, BAND_TOP - 6, 6, 6, 3, caption);
                }
            } else if hovered {
                v.rounded(px, py + 2, pw, TAB_HEIGHT - 6, 6, look.hover);
            } else if i + 1 < c.range().end && i + 1 != self.active {
                v.rect(r.x + r.width as i32 + 1, TAB_TOP + 8, 1, 16, look.divider);
            }
            let name = doc.path.rsplit('/').next().unwrap_or("Untitled");
            let name = if name.starts_with("Untitled-") {
                "Untitled"
            } else {
                name
            };
            let close = c.close(i);
            let label_end = close.map_or(r.x + r.width as i32, |b| b.x);
            v.text_clip_centered(
                r.x + TAB_TEXT,
                (TAB_TOP, TAB_HEIGHT),
                (label_end - r.x - TAB_TEXT - 4).max(0) as u32,
                &ui::display_ascii(name),
                TextStyle::Regular12,
                look.text,
            );
            let Some(b) = close else { continue };
            let (cx, cy) = (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
            if hover(v, b) {
                v.rounded(b.x, b.y, b.width, b.height, 4, look.hover);
            }
            if doc.save_state.attention(&doc.document) && !hovered {
                dot(v, cx - 8, cy - 8, 2.7, look.text);
            } else if selected || hovered {
                let mask = match v.scale {
                    3 => door_workspace::asset!(
                        "../../../assets/icons/windows-shell/chrome-close.36.mask"
                    ),
                    2 => door_workspace::asset!(
                        "../../../assets/icons/windows-shell/chrome-close.24.mask"
                    ),
                    _ => door_workspace::asset!(
                        "../../../assets/icons/windows-shell/chrome-close.12.mask"
                    ),
                };
                v.mask_pixels(cx - 6, cy - 6, 12, mask, look.text, None);
            }
        }
        if let Some(p) = c.plus() {
            if hover(v, p) {
                v.rounded(p.x, p.y, p.width, p.height, 4, look.hover);
            }
            v.compact_icon(p.x + 8, p.y + 4, Icon::Add, look.text);
        }
    }
    /// The menu + formatting band: File, Edit and View; the centred
    /// toolbar (drop-downs with chevrons, toggles checked at the caret);
    /// What's new with its accent dot, and Settings.
    fn draw_band(&self, v: &mut ui::View<'_>, look: &Look) {
        use door_workspace::fluent::Glyph as G;
        let c = self.chrome();
        let hover = |v: &ui::View<'_>, r: door_workspace::explorer_layout::Bounds| {
            v.hover.is_some_and(|(x, y)| r.contains(x, y))
        };
        self.menu_widths
            .set(MENU_LABELS.map(|label| v.measure_style(label, TextStyle::Regular14)));
        for (i, (r, label)) in self.menu_bar().into_iter().zip(MENU_LABELS).enumerate() {
            let open = self.menu.open() && usize::from(self.menu_kind) == i;
            if open || hover(v, r) {
                v.rounded(r.x, r.y + 4, r.width, r.height - 8, 4, look.hover);
            }
            v.text_in_box(
                (r.x, r.y, r.width, r.height),
                label,
                TextStyle::Regular14,
                look.text,
            );
        }
        let checked = self.caret_marks();
        for tool in Tool::ALL {
            let Some(r) = c.tool(tool) else { continue };
            let open = self.menu.open()
                && matches!(
                    (tool, self.menu_kind),
                    (Tool::Headings, 3) | (Tool::Lists, 4) | (Tool::Table, 5)
                );
            let on = match tool {
                Tool::Bold => checked[0],
                Tool::Italic => checked[1],
                Tool::Strike => checked[2],
                _ => false,
            };
            let hovered = hover(v, r);
            if open || on || hovered {
                let fill = if on && !hovered {
                    look.pressed
                } else {
                    look.hover
                };
                v.rounded(r.x + 2, r.y + 4, r.width - 4, r.height - 8, 4, fill);
            }
            let ((gx, gy), chevron) = Chrome::glyph(tool, r);
            let s = v.scale;
            match tool {
                Tool::Headings => v.text_clip_centered(
                    gx,
                    (r.y, r.height),
                    22,
                    "H1",
                    TextStyle::Regular14,
                    look.text,
                ),
                Tool::Lists => glyph(v, gx, gy, 16, G::Processes.mask(s), look.text),
                Tool::Bold => glyph(v, gx, gy, 16, G::Bold.mask(s), look.text),
                Tool::Italic => glyph(v, gx, gy, 16, G::Italic.mask(s), look.text),
                Tool::Strike => glyph(v, gx, gy, 16, G::Strike.mask(s), look.text),
                Tool::Link => glyph(v, gx, gy, 16, G::Link.mask(s), look.text),
                Tool::Table => stroked(v, gx, gy, &fmt::table_glyph(s), look.text),
                Tool::Clear => stroked(v, gx, gy, &fmt::clear_glyph(s), look.text),
            }
            if let Some((cx, cy)) = chevron {
                v.tiny_icon(cx, cy, Icon::Chevron, look.text);
            }
        }
        let news = c.whats_new();
        if hover(v, news) {
            v.rounded(news.x, news.y, news.width, news.height, 4, look.hover);
        }
        let megaphone = fmt::megaphone_glyph(v.scale);
        stroked(v, news.x + 7, news.y + 6, &megaphone, look.text);
        if !self.whats_new_seen {
            dot(v, news.x + 16, news.y - 3, 1.75, ACCENT);
        }
        let gear = c.settings();
        let open = self.menu.open() && self.menu_kind == 2;
        if hover(v, gear) || (open && hover(v, gear)) {
            v.rounded(
                gear.x,
                gear.y + 4,
                gear.width,
                gear.height - 8,
                4,
                look.hover,
            );
        }
        glyph(
            v,
            gear.x + 7,
            gear.y + 8,
            16,
            G::Settings.mask(v.scale),
            look.text,
        );
    }
    pub fn draw(&self, v: &mut ui::View<'_>) {
        let theme = Theme::resolve(v.palette);
        let look = Look::resolve(v.palette.dark, theme);
        let (width, height) = (v.width, v.height);
        let body = height.saturating_sub(fmt::BODY_TOP as u32);
        v.rect(0, fmt::BODY_TOP, width, body, look.body);
        v.material_rect(0, 0, width, CAPTION, look.caption, 220);
        v.material_rect(0, BAND_TOP, width, BAND_HEIGHT, look.layer, LAYER_ALPHA);
        v.rect(0, SEPARATOR, width, 1, look.stroke);
        v.compact_icon(fmt::ICON.0, fmt::ICON.1, Icon::Notepad, look.text);
        self.draw_tabs(v, &look);
        self.draw_band(v, &look);
        v.with_editor_font(|v| self.draw_body(v, theme, &look));
        if self.status_bar {
            let zoom = self.engine(v.editor_fonts()).zoom();
            self.draw_status(v, &look, zoom);
        }
        v.popup(&self.menu, self.menu_items());
        let body = if !self.current.prompt_error.is_empty() {
            &self.current.prompt_error
        } else if self.current.save_state.outcome() == Outcome::Uncertain {
            "A previous save may have reached disk. Your edits are kept in this window."
        } else {
            "Your changes will be lost from this window if you don't save."
        };
        v.dialog(
            &self.current.prompt.dialog,
            &format!("Save changes to {}?", self.file_name()),
            body,
            &["Save", "Don't save", "Cancel"],
        );
    }
}
