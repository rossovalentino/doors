//! Windows 11 25H2 Start: a single scrolling page with a search pill, the
//! Pinned grid (Show all / Show less), the All section (Category, Grid or List
//! view over the installed apps) and a user/power footer. Metrics come from
//! the owner's 150 % reference (3.webp) converted to logical pixels.
extern crate alloc;
use crate::acrylic::Acrylic;
use crate::apps::{self, App, Category};
use crate::paint::{self, Palette};
use crate::search::{self, Filter, Hit};
use crate::taskbar;
use alloc::{string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::desktop::{Controls, Edge};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Category,
    Grid,
    List,
}
impl View {
    pub const ALL: [Self; 3] = [Self::Category, Self::Grid, Self::List];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Category => "Category",
            Self::Grid => "Grid",
            Self::List => "List",
        }
    }
    pub const fn index(self) -> u8 {
        match self {
            Self::Category => 0,
            Self::Grid => 1,
            Self::List => 2,
        }
    }
    pub fn from_index(i: u8) -> Self {
        match i {
            1 => Self::Grid,
            2 => Self::List,
            _ => Self::Category,
        }
    }
}
/// Every interactive Start element (hover, keyboard focus and hit testing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Search,
    Pinned(usize),
    ShowAll,
    View,
    /// App `slot` inside category card `card`.
    CategoryApp(usize, usize),
    GridApp(usize),
    ListApp(usize),
    User,
    Power,
    Chip(Filter),
    Result(usize),
    Action(usize),
    /// Start > Recent item (Search home: its Recent list).
    Recent(usize),
    /// Search home Recent list: recent search `n` (Start search history),
    /// newest first. Activating it fills the search box and runs it.
    History(usize),
    /// Search home > Top apps tile.
    TopApp(usize),
    /// Search home > Quick searches pill (`QUICK_SEARCHES`).
    QuickSearch(usize),
    /// Start > Folders next to Power: folder `n` of
    /// `preferences::start_folders::NAMES`.
    Folder(usize),
    /// Inside the panel but on no control.
    Panel,
}
/// Which shell panel the Start surface shows: Start itself, or the 26H2
/// Windows Search panel (taskbar Search, Win+S, or typing in Start). Both
/// share the search engine, results, provider and acrylic/motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Start,
    Search,
}
/// Search panel height (logical px) where the work area allows it.
pub const SEARCH_HEIGHT: i32 = 720;
/// Search home: at most this many Recent rows and Top apps tiles.
pub const SEARCH_RECENT: usize = 8;
pub const TOP_APPS: usize = 6;
/// 26H2 Search home "Quick searches" pills and the Settings pages they open.
pub const QUICK_SEARCHES: [(&str, u32); 6] = [
    ("Focus settings", crate::search::page::NOTIFICATIONS),
    ("Sound settings", crate::search::page::SOUND),
    ("Bluetooth & devices", crate::search::page::BLUETOOTH),
    ("Display settings", crate::search::page::DISPLAY),
    ("Color settings", crate::search::page::COLORS),
    ("Search settings", crate::search::page::PRIVACY),
];
/// Quick searches card metrics (`reports/artifacts/reference-26h2/_crop-search.png`,
/// px / 1.5): 13 px side and 17 px top/bottom padding, 30 px pills 13 px
/// wider than their label on each side, 11 px apart, rows 42 px apart.
const QUICK_PAD_X: i32 = 13;
const QUICK_PAD_Y: i32 = 17;
const QUICK_PILL: i32 = 30;
const QUICK_PITCH: i32 = 42;
const QUICK_GAP: i32 = 11;
/// Top apps plates: 91 px tall, 9 px apart.
const PLATE_HEIGHT: i32 = 91;
const PLATE_GAP: i32 = 9;
/// Row content of the List view: a letter header or an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListRow {
    Letter(u8),
    App(apps::Entry),
}
pub fn list_rows() -> Vec<ListRow> {
    let mut rows = Vec::new();
    let mut letter = 0u8;
    for app in apps::entries_alphabetical() {
        let first = app.label().as_bytes()[0].to_ascii_uppercase();
        if first != letter {
            letter = first;
            rows.push(ListRow::Letter(first));
        }
        rows.push(ListRow::App(app));
    }
    rows
}
pub const HEADER: i32 = 65;
pub const FOOTER: i32 = 65;
const TILE_PITCH_X: i32 = 96;
const TILE_PITCH_Y: i32 = 84;
const CARD_GAP: i32 = 30;
const LIST_ROW: i32 = 40;
const LETTER_ROW: i32 = 32;
/// Search filter chip padding (each side) and spacing.
const CHIP_PAD: i32 = 12;
const CHIP_GAP: i32 = 8;
/// Recent (26H2): header plus its empty-state line.
const RECENT_BODY: i32 = 48;
/// Recent items: two columns of 52 px items on a 56 px row pitch.
const RECENT_ROW: i32 = 56;
const RECENT_ITEM: i32 = 52;
pub const MAX_RECENT: usize = 6;
// Accessibility > Text size: every band that holds text grows by the growth
// of its own style's line box (`paint::text_grow`, identity at 100%); app
// icons, glyph buttons, chips' radius and the panel width keep their size.
/// Growth of one text row of `style` (0 at 100%).
fn grow(style: TextStyle) -> i32 {
    paint::text_grow(0, style) as i32
}
/// Header band (search box) height.
pub fn header_height() -> i32 {
    HEADER + grow(TextStyle::Regular14)
}
/// Footer band (account name, power) height.
pub fn footer_height() -> i32 {
    FOOTER + grow(TextStyle::Regular14)
}
/// Pinned/grid app tile: 32 px icon over a one-line Regular12 label band.
fn tile_height() -> i32 {
    80 + grow(TextStyle::Regular12)
}
fn tile_pitch_y() -> i32 {
    TILE_PITCH_Y + grow(TextStyle::Regular12)
}
/// The label band inside a tile (`r.y() + 44`).
fn tile_label_height() -> u32 {
    paint::text_grow(20, TextStyle::Regular12)
}
/// Section header row ("Pinned", "Recent", "All") height, and the extra
/// half-row above its centre line.
fn section_height() -> u32 {
    paint::text_grow(20, TextStyle::Semibold14)
}
fn section_half() -> i32 {
    grow(TextStyle::Semibold14) / 2
}
fn section_below() -> i32 {
    grow(TextStyle::Semibold14) - section_half()
}
fn list_row() -> i32 {
    paint::text_grow_i(LIST_ROW, TextStyle::Regular14)
}
fn letter_row() -> i32 {
    paint::text_grow_i(LETTER_ROW, TextStyle::Semibold14)
}
/// Recent items hold a Regular14 name over a Regular12 location.
fn recent_item_height() -> i32 {
    RECENT_ITEM + grow(TextStyle::Regular14) + grow(TextStyle::Regular12)
}
fn recent_row() -> i32 {
    recent_item_height() + (RECENT_ROW - RECENT_ITEM)
}
/// Search result rows: (Best match header, best match, group header, row).
fn result_heights() -> (i32, i32, i32, i32) {
    (
        paint::text_grow_i(24, TextStyle::Semibold14),
        64 + grow(TextStyle::Semibold14) + grow(TextStyle::Regular12),
        paint::text_grow_i(32, TextStyle::Semibold14),
        list_row(),
    )
}
/// Space above the Explorer row for "No results" and its explanation.
pub fn no_results_height() -> i32 {
    NO_RESULTS_HEIGHT + grow(TextStyle::Semibold14) + grow(TextStyle::Regular12)
}
/// Pure Start geometry for one output, controls and content state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub panel: Rect,
    pub columns: usize,
    pub category_columns: usize,
    pins: usize,
    rows_shown: usize,
    pub view: View,
    pub expanded: bool,
    /// 26H2 section toggles (Personalization > Start).
    pub show_pinned: bool,
    pub show_recent: bool,
    pub show_all: bool,
    pub show_account: bool,
    /// Recent items shown (none with "Show recently opened files" off).
    recent: usize,
    /// The 26H2 Search panel: no account/Power footer, fixed height.
    pub search: bool,
}
impl Geometry {
    /// Default content: every bundled app pinned, Category view.
    pub fn new(layout: taskbar::Layout) -> Self {
        Self::with(layout, apps::COUNT, View::Category, false)
    }
    /// The 26H2 Windows Search panel: Start's width and anchor, a fixed
    /// height (720 px, or the work area less margins), no footer.
    pub fn search_panel(layout: taskbar::Layout) -> Self {
        let mut g = Self::new(layout);
        g.search = true;
        g.show_account = false;
        let area = layout.work_area();
        let max = (area.height() as i32 - 24).clamp(200, 866);
        let height = SEARCH_HEIGHT.min(max) as u32;
        g.panel = place(layout, g.panel.width(), height);
        g
    }
    pub fn with(layout: taskbar::Layout, pins: usize, view: View, expanded: bool) -> Self {
        Self::with_recent(layout, pins, view, expanded, 0)
    }
    /// With `recent` provider items for the Recent section.
    pub fn with_recent(
        layout: taskbar::Layout,
        pins: usize,
        view: View,
        expanded: bool,
        recent: usize,
    ) -> Self {
        let area = layout.work_area();
        let controls = layout.controls();
        // Start size (26H2): Automatic adapts to the display (8 pinned and 4
        // category columns on larger screens, 6 and 3 on smaller ones);
        // Small and Large fix it, Large only where 8 columns fit.
        let fits_large = area.width() >= 8 * TILE_PITCH_X as u32 + 64 + 24;
        let large = match controls.start_size() {
            1 => false,
            2 => fits_large,
            _ => area.width() >= 1600 && area.height() >= 800,
        };
        let columns = if large { 8 } else { 6 };
        let category_columns = if large { 4 } else { 3 };
        // 26H2 has no Start Layout setting: Pinned shows two rows until
        // Show all. Legacy Start Layout bits (7..=8) are ignored.
        let collapsed = 2;
        let total_rows = pins.div_ceil(columns);
        let rows_shown = if expanded {
            total_rows
        } else {
            total_rows.min(collapsed)
        };
        let width = columns as u32 * TILE_PITCH_X as u32 + 64;
        let mut g = Self {
            panel: Rect::new(0, 0, width, 400).unwrap(),
            columns,
            category_columns,
            pins,
            rows_shown,
            view,
            expanded,
            show_pinned: !controls.has(Controls::START_HIDE_PINNED),
            show_recent: !controls.has(Controls::START_HIDE_RECENT),
            show_all: !controls.has(Controls::START_HIDE_ALL),
            show_account: !controls.has(Controls::START_HIDE_ACCOUNT),
            recent: if controls.has(Controls::START_HIDE_RECENT_FILES) {
                0
            } else {
                recent.min(MAX_RECENT)
            },
            search: false,
        };
        let content = g.content_height();
        let max = (area.height() as i32 - 24).clamp(200, 866);
        let height = (header_height() + content + footer_height()).clamp(320.min(max), max) as u32;
        let width = width.min(area.width().saturating_sub(24)).max(320);
        g.panel = place(layout, width, height);
        g
    }
    pub fn width(&self) -> i32 {
        self.panel.width() as i32
    }
    pub fn search_box(&self) -> Rect {
        let h = paint::text_grow(32, TextStyle::Regular14);
        Rect::new(
            self.panel.x() + 32,
            self.panel.y() + 16,
            self.panel.width() - 64,
            h,
        )
        .unwrap()
    }
    /// Top of the footer band (the panel bottom in the Search panel).
    fn footer_top(&self) -> i32 {
        if self.search {
            self.panel.bottom()
        } else {
            self.panel.bottom() - footer_height()
        }
    }
    pub fn body(&self) -> Rect {
        Rect::new(
            self.panel.x(),
            self.panel.y() + header_height(),
            self.panel.width(),
            (self.footer_top() - self.panel.y() - header_height()).max(1) as u32,
        )
        .unwrap()
    }
}
/// Anchor a `width` x `height` panel where Windows opens Start: above (or
/// beside) the taskbar, centred or start-aligned with it, 12 px margins.
fn place(layout: taskbar::Layout, width: u32, height: u32) -> Rect {
    let area = layout.work_area();
    {
        let vertical = layout.vertical();
        let x = if vertical {
            if layout.controls().edge() == Edge::Left {
                area.x() + 12
            } else {
                area.right() - 12 - width as i32
            }
        } else if layout.controls().start_aligned() {
            area.x() + 12
        } else {
            area.x() + (area.width() - width) as i32 / 2
        };
        let y = match layout.controls().edge() {
            Edge::Bottom => area.bottom() - 12 - height as i32,
            Edge::Top => area.y() + 12,
            _ => {
                if layout.controls().start_aligned() {
                    area.y() + 12
                } else {
                    area.y() + (area.height() - height) as i32 / 2
                }
            }
        };
        Rect::new(x, y, width, height).unwrap()
    }
}
impl Geometry {
    pub fn footer(&self) -> Rect {
        Rect::new(
            self.panel.x(),
            self.panel.bottom() - footer_height(),
            self.panel.width(),
            footer_height() as u32,
        )
        .unwrap()
    }
    pub fn pinned_total_rows(&self) -> usize {
        self.pins.div_ceil(self.columns)
    }
    pub fn shows_toggle(&self) -> bool {
        self.show_pinned
            && (self.pinned_total_rows() > self.rows_shown
                || self.expanded && self.pinned_total_rows() > 2)
    }
    /// Header-row centers of the shown sections (content coordinates): the
    /// first section's header sits 39 px below the body top, later ones 41 px
    /// below the previous section's content (3.webp metrics).
    fn sections(&self) -> (Option<i32>, Option<i32>, Option<i32>, i32) {
        // A grown header row extends `above` over its centre line and
        // `below` under it; the section content follows its bottom.
        let (above, below) = (section_half(), section_below());
        let mut bottom = None::<i32>;
        let mut next = |height_below_center: i32| {
            let center = bottom.map_or(39 + above, |b| b + 41 + above);
            bottom = Some(center + height_below_center);
            center
        };
        let pinned = self.show_pinned.then(|| {
            next(if self.pins == 0 {
                34 + below + grow(TextStyle::Regular12)
            } else {
                32 + below + self.rows_shown as i32 * tile_pitch_y()
            })
        });
        let recent = self.show_recent.then(|| next(self.recent_body()));
        let all = self.show_all.then(|| next(28 + below + self.view_height()));
        (pinned, recent, all, bottom.unwrap_or(0))
    }
    fn pinned_center(&self) -> i32 {
        self.sections().0.unwrap_or(39 + section_half())
    }
    /// Center of the Recent header row (content coordinates).
    pub fn recent_center(&self) -> Option<i32> {
        self.sections().1
    }
    /// Center of the All header row (content coordinates).
    fn all_center(&self) -> i32 {
        self.sections().2.unwrap_or(39 + section_half())
    }
    fn all_top(&self) -> i32 {
        self.all_center() + 28 + section_below()
    }
    /// A section header row centred (by design) on content y `center`.
    fn section_header(&self, center: i32, scroll: i32) -> Rect {
        Rect::new(
            self.panel.x() + 64,
            self.y(center - 10 - section_half(), scroll),
            self.panel.width() - 128,
            section_height(),
        )
        .unwrap()
    }
    pub fn recent_header(&self, scroll: i32) -> Option<Rect> {
        let center = self.recent_center()?;
        Some(self.section_header(center, scroll))
    }
    fn recent_body(&self) -> i32 {
        section_below()
            + if self.recent == 0 {
                RECENT_BODY + grow(TextStyle::Regular12)
            } else {
                18 + self.recent.div_ceil(2) as i32 * recent_row() - (RECENT_ROW - RECENT_ITEM)
            }
    }
    /// Recent items shown.
    pub fn recent_count(&self) -> usize {
        if self.show_recent { self.recent } else { 0 }
    }
    /// Recent item `index` (two columns, newest first, left to right).
    pub fn recent_item(&self, index: usize, scroll: i32) -> Option<Rect> {
        if index >= self.recent_count() {
            return None;
        }
        let center = self.recent_center()?;
        let width = (self.panel.width() as i32 - 104 - 8) / 2;
        let x = self.panel.x() + 52 + (index % 2) as i32 * (width + 8);
        let top = center + 18 + section_below() + (index / 2) as i32 * recent_row();
        Rect::new(
            x,
            self.y(top, scroll),
            width.max(1) as u32,
            recent_item_height() as u32,
        )
        .ok()
    }
    pub fn card_size(&self) -> i32 {
        let width = self.width() - 128;
        (width - (self.category_columns as i32 - 1) * CARD_GAP) / self.category_columns as i32
    }
    fn view_height(&self) -> i32 {
        match self.view {
            View::Category => {
                let rows = apps::categories().len().div_ceil(self.category_columns) as i32;
                rows * (self.card_size() + 52 + grow(TextStyle::Regular12)) - 19
            }
            View::Grid => apps::COUNT.div_ceil(self.columns) as i32 * tile_pitch_y(),
            View::List => list_rows()
                .iter()
                .map(|r| match r {
                    ListRow::Letter(_) => letter_row(),
                    ListRow::App(_) => list_row(),
                })
                .sum(),
        }
    }
    /// Scrollable content height of the body.
    pub fn content_height(&self) -> i32 {
        self.sections().3 + 24
    }
    pub fn max_scroll(&self) -> i32 {
        if self.search {
            return 0;
        }
        (self.content_height() - self.body().height() as i32).max(0)
    }
    /// Content y (unscrolled) → logical y.
    fn y(&self, content_y: i32, scroll: i32) -> i32 {
        self.panel.y() + header_height() + content_y - scroll
    }
    pub fn pinned_header(&self, scroll: i32) -> Rect {
        self.section_header(self.pinned_center(), scroll)
    }
    pub fn all_header(&self, scroll: i32) -> Rect {
        self.section_header(self.all_center(), scroll)
    }
    /// "Show all"/"Show less" or "View: ..." button, right aligned to the
    /// content column, `text` logical px wide plus a 12 px chevron.
    pub fn header_button(&self, header: Rect, text: u32) -> Rect {
        let w = text + 8 + 12 + 16;
        let h = paint::text_grow(28, TextStyle::Regular14);
        Rect::new(
            header.right() + 8 - w as i32,
            header.y() + (header.height() as i32 - h as i32) / 2,
            w,
            h,
        )
        .unwrap()
    }
    pub fn pinned_tile(&self, index: usize, scroll: i32) -> Option<Rect> {
        let row = index / self.columns;
        if !self.show_pinned || index >= self.pins || row >= self.rows_shown {
            return None;
        }
        Rect::new(
            self.panel.x() + 32 + (index % self.columns) as i32 * TILE_PITCH_X + 4,
            self.y(
                self.pinned_center() + 32 + section_below() + row as i32 * tile_pitch_y(),
                scroll,
            ),
            88,
            tile_height() as u32,
        )
        .ok()
    }
    pub fn card(&self, index: usize, scroll: i32) -> Rect {
        let s = self.card_size();
        let col = (index % self.category_columns) as i32;
        let row = (index / self.category_columns) as i32;
        Rect::new(
            self.panel.x() + 64 + col * (s + CARD_GAP),
            self.y(
                self.all_top() + row * (s + 52 + grow(TextStyle::Regular12)),
                scroll,
            ),
            s as u32,
            s as u32,
        )
        .unwrap()
    }
    /// One of the 2×2 icon cells in a category card (48×48 hover cell).
    pub fn card_cell(&self, card: Rect, slot: usize) -> Rect {
        let s = card.width() as i32;
        let cx = card.x()
            + if slot.is_multiple_of(2) {
                s * 28 / 100
            } else {
                s * 72 / 100
            };
        let cy = card.y() + if slot < 2 { s * 28 / 100 } else { s * 72 / 100 };
        Rect::new(cx - 24, cy - 24, 48, 48).unwrap()
    }
    pub fn grid_tile(&self, index: usize, scroll: i32) -> Rect {
        Rect::new(
            self.panel.x() + 32 + (index % self.columns) as i32 * TILE_PITCH_X + 4,
            self.y(
                self.all_top() - 8 + (index / self.columns) as i32 * tile_pitch_y(),
                scroll,
            ),
            88,
            tile_height() as u32,
        )
        .unwrap()
    }
    /// List rows in order with their rectangles.
    pub fn list(&self, scroll: i32) -> Vec<(ListRow, Rect)> {
        let mut y = self.all_top() - 8;
        list_rows()
            .into_iter()
            .map(|row| {
                let h = match row {
                    ListRow::Letter(_) => letter_row(),
                    ListRow::App(_) => list_row(),
                };
                let r = Rect::new(
                    self.panel.x() + 52,
                    self.y(y, scroll),
                    self.panel.width() - 104,
                    h as u32,
                )
                .unwrap();
                y += h;
                (row, r)
            })
            .collect()
    }
    /// Account button: avatar plus the account name; it grows with the
    /// name's text, the 40 px power button stays centred beside it.
    pub fn user_button(&self, name_width: u32) -> Rect {
        let h = paint::text_grow_i(40, TextStyle::Regular14);
        Rect::new(
            self.panel.x() + 52,
            self.panel.bottom() - 12 - h,
            12 + 32 + 14 + name_width + 14,
            h as u32,
        )
        .unwrap()
    }
    /// Folder button `rank` of `count` shown, left of Power, 44 px apart.
    pub fn folder_button(&self, rank: usize, count: usize) -> Rect {
        let p = self.power_button();
        let x = p.x() - (count - rank) as i32 * 44;
        Rect::new(x, p.y(), 40, 40).unwrap()
    }
    pub fn power_button(&self) -> Rect {
        let h = paint::text_grow_i(40, TextStyle::Regular14);
        Rect::new(
            self.panel.right() - 92,
            self.panel.bottom() - 12 - h + (h - 40) / 2,
            40,
            40,
        )
        .unwrap()
    }
    // Search results.
    /// Search filter chip: the label plus 12 px on each side, 32 px tall,
    /// 8 px apart (Windows 11 Start search pills).
    pub fn chip(&self, fonts: &FontAtlas, filter: Filter) -> Rect {
        let width = |f: Filter| {
            paint::measure(fonts, f.label(), TextStyle::Regular14) as i32 + 2 * CHIP_PAD
        };
        let x: i32 = Filter::ALL
            .iter()
            .take_while(|f| **f != filter)
            .map(|f| width(*f) + CHIP_GAP)
            .sum();
        Rect::new(
            self.panel.x() + 32 + x,
            self.panel.y() + header_height() + 12,
            width(filter) as u32,
            paint::text_grow(32, TextStyle::Regular14),
        )
        .unwrap()
    }
    /// Top of the results and details columns, under the (grown) chips.
    fn results_top(&self) -> i32 {
        self.panel.y() + header_height() + 60 + grow(TextStyle::Regular14)
    }
    fn split(&self) -> i32 {
        self.panel.x() + 24 + (self.panel.width() as i32 - 48) * 52 / 100
    }
    /// Left column of search results: (x, top, width).
    pub fn results_column(&self) -> (i32, i32, u32) {
        let x = self.panel.x() + 24;
        (x, self.results_top(), (self.split() - 12 - x) as u32)
    }
    pub fn details(&self) -> Rect {
        let x = self.split();
        let top = self.results_top();
        Rect::new(
            x,
            top,
            (self.panel.right() - 24 - x) as u32,
            (self.footer_top() - 16 - top).max(120) as u32,
        )
        .unwrap()
    }
    /// Details pane text bands: (title, kind line, divider y), grown with
    /// the text size under the fixed-size result icon.
    pub fn details_text(&self) -> (Rect, Rect, i32) {
        let d = self.details();
        let g20 = grow(TextStyle::Semibold20);
        let title = Rect::new(
            d.x() + 8,
            d.y() + 84,
            d.width() - 16,
            paint::text_grow(28, TextStyle::Semibold20),
        )
        .unwrap();
        let kind = Rect::new(
            d.x() + 8,
            d.y() + 114 + g20,
            d.width() - 16,
            paint::text_grow(18, TextStyle::Regular12),
        )
        .unwrap();
        (title, kind, d.y() + 152 + g20 + grow(TextStyle::Regular12))
    }
    pub fn action(&self, index: usize) -> Rect {
        let d = self.details();
        let (_, _, divider) = self.details_text();
        let h = paint::text_grow_i(36, TextStyle::Regular14);
        Rect::new(
            d.x() + 8,
            divider + 16 + index as i32 * h,
            d.width() - 16,
            h as u32,
        )
        .unwrap()
    }
}
impl Geometry {
    /// The pill fits the row (16 px clear of the panel edge); at large
    /// text sizes the trailing pills that do not fit are not shown.
    pub fn chip_fits(&self, fonts: &FontAtlas, filter: Filter) -> bool {
        self.chip(fonts, filter).right() <= self.panel.right() - 16
    }
    /// Search home top (no filter pills before typing, as in 26H2).
    fn home_top(&self) -> i32 {
        self.panel.y() + header_height() + 12
    }
    /// Search home right column: from 42.5 % of the panel to 27 px before
    /// its right edge (833 px panel: x 354..806).
    fn home_right(&self) -> (i32, u32) {
        let x = self.panel.x() + self.panel.width() as i32 * 425 / 1000;
        (x, (self.panel.right() - 27 - x).max(1) as u32)
    }
    /// Search home: "Recent" header over the left column.
    pub fn home_recent_header(&self) -> Rect {
        let x = self.panel.x() + 24;
        let (right, _) = self.home_right();
        Rect::new(
            x,
            self.home_top(),
            (right - 24 - x).max(1) as u32,
            result_heights().2 as u32,
        )
        .unwrap()
    }
    /// Search home: "Quick searches" header over the right column.
    pub fn quick_header(&self) -> Rect {
        let (x, w) = self.home_right();
        Rect::new(x, self.home_top(), w, result_heights().2 as u32).unwrap()
    }
    /// Pill rows the Quick searches card holds: two in the wide 26H2
    /// column, three in a narrower one.
    fn quick_rows(&self) -> i32 {
        if self.home_right().1 >= 420 { 2 } else { 3 }
    }
    /// The Quick searches card under its header.
    pub fn quick_card(&self) -> Rect {
        let header = self.quick_header();
        let rows = self.quick_rows();
        let pill = paint::text_grow_i(QUICK_PILL, TextStyle::Regular14);
        let height = 2 * QUICK_PAD_Y + (rows - 1) * (QUICK_PITCH - QUICK_PILL + pill) + pill;
        Rect::new(
            header.x(),
            header.bottom() + 8,
            header.width(),
            height as u32,
        )
        .unwrap()
    }
    /// Quick search pill `index`: label width plus padding, wrapped into
    /// the card's rows; None when it does not fit them.
    pub fn quick_pill(&self, fonts: &FontAtlas, index: usize) -> Option<Rect> {
        let card = self.quick_card();
        let pill = paint::text_grow_i(QUICK_PILL, TextStyle::Regular14);
        let pitch = QUICK_PITCH - QUICK_PILL + pill;
        if self.quick_rows() == 3 {
            // A narrower column than 26H2's: two equal pills per row.
            if index >= QUICK_SEARCHES.len() {
                return None;
            }
            let w = (card.width() as i32 - 2 * QUICK_PAD_X - QUICK_GAP) / 2;
            let (col, row) = ((index % 2) as i32, (index / 2) as i32);
            return Rect::new(
                card.x() + QUICK_PAD_X + col * (w + QUICK_GAP),
                card.y() + QUICK_PAD_Y + row * pitch,
                w.max(1) as u32,
                pill as u32,
            )
            .ok();
        }
        let (mut x, mut row) = (card.x() + QUICK_PAD_X, 0);
        for (i, (label, _)) in QUICK_SEARCHES.iter().enumerate() {
            let w = paint::measure(fonts, label, TextStyle::Regular14) as i32 + 2 * QUICK_PAD_X;
            let w = w.min(card.width() as i32 - 2 * QUICK_PAD_X);
            if x + w > card.right() - QUICK_PAD_X {
                row += 1;
                x = card.x() + QUICK_PAD_X;
            }
            if row >= self.quick_rows() {
                return None;
            }
            if i == index {
                return Rect::new(
                    x,
                    card.y() + QUICK_PAD_Y + row * pitch,
                    w as u32,
                    pill as u32,
                )
                .ok();
            }
            x += w + QUICK_GAP;
        }
        None
    }
    /// Search home Recent row `index` (one line, 40 px), None past the body.
    pub fn home_recent(&self, index: usize) -> Option<Rect> {
        if index >= SEARCH_RECENT {
            return None;
        }
        let header = self.home_recent_header();
        let h = list_row();
        let r = Rect::new(
            header.x(),
            header.bottom() + index as i32 * h,
            header.width(),
            h as u32,
        )
        .ok()?;
        (r.bottom() <= self.body().bottom() - 8).then_some(r)
    }
    /// Search home: "Top apps" header under the Quick searches card.
    pub fn top_apps_header(&self) -> Rect {
        let card = self.quick_card();
        Rect::new(
            card.x(),
            card.bottom() + 16,
            card.width(),
            result_heights().2 as u32,
        )
        .unwrap()
    }
    /// Search home Top apps plate `index`: three per row filling the column
    /// (26H2: 146 x 91 plates 9 px apart), icon and label centred.
    pub fn top_app(&self, index: usize) -> Option<Rect> {
        if index >= TOP_APPS {
            return None;
        }
        let header = self.top_apps_header();
        let w = ((header.width() as i32 - 2 * PLATE_GAP) / 3).max(1);
        let h = PLATE_HEIGHT + grow(TextStyle::Regular12);
        let r = Rect::new(
            header.x() + (index % 3) as i32 * (w + PLATE_GAP),
            header.bottom() + 8 + (index / 3) as i32 * (h + PLATE_GAP),
            w as u32,
            h as u32,
        )
        .ok()?;
        (r.bottom() <= self.body().bottom() - 8).then_some(r)
    }
}
/// Live Start state owned by the shell.
pub struct Start {
    pub query: String,
    /// Byte caret in `query` (grapheme boundary).
    pub query_caret: usize,
    /// IME composition at the caret (not part of the query).
    pub preedit: crate::text_edit::Preedit,
    pub filter: Filter,
    pub view: View,
    pub expanded: bool,
    pub scroll: i32,
    pub focus: Option<Target>,
    pub selected: usize,
    /// Apps pinned to Start, in order.
    pub pins: Vec<App>,
    /// Text caret visible (search box focus).
    pub caret: bool,
    pub backdrop: Acrylic,
    /// Provider results for the current query (files and folders).
    pub files: Vec<FileResult>,
    pub file_search: FileSearch,
    /// Start > Recent items from the provider, newest first.
    pub recent: Vec<FileResult>,
    /// Recent searches from the provider, newest first (empty while
    /// "Search history on this device" is off).
    pub history: Vec<String>,
    /// Personalization > Start > Folders (`preferences::START_FOLDERS`).
    pub folders: u32,
    /// Start or the Search panel.
    pub mode: Mode,
    /// The Search panel was entered by typing in Start: clearing the query
    /// (Backspace or Esc) returns to Start, as in Windows.
    pub from_start: bool,
    /// Search-only tools (bit = `Tool::index`) that cannot start now: every
    /// kernel process slot is taken. Their results say so and activating
    /// them launches nothing.
    pub blocked_tools: u16,
    /// Registered apps the desktop shell published (door.shell catalog):
    /// searchable and launchable through the shell.
    pub catalog: Vec<CatalogApp>,
}
/// One registered app of the desktop shell's catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogApp {
    pub key: u32,
    pub icon: u16,
    pub label: String,
}
/// What a blocked tool's result says instead of its kind.
pub const TOOL_NO_ROOM: &str = "Close an app first: Doors runs at most 32 processes";
const _: () = assert!(door_abi::telemetry::MAX_PROCESSES == 32);
/// One file or folder result from the search provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileResult {
    pub folder: bool,
    pub name: String,
    /// Windows-style location (display only; the provider opens it).
    pub path: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FileSearch {
    /// No provider is connected: show the honest placeholder row.
    #[default]
    Unavailable,
    /// A query is in flight (debounced or awaiting results).
    Searching,
    /// Results arrived; `partial` when the provider's index is incomplete.
    Ready { partial: bool },
}
/// Space above the File Explorer row for the "No results" message.
pub const NO_RESULTS_HEIGHT: i32 = 56;
/// Search result list rows: group headers and results (by result index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    Result(usize),
}
impl Default for Start {
    fn default() -> Self {
        Self {
            query: String::new(),
            query_caret: 0,
            preedit: crate::text_edit::Preedit::default(),
            filter: Filter::All,
            view: View::Category,
            expanded: false,
            scroll: 0,
            focus: Some(Target::Search),
            selected: 0,
            pins: App::ALL.to_vec(),
            caret: true,
            backdrop: Acrylic::default(),
            files: Vec::new(),
            file_search: FileSearch::Unavailable,
            recent: Vec::new(),
            history: Vec::new(),
            folders: 0,
            mode: Mode::Start,
            from_start: false,
            blocked_tools: 0,
            catalog: Vec::new(),
        }
    }
}
/// What activating a target should do; the shell performs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    Launch(App),
    /// Terminal on a tab of this shell.
    Shell(apps::Shell),
    /// The same, as administrator: User Account Control asks first.
    RunAsAdministrator(apps::Shell),
    /// A search-only tool (gallery, Captions).
    LaunchTool(apps::Tool),
    /// A registered app of the desktop shell (launched by the shell).
    LaunchRegistered(u32),
    Settings(u32),
    /// Ask the provider to open file result `n` (its location for files).
    OpenFile(usize),
    /// "Open file location": File Explorer at the file's folder.
    OpenFileLocation(usize),
    /// Ask the provider to open File Explorer running the query.
    ExploreFiles,
    /// Open Start > Recent item `n` through the provider.
    OpenRecent(usize),
    PowerMenu,
    /// Open Start folder `n` (`preferences::start_folders::NAMES`).
    Folder(usize),
    UserMenu,
    ViewMenu,
    Redraw,
    Nothing,
}
pub const USER_NAME: &str = "Local account";
impl Start {
    /// The folders shown next to Power, in Start's order.
    pub fn shown_folders(&self) -> Vec<usize> {
        (0..door_service_protocol::preferences::start_folders::NAMES.len())
            .filter(|i| self.folders & (1 << i) != 0)
            .collect()
    }
    pub fn geometry(&self, layout: taskbar::Layout) -> Geometry {
        if self.mode == Mode::Search {
            return Geometry::search_panel(layout);
        }
        Geometry::with_recent(
            layout,
            self.pins.len(),
            self.view,
            self.expanded,
            self.recent.len(),
        )
    }
    /// Reset transient state when Start opens.
    pub fn open(&mut self) {
        self.open_mode(Mode::Start);
    }
    /// Reset transient state when Start or the Search panel opens (also
    /// when one switches to the other while the surface is shown).
    pub fn open_mode(&mut self, mode: Mode) {
        self.query.clear();
        self.query_caret = 0;
        self.preedit = Default::default();
        self.filter = Filter::All;
        self.scroll = 0;
        self.focus = Some(Target::Search);
        self.selected = 0;
        self.caret = true;
        self.mode = mode;
        self.from_start = false;
    }
    /// The surface shows the Search panel.
    pub fn search_panel(&self) -> bool {
        self.mode == Mode::Search
    }
    pub fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }
    /// Search home Recent list: (recent searches, recent items) shown in the
    /// rows that fit; searches come first.
    pub fn home_rows(&self, g: &Geometry) -> (usize, usize) {
        let fit = (0..SEARCH_RECENT)
            .take_while(|i| g.home_recent(*i).is_some())
            .count();
        crate::preferences::recent_rows(self.history.len(), self.recent.len(), fit)
    }
    /// Search home Recent rows shown (provider items that fit).
    pub fn home_recent_count(&self, g: &Geometry) -> usize {
        self.home_rows(g).1
    }
    /// Search home recent searches shown.
    pub fn home_history_count(&self, g: &Geometry) -> usize {
        self.home_rows(g).0
    }
    /// Search home Top apps: Start's first pinned apps (Doors keeps no app
    /// usage history), at most six.
    pub fn top_apps(&self) -> &[App] {
        &self.pins[..self.pins.len().min(TOP_APPS)]
    }
    /// Ranked apps and Settings, then provider documents and folders, then
    /// "Search in File Explorer" (or the honest no-provider row).
    pub fn results(&self) -> Vec<Hit> {
        let registered: Vec<(&str, &[&str])> = self
            .catalog
            .iter()
            .map(|a| (a.label.as_str(), &[][..]))
            .collect();
        let mut r = search::search_with(&self.query, self.filter, &registered);
        if self.searching()
            && matches!(
                self.filter,
                Filter::All | Filter::Documents | Filter::Folders
            )
        {
            match self.file_search {
                FileSearch::Unavailable => r.push(Hit::FilesUnavailable),
                _ => {
                    for folders in [false, true] {
                        for (i, f) in self.files.iter().enumerate() {
                            if f.folder == folders && i < 256 && self.filter.shows_file(f.folder) {
                                r.push(Hit::File(i as u8));
                            }
                        }
                    }
                    r.push(Hit::Explore);
                }
            }
        }
        r
    }
    pub fn file(&self, hit: Hit) -> Option<&FileResult> {
        match hit {
            Hit::File(i) => self.files.get(i as usize),
            _ => None,
        }
    }
    pub fn title(&self, hit: Hit) -> &str {
        if let Hit::Registered(i) = hit {
            return self
                .catalog
                .get(usize::from(i))
                .map_or("", |a| a.label.as_str());
        }
        self.file(hit).map_or(hit.title(), |f| f.name.as_str())
    }
    pub fn kind(&self, hit: Hit) -> &str {
        if let Hit::Tool(tool) = hit
            && self.blocked_tools & (1 << tool.index()) != 0
        {
            return TOOL_NO_ROOM;
        }
        match self.file(hit) {
            Some(f) => apps::FileIcon::for_name(&f.name, f.folder).label(),
            None => hit.kind(),
        }
    }
    fn group(&self, hit: Hit) -> &'static str {
        match hit {
            Hit::App(_) | Hit::Shell(_) | Hit::Tool(_) | Hit::Registered(_) => "Apps",
            Hit::Settings(_) => "Settings",
            Hit::File(_) if self.file(hit).is_some_and(|f| f.folder) => "Folders",
            Hit::File(_) => "Documents",
            Hit::Explore | Hit::FilesUnavailable => "",
        }
    }
    /// The query matched an app, a Settings page or a file (a best match).
    pub fn has_matches(&self) -> bool {
        self.results().iter().any(|h| {
            matches!(
                h,
                Hit::App(_)
                    | Hit::Shell(_)
                    | Hit::Tool(_)
                    | Hit::Registered(_)
                    | Hit::Settings(_)
                    | Hit::File(_)
            )
        })
    }
    /// "No results" is shown above the File Explorer row: nothing matched
    /// and no provider answer is still pending.
    pub fn no_results(&self) -> bool {
        self.searching() && !self.has_matches() && self.file_search != FileSearch::Searching
    }
    /// A row is chosen: there is a best match, or the user moved onto the
    /// File Explorer row with the arrow keys.
    pub fn chosen(&self) -> bool {
        self.has_matches() || matches!(self.focus, Some(Target::Result(_)))
    }
    /// Enter in the search box: the selected row when one is chosen. The
    /// File Explorer row alone is not a best match, so Enter does nothing.
    pub fn enter_target(&self) -> Option<Target> {
        (self.chosen() && self.selected < self.results().len())
            .then_some(Target::Result(self.selected))
    }
    /// Result list layout: "Best match" (64 px row), then grouped rows
    /// (32 px headers, 40 px rows), with the Explorer row last.
    pub fn result_layout(&self, g: &Geometry) -> Vec<(Row, Rect)> {
        let results = self.results();
        let (x, mut y, w) = g.results_column();
        // Without a chosen row there is no details pane: rows span the body.
        let w = if self.chosen() {
            w
        } else {
            (g.panel.right() - 24 - x).max(1) as u32
        };
        let mut out = Vec::new();
        let mut first = 0;
        let (best_header, best_row, group_header, row_h) = result_heights();
        if self.no_results() {
            y += no_results_height();
        }
        if matches!(
            results.first(),
            Some(
                Hit::App(_)
                    | Hit::Shell(_)
                    | Hit::Tool(_)
                    | Hit::Registered(_)
                    | Hit::Settings(_)
                    | Hit::File(_)
            )
        ) {
            out.push((
                Row::Header("Best match"),
                Rect::new(x, y, w, best_header as u32).unwrap(),
            ));
            out.push((
                Row::Result(0),
                Rect::new(x, y + best_header, w, best_row as u32).unwrap(),
            ));
            y += best_header + best_row + 8;
            first = 1;
        }
        let mut group = "";
        for (i, hit) in results.iter().enumerate().skip(first) {
            let g = self.group(*hit);
            if g != group {
                if g.is_empty() {
                    y += 8;
                } else {
                    out.push((
                        Row::Header(g),
                        Rect::new(x, y, w, group_header as u32).unwrap(),
                    ));
                    y += group_header;
                }
                group = g;
            }
            out.push((Row::Result(i), Rect::new(x, y, w, row_h as u32).unwrap()));
            y += row_h;
        }
        out
    }
    fn result_rect(&self, g: &Geometry, index: usize) -> Option<Rect> {
        self.result_layout(g)
            .into_iter()
            .find(|(row, _)| *row == Row::Result(index))
            .map(|v| v.1)
    }
    /// App order in the current All view (keyboard order).
    pub fn all_apps(&self) -> Vec<(Target, apps::Entry)> {
        match self.view {
            View::Category => apps::categories()
                .into_iter()
                .enumerate()
                .flat_map(|(card, (_, apps))| {
                    apps.into_iter()
                        .take(4)
                        .enumerate()
                        .map(move |(slot, app)| (Target::CategoryApp(card, slot), app))
                        .collect::<Vec<_>>()
                })
                .collect(),
            View::Grid => apps::entries_alphabetical()
                .into_iter()
                .enumerate()
                .map(|(i, app)| (Target::GridApp(i), app))
                .collect(),
            View::List => list_rows()
                .into_iter()
                .enumerate()
                .filter_map(|(i, row)| match row {
                    ListRow::App(app) => Some((Target::ListApp(i), app)),
                    ListRow::Letter(_) => None,
                })
                .collect(),
        }
    }
    /// Rectangle of a target (for hit testing, hover and focus reveal).
    pub fn rect(&self, g: &Geometry, fonts: &FontAtlas, target: Target) -> Option<Rect> {
        let s = self.scroll;
        Some(match target {
            Target::Search => g.search_box(),
            Target::Pinned(i) => g.pinned_tile(i, s)?,
            Target::ShowAll => g.header_button(
                g.pinned_header(s),
                paint::measure(
                    fonts,
                    if self.expanded {
                        "Show less"
                    } else {
                        "Show all"
                    },
                    TextStyle::Regular14,
                ),
            ),
            Target::View => g.header_button(
                g.all_header(s),
                paint::measure(fonts, &view_label(self.view), TextStyle::Regular14),
            ),
            Target::CategoryApp(card, slot) => g.card_cell(g.card(card, s), slot),
            Target::GridApp(i) => g.grid_tile(i, s),
            Target::ListApp(i) => g.list(s).get(i)?.1,
            Target::User => g.user_button(paint::measure(fonts, USER_NAME, TextStyle::Regular14)),
            Target::Power => g.power_button(),
            Target::Folder(i) => {
                let shown = self.shown_folders();
                let rank = shown.iter().position(|f| *f == i)?;
                g.folder_button(rank, shown.len())
            }
            Target::Chip(f) if g.chip_fits(fonts, f) => g.chip(fonts, f),
            Target::Chip(_) => return None,
            Target::Result(i) => self.result_rect(g, i)?,
            Target::Action(i) => g.action(i),
            Target::History(i) if g.search && i < self.home_history_count(g) => g.home_recent(i)?,
            Target::History(_) => return None,
            Target::Recent(i) if g.search => g.home_recent(self.home_history_count(g) + i)?,
            Target::Recent(i) => g.recent_item(i, s)?,
            Target::TopApp(i) => g.top_app(i)?,
            Target::QuickSearch(i) => g.quick_pill(fonts, i)?,
            Target::Panel => g.panel,
        })
    }
    /// Every visible target in keyboard (Tab) order; hit testing and the
    /// semantic tree (`shell_semantics`) walk the same list. The Search
    /// panel lists its pills (Web and Photos disabled) before its home or
    /// results, and has no account or Power footer.
    pub fn targets(&self, g: &Geometry) -> Vec<Target> {
        let mut t = alloc::vec![Target::Search];
        if self.searching() {
            t.extend(Filter::ALL.map(Target::Chip));
            t.extend((0..self.results().len()).map(Target::Result));
            if self.chosen() {
                t.extend((0..self.actions().len()).map(Target::Action));
            }
        } else if g.search {
            t.extend((0..self.home_history_count(g)).map(Target::History));
            t.extend((0..self.home_recent_count(g)).map(Target::Recent));
            t.extend((0..QUICK_SEARCHES.len()).map(Target::QuickSearch));
            t.extend(
                (0..self.top_apps().len())
                    .filter(|i| g.top_app(*i).is_some())
                    .map(Target::TopApp),
            );
            return t;
        } else {
            if g.show_pinned {
                t.extend((0..self.pins.len()).map(Target::Pinned));
            }
            if g.shows_toggle() {
                t.push(Target::ShowAll);
            }
            t.extend((0..g.recent_count()).map(Target::Recent));
            if g.show_all {
                t.push(Target::View);
                t.extend(self.all_apps().into_iter().map(|v| v.0));
            }
        }
        if g.show_account {
            t.push(Target::User);
        }
        if !g.search {
            t.extend(self.shown_folders().into_iter().map(Target::Folder));
            t.push(Target::Power);
        }
        t
    }
    /// Topmost target under a logical point, or None outside the panel.
    pub fn hit(&self, g: &Geometry, fonts: &FontAtlas, x: i32, y: i32) -> Option<Target> {
        if !g.panel.contains(x, y) {
            return None;
        }
        let body = g.body();
        for target in self.targets(g) {
            let in_body = !matches!(target, Target::Search | Target::User | Target::Power);
            if in_body && !body.contains(x, y) {
                continue;
            }
            if self
                .rect(g, fonts, target)
                .is_some_and(|r| r.contains(x, y))
            {
                return Some(target);
            }
        }
        Some(Target::Panel)
    }
    /// App behind a target, if any.
    pub fn app(&self, target: Target) -> Option<App> {
        match target {
            Target::Pinned(i) => self.pins.get(i).copied(),
            Target::TopApp(i) => self.top_apps().get(i).copied(),
            Target::CategoryApp(..) | Target::GridApp(_) | Target::ListApp(_) => self
                .all_apps()
                .into_iter()
                .find(|(t, _)| *t == target)
                .and_then(|v| v.1.app()),
            Target::Result(i) => match self.results().get(i) {
                Some(Hit::App(app)) => Some(*app),
                _ => None,
            },
            _ => None,
        }
    }
    /// Detail-pane actions for the selected search result. A file opens its
    /// location (the provider has no in-process handler app); a folder opens.
    pub fn actions(&self) -> Vec<&'static str> {
        match self.results().get(self.selected) {
            // Shells and Terminal can run as administrator (UAC first).
            Some(Hit::Shell(s)) if !matches!(s, apps::Shell::PowerShell | apps::Shell::Dsl) => {
                alloc::vec!["Open", "Run as administrator"]
            }
            Some(Hit::App(App::Terminal)) => alloc::vec!["Open", "Run as administrator"],
            Some(
                Hit::App(_) | Hit::Shell(_) | Hit::Tool(_) | Hit::Registered(_) | Hit::Settings(_),
            ) => {
                alloc::vec!["Open"]
            }
            Some(hit @ Hit::File(_)) => {
                if self.file(*hit).is_some_and(|f| f.folder) {
                    alloc::vec!["Open"]
                } else {
                    alloc::vec!["Open", "Open file location"]
                }
            }
            Some(Hit::Explore) => alloc::vec!["Search in File Explorer"],
            Some(Hit::FilesUnavailable) => alloc::vec!["Open File Explorer"],
            None => Vec::new(),
        }
    }
    pub fn activate(&mut self, target: Target) -> Activation {
        match target {
            Target::ShowAll => {
                self.expanded = !self.expanded;
                self.scroll = 0;
                Activation::Redraw
            }
            Target::View => Activation::ViewMenu,
            Target::Power => Activation::PowerMenu,
            Target::Folder(i) => Activation::Folder(i),
            Target::User => Activation::UserMenu,
            // Web and Photos have no backend: their pills stay disabled.
            Target::Chip(filter) if !filter.available() => Activation::Nothing,
            Target::Chip(filter) => {
                self.filter = filter;
                self.selected = 0;
                Activation::Redraw
            }
            Target::Result(i) => {
                self.selected = i;
                self.open_selected()
            }
            Target::Action(1) => match self.results().get(self.selected) {
                Some(Hit::File(i)) if self.files.get(*i as usize).is_some_and(|f| !f.folder) => {
                    Activation::OpenFileLocation(*i as usize)
                }
                Some(Hit::Shell(s)) => Activation::RunAsAdministrator(*s),
                Some(Hit::App(App::Terminal)) => {
                    Activation::RunAsAdministrator(apps::Shell::Doorsh)
                }
                _ => self.open_selected(),
            },
            Target::Action(_) => self.open_selected(),
            Target::Search => {
                self.focus = Some(Target::Search);
                self.caret = true;
                Activation::Redraw
            }
            Target::Panel => Activation::Nothing,
            Target::Recent(i) if i < self.recent.len() => Activation::OpenRecent(i),
            Target::History(i) => match self.history.get(i).cloned() {
                Some(query) => {
                    self.search_for(query);
                    Activation::Redraw
                }
                None => Activation::Nothing,
            },
            Target::QuickSearch(i) => QUICK_SEARCHES
                .get(i)
                .map_or(Activation::Nothing, |(_, page)| Activation::Settings(*page)),
            Target::CategoryApp(..) | Target::GridApp(_) | Target::ListApp(_) => {
                match self
                    .all_apps()
                    .into_iter()
                    .find(|(t, _)| *t == target)
                    .map(|v| v.1)
                {
                    Some(apps::Entry::App(app)) => Activation::Launch(app),
                    Some(apps::Entry::Tool(tool)) => Activation::LaunchTool(tool),
                    None => Activation::Nothing,
                }
            }
            other => self
                .app(other)
                .map_or(Activation::Nothing, Activation::Launch),
        }
    }
    fn open_selected(&self) -> Activation {
        match self.results().get(self.selected) {
            Some(Hit::App(app)) => Activation::Launch(*app),
            Some(Hit::Shell(shell)) => Activation::Shell(*shell),
            Some(Hit::Tool(tool)) => Activation::LaunchTool(*tool),
            Some(Hit::Registered(i)) => self
                .catalog
                .get(usize::from(*i))
                .map_or(Activation::Nothing, |a| Activation::LaunchRegistered(a.key)),
            Some(Hit::Settings(page)) => Activation::Settings(page.page),
            Some(Hit::File(i)) => Activation::OpenFile(*i as usize),
            Some(Hit::Explore) => Activation::ExploreFiles,
            Some(Hit::FilesUnavailable) => Activation::Launch(App::Files),
            None => Activation::Nothing,
        }
    }
    /// Fill the search box with `query` and run it (a recent search).
    pub fn search_for(&mut self, query: String) {
        self.query_caret = query.len();
        self.query = query;
        self.preedit = Default::default();
        self.filter = Filter::All;
        self.scroll = 0;
        self.selected = 0;
        self.focus = Some(Target::Search);
        self.caret = true;
    }
    /// Typing always edits the search pill, as in Windows.
    pub fn type_char(&mut self, ch: char) -> bool {
        let mut b = [0u8; 4];
        self.type_text(ch.encode_utf8(&mut b))
    }
    /// Key text or an IME commit at the caret (controls dropped): 64
    /// characters, within the provider query bound in bytes. A leading
    /// space is ignored, as in Windows.
    pub fn type_text(&mut self, text: &str) -> bool {
        let text = if self.query.is_empty() {
            text.trim_start_matches(' ')
        } else {
            text
        };
        let fits = |t: &str| {
            t.chars().count() <= 64 && t.len() <= door_service_protocol::window::search::MAX_QUERY
        };
        if !crate::text_edit::insert(&mut self.query, &mut self.query_caret, text, fits) {
            return false;
        }
        self.after_query_change();
        true
    }
    /// Backspace: the grapheme cluster before the caret.
    pub fn backspace(&mut self) -> bool {
        if !crate::text_edit::backspace(&mut self.query, &mut self.query_caret) {
            return false;
        }
        self.after_query_change();
        true
    }
    /// IME delete-surrounding around the caret (UTF-8 bytes).
    pub fn ime_delete(&mut self, before: usize, after: usize) -> bool {
        if !crate::text_edit::delete_around(&mut self.query, &mut self.query_caret, before, after) {
            return false;
        }
        self.after_query_change();
        true
    }
    /// Caret keys in the search box (Left/Right/Home/End by cluster, Delete).
    /// False when the box is not focused or the key is not a caret key.
    pub fn edit_key(&mut self, scan: u32) -> bool {
        if self.focus != Some(Target::Search) || self.query.is_empty() {
            return false;
        }
        if scan == 0x153 {
            if crate::text_edit::delete(&mut self.query, &mut self.query_caret) {
                self.after_query_change();
            }
            return true;
        }
        let moved = crate::text_edit::move_caret(&self.query, &mut self.query_caret, scan);
        if moved {
            self.caret = true;
        }
        moved
    }
    fn after_query_change(&mut self) {
        self.selected = 0;
        self.scroll = 0;
        self.focus = Some(Target::Search);
        self.caret = true;
        // Typing in Start switches to the Search panel; emptying a query
        // typed there returns to Start (Windows 11 26H2).
        if self.searching() && self.mode == Mode::Start {
            self.mode = Mode::Search;
            self.from_start = true;
        } else if !self.searching() && self.from_start && self.preedit.text.is_empty() {
            self.mode = Mode::Start;
            self.from_start = false;
            self.filter = Filter::All;
        }
    }
    /// Escape backs out of a search typed in Start (back to Start), and
    /// otherwise closes. Returns true when the surface should close.
    pub fn escape(&mut self) -> bool {
        if self.from_start && !self.query.is_empty() {
            self.query.clear();
            self.query_caret = 0;
            self.after_query_change();
            return false;
        }
        if self.mode == Mode::Start && !self.query.is_empty() {
            self.query.clear();
            self.query_caret = 0;
            self.after_query_change();
            return false;
        }
        true
    }
    /// Arrow/Tab navigation. `key`: 0 up, 1 down, 2 left, 3 right, 4 tab,
    /// 5 shift+tab. Returns true when something changed.
    pub fn navigate(&mut self, g: &Geometry, fonts: &FontAtlas, key: u8) -> bool {
        if g.search && matches!(key, 4 | 5) {
            return self.tab_walk(g, fonts, key == 5);
        }
        if g.search && !self.searching() {
            return self.home_arrow(g, key);
        }
        if self.searching() {
            let n = self.results().len();
            if key == 1 && n > 0 && !self.chosen() {
                // No best match: Down chooses the File Explorer row first.
                self.selected = 0;
                self.focus = Some(Target::Result(0));
                self.caret = false;
                return true;
            }
            match key {
                0 if self.selected > 0 => self.selected -= 1,
                1 if self.selected + 1 < n => self.selected += 1,
                _ => return false,
            }
            self.focus = Some(Target::Result(self.selected));
            self.caret = false;
            return true;
        }
        let pins = if g.show_pinned { self.pins.len() } else { 0 };
        let all = if g.show_all {
            self.all_apps()
        } else {
            Vec::new()
        };
        let cols = g.columns;
        let current = self.focus.unwrap_or(Target::Search);
        let first_all = all.first().map(|v| v.0);
        let recent = g.recent_count();
        let next = match (current, key) {
            (Target::Recent(i), 2) if i % 2 == 1 => Some(Target::Recent(i - 1)),
            (Target::Recent(i), 3) if i % 2 == 0 && i + 1 < recent => Some(Target::Recent(i + 1)),
            (Target::Recent(i), 0) => Some(if i >= 2 {
                Target::Recent(i - 2)
            } else if pins > 0 {
                Target::Pinned(0)
            } else {
                Target::Search
            }),
            (Target::Recent(i), 1) => Some(if i + 2 < recent {
                Target::Recent(i + 2)
            } else {
                first_all.unwrap_or(Target::View)
            }),
            (Target::Recent(_), 4) => Some(Target::View),
            (Target::Recent(_), 5) => Some(if pins > 0 {
                Target::Pinned(0)
            } else {
                Target::Search
            }),
            (Target::Recent(_), _) => None,
            (Target::Search, 1 | 3 | 4) => {
                if pins > 0 {
                    Some(Target::Pinned(0))
                } else {
                    first_all.or(Some(Target::View))
                }
            }
            (Target::Search, 5) => Some(Target::Power),
            (Target::Search, _) => None,
            (Target::Pinned(i), 2) if i > 0 => Some(Target::Pinned(i - 1)),
            (Target::Pinned(i), 3) if i + 1 < pins && g.pinned_tile(i + 1, 0).is_some() => {
                Some(Target::Pinned(i + 1))
            }
            (Target::Pinned(i), 0) => Some(if i >= cols {
                Target::Pinned(i - cols)
            } else {
                Target::Search
            }),
            (Target::Pinned(i), 1) => {
                if g.pinned_tile(i + cols, 0).is_some() {
                    Some(Target::Pinned(i + cols))
                } else if pins > 0
                    && (pins - 1) / cols > i / cols
                    && g.pinned_tile(pins - 1, 0).is_some()
                {
                    Some(Target::Pinned(pins - 1))
                } else if recent > 0 {
                    Some(Target::Recent(0))
                } else {
                    first_all.or(Some(Target::View))
                }
            }
            (Target::Pinned(_), 4) => Some(if g.shows_toggle() {
                Target::ShowAll
            } else {
                Target::View
            }),
            (Target::Pinned(_), 5) => Some(Target::Search),
            (Target::ShowAll, 4) => Some(Target::View),
            (Target::ShowAll, 5 | 0) => Some(Target::Search),
            (Target::ShowAll, 1) => Some(Target::Pinned(0)),
            (Target::View, 4) | (Target::View, 1) => first_all.or(Some(Target::User)),
            (Target::View, 5) | (Target::View, 0) => {
                if pins > 0 {
                    Some(Target::Pinned(0))
                } else {
                    Some(Target::Search)
                }
            }
            (Target::User, 3) => Some(Target::Power),
            (Target::User, 4) => Some(Target::Power),
            (Target::User, 0 | 5) => all.last().map(|v| v.0).or(Some(Target::View)),
            (Target::Power, 2) => Some(Target::User),
            (Target::Power, 5) => Some(Target::User),
            (Target::Power, 4) => Some(Target::Search),
            (Target::Power, 0) => all.last().map(|v| v.0).or(Some(Target::View)),
            (t, k) => {
                let i = all.iter().position(|v| v.0 == t);
                match (i, k) {
                    (Some(i), 2) if i > 0 => Some(all[i - 1].0),
                    (Some(i), 3) if i + 1 < all.len() => Some(all[i + 1].0),
                    (Some(i), 0 | 1) => {
                        let step = match self.view {
                            View::List => 1,
                            View::Grid => cols,
                            View::Category => 2,
                        };
                        if k == 0 {
                            if i >= step {
                                Some(all[i - step].0)
                            } else {
                                Some(Target::View)
                            }
                        } else if i + step < all.len() {
                            Some(all[i + step].0)
                        } else if i + 1 < all.len() && self.view != View::List {
                            Some(all[all.len() - 1].0)
                        } else {
                            Some(Target::User)
                        }
                    }
                    (Some(_), 4) => Some(Target::User),
                    (Some(_), 5) => Some(Target::View),
                    _ => None,
                }
            }
        };
        // Hidden sections (26H2 Start settings) pass focus on to the footer.
        let next = next.map(|t| match t {
            Target::View if !g.show_all => {
                if g.show_account {
                    Target::User
                } else {
                    Target::Power
                }
            }
            Target::User if !g.show_account => Target::Power,
            t => t,
        });
        let Some(next) = next else {
            return false;
        };
        self.focus = Some(next);
        self.caret = next == Target::Search;
        self.reveal(g, fonts, next);
        true
    }
    /// Search panel Tab order: every enabled, shown target in `targets`
    /// order, wrapping (disabled Web/Photos pills are skipped).
    fn tab_walk(&mut self, g: &Geometry, fonts: &FontAtlas, back: bool) -> bool {
        let stops: Vec<Target> = self
            .targets(g)
            .into_iter()
            .filter(|t| !matches!(t, Target::Chip(f) if !f.available()))
            .filter(|t| self.rect(g, fonts, *t).is_some())
            .collect();
        let n = stops.len();
        if n == 0 {
            return false;
        }
        let current = self.focus.unwrap_or(Target::Search);
        let i = stops.iter().position(|t| *t == current).unwrap_or(0);
        let next = stops[if back { (i + n - 1) % n } else { (i + 1) % n }];
        self.focus_on(next);
        true
    }
    /// Search home arrows: Up/Down walk the search box, Recent rows and Top
    /// apps; Left/Right move between the columns and along a tile row.
    fn home_arrow(&mut self, g: &Geometry, key: u8) -> bool {
        let (history, recent) = self.home_rows(g);
        let top = (0..self.top_apps().len())
            .filter(|i| g.top_app(*i).is_some())
            .count();
        let list: Vec<Target> = (0..history)
            .map(Target::History)
            .chain((0..recent).map(Target::Recent))
            .chain((0..top).map(Target::TopApp))
            .collect();
        let current = self.focus.unwrap_or(Target::Search);
        let at = list.iter().position(|t| *t == current);
        let next = match (current, key) {
            (Target::History(_) | Target::Recent(_), 3) if top > 0 => Some(Target::TopApp(0)),
            (Target::TopApp(i), 3) if i + 1 < top => Some(Target::TopApp(i + 1)),
            (Target::TopApp(i), 2) if i % 3 == 0 && history + recent > 0 => Some(list[0]),
            (Target::TopApp(i), 2) if i > 0 => Some(Target::TopApp(i - 1)),
            (_, 1) => match at {
                None => list.first().copied(),
                Some(i) => list.get(i + 1).copied(),
            },
            (_, 0) => match at {
                Some(0) => Some(Target::Search),
                Some(i) => list.get(i - 1).copied(),
                None => None,
            },
            _ => None,
        };
        let Some(next) = next else {
            return false;
        };
        self.focus_on(next);
        true
    }
    fn focus_on(&mut self, target: Target) {
        self.focus = Some(target);
        self.caret = target == Target::Search;
        if let Target::Result(i) = target {
            self.selected = i;
        }
    }
    /// Scroll the body so `target` is fully visible.
    pub fn reveal(&mut self, g: &Geometry, fonts: &FontAtlas, target: Target) {
        if matches!(
            target,
            Target::Search | Target::User | Target::Power | Target::Panel
        ) {
            return;
        }
        let Some(r) = self.rect(g, fonts, target) else {
            return;
        };
        let body = g.body();
        if r.y() < body.y() + 8 {
            self.scroll -= body.y() + 8 - r.y();
        } else if r.bottom() > body.bottom() - 8 {
            self.scroll += r.bottom() - (body.bottom() - 8);
        }
        self.scroll = self.scroll.clamp(0, g.max_scroll());
    }
    /// Wheel: positive notches scroll up. Returns true when it moved.
    pub fn wheel(&mut self, g: &Geometry, notches: i32) -> bool {
        let old = self.scroll;
        self.scroll = (self.scroll - notches * 48).clamp(0, g.max_scroll());
        old != self.scroll
    }
}
pub fn view_label(view: View) -> String {
    alloc::format!("View: {}", view.label())
}
/// Per-frame inputs for painting Start.
pub struct Frame<'a> {
    pub pointer: (i32, i32),
    pub dark: bool,
    /// Vertical offset (logical px) of the rise animation (`motion::rise`).
    pub slide: i32,
    pub transparency: bool,
    pub taskbar_pins: &'a [App],
}
#[allow(clippy::too_many_arguments)]
fn header_toggle(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    r: Rect,
    label: &str,
    up: bool,
    palette: &Palette,
    hover: bool,
    focus: bool,
) {
    if hover {
        paint::rounded(c, r, 4, palette.subtle_hover);
    }
    if focus {
        paint::focus_ring(c, r, palette);
    }
    let text_w = paint::measure(fonts, label, TextStyle::Regular14) as i32;
    paint::text_row(
        c,
        fonts,
        r.x() + 8,
        r.y(),
        r.height(),
        label,
        TextStyle::Regular14,
        palette.text,
        text_w as u32 + 2,
    );
    let mask = if up {
        crate::shell_mask!("shell-chevron-up-small", 12)
    } else {
        crate::shell_mask!("shell-chevron-down-small", 12)
    };
    paint::glyph(
        c,
        r.x() + 8 + text_w + 8,
        r.y() + (r.height() as i32 - 12) / 2,
        12,
        mask,
        palette.text,
    );
}
fn tile(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    r: Rect,
    app: App,
    palette: &Palette,
    hover: bool,
    focus: bool,
) {
    if hover || focus {
        paint::rounded(c, r, 4, palette.subtle_hover);
    }
    if focus {
        paint::focus_ring(c, r, palette);
    }
    apps::draw_icon(c, r.x() + 28, r.y() + 8, app, 32);
    let band = tile_label(r);
    let label = paint::ellipsize(fonts, app.label(), TextStyle::Regular12, band.width() - 8);
    paint::text_span(
        c,
        fonts,
        band.x(),
        band.width(),
        band.y(),
        band.height(),
        &label,
        TextStyle::Regular12,
        palette.text,
    );
}
/// A Grid-view tile for any All-list entry.
fn entry_tile(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    r: Rect,
    entry: apps::Entry,
    palette: &Palette,
    hover: bool,
    focus: bool,
) {
    if let apps::Entry::App(app) = entry {
        return tile(c, fonts, r, app, palette, hover, focus);
    }
    if hover || focus {
        paint::rounded(c, r, 4, palette.subtle_hover);
    }
    if focus {
        paint::focus_ring(c, r, palette);
    }
    apps::draw_entry_icon(c, r.x() + 28, r.y() + 8, entry, 32);
    let band = tile_label(r);
    let label = paint::ellipsize(fonts, entry.label(), TextStyle::Regular12, band.width() - 8);
    paint::text_span(
        c,
        fonts,
        band.x(),
        band.width(),
        band.y(),
        band.height(),
        &label,
        TextStyle::Regular12,
        palette.text,
    );
}
/// The one-line label band of a pinned/grid tile `r` (under its 32 px
/// icon); labels wider than it, less 8 px, end in an ellipsis.
pub fn tile_label(r: Rect) -> Rect {
    Rect::new(r.x(), r.y() + 44, r.width(), tile_label_height()).unwrap()
}
/// Paint Start at its geometry, shifted down by `dy` (slide animation).
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    layout: taskbar::Layout,
    frame: &Frame<'_>,
) {
    let g0 = start.geometry(layout);
    let dy = frame.slide;
    let mut g = g0;
    g.panel = Rect::new(
        g0.panel.x(),
        g0.panel.y() + dy,
        g0.panel.width(),
        g0.panel.height(),
    )
    .unwrap();
    let palette = Palette::new(frame.dark);
    let pointer = (frame.pointer.0, frame.pointer.1 - dy);
    let hover = start.hit(&g0, fonts, pointer.0, pointer.1);
    let p = g.panel;
    paint::shadow(c, p, 8, frame.dark);
    // Acrylic body: blurred backdrop under the Windows tint, or solid when
    // transparency effects are off.
    let (body_tint, _band_tint) = if frame.dark {
        (paint::rgb(44, 44, 44), paint::rgb(36, 36, 36))
    } else {
        (paint::rgb(243, 243, 243), paint::rgb(236, 236, 236))
    };
    // "Show accent colour on Start and taskbar" (dark mode only).
    let accent = paint::start_tint(frame.dark);
    let body_tint = accent.unwrap_or(body_tint);
    paint::rounded(c, p, 8, palette.surface_border);
    let inner = paint::inset(p, 1, 1).unwrap_or(p);
    if frame.transparency && start.backdrop.region().is_some() {
        start.backdrop.paint(c, inner, 7, dy, body_tint, 232);
    } else {
        paint::rounded(c, inner, 7, accent.unwrap_or(palette.surface));
    }
    // Header and footer bands with their hairline dividers.
    let header = Rect::new(
        inner.x(),
        inner.y(),
        inner.width(),
        (header_height() - 2) as u32,
    )
    .unwrap();
    let footer = Rect::new(
        inner.x(),
        g.footer().y() + 1,
        inner.width(),
        (footer_height() - 2) as u32,
    )
    .unwrap();
    let shade = if palette.contrast {
        paint::black(0)
    } else if frame.dark {
        paint::black(46)
    } else {
        paint::black(9)
    };
    // The Search panel has no account/Power footer band.
    let bands: &[Rect] = if g.search {
        &[header]
    } else {
        &[header, footer]
    };
    for r in bands {
        let old = c.clip_rect();
        let scale = c.scale();
        if let Some(clip) = old.and_then(|cl| cl.intersection(paint::physical(*r, scale))) {
            c.set_clip(clip);
            paint::rounded(c, inner, 7, shade);
        }
        if let Some(old) = old {
            c.set_clip(old);
        }
    }
    paint::rect(
        c,
        inner.x(),
        p.y() + header_height() - 1,
        inner.width(),
        1,
        palette.surface_divider,
    );
    if !g.search {
        paint::rect(
            c,
            inner.x(),
            g.footer().y(),
            inner.width(),
            1,
            palette.surface_divider,
        );
    }
    // Search pill.
    let search = g.search_box();
    paint::framed(c, search, 16, palette.field_border, palette.field);
    paint::glyph(
        c,
        search.x() + 15,
        search.y() + (search.height() as i32 - 16) / 2,
        16,
        crate::shell_mask!("shell-search", 16),
        palette.accent,
    );
    let text_x = search.x() + 44;
    if start.query.is_empty() && start.preedit.text.is_empty() {
        let hint = paint::ellipsize(
            fonts,
            "Search for apps, settings, and documents",
            TextStyle::Regular14,
            search.width() - 60,
        );
        paint::text_row(
            c,
            fonts,
            text_x,
            search.y(),
            search.height(),
            &hint,
            TextStyle::Regular14,
            palette.text_secondary,
            search.width() - 60,
        );
    }
    // Unicode query, caret and inline IME composition (scrolled to the caret).
    let caret_h = paint::text_len(16);
    let caret = (start.caret && start.focus == Some(Target::Search))
        .then_some((search.y() + (search.height() - caret_h) as i32 / 2, caret_h));
    let area = Rect::new(
        text_x,
        search.y(),
        search.width().saturating_sub(60).max(1),
        search.height(),
    )
    .unwrap();
    let field = crate::text_edit::Field {
        text: &start.query,
        caret: start.query_caret,
        preedit: &start.preedit,
    };
    crate::text_edit::draw(
        c,
        fonts,
        area,
        &field,
        TextStyle::Regular14,
        palette.text,
        caret,
        false,
    );
    // Body (clipped, scrolled).
    let body = g.body();
    let saved = c.clip_rect();
    let scale = c.scale();
    if let Some(clip) = saved.and_then(|cl| cl.intersection(paint::physical(body, scale))) {
        c.set_clip(clip);
        if start.searching() {
            draw_results(c, fonts, start, &g, &palette, pointer, hover, frame);
        } else if g.search {
            draw_search_home(c, fonts, start, &g, &palette, hover);
        } else {
            draw_home(c, fonts, start, &g, &palette, hover);
            // Collapsed (indicator) scrollbar, as WinUI shows it until hovered.
            let max = g.max_scroll();
            if max > 0 && body.height() > 48 {
                let track = body.height() as i32 - 16;
                let content = g.content_height().max(1);
                let thumb = (track * body.height() as i32 / content).clamp(24, track);
                let top = body.y() + 8 + (track - thumb) * start.scroll.clamp(0, max) / max;
                paint::rounded(
                    c,
                    Rect::new(body.right() - 6, top, 2, thumb as u32).unwrap(),
                    1,
                    palette.text_secondary,
                );
            }
        }
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
    if g.search {
        return;
    }
    // Footer: account (unless hidden, 26H2) and power.
    let name_w = paint::measure(fonts, USER_NAME, TextStyle::Regular14);
    let user = g.user_button(name_w);
    if g.show_account {
        draw_account(
            c,
            fonts,
            start,
            user,
            name_w,
            &palette,
            hover == Some(Target::User),
            frame.dark,
        );
    }
    // Personalization > Start > Folders, left of Power.
    let shown = start.shown_folders();
    for (rank, i) in shown.iter().copied().enumerate() {
        let t = Target::Folder(i);
        let r = g.folder_button(rank, shown.len());
        if hover == Some(t) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        if start.focus == Some(t) {
            paint::focus_ring(c, r, &palette);
        }
        let mask = match i {
            0 => crate::shell_mask!("shell-settings", 16),
            1 => crate::shell_mask!("shell-folder", 16),
            2 => crate::shell_mask!("explorer-document", 16),
            3 => crate::shell_mask!("explorer-download", 16),
            4 => crate::shell_mask!("explorer-music", 16),
            5 => crate::shell_mask!("shell-pictures", 16),
            6 => crate::shell_mask!("explorer-video", 16),
            7 => crate::shell_mask!("shell-network", 16),
            _ => crate::shell_mask!("shell-personal", 16),
        };
        paint::glyph_in(c, r, 16, mask, palette.text);
    }
    let power = g.power_button();
    if hover == Some(Target::Power) {
        paint::rounded(c, power, 4, palette.subtle_hover);
    }
    if start.focus == Some(Target::Power) {
        paint::focus_ring(c, power, &palette);
    }
    paint::glyph_in(
        c,
        power,
        16,
        crate::shell_mask!("shell-power", 16),
        palette.text,
    );
}
#[allow(clippy::too_many_arguments)]
fn draw_account(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    user: Rect,
    name_w: u32,
    palette: &Palette,
    hovered: bool,
    dark: bool,
) {
    if hovered {
        paint::rounded(c, user, 4, palette.subtle_hover);
    }
    if start.focus == Some(Target::User) {
        paint::focus_ring(c, user, palette);
    }
    let avatar_cx = user.x() + 12 + 16;
    let avatar_cy = user.y() + user.height() as i32 / 2;
    let (disc, person) = if palette.contrast {
        (palette.text, palette.surface)
    } else if dark {
        (paint::rgb(90, 90, 90), paint::rgb(230, 230, 230))
    } else {
        (paint::rgb(200, 200, 200), paint::rgb(255, 255, 255))
    };
    paint::circle(c, avatar_cx, avatar_cy, 16, disc);
    paint::glyph(
        c,
        avatar_cx - 10,
        avatar_cy - 10,
        20,
        crate::shell_mask!("shell-person", 20),
        person,
    );
    paint::text_row(
        c,
        fonts,
        user.x() + 12 + 32 + 14,
        user.y(),
        user.height(),
        USER_NAME,
        TextStyle::Regular14,
        palette.text,
        name_w + 2,
    );
}
fn draw_home(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    hover: Option<Target>,
) {
    let s = start.scroll;
    let focus = start.focus;
    if g.show_pinned {
        draw_pinned(c, fonts, start, g, palette, hover);
    }
    // Recent (26H2, formerly Recommended): files and folders the user
    // opened, from the search host's per-user list. Every app is bundled, so
    // there are no newly installed apps; without the host (or with "Show
    // recently opened files" off) the honest state is empty.
    if let Some(header) = g.recent_header(s) {
        paint::text_row(
            c,
            fonts,
            header.x(),
            header.y(),
            header.height(),
            "Recent",
            TextStyle::Semibold14,
            palette.text,
            200,
        );
        if g.recent_count() == 0 {
            let empty = paint::ellipsize(
                fonts,
                "No recent files or newly installed apps",
                TextStyle::Regular12,
                header.width(),
            );
            paint::text_row(
                c,
                fonts,
                header.x(),
                header.bottom() + 8,
                tile_label_height(),
                &empty,
                TextStyle::Regular12,
                palette.text_secondary,
                header.width(),
            );
        }
        for (i, item) in start.recent.iter().enumerate().take(g.recent_count()) {
            let Some(r) = g.recent_item(i, s) else {
                continue;
            };
            let t = Target::Recent(i);
            if hover == Some(t) || focus == Some(t) {
                paint::rounded(c, r, 4, palette.subtle_hover);
            }
            if focus == Some(t) {
                paint::focus_ring(c, r, palette);
            }
            apps::draw_file_icon(
                c,
                r.x() + 10,
                r.y() + (r.height() as i32 - 32) / 2,
                apps::FileIcon::for_name(&item.name, item.folder),
                32,
            );
            let w = (r.width() as i32 - 60).max(1) as u32;
            let name = paint::ellipsize(fonts, &item.name, TextStyle::Regular14, w);
            paint::text_row(
                c,
                fonts,
                r.x() + 52,
                r.y() + 7,
                paint::text_grow(20, TextStyle::Regular14),
                &name,
                TextStyle::Regular14,
                palette.text,
                w,
            );
            let place = paint::ellipsize(fonts, &item.path, TextStyle::Regular12, w);
            paint::text_row(
                c,
                fonts,
                r.x() + 52,
                r.y() + 27 + grow(TextStyle::Regular14),
                paint::text_grow(18, TextStyle::Regular12),
                &place,
                TextStyle::Regular12,
                palette.text_secondary,
                w,
            );
        }
    }
    if !g.show_all {
        return;
    }
    let _ = focus;
    draw_all(c, fonts, start, g, palette, hover);
}
fn draw_pinned(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    hover: Option<Target>,
) {
    let s = start.scroll;
    let focus = start.focus;
    let header = g.pinned_header(s);
    paint::text_row(
        c,
        fonts,
        header.x(),
        header.y(),
        header.height(),
        "Pinned",
        TextStyle::Semibold14,
        palette.text,
        200,
    );
    if g.shows_toggle() {
        let label = if start.expanded {
            "Show less"
        } else {
            "Show all"
        };
        let r = start.rect(g, fonts, Target::ShowAll).unwrap();
        header_toggle(
            c,
            fonts,
            r,
            label,
            start.expanded,
            palette,
            hover == Some(Target::ShowAll),
            focus == Some(Target::ShowAll),
        );
    }
    if start.pins.is_empty() {
        let empty = paint::ellipsize(
            fonts,
            "Pin apps here from All or by right-clicking an app.",
            TextStyle::Regular12,
            header.width(),
        );
        paint::text_row(
            c,
            fonts,
            header.x(),
            header.bottom() + 8,
            tile_label_height(),
            &empty,
            TextStyle::Regular12,
            palette.text_secondary,
            header.width(),
        );
    }
    for (i, app) in start.pins.iter().enumerate() {
        if let Some(r) = g.pinned_tile(i, s) {
            tile(
                c,
                fonts,
                r,
                *app,
                palette,
                hover == Some(Target::Pinned(i)),
                focus == Some(Target::Pinned(i)),
            );
        }
    }
}
fn draw_all(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    hover: Option<Target>,
) {
    let s = start.scroll;
    let focus = start.focus;
    let all = g.all_header(s);
    paint::text_row(
        c,
        fonts,
        all.x(),
        all.y(),
        all.height(),
        "All",
        TextStyle::Semibold14,
        palette.text,
        200,
    );
    let view = view_label(start.view);
    let r = start.rect(g, fonts, Target::View).unwrap();
    header_toggle(
        c,
        fonts,
        r,
        &view,
        false,
        palette,
        hover == Some(Target::View),
        focus == Some(Target::View),
    );
    match start.view {
        View::Category => {
            for (card, (category, apps)) in apps::categories().into_iter().enumerate() {
                let r = g.card(card, s);
                paint::rounded(c, r, 8, palette.card);
                for (slot, app) in apps.iter().take(4).enumerate() {
                    let cell = g.card_cell(r, slot);
                    let t = Target::CategoryApp(card, slot);
                    if hover == Some(t) || focus == Some(t) {
                        paint::rounded(c, cell, 4, palette.subtle_hover);
                    }
                    if focus == Some(t) {
                        paint::focus_ring(c, cell, palette);
                    }
                    apps::draw_entry_icon(c, cell.x() + 8, cell.y() + 8, *app, 32);
                }
                let label: &str = category_label(category);
                let label = paint::ellipsize(fonts, label, TextStyle::Regular12, r.width() + 24);
                paint::text_span(
                    c,
                    fonts,
                    r.x() - 12,
                    r.width() + 24,
                    r.bottom() + 13,
                    tile_label_height(),
                    &label,
                    TextStyle::Regular12,
                    palette.text,
                );
            }
        }
        View::Grid => {
            for (i, app) in apps::entries_alphabetical().into_iter().enumerate() {
                let r = g.grid_tile(i, s);
                entry_tile(
                    c,
                    fonts,
                    r,
                    app,
                    palette,
                    hover == Some(Target::GridApp(i)),
                    focus == Some(Target::GridApp(i)),
                );
            }
        }
        View::List => {
            for (i, (row, r)) in g.list(s).into_iter().enumerate() {
                match row {
                    ListRow::Letter(letter) => {
                        let text = [letter];
                        let label = core::str::from_utf8(&text).unwrap_or("#");
                        paint::text_row(
                            c,
                            fonts,
                            r.x() + 12,
                            r.y(),
                            r.height(),
                            label,
                            TextStyle::Semibold14,
                            palette.text,
                            40,
                        );
                    }
                    ListRow::App(app) => {
                        let t = Target::ListApp(i);
                        if hover == Some(t) || focus == Some(t) {
                            paint::rounded(c, r, 4, palette.subtle_hover);
                        }
                        if focus == Some(t) {
                            paint::focus_ring(c, r, palette);
                        }
                        apps::draw_entry_icon(
                            c,
                            r.x() + 12,
                            r.y() + (r.height() as i32 - 24) / 2,
                            app,
                            24,
                        );
                        paint::text_row(
                            c,
                            fonts,
                            r.x() + 48,
                            r.y(),
                            r.height(),
                            app.label(),
                            TextStyle::Regular14,
                            palette.text,
                            r.width() - 56,
                        );
                    }
                }
            }
        }
    }
}
pub fn category_label(category: Category) -> &'static str {
    category.label()
}
fn result_icon(c: &mut Canvas<'_>, start: &Start, hit: Hit, x: i32, y: i32, size: u32) {
    match hit {
        Hit::App(app) => apps::draw_icon(c, x, y, app, size),
        Hit::Shell(shell) => apps::draw_shell_icon(c, x, y, shell, size),
        // Inbox apps carry their Windows icons, the gallery the Doors logo.
        Hit::Tool(apps::Tool::Gallery) => apps::draw_logo(c, x, y, size),
        Hit::Tool(tool) => apps::draw_tool_icon(c, x, y, tool, size),
        Hit::Registered(i) => {
            if let Some(a) = start.catalog.get(usize::from(i)) {
                crate::shell_view::draw_icon(c, x, y, size, a.icon);
            }
        }
        // Settings results use the Settings app's gear tile.
        Hit::Settings(_) => apps::draw_icon(c, x, y, App::Settings, size),
        Hit::File(_) => {
            if let Some(f) = start.file(hit) {
                apps::draw_file_icon(c, x, y, apps::FileIcon::for_name(&f.name, f.folder), size);
            }
        }
        Hit::Explore | Hit::FilesUnavailable => apps::draw_icon(c, x, y, App::Files, size),
    }
}
/// Short status beside the File Explorer row.
fn file_status(start: &Start) -> &'static str {
    match start.file_search {
        FileSearch::Searching => "Searching files...",
        FileSearch::Ready { partial: true } => "Index incomplete",
        FileSearch::Ready { partial: false } if start.files.is_empty() => "No matching files",
        _ => "",
    }
}
/// Filter pills: the selected one in the accent, Web and Photos disabled
/// (no backend), pills that do not fit the row left out.
fn draw_chips(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    hover: Option<Target>,
) {
    for filter in Filter::ALL {
        if !g.chip_fits(fonts, filter) {
            continue;
        }
        let r = g.chip(fonts, filter);
        let selected = start.filter == filter;
        let enabled = filter.available();
        if selected {
            paint::rounded(c, r, 16, palette.accent);
        } else {
            paint::framed(
                c,
                r,
                16,
                palette.field_border,
                if enabled && hover == Some(Target::Chip(filter)) {
                    palette.tile
                } else {
                    palette.field
                },
            );
        }
        if start.focus == Some(Target::Chip(filter)) {
            paint::focus_ring(c, r, palette);
        }
        paint::text_in(
            c,
            fonts,
            r,
            filter.label(),
            TextStyle::Regular14,
            if selected {
                palette.accent_text
            } else if enabled {
                palette.text
            } else {
                palette.text_disabled
            },
        );
    }
}
/// The Search panel before typing: Recent (provider items, newest first)
/// on the left and Top apps on the right. Web highlights and Quick searches
/// need a network service Doors does not have, so they are not shown.
fn draw_search_home(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    hover: Option<Target>,
) {
    let header = |c: &mut Canvas<'_>, r: Rect, label: &str| {
        let line = section_height();
        paint::text_row(
            c,
            fonts,
            r.x() + 12,
            r.y() + (r.height() as i32 - line as i32).max(0),
            line,
            label,
            TextStyle::Semibold14,
            palette.text,
            r.width().saturating_sub(12),
        );
    };
    let recent_header = g.home_recent_header();
    header(c, recent_header, "Recent");
    let (searches, shown) = start.home_rows(g);
    if searches + shown == 0 {
        let note = paint::ellipsize(
            fonts,
            "Files you open will show up here",
            TextStyle::Regular12,
            recent_header.width().saturating_sub(24),
        );
        paint::text_row(
            c,
            fonts,
            recent_header.x() + 12,
            recent_header.bottom() + 4,
            tile_label_height(),
            &note,
            TextStyle::Regular12,
            palette.text_secondary,
            recent_header.width().saturating_sub(24),
        );
    }
    // Recent searches (Start search history) lead the list with the
    // History glyph; a click searches again.
    for (i, query) in start.history.iter().take(searches).enumerate() {
        let Some(r) = g.home_recent(i) else { break };
        let t = Target::History(i);
        if hover == Some(t) || start.focus == Some(t) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        if start.focus == Some(t) {
            paint::focus_ring(c, r, palette);
        }
        paint::glyph(
            c,
            r.x() + 12,
            r.y() + (r.height() as i32 - 16) / 2,
            16,
            crate::shell_mask!("shell-history", 16),
            palette.text_secondary,
        );
        let text = paint::ellipsize(fonts, query, TextStyle::Regular14, r.width() - 48);
        paint::text_row(
            c,
            fonts,
            r.x() + 40,
            r.y(),
            r.height(),
            &text,
            TextStyle::Regular14,
            palette.text,
            r.width() - 48,
        );
    }
    for (i, item) in start.recent.iter().take(shown).enumerate() {
        let Some(r) = g.home_recent(searches + i) else {
            break;
        };
        let t = Target::Recent(i);
        if hover == Some(t) || start.focus == Some(t) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        if start.focus == Some(t) {
            paint::focus_ring(c, r, palette);
        }
        apps::draw_file_icon(
            c,
            r.x() + 12,
            r.y() + (r.height() as i32 - 16) / 2,
            apps::FileIcon::for_name(&item.name, item.folder),
            16,
        );
        let name = paint::ellipsize(fonts, &item.name, TextStyle::Regular14, r.width() - 48);
        paint::text_row(
            c,
            fonts,
            r.x() + 40,
            r.y(),
            r.height(),
            &name,
            TextStyle::Regular14,
            palette.text,
            r.width() - 48,
        );
    }
    // Quick searches: a card of Settings pills.
    header(c, g.quick_header(), "Quick searches");
    let card = g.quick_card();
    paint::rounded(c, card, 8, palette.card);
    for (i, (label, _)) in QUICK_SEARCHES.iter().enumerate() {
        let Some(r) = g.quick_pill(fonts, i) else {
            continue;
        };
        let t = Target::QuickSearch(i);
        let fill = if hover == Some(t) || start.focus == Some(t) {
            palette.subtle_hover
        } else {
            palette.card
        };
        paint::rounded(c, r, r.height() / 2, fill);
        if start.focus == Some(t) {
            paint::focus_ring(c, r, palette);
        }
        let text = paint::ellipsize(
            fonts,
            label,
            TextStyle::Regular14,
            r.width().saturating_sub(16),
        );
        paint::text_in(c, fonts, r, &text, TextStyle::Regular14, palette.text);
    }
    header(c, g.top_apps_header(), "Top apps");
    for (i, app) in start.top_apps().iter().enumerate() {
        let Some(r) = g.top_app(i) else { break };
        let t = Target::TopApp(i);
        paint::rounded(c, r, 4, palette.card);
        if hover == Some(t) || start.focus == Some(t) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        if start.focus == Some(t) {
            paint::focus_ring(c, r, palette);
        }
        apps::draw_icon(c, r.x() + (r.width() as i32 - 32) / 2, r.y() + 18, *app, 32);
        let label = paint::ellipsize(fonts, app.label(), TextStyle::Regular12, r.width() - 8);
        paint::text_span(
            c,
            fonts,
            r.x(),
            r.width(),
            r.y() + 56,
            tile_label_height(),
            &label,
            TextStyle::Regular12,
            palette.text,
        );
    }
}
#[allow(clippy::too_many_arguments)]
fn draw_results(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    start: &Start,
    g: &Geometry,
    palette: &Palette,
    pointer: (i32, i32),
    hover: Option<Target>,
    frame: &Frame<'_>,
) {
    let _ = (pointer, frame);
    draw_chips(c, fonts, start, g, palette, hover);
    let results = start.results();
    let layout = start.result_layout(g);
    let (left, top, _) = g.results_column();
    if layout.is_empty() || start.no_results() {
        paint::text_row(
            c,
            fonts,
            left + 12,
            top,
            section_height(),
            "No results",
            TextStyle::Semibold14,
            palette.text,
            300,
        );
        let why = paint::ellipsize(
            fonts,
            "Doors searches apps, Settings and file names on this PC, never the web.",
            TextStyle::Regular12,
            g.panel.width() - 96,
        );
        paint::text_row(
            c,
            fonts,
            left + 12,
            top + 24 + grow(TextStyle::Semibold14),
            tile_label_height(),
            &why,
            TextStyle::Regular12,
            palette.text_secondary,
            g.panel.width() - 96,
        );
        if layout.is_empty() {
            return;
        }
    }
    let chosen = start.chosen();
    let bottom = g.body().bottom();
    for (row, r) in &layout {
        if r.y() > bottom {
            break;
        }
        let i = match row {
            Row::Header(label) => {
                let line = section_height();
                paint::text_row(
                    c,
                    fonts,
                    r.x() + 12,
                    r.y() + (r.height() as i32 - line as i32).max(0),
                    line,
                    label,
                    TextStyle::Semibold14,
                    palette.text,
                    r.width(),
                );
                continue;
            }
            Row::Result(i) => *i,
        };
        let hit = results[i];
        let t = Target::Result(i);
        if chosen && start.selected == i {
            paint::rounded(c, *r, 4, palette.subtle_hover);
            paint::rounded(
                c,
                Rect::new(r.x(), r.y() + r.height() as i32 / 2 - 8, 3, 16).unwrap(),
                1,
                palette.accent,
            );
        } else if hover == Some(t) {
            paint::rounded(c, *r, 4, palette.subtle_pressed);
        }
        if r.height() as i32 == result_heights().1 {
            result_icon(
                c,
                start,
                hit,
                r.x() + 12,
                r.y() + (r.height() as i32 - 32) / 2,
                32,
            );
            let title = paint::ellipsize(
                fonts,
                start.title(hit),
                TextStyle::Semibold14,
                r.width() - 64,
            );
            paint::text_row(
                c,
                fonts,
                r.x() + 56,
                r.y() + 12,
                section_height(),
                &title,
                TextStyle::Semibold14,
                palette.text,
                r.width() - 64,
            );
            let kind = match start.file(hit) {
                Some(f) => alloc::format!("{} - {}", start.kind(hit), f.path),
                None => String::from(start.kind(hit)),
            };
            let kind = paint::ellipsize(fonts, &kind, TextStyle::Regular12, r.width() - 64);
            paint::text_row(
                c,
                fonts,
                r.x() + 56,
                r.y() + 32 + grow(TextStyle::Semibold14),
                paint::text_grow(18, TextStyle::Regular12),
                &kind,
                TextStyle::Regular12,
                palette.text_secondary,
                r.width() - 64,
            );
        } else {
            result_icon(
                c,
                start,
                hit,
                r.x() + 12,
                r.y() + (r.height() as i32 - 16) / 2,
                16,
            );
            let status = if hit == Hit::Explore {
                file_status(start)
            } else {
                ""
            };
            let status_w = if status.is_empty() {
                0
            } else {
                paint::measure(fonts, status, TextStyle::Regular12) + 16
            };
            let title = paint::ellipsize(
                fonts,
                start.title(hit),
                TextStyle::Regular14,
                (r.width() - 48).saturating_sub(status_w),
            );
            paint::text_row(
                c,
                fonts,
                r.x() + 40,
                r.y(),
                r.height(),
                &title,
                TextStyle::Regular14,
                palette.text,
                r.width() - 48,
            );
            if !status.is_empty() {
                paint::text_right(
                    c,
                    fonts,
                    r.right() - 12,
                    r.y(),
                    r.height(),
                    status,
                    TextStyle::Regular12,
                    palette.text_secondary,
                );
            }
        }
    }
    // Details of the selected result (none without a chosen row).
    let Some(hit) = results.get(start.selected).copied().filter(|_| chosen) else {
        return;
    };
    let d = g.details();
    paint::rounded(c, d, 8, palette.card);
    let cx = d.x() + d.width() as i32 / 2;
    match hit {
        Hit::File(_) => result_icon(c, start, hit, cx - 16, d.y() + 32, 32),
        _ => {
            let size = if c.scale().percent() >= 300 { 40 } else { 48 };
            result_icon(c, start, hit, cx - size as i32 / 2, d.y() + 24, size);
        }
    }
    let (title_band, kind_band, divider) = g.details_text();
    let title = paint::ellipsize(
        fonts,
        start.title(hit),
        TextStyle::Semibold20,
        d.width() - 24,
    );
    paint::text_span(
        c,
        fonts,
        title_band.x(),
        title_band.width(),
        title_band.y(),
        title_band.height(),
        &title,
        TextStyle::Semibold20,
        palette.text,
    );
    let kind = match (hit, start.file(hit)) {
        (_, Some(f)) => alloc::format!("{} - {}", start.kind(hit), f.path),
        (Hit::Explore, _) => alloc::format!("\"{}\" on Local Disk (C:)", start.query.trim()),
        _ => String::from(start.kind(hit)),
    };
    let kind = paint::ellipsize(fonts, &kind, TextStyle::Regular12, d.width() - 24);
    paint::text_span(
        c,
        fonts,
        kind_band.x(),
        kind_band.width(),
        kind_band.y(),
        kind_band.height(),
        &kind,
        TextStyle::Regular12,
        palette.text_secondary,
    );
    paint::rect(
        c,
        d.x() + 16,
        divider,
        d.width() - 32,
        1,
        palette.surface_divider,
    );
    for (i, label) in start.actions().iter().enumerate() {
        let r = g.action(i);
        let t = Target::Action(i);
        if hover == Some(t) || start.focus == Some(t) {
            paint::rounded(c, r, 4, palette.subtle_hover);
        }
        let glyph = match hit {
            Hit::File(_) if *label == "Open file location" => {
                crate::shell_mask!("shell-folder", 16)
            }
            _ if *label == "Run as administrator" => crate::shell_mask!("shell-shield", 16),
            Hit::Explore | Hit::FilesUnavailable => crate::shell_mask!("shell-folder", 16),
            _ => crate::shell_mask!("shell-more-options", 16),
        };
        paint::glyph(
            c,
            r.x() + 12,
            r.y() + (r.height() as i32 - 16) / 2,
            16,
            glyph,
            palette.text,
        );
        paint::text_row(
            c,
            fonts,
            r.x() + 40,
            r.y(),
            r.height(),
            label,
            TextStyle::Regular14,
            palette.text,
            r.width() - 48,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_service_protocol::desktop::Controls;
    fn fonts() -> FontAtlas {
        FontAtlas::windows_reference().unwrap()
    }
    #[test]
    fn panel_matches_25h2_metrics_and_fits_the_work_area() {
        let g = Geometry::new(taskbar::Layout::new(1920, 1080));
        assert_eq!((g.columns, g.category_columns), (8, 4));
        assert_eq!(g.panel.width(), 832);
        assert_eq!(g.panel.x() * 2 + 832, 1920);
        assert_eq!(g.panel.bottom(), 1080 - 48 - 12);
        assert_eq!(
            g.search_box(),
            Rect::new(g.panel.x() + 32, g.panel.y() + 16, 768, 32).unwrap()
        );
        assert_eq!(g.card_size(), 153);
        let small = Geometry::new(taskbar::Layout::new(1280, 720));
        assert_eq!(
            (small.columns, small.category_columns, small.panel.width()),
            (6, 3, 640)
        );
        for (w, h) in [
            (800, 600),
            (1024, 768),
            (1280, 720),
            (1920, 1080),
            (3840, 2160),
        ] {
            for bits in [0, 1, 2, 3, 4, 8, 12] {
                let layout =
                    taskbar::Layout::with_controls(w, h, Controls::from_bits(bits | 48).unwrap());
                for view in View::ALL {
                    let g = Geometry::with(layout, 6, view, false);
                    let area = layout.work_area();
                    assert_eq!(area.intersection(g.panel), Some(g.panel), "{w}x{h} {bits}");
                    assert!(g.body().height() > 40);
                }
            }
        }
    }
    #[test]
    fn every_target_hits_itself_inside_the_body_and_gaps_miss() {
        let fonts = fonts();
        for (w, h) in [(1280, 720), (1920, 1080)] {
            let layout = taskbar::Layout::new(w, h);
            for view in View::ALL {
                let mut start = Start {
                    view,
                    ..Start::default()
                };
                let g = start.geometry(layout);
                for target in start.targets(&g) {
                    let r = start.rect(&g, &fonts, target).unwrap();
                    let (x, y) = (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2);
                    if g.body().contains(x, y)
                        || matches!(target, Target::Search | Target::User | Target::Power)
                    {
                        assert_eq!(
                            start.hit(&g, &fonts, x, y),
                            Some(target),
                            "{view:?} {target:?}"
                        );
                    }
                }
                // The 8 px gap between pinned tiles answers nothing.
                let a = g.pinned_tile(0, 0).unwrap();
                assert_eq!(
                    start.hit(&g, &fonts, a.right() + 3, a.y() + 10),
                    Some(Target::Panel)
                );
                assert_eq!(start.hit(&g, &fonts, g.panel.x() - 1, g.panel.y()), None);
                start.scroll = g.max_scroll();
                assert!(start.scroll >= 0);
            }
        }
    }
    #[test]
    fn keyboard_moves_from_search_through_pins_all_and_footer() {
        let fonts = fonts();
        let layout = taskbar::Layout::new(1280, 720);
        let mut start = Start::default();
        start.open();
        let g = start.geometry(layout);
        assert_eq!(start.focus, Some(Target::Search));
        assert!(start.navigate(&g, &fonts, 3));
        assert_eq!(start.focus, Some(Target::Pinned(0)));
        for i in 1..6 {
            start.navigate(&g, &fonts, 3);
            assert_eq!(start.focus, Some(Target::Pinned(i)));
        }
        assert_eq!(
            start.activate(start.focus.unwrap()),
            Activation::Launch(App::Monitor)
        );
        start.navigate(&g, &fonts, 4);
        assert_eq!(start.focus, Some(Target::View));
        start.navigate(&g, &fonts, 1);
        assert_eq!(start.focus, Some(Target::CategoryApp(0, 0)));
        start.navigate(&g, &fonts, 4);
        assert_eq!(start.focus, Some(Target::User));
        start.navigate(&g, &fonts, 3);
        assert_eq!(start.focus, Some(Target::Power));
        assert_eq!(start.activate(Target::Power), Activation::PowerMenu);
        // Typing searches; Escape backs out, then closes.
        assert!(start.type_char('t'));
        assert!(start.type_char('e'));
        assert!(start.searching());
        assert_eq!(start.results()[0], Hit::App(App::Terminal));
        assert_eq!(
            start.activate(Target::Result(0)),
            Activation::Launch(App::Terminal)
        );
        assert!(!start.escape());
        assert!(!start.searching());
        assert!(start.escape());
    }
    #[test]
    fn start_26h2_sections_sizes_and_account_toggles_lay_out_cleanly() {
        let fonts = fonts();
        let d = Controls::DEFAULT;
        // Pins-only Start (Recent and All hidden).
        let pins_only = d
            .with(Controls::START_HIDE_RECENT, true)
            .with(Controls::START_HIDE_ALL, true);
        for (w, h) in [(1280, 720), (1920, 1080)] {
            let layout = taskbar::Layout::with_controls(w, h, pins_only);
            let mut start = Start::default();
            start.open();
            let g = start.geometry(layout);
            assert!(g.recent_header(0).is_none());
            assert_eq!(g.max_scroll(), 0);
            let targets = start.targets(&g);
            assert!(!targets.contains(&Target::View));
            assert!(!targets.iter().any(|t| matches!(t, Target::CategoryApp(..))));
            for i in 0..6 {
                let r = g.pinned_tile(i, 0).unwrap();
                assert_eq!(g.body().intersection(r), Some(r));
                assert_eq!(
                    start.hit(&g, &fonts, r.x() + 4, r.y() + 4),
                    Some(Target::Pinned(i))
                );
            }
            start.navigate(&g, &fonts, 1);
            assert_eq!(start.focus, Some(Target::Pinned(0)));
            start.navigate(&g, &fonts, 4);
            assert_eq!(start.focus, Some(Target::User));
        }
        // Recent is shown by default between Pinned and All.
        let g = Start::default().geometry(taskbar::Layout::new(1920, 1080));
        let recent = g.recent_header(0).unwrap();
        assert!(g.pinned_header(0).bottom() < recent.y() && recent.bottom() < g.all_header(0).y());
        // Only All shown.
        let all_only = d
            .with(Controls::START_HIDE_PINNED, true)
            .with(Controls::START_HIDE_RECENT, true);
        let mut start = Start::default();
        let g = start.geometry(taskbar::Layout::with_controls(1280, 720, all_only));
        assert!(g.pinned_tile(0, 0).is_none());
        start.navigate(&g, &fonts, 1);
        assert_eq!(start.focus, Some(Target::CategoryApp(0, 0)));
        // Start size: Small, Large and Automatic.
        let small = Geometry::new(taskbar::Layout::with_controls(
            1920,
            1080,
            d.with(Controls::START_SIZE_SMALL, true),
        ));
        assert_eq!((small.columns, small.panel.width()), (6, 640));
        let large = Geometry::new(taskbar::Layout::with_controls(
            1280,
            720,
            d.with(Controls::START_SIZE_LARGE, true),
        ));
        assert_eq!(
            (large.columns, large.category_columns, large.panel.width()),
            (8, 4, 832)
        );
        let tiny = Geometry::new(taskbar::Layout::with_controls(
            800,
            600,
            d.with(Controls::START_SIZE_LARGE, true),
        ));
        assert_eq!(
            tiny.columns, 6,
            "Large falls back where 8 columns cannot fit"
        );
        // Account hidden: no user target; focus wraps between search and power.
        let mut start = Start::default();
        start.open();
        let g = start.geometry(taskbar::Layout::with_controls(
            1280,
            720,
            d.with(Controls::START_HIDE_ACCOUNT, true),
        ));
        assert!(!start.targets(&g).contains(&Target::User));
        start.navigate(&g, &fonts, 5);
        assert_eq!(start.focus, Some(Target::Power));
        // Everything hidden still yields a valid, non-empty panel.
        let none = d
            .with(Controls::START_HIDE_PINNED, true)
            .with(Controls::START_HIDE_RECENT, true)
            .with(Controls::START_HIDE_ALL, true);
        let g = Geometry::new(taskbar::Layout::with_controls(1280, 720, none));
        assert!(g.body().height() > 0 && g.panel.height() >= 320);
    }
    #[test]
    fn file_results_group_after_apps_and_open_through_the_provider() {
        let fonts = fonts();
        let mut start = Start::default();
        start.open();
        for ch in "quarterly".chars() {
            start.type_char(ch);
        }
        assert_eq!(start.results(), [Hit::FilesUnavailable]);
        // No best match: "No results" above the row, Enter does nothing until
        // the row is chosen with the arrows; a click opens File Explorer.
        assert!(start.no_results() && !start.chosen());
        assert_eq!(start.enter_target(), None);
        let g = start.geometry(taskbar::Layout::new(1920, 1080));
        let (_, top, _) = g.results_column();
        assert_eq!(start.result_layout(&g)[0].1.y(), top + NO_RESULTS_HEIGHT);
        assert!(start.navigate(&g, &fonts, 1));
        assert_eq!(start.enter_target(), Some(Target::Result(0)));
        start.focus = Some(Target::Search);
        assert_eq!(
            start.activate(Target::Result(0)),
            Activation::Launch(App::Files)
        );
        // While the provider answers, the row is not "No results".
        start.file_search = FileSearch::Searching;
        assert!(!start.no_results());
        start.file_search = FileSearch::Ready { partial: false };
        start.files = alloc::vec![
            FileResult {
                folder: true,
                name: "Quarterly".into(),
                path: r"C:\Users\1001\Quarterly".into()
            },
            FileResult {
                folder: false,
                name: "quarterly.txt".into(),
                path: r"C:\Users\1001\notes".into()
            },
        ];
        // Documents lead (the best match), folders follow, Explorer row last.
        let results = start.results();
        assert_eq!(results, [Hit::File(1), Hit::File(0), Hit::Explore]);
        assert_eq!(start.title(results[0]), "quarterly.txt");
        assert_eq!(start.kind(results[0]), "Text Document");
        assert_eq!(start.kind(results[1]), "File folder");
        let g = start.geometry(taskbar::Layout::new(1920, 1080));
        let rows: alloc::vec::Vec<Row> = start.result_layout(&g).into_iter().map(|v| v.0).collect();
        assert_eq!(
            rows,
            [
                Row::Header("Best match"),
                Row::Result(0),
                Row::Header("Folders"),
                Row::Result(1),
                Row::Result(2)
            ]
        );
        for i in 0..3 {
            let r = start.rect(&g, &fonts, Target::Result(i)).unwrap();
            assert_eq!(
                start.hit(&g, &fonts, r.x() + 4, r.y() + 4),
                Some(Target::Result(i))
            );
        }
        // A file opens in its app; "Open file location" opens its folder.
        assert_eq!(start.actions(), ["Open", "Open file location"]);
        assert_eq!(start.activate(Target::Result(0)), Activation::OpenFile(1));
        assert_eq!(start.activate(Target::Action(0)), Activation::OpenFile(1));
        assert_eq!(
            start.activate(Target::Action(1)),
            Activation::OpenFileLocation(1)
        );
        assert_eq!(start.activate(Target::Result(1)), Activation::OpenFile(0));
        assert_eq!(start.actions(), ["Open"]);
        assert_eq!(start.activate(Target::Result(2)), Activation::ExploreFiles);
        // Filters: Apps hides files; Documents shows only files and
        // Folders only folders (26H2 pills), each with the Explorer row.
        start.activate(Target::Chip(Filter::Apps));
        assert!(start.results().is_empty());
        start.activate(Target::Chip(Filter::Documents));
        assert_eq!(start.results(), [Hit::File(1), Hit::Explore]);
        start.activate(Target::Chip(Filter::Folders));
        assert_eq!(start.results(), [Hit::File(0), Hit::Explore]);
        // Web and Photos have no backend: their pills do nothing.
        assert_eq!(
            start.activate(Target::Chip(Filter::Web)),
            Activation::Nothing
        );
        assert_eq!(start.filter, Filter::Folders);
    }
    #[test]
    fn recent_items_show_in_two_columns_and_respect_the_26h2_toggles() {
        let fonts = fonts();
        let mut start = Start::default();
        start.open();
        let layout = taskbar::Layout::new(1920, 1080);
        assert_eq!(start.geometry(layout).recent_count(), 0);
        start.recent = (0..7)
            .map(|i| FileResult {
                folder: i == 0,
                name: alloc::format!("file{i}.txt"),
                path: r"C:\Users\1001\notes".into(),
            })
            .collect();
        let g = start.geometry(layout);
        assert_eq!(g.recent_count(), MAX_RECENT);
        let (a, b, c) = (
            g.recent_item(0, 0).unwrap(),
            g.recent_item(1, 0).unwrap(),
            g.recent_item(2, 0).unwrap(),
        );
        assert_eq!(a.y(), b.y());
        assert!(b.x() > a.right() && c.x() == a.x() && c.y() >= a.bottom());
        assert!(a.y() > g.recent_header(0).unwrap().bottom());
        assert!(g.recent_item(5, 0).unwrap().bottom() < g.all_header(0).y());
        assert!(g.recent_item(6, 0).is_none());
        let r = start.rect(&g, &fonts, Target::Recent(3)).unwrap();
        assert_eq!(
            start.hit(&g, &fonts, r.x() + 4, r.y() + 4),
            Some(Target::Recent(3))
        );
        assert_eq!(start.activate(Target::Recent(3)), Activation::OpenRecent(3));
        // Arrows move across the two columns and rows.
        start.focus = Some(Target::Recent(0));
        assert!(start.navigate(&g, &fonts, 3));
        assert_eq!(start.focus, Some(Target::Recent(1)));
        assert!(start.navigate(&g, &fonts, 1));
        assert_eq!(start.focus, Some(Target::Recent(3)));
        // "Show recently opened files" off, or Recent hidden: no items.
        for flag in [
            Controls::START_HIDE_RECENT_FILES,
            Controls::START_HIDE_RECENT,
        ] {
            let layout =
                taskbar::Layout::with_controls(1920, 1080, Controls::DEFAULT.with(flag, true));
            assert_eq!(start.geometry(layout).recent_count(), 0);
        }
    }
    /// Search filter chips (painted by `draw`): sized from their label with
    /// 12 px each side and the label's ink centred within 0.5 logical px,
    /// at 1x and 2x.
    #[test]
    fn search_chips_fit_and_centre_their_labels_at_1x_and_2x() {
        use crate::paint::tests::{ink, physical};
        let layout = taskbar::Layout::new(1280, 720);
        let palette = Palette::new(false);
        for scale in [1u32, 2] {
            let s = scale as i32;
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            // Each chip is measured while unselected (gray on gray, where
            // every channel resolves coverage); the selected one uses the
            // same placement with accent colours.
            for selected in [Filter::All, Filter::Apps] {
                let mut start = Start::default();
                start.open();
                for ch in "set".chars() {
                    start.type_char(ch);
                }
                start.filter = selected;
                let g = start.geometry(layout);
                let (pw, ph) = (1280 * scale as usize, 720 * scale as usize);
                let mut bytes = alloc::vec![0u8; pw * ph * 4];
                for p in bytes.chunks_exact_mut(4) {
                    p.copy_from_slice(&[40, 40, 40, 255]);
                }
                let mut c = Canvas::new(&mut bytes, pw as u32, ph as u32, pw * 4).unwrap();
                c.set_scale(door_graphics_core::Scale::from_factor(scale).unwrap())
                    .unwrap();
                draw(
                    &mut c,
                    &fonts,
                    &start,
                    layout,
                    &Frame {
                        pointer: (0, 0),
                        dark: false,
                        slide: 0,
                        transparency: false,
                        taskbar_pins: &App::ALL,
                    },
                );
                let mut previous: Option<Rect> = None;
                for f in Filter::ALL {
                    let r = g.chip(&fonts, f);
                    assert_eq!(
                        r.width(),
                        paint::measure(&fonts, f.label(), TextStyle::Regular14) + 24,
                        "{f:?}"
                    );
                    assert_eq!(r.height(), 32);
                    if let Some(p) = previous {
                        assert_eq!(r.x(), p.right() + 8);
                    }
                    previous = Some(r);
                    if f == selected {
                        continue;
                    }
                    // Inside the pill ends (radius 16), clear of the background.
                    let inner = Rect::new(r.x() + 4, r.y() + 8, r.width() - 8, 16).unwrap();
                    let (l, rr, _, _) = ink(
                        &bytes,
                        pw,
                        physical(inner, s),
                        palette.field,
                        if f.available() {
                            palette.text
                        } else {
                            palette.text_disabled
                        },
                    )
                    .unwrap();
                    let target = (2 * r.x() + r.width() as i32) * s;
                    assert!(
                        (l + rr - target).abs() <= s,
                        "{scale}x {f:?}: ink {l}..{rr} centre vs {target}/2"
                    );
                    // Symmetric padding: the ink keeps ~12 px from each end.
                    assert!(
                        l >= (r.x() + 10) * s && rr <= (r.right() - 10) * s,
                        "{scale}x {f:?} padding {l}..{rr} in {r:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn settings_results_deep_link_and_filters_apply() {
        let mut start = Start::default();
        for ch in "display".chars() {
            start.type_char(ch);
        }
        assert!(
            matches!(start.activate(Target::Result(0)), Activation::Settings(p) if p == search::page::DISPLAY)
        );
        start.activate(Target::Chip(Filter::Apps));
        assert!(start.results().is_empty());
    }
    #[test]
    fn search_box_takes_unicode_at_its_caret_with_cluster_safe_editing() {
        let mut start = Start::default();
        start.open();
        assert!(!start.type_text("  "), "a leading space is ignored");
        assert!(start.type_text("设置"));
        assert!(start.edit_key(0x147), "Home");
        assert!(start.type_text("系统"));
        assert_eq!(
            (start.query.as_str(), start.query_caret),
            ("系统设置", "系统".len())
        );
        assert!(start.edit_key(0x14f) && start.type_text("👍🏽"));
        assert!(start.backspace());
        assert_eq!(
            start.query, "系统设置",
            "the emoji and its skin tone go together"
        );
        assert!(start.ime_delete("设置".len(), 0));
        assert_eq!(start.query, "系统");
        // Caret keys belong to the box only while it has focus.
        start.focus = Some(Target::Pinned(0));
        assert!(!start.edit_key(0x14b));
        start.focus = Some(Target::Search);
        assert!(start.edit_key(0x14b) && start.edit_key(0x153));
        assert_eq!(start.query, "系");
        // The provider bound: 64 characters.
        start.query.clear();
        while start.type_text("字") {}
        assert_eq!(
            start.query.chars().count(),
            64.min(door_service_protocol::window::search::MAX_QUERY / 3)
        );
    }
}
