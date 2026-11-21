//! Modern Explorer geometry. Values are logical pixels at 96 DPI (= Windows
//! effective pixels); hit testing and drawing share these rectangles. The
//! vertical bands, navigation pane, Details and Tiles metrics follow the
//! owner's Windows 11 26H2 reference (`docs/design/fidelity-26h2-diff.md`
//! E1/E4/E8/E14, `reports/artifacts/reference-26h2/explorer-*.uia.tsv` at 150 %).
use door_design_tokens::{SpacingToken, spacing};
/// Navigation pane width including its 1 px divider (E4): Windows' tree is
/// 231 px plus a 2+2 px divider at 150 % (154–157 effective px); its divider
/// sits at 157 and the item view starts at 160 in `explorer-folder-details`.
pub const SIDEBAR: i32 = 157;
/// Narrowest navigation pane the splitter allows (the icons and a few
/// letters of each label stay visible; longer labels end in an ellipsis).
pub const SIDEBAR_MIN: i32 = 96;
/// Logical half-width of the splitter grab zone on the pane's right edge.
pub const SPLITTER_GRAB: i32 = 4;
/// Tab strip (caption) height; the navigation band starts under it.
pub const CAPTION: i32 = 40;
/// Navigation band (E1): Back/Forward/Up/Refresh, address and search,
/// 48 px under the caption, then a 1 px rule (Windows 72 px at 150 %).
pub const NAV_BAND: i32 = 48;
/// Command bar (E1): buttons fill it edge to edge (Windows 68/67 px).
pub const COMMAND_BAR: u32 = 45;
/// First content row: caption 40 + band 48 + rule + bar 45 + rule
/// (Windows 136; the old concept had 154).
pub const CONTENT_TOP: i32 = 136;
pub const ROW: i32 = 32;
/// Windows 11 address/search boxes are a standard 32 px control, centred in
/// the navigation band (y 40..88) under the 40 px tab caption.
pub const ADDRESS_HEIGHT: u32 = spacing(SpacingToken::ControlHeight) as u32;
pub const ADDRESS_TOP: i32 = CAPTION + (NAV_BAND - ADDRESS_HEIGHT as i32) / 2;
/// Details column header (E8): 27 px at the content top with no rule under
/// it (Windows 41 px), and the first row 6 px below it.
pub const DETAILS_HEADER: u32 = 27;
pub const DETAILS_GAP: i32 = 6;
/// Status bar (E8): 26 px (Windows 39 px) with 26 px view toggles.
pub const STATUS_BAR: u32 = 26;
pub const STATUS_TOGGLE: u32 = 26;
/// Accessibility > Text size growth of Explorer's 14 px line box: 0 at
/// 100%, 25 px at 225%. Every band holding one line of body text (address
/// row, command bar, column header, item rows, status bar) grows by it per
/// line, so the larger text is never clipped; icons, gaps, the sidebar
/// width and the caption keep their size, as on Windows.
pub fn grow() -> i32 {
    crate::palette::text_grow_i(0, door_graphics_text::TextStyle::Regular14)
}
/// Address, search and breadcrumb boxes (`ADDRESS_HEIGHT` at 100%).
pub fn address_height() -> u32 {
    ADDRESS_HEIGHT + grow() as u32
}
/// Top of the command bar band, under the navigation band and its rule
/// (89 at 100%).
pub fn command_top() -> i32 {
    CAPTION + NAV_BAND + 1 + grow()
}
/// Command bar band height including the rule under it (47 at 100%).
pub fn command_height() -> u32 {
    COMMAND_BAR + 2 + grow() as u32
}
/// Command buttons: their top and height, filling the bar (45 at 100%).
pub fn command_button() -> (i32, u32) {
    (command_top(), COMMAND_BAR + grow() as u32)
}
/// First content row, under the command bar (`CONTENT_TOP` at 100%).
pub fn content_top() -> i32 {
    CONTENT_TOP + 2 * grow()
}
/// Details column header height (`DETAILS_HEADER` at 100%).
pub fn details_header() -> u32 {
    DETAILS_HEADER + grow() as u32
}
/// Status bar height at the window bottom (`STATUS_BAR` at 100%).
pub fn status_height() -> u32 {
    STATUS_BAR + grow() as u32
}
/// Initial normal window, in logical pixels. Keep its footer and resize edge
/// above the standard48DIP taskbar, including on the smaller high-DPI modes.
pub fn initial_bounds(width: u32, height: u32) -> Bounds {
    Bounds {
        x: 56,
        y: 56,
        width: width.saturating_sub(112).min(1000),
        height: height.saturating_sub(120).min(700),
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Bounds {
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}
#[cfg(test)]
mod initial_window_tests {
    use super::*;
    #[test]
    fn cached_motion_fixture_stays_contained_and_returns_to_probe_geometry() {
        // The private4K motion consumer starts at600,72, moves to720,52,
        // then makes reciprocal trips to800,32. Keep its entire window inside
        // the work area: crossing an edge deliberately disables pixel reuse.
        let initial = initial_bounds(1280, 720);
        let mut r = initial;
        for (dx, dy) in [(0, 0), (120, -20), (80, -20), (-80, 20)] {
            r.x += dx;
            r.y += dy;
            assert!(r.x >= 0 && r.y >= 0);
            assert!(r.x as u32 + r.width <= 1280);
            assert!(r.y as u32 + r.height <= 672);
        }
        assert_eq!((r.x, r.y), (initial.x + 120, initial.y - 20));
    }
    #[test]
    fn supported_native_modes_keep_all_controls_and_footer_in_work_area() {
        for (physical_width, physical_height, scale) in [
            (1024, 768, 1),
            (1280, 720, 1),
            (1920, 1080, 2),
            (1920, 1200, 2),
            (2560, 1440, 2),
            (2560, 1600, 2),
            (3840, 2160, 3),
        ] {
            let (w, h) = (physical_width / scale, physical_height / scale);
            let r = initial_bounds(w, h);
            assert!(r.width >= 516 && r.height >= 300);
            assert!(r.x as u32 + r.width <= w - 16);
            assert!(r.y as u32 + r.height <= h - 48 - 16);
            let layout = ExplorerLayout::new(r.width, r.height);
            let footer = layout.status_view(true);
            assert!(r.contains(r.x + footer.x, r.y + footer.y));
            assert!(r.contains(
                r.x + footer.x + footer.width as i32 - 1,
                r.y + footer.y + footer.height as i32 - 1
            ));
        }
    }
}
/// Explorer's eight item views (Windows order and Ctrl+Shift+1..8 shortcuts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    ExtraLarge,
    #[default]
    Large,
    Medium,
    Small,
    List,
    Details,
    Tiles,
    Content,
}
/// How a view places consecutive items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Left to right, then the next row (icon views and Tiles).
    Grid,
    /// Top to bottom, then the next column (List).
    Columns,
    /// One full-width item per row (Details and Content).
    Rows,
}
impl ViewMode {
    pub const ALL: [Self; 8] = [
        Self::ExtraLarge,
        Self::Large,
        Self::Medium,
        Self::Small,
        Self::List,
        Self::Details,
        Self::Tiles,
        Self::Content,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::ExtraLarge => "Extra large icons",
            Self::Large => "Large icons",
            Self::Medium => "Medium icons",
            Self::Small => "Small icons",
            Self::List => "List",
            Self::Details => "Details",
            Self::Tiles => "Tiles",
            Self::Content => "Content",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(1)
    }
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    /// Ctrl+wheel order: smaller icons are later in `ALL` up to Small.
    pub fn larger(self) -> Self {
        match self {
            Self::Large => Self::ExtraLarge,
            Self::Medium => Self::Large,
            Self::Small => Self::Medium,
            Self::List | Self::Details | Self::Tiles | Self::Content => Self::Small,
            Self::ExtraLarge => Self::ExtraLarge,
        }
    }
    pub fn smaller(self) -> Self {
        match self {
            Self::ExtraLarge => Self::Large,
            Self::Large => Self::Medium,
            Self::Medium => Self::Small,
            Self::Small => Self::List,
            Self::List => Self::Details,
            other => other,
        }
    }
    pub fn flow(self) -> Flow {
        match self {
            Self::List => Flow::Columns,
            Self::Details | Self::Content => Flow::Rows,
            _ => Flow::Grid,
        }
    }
    /// Icon above its label, centered in the cell.
    pub fn stacked(self) -> bool {
        matches!(self, Self::ExtraLarge | Self::Large | Self::Medium)
    }
    pub fn icon_size(self) -> u32 {
        match self {
            Self::ExtraLarge => 192,
            Self::Large => 96,
            Self::Medium | Self::Tiles => 48,
            Self::Content => 32,
            Self::Small | Self::List | Self::Details => 16,
        }
    }
    /// Whether image thumbnails and folder previews replace the type icon.
    pub fn thumbnails(self) -> bool {
        self.icon_size() >= 48
    }
    /// Whether a folder holding an image shows its thumbnail (Medium and
    /// Large icons, as in Windows 11); other thumbnail views show sheets.
    pub fn folder_thumbnails(self) -> bool {
        matches!(self, Self::Medium | Self::Large | Self::ExtraLarge)
    }
    /// Cell size including spacing; Rows views use the content width.
    /// Heights grow by one line box per label line at the applied text size.
    fn cell(self) -> (u32, u32) {
        let (w, h) = self.base_cell();
        let lines = match self {
            Self::ExtraLarge | Self::Large | Self::Medium | Self::Content => 2,
            Self::Tiles => 3,
            _ => 1,
        };
        (w, (h as i32 + lines * grow()) as u32)
    }
    fn base_cell(self) -> (u32, u32) {
        match self {
            Self::ExtraLarge => (216, 248),
            Self::Large => (120, 150),
            Self::Medium => (86, 96),
            Self::Small => (220, 26),
            Self::List => (240, 26),
            // Windows Details rows: 43 px pitch at 150 % (E8).
            Self::Details => (0, 29),
            // Windows Tiles: 375x87 px items at a 381x93 px pitch (E14).
            Self::Tiles => (254, 62),
            Self::Content => (0, 58),
        }
    }
    /// Label lines below a stacked icon.
    pub fn label_lines(self) -> usize {
        if self.stacked() { 2 } else { 1 }
    }
}
#[derive(Clone, Copy)]
pub struct ExplorerLayout {
    pub width: u32,
    pub height: u32,
    /// Navigation pane width including its divider (`SIDEBAR` until the
    /// splitter on its right edge is dragged).
    pub sidebar: i32,
}
impl ExplorerLayout {
    /// A window of `width` x `height` with the default navigation pane.
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            sidebar: SIDEBAR,
        }
    }
    /// The navigation pane width a splitter drag asks for, kept between
    /// `SIDEBAR_MIN` and what leaves the item view 320 px (at most 600).
    pub fn clamp_sidebar(self, width: i32) -> i32 {
        let max = (self.width as i32 - 320).clamp(SIDEBAR_MIN, 600);
        width.clamp(SIDEBAR_MIN, max)
    }
    /// Whether (x, y) grabs the splitter between the navigation pane and
    /// the item view (the content band, above the status bar).
    pub fn on_splitter(self, x: i32, y: i32) -> bool {
        (x - self.sidebar).abs() <= SPLITTER_GRAB
            && y >= content_top()
            && y < self.height as i32 - status_height() as i32
    }
    fn view_left(self) -> i32 {
        self.sidebar + 12
    }
    fn view_width(self) -> u32 {
        self.width.saturating_sub(self.sidebar as u32 + 24)
    }
    /// First item row; Details reserves its column header and the gap
    /// under it.
    pub fn view_top(self, mode: ViewMode) -> i32 {
        content_top()
            + match mode {
                ViewMode::Details => details_header() as i32 + DETAILS_GAP,
                _ => 8,
            }
    }
    fn view_bottom(self) -> i32 {
        self.height as i32 - status_height() as i32 - 4
    }
    fn cell(self, mode: ViewMode) -> (u32, u32) {
        // A single column narrower than the cell shrinks to fit the view.
        let (w, h) = mode.cell();
        (
            if w == 0 {
                self.view_width()
            } else {
                w.min(self.view_width())
            },
            h,
        )
    }
    /// Rows of whole cells that fit vertically.
    pub fn view_rows(self, mode: ViewMode) -> usize {
        let (_, h) = self.cell(mode);
        (self.view_bottom() - self.view_top(mode)).max(0) as usize / h as usize
    }
    pub fn view_columns(self, mode: ViewMode) -> usize {
        match mode.flow() {
            Flow::Rows => 1,
            _ => (self.view_width() / self.cell(mode).0).max(1) as usize,
        }
    }
    pub fn capacity(self, mode: ViewMode) -> usize {
        self.view_rows(mode) * self.view_columns(mode)
    }
    /// Items per scroll step; offsets stay multiples of it so cells keep
    /// their column (Grid) or row (List) when scrolling.
    pub fn step(self, mode: ViewMode) -> usize {
        match mode.flow() {
            Flow::Grid => self.view_columns(mode),
            Flow::Columns => self.view_rows(mode).max(1),
            Flow::Rows => 1,
        }
    }
    /// Item delta for arrow keys: (left/right, up/down).
    pub fn arrow_steps(self, mode: ViewMode) -> (isize, isize) {
        match mode.flow() {
            Flow::Grid => (1, self.view_columns(mode) as isize),
            Flow::Columns => (self.view_rows(mode).max(1) as isize, 1),
            Flow::Rows => (0, 1),
        }
    }
    /// Bounds of the visible slot (item index minus offset).
    pub fn cell_bounds(self, slot: usize, mode: ViewMode) -> Bounds {
        let (w, h) = self.cell(mode);
        let (column, row) = match mode.flow() {
            Flow::Grid => (
                slot % self.view_columns(mode),
                slot / self.view_columns(mode),
            ),
            Flow::Columns => {
                let rows = self.view_rows(mode).max(1);
                (slot / rows, slot % rows)
            }
            Flow::Rows => (0, slot),
        };
        let gap = match mode.flow() {
            Flow::Rows => 0,
            _ => 4,
        };
        Bounds {
            x: self.view_left() + (column as u32 * w) as i32,
            y: self.view_top(mode) + (row as u32 * h) as i32,
            width: w.saturating_sub(gap),
            height: h.saturating_sub(match mode {
                _ if mode.stacked() => 4,
                ViewMode::Tiles => 4,
                // Windows' Details plate is 24.7 px at a 28.7 px pitch (E8);
                // 26 of 29 keeps an even spare height around the 20 px text
                // line, so labels and the F2 editor share one baseline.
                ViewMode::Details => 3,
                _ => 2,
            }),
        }
    }
    /// Icon box inside a cell.
    pub fn icon_bounds(self, cell: Bounds, mode: ViewMode) -> Bounds {
        let size = mode.icon_size().min(cell.width.saturating_sub(8));
        if mode.stacked() {
            Bounds {
                x: cell.x + (cell.width as i32 - size as i32) / 2,
                y: cell.y + 4,
                width: size,
                height: size,
            }
        } else {
            Bounds {
                x: cell.x + 6,
                y: cell.y + (cell.height as i32 - size as i32) / 2,
                width: size,
                height: size,
            }
        }
    }
    /// The (first) label line inside a cell.
    pub fn label_bounds(self, cell: Bounds, mode: ViewMode) -> Bounds {
        let icon = self.icon_bounds(cell, mode);
        if mode.stacked() {
            Bounds {
                x: cell.x + 4,
                y: icon.y + icon.height as i32 + 4,
                width: cell.width.saturating_sub(8),
                height: (18 + grow()) as u32,
            }
        } else {
            let x = icon.x + icon.width as i32 + if mode.icon_size() > 16 { 10 } else { 8 };
            let (y, height) = match mode {
                ViewMode::Tiles | ViewMode::Content => (cell.y + 6, (20 + grow()) as u32),
                _ => (cell.y, cell.height),
            };
            Bounds {
                x,
                y,
                width: (cell.x + cell.width as i32 - x - 4).max(0) as u32,
                height,
            }
        }
    }
    /// Smallest step-aligned offset that shows `position`; None without room.
    pub fn reveal_in(self, position: usize, offset: usize, mode: ViewMode) -> Option<usize> {
        let capacity = self.capacity(mode);
        if capacity == 0 {
            return None;
        }
        // Capacity is a whole number of steps (rows × columns).
        let step = self.step(mode).max(1);
        let offset = offset.min(position) / step * step;
        Some(if position - offset >= capacity {
            // Put the step containing `position` on the last visible line.
            (position / step + 1) * step - capacity
        } else {
            offset
        })
    }
    /// Inline-rename box over the label of a visible slot.
    pub fn rename_in(self, slot: usize, mode: ViewMode) -> Option<Bounds> {
        if slot >= self.capacity(mode) {
            return None;
        }
        let label = self.label_bounds(self.cell_bounds(slot, mode), mode);
        let line = (20 + grow()) as u32;
        let bounds = Bounds {
            y: label.y + (label.height.saturating_sub(line) / 2) as i32,
            height: line,
            ..label
        };
        (bounds.width >= 32
            && bounds.x >= 0
            && bounds.y >= 0
            && i64::from(bounds.x) + i64::from(bounds.width) <= i64::from(self.width)
            && i64::from(bounds.y) + i64::from(bounds.height)
                <= i64::from(self.height.saturating_sub(status_height())))
        .then_some(bounds)
    }
    pub fn properties(self) -> Bounds {
        let width = self.width.saturating_sub(16).min(420);
        let height = self.height.saturating_sub(16).min(220);
        Bounds {
            x: ((self.width - width) / 2) as i32,
            y: ((self.height - height) / 2) as i32,
            width,
            height,
        }
    }
    pub fn properties_close(self) -> Bounds {
        let r = self.properties();
        Bounds {
            x: r.x + r.width.saturating_sub(100) as i32,
            y: r.y + r.height.saturating_sub(42) as i32,
            width: 80.min(r.width),
            height: 32.min(r.height),
        }
    }
    /// Status bar view toggles (E8): 26 px squares, Details then Large
    /// icons, 4 px from the right edge and centred in the bar.
    pub fn status_view(self, icons: bool) -> Bounds {
        let bar = status_height() as i32;
        let side = STATUS_TOGGLE as i32;
        Bounds {
            x: self.width as i32 - 4 - side * if icons { 1 } else { 2 },
            y: self.height as i32 - bar + (bar - side) / 2,
            width: STATUS_TOGGLE,
            height: STATUS_TOGGLE,
        }
    }
    /// Back/Forward/Up/Refresh: icon buttons keep their 48x32 size and stay
    /// centred on the (text-grown) address row.
    pub fn navigation(index: usize) -> Option<Bounds> {
        let (x, width) = *[(8, 48), (66, 48), (124, 48), (182, 48)].get(index)?;
        Some(Bounds {
            x,
            y: ADDRESS_TOP + grow() / 2,
            width,
            height: ADDRESS_HEIGHT,
        })
    }
    pub fn address(self) -> Bounds {
        Bounds {
            x: 242,
            y: ADDRESS_TOP,
            width: self
                .width
                .saturating_sub(252 + (self.width / 4).clamp(160, 520))
                .max(32),
            height: address_height(),
        }
    }
    pub fn search(self) -> Bounds {
        let address = self.address();
        let x = address.x + address.width as i32 + 10;
        Bounds {
            x,
            y: ADDRESS_TOP,
            width: self.width.saturating_sub(x as u32 + 12),
            height: address_height(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const SIZES: [(u32, u32); 6] = [
        (240, 200),
        (490, 498),
        (742, 498),
        (1000, 700),
        (1280, 720),
        (2048, 1200),
    ];
    #[test]
    fn every_view_keeps_cells_inside_the_content_area_without_overlap() {
        for (width, height) in SIZES {
            let l = ExplorerLayout::new(width, height);
            let status = l.status_view(false);
            for mode in ViewMode::ALL {
                let cells: alloc::vec::Vec<_> = (0..l.capacity(mode))
                    .map(|i| l.cell_bounds(i, mode))
                    .collect();
                for (i, r) in cells.iter().enumerate() {
                    assert!(r.x >= SIDEBAR && r.y >= CONTENT_TOP, "{mode:?} {i}");
                    assert!(
                        r.x + r.width as i32 <= width as i32,
                        "{mode:?} {i} at {width}"
                    );
                    assert!(r.y + r.height as i32 <= height as i32 - 24, "{mode:?} {i}");
                    assert!(!r.contains(status.x, status.y));
                    for other in &cells[..i] {
                        assert!(!other.contains(r.x, r.y), "{mode:?} overlap {i}");
                    }
                    // The label never leaves its own cell.
                    let label = l.label_bounds(*r, mode);
                    assert!(label.x >= r.x && label.x + label.width as i32 <= r.x + r.width as i32);
                }
            }
        }
    }
    #[test]
    fn views_have_distinct_geometry_and_density() {
        let l = ExplorerLayout::new(1280, 720);
        let caps: alloc::vec::Vec<_> = ViewMode::ALL.iter().map(|m| l.capacity(*m)).collect();
        // Smaller icons show more items; List/Small are the densest views.
        assert!(caps[0] < caps[1] && caps[1] < caps[2] && caps[2] < caps[3]);
        assert!(caps[4] >= caps[5] && caps[5] > caps[7]);
        assert_eq!(l.view_columns(ViewMode::Details), 1);
        assert_eq!(
            l.cell_bounds(1, ViewMode::List).x,
            l.cell_bounds(0, ViewMode::List).x
        );
        assert!(l.cell_bounds(1, ViewMode::Small).x > l.cell_bounds(0, ViewMode::Small).x);
        for mode in ViewMode::ALL {
            assert_eq!(ViewMode::from_index(mode.index()), Some(mode));
        }
    }
    #[test]
    fn reveal_is_step_aligned_and_shows_the_position_or_refuses_zero_capacity() {
        for (width, height) in SIZES.into_iter().chain([(400, 150), (320, 164)]) {
            let l = ExplorerLayout::new(width, height);
            for mode in ViewMode::ALL {
                let mut offset = 0;
                for position in [0, 1, 17, 5, 200, 3, 199, 0] {
                    match l.reveal_in(position, offset, mode) {
                        Some(next) => {
                            assert_eq!(next % l.step(mode).max(1), 0, "{mode:?}");
                            assert!(next <= position && position - next < l.capacity(mode));
                            offset = next;
                        }
                        None => assert_eq!(l.capacity(mode), 0),
                    }
                }
            }
        }
    }
    #[test]
    fn rename_box_sits_on_the_label_or_is_refused() {
        for (width, height) in SIZES {
            let l = ExplorerLayout::new(width, height);
            for mode in ViewMode::ALL {
                assert_eq!(l.rename_in(usize::MAX, mode), None);
                for slot in 0..l.capacity(mode).min(8) {
                    if let Some(r) = l.rename_in(slot, mode) {
                        let cell = l.cell_bounds(slot, mode);
                        assert!(cell.contains(r.x, r.y));
                        assert!(r.x + r.width as i32 <= width as i32);
                    }
                }
            }
        }
    }
    #[test]
    fn metadata_dialog_and_its_close_button_stay_inside_small_clients() {
        for (width, height) in [(160, 120), (320, 160), (742, 498), (1920, 1080)] {
            let layout = ExplorerLayout::new(width, height);
            let panel = layout.properties();
            let close = layout.properties_close();
            assert!(panel.x >= 0 && panel.y >= 0);
            assert!(
                panel.x + panel.width as i32 <= width as i32
                    && panel.y + panel.height as i32 <= height as i32
            );
            assert!(panel.contains(close.x, close.y));
        }
    }
    #[test]
    fn reference_address_split_is_preserved() {
        let normal = ExplorerLayout::new(742, 498);
        assert_eq!(normal.address().width, 305);
        assert_eq!(normal.search().x, 557);
        assert_eq!(normal.search().width, 173);
        for i in 0..4 {
            let r = ExplorerLayout::navigation(i).unwrap();
            assert!(r.x + r.width as i32 <= 242);
        }
    }
}
