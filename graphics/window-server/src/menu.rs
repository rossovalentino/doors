//! Windows 11 shell menus (context menus, jump lists, the power menu and the
//! Win+X quick-link menu). Paint and hit testing share one geometry. Items
//! without a working backend are present but disabled, never fake actions.
extern crate alloc;
use crate::apps::App;
use crate::paint::{self, Mask, Palette};
use alloc::{boxed::Box, string::String, vec::Vec};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
/// Everything a menu item can ask the shell to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Launch(App),
    NewWindow(App),
    PinTaskbar(App),
    UnpinTaskbar(App),
    PinStart(App),
    UnpinStart(App),
    MoveToFront(App),
    CloseWindows(App),
    /// Open Settings at a route (`search::page`).
    Settings(u32),
    Refresh,
    ToggleDesktopIcons,
    SortByName,
    ShowMoreOptions,
    StartView(u8),
    Shutdown,
    Restart,
    SignOut,
    Lock,
    Search,
    ShowDesktop,
    /// Win+R: the Run dialog.
    Run,
    /// Start > Recent item: "Open file location" / "Remove from list".
    RecentLocation(u8),
    RecentRemove(u8),
    /// Opens the item's submenu.
    Submenu(u8),
    /// A door.shell command (`shell::command`) for a registered taskbar
    /// group or catalog key, carried out by the desktop shell.
    Shell(u32, u32),
    /// Close every window of the extra taskbar button with this key.
    CloseExtra(u32),
    /// System > Advanced > End task: end the app's processes (taskbar
    /// right-click), for an app button or an extra button's key.
    EndTask(App),
    EndTaskExtra(u32),
    /// The desktop Recycle Bin: open, empty or properties (`menus::BIN_*`).
    RecycleBin(u8),
    /// Win+X > Terminal (Admin): User Account Control, then the elevated
    /// Terminal.
    AdminTerminal,
    Nothing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Glyph(Mask),
    App(App),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Check {
    #[default]
    None,
    Checked,
    Radio,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub icon: Option<Icon>,
    pub enabled: bool,
    /// Keyboard shortcut or a short disabled reason, right aligned.
    pub detail: Option<&'static str>,
    pub check: Check,
    pub command: Command,
}
impl Item {
    pub fn new(label: &str, command: Command) -> Self {
        Self {
            label: String::from(label),
            icon: None,
            enabled: true,
            detail: None,
            check: Check::None,
            command,
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn glyph(self, mask: Mask) -> Self {
        self.icon(Icon::Glyph(mask))
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn enabled_if(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    pub fn detail(mut self, detail: &'static str) -> Self {
        self.detail = Some(detail);
        self
    }
    pub fn checked(mut self, check: Check) -> Self {
        self.check = check;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Item(Item),
    Separator,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    /// Windows 11 rounded menu (14 px text, 32 px rows).
    #[default]
    Fluent,
    /// "Show more options" classic menu (12 px text, compact rows).
    Classic,
    /// The classic-menu setting: Windows 10's flat menu, measured from a
    /// 100% capture (square #A0A0A0 frame, 22 px rows, 7 px separators,
    /// text 36 px in, grey hot-tracking, a short drop shadow).
    Win10,
}
/// How a `Anchor::Bar` menu lines up with its button along the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarAlign {
    Center,
    /// Left edges on a horizontal bar, top edges on a vertical one.
    Start,
}
/// Where the menu opens relative to its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Context menu at the pointer: down-right, flipped to fit.
    Point(i32, i32),
    /// Above a taskbar target (bottom bar), horizontally centered on it.
    Above(Rect),
    /// Above a target, right edges aligned (Start power/user menus).
    AboveRight(Rect),
    /// Above a target, left edges aligned.
    AboveLeft(Rect),
    /// Below a target, left aligned (dropdowns).
    Below(Rect),
    /// Off a taskbar target on the work-area side of a bar at that edge
    /// (above a bottom bar, below a top one, beside a left or right one:
    /// the 26H2 positional taskbar), lined up along the bar by `BarAlign`.
    Bar(Rect, door_service_protocol::desktop::Edge, BarAlign),
    /// Beside a parent item (submenus).
    Beside(Rect),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub entries: Vec<Entry>,
    pub style: Style,
    pub bounds: Rect,
    pub focus: Option<usize>,
    pub child: Option<Box<Menu>>,
    /// Caller tag identifying the menu's purpose.
    pub tag: u32,
    min_width: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuHit {
    /// Item (depth, index) — depth 0 is the root menu.
    Item(usize, usize),
    Inside,
    Outside,
}
const PAD: i32 = 4;
impl Style {
    /// Item row height, grown with Accessibility > Text size by the growth
    /// of the row's label line box (separators, icons and padding keep
    /// their size, as in Windows).
    fn row(self) -> u32 {
        let base = match self {
            Self::Fluent => 32,
            Self::Classic => 26,
            Self::Win10 => 22,
        };
        paint::text_grow(base, self.text())
    }
    fn separator(self) -> u32 {
        match self {
            Self::Fluent => 9,
            Self::Classic | Self::Win10 => 7,
        }
    }
    fn text(self) -> TextStyle {
        match self {
            Self::Fluent => TextStyle::Regular14,
            Self::Classic | Self::Win10 => TextStyle::Regular12,
        }
    }
    fn radius(self) -> u32 {
        match self {
            Self::Win10 => 0,
            _ => 8,
        }
    }
    /// Panel padding around the rows: (sides, top and bottom). Windows 10
    /// rows sit 3 px in from the frame's sides and 5 px from its top.
    fn pad(self) -> (i32, i32) {
        match self {
            Self::Win10 => (3, 5),
            _ => (PAD, PAD),
        }
    }
}
impl Menu {
    pub fn new(entries: Vec<Entry>, style: Style, tag: u32) -> Self {
        Self {
            entries,
            style,
            bounds: Rect::new(0, 0, 1, 1).unwrap(),
            focus: None,
            child: None,
            tag,
            min_width: 0,
        }
    }
    pub fn min_width(mut self, width: u32) -> Self {
        self.min_width = width;
        self
    }
    fn has_icons(&self) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(e, Entry::Item(i) if i.icon.is_some() || i.check != Check::None))
    }
    fn text_left(&self) -> i32 {
        match (self.style, self.has_icons()) {
            (Style::Fluent, true) => 44,
            (Style::Fluent, false) => 16,
            (Style::Classic, true) => 36,
            (Style::Classic, false) => 12,
            // Windows 10 keeps its icon column whether or not it is used.
            (Style::Win10, _) => 36,
        }
    }
    /// Space after the label: the submenu arrow column or end padding.
    fn right_pad(&self, item: &Item) -> i32 {
        match (self.style, matches!(item.command, Command::Submenu(_))) {
            (Style::Win10, true) => 24,
            (Style::Win10, false) => 20,
            (_, true) => 36,
            (_, false) => 12,
        }
    }
    /// Natural size of the menu for its entries.
    pub fn measure(&self, fonts: &FontAtlas) -> (u32, u32) {
        let style = self.style.text();
        let mut width = 0;
        let mut height = 2 * self.style.pad().1 as u32;
        for entry in &self.entries {
            match entry {
                Entry::Separator => height += self.style.separator(),
                Entry::Item(item) => {
                    height += self.style.row();
                    let mut w = self.text_left() as u32 + paint::measure(fonts, &item.label, style);
                    if let Some(detail) = item.detail {
                        w += 32 + paint::measure(fonts, detail, TextStyle::Regular12);
                    }
                    if self.style == Style::Win10 {
                        // Windows 10 sizes the frame to the text plus its
                        // shortcut/arrow column.
                        width = width.max(w + self.right_pad(item) as u32);
                        continue;
                    }
                    if matches!(item.command, Command::Submenu(_)) {
                        w += 32;
                    }
                    width = width.max(w + 24);
                }
            }
        }
        // The widest-menu cap grows with the text it holds.
        (
            width.max(self.min_width).clamp(160, paint::text_len(420)),
            height,
        )
    }
    /// Lay the menu out inside `area` (logical work area plus bar).
    pub fn place(&mut self, fonts: &FontAtlas, anchor: Anchor, area: Rect) {
        let (w, h) = self.measure(fonts);
        let (w, h) = (w.min(area.width()), h.min(area.height()));
        let (mut x, mut y) = match anchor {
            Anchor::Point(x, y) => {
                let x = if x + w as i32 > area.right() {
                    x - w as i32
                } else {
                    x
                };
                let y = if y + h as i32 > area.bottom() {
                    y - h as i32
                } else {
                    y
                };
                (x, y)
            }
            Anchor::Above(r) => (
                r.x() + r.width() as i32 / 2 - w as i32 / 2,
                r.y() - 12 - h as i32,
            ),
            Anchor::AboveRight(r) => (r.right() - w as i32, r.y() - 4 - h as i32),
            Anchor::AboveLeft(r) => (r.x(), r.y() - 4 - h as i32),
            Anchor::Below(r) => (r.x(), r.bottom() + 4),
            Anchor::Bar(r, edge, align) => {
                use door_service_protocol::desktop::Edge;
                let gap = if align == BarAlign::Center { 12 } else { 4 };
                let along = |start: i32, len: u32, size: u32| match align {
                    BarAlign::Start => start,
                    BarAlign::Center => start + len as i32 / 2 - size as i32 / 2,
                };
                match edge {
                    Edge::Bottom => (along(r.x(), r.width(), w), r.y() - gap - h as i32),
                    Edge::Top => (along(r.x(), r.width(), w), r.bottom() + gap),
                    Edge::Left => (r.right() + gap, along(r.y(), r.height(), h)),
                    Edge::Right => (r.x() - gap - w as i32, along(r.y(), r.height(), h)),
                }
            }
            // Windows 10 submenus overlap the parent frame by 5 px and line
            // their first row up with the parent row.
            Anchor::Beside(r) if self.style == Style::Win10 => {
                let (side, top) = self.style.pad();
                let right = r.right() + side - 6;
                if right + w as i32 <= area.right() {
                    (right, r.y() - top)
                } else {
                    (r.x() - side + 6 - w as i32, r.y() - top)
                }
            }
            Anchor::Beside(r) => {
                let right = r.right() + 4;
                if right + w as i32 <= area.right() {
                    (right, r.y() - PAD)
                } else {
                    (r.x() - 4 - w as i32, r.y() - PAD)
                }
            }
        };
        x = x.clamp(area.x(), area.right() - w as i32);
        y = y.clamp(area.y(), area.bottom() - h as i32);
        self.bounds = Rect::new(x, y, w, h).unwrap_or(self.bounds);
    }
    /// Every entry band in order: (index, band, separator).
    pub fn slots(&self) -> impl Iterator<Item = (usize, Rect, bool)> + '_ {
        let (side, top) = self.style.pad();
        let mut y = self.bounds.y() + top;
        let x = self.bounds.x() + side;
        let w = self.bounds.width().saturating_sub(2 * side as u32).max(1);
        self.entries.iter().enumerate().map(move |(i, entry)| {
            let (h, separator) = match entry {
                Entry::Separator => (self.style.separator(), true),
                Entry::Item(_) => (self.style.row(), false),
            };
            let r = Rect::new(x, y, w, h).unwrap();
            y += h as i32;
            (i, r, separator)
        })
    }
    /// Row rectangles (hover plates) in entry order; separators yield None.
    pub fn rows(&self) -> impl Iterator<Item = (usize, Option<Rect>)> + '_ {
        self.slots()
            .map(|(i, r, separator)| (i, (!separator).then_some(r)))
    }
    pub fn row(&self, index: usize) -> Option<Rect> {
        self.rows().find(|(i, _)| *i == index).and_then(|(_, r)| r)
    }
    /// The label's text box in item `index`'s row: after the icon column,
    /// before the submenu chevron (or the end padding).
    pub fn label_rect(&self, index: usize) -> Option<Rect> {
        let row = self.row(index)?;
        let item = self.item(index)?;
        let left = self.bounds.x() + self.text_left();
        let right_pad = self.right_pad(item);
        Rect::new(
            left,
            row.y(),
            (row.right() - right_pad - left).max(1) as u32,
            row.height(),
        )
        .ok()
    }
    pub fn item(&self, index: usize) -> Option<&Item> {
        match self.entries.get(index) {
            Some(Entry::Item(item)) => Some(item),
            _ => None,
        }
    }
    /// Deepest open menu first.
    pub fn hit(&self, x: i32, y: i32) -> MenuHit {
        self.hit_depth(x, y, 0)
    }
    fn hit_depth(&self, x: i32, y: i32, depth: usize) -> MenuHit {
        if let Some(child) = &self.child {
            match child.hit_depth(x, y, depth + 1) {
                MenuHit::Outside => (),
                hit => return hit,
            }
        }
        if !self.bounds.contains(x, y) {
            return MenuHit::Outside;
        }
        self.rows()
            .find(|(_, r)| r.is_some_and(|r| r.contains(x, y)))
            .map_or(MenuHit::Inside, |(i, _)| MenuHit::Item(depth, i))
    }
    pub fn at_depth(&self, depth: usize) -> Option<&Menu> {
        if depth == 0 {
            Some(self)
        } else {
            self.child.as_deref()?.at_depth(depth - 1)
        }
    }
    pub fn at_depth_mut(&mut self, depth: usize) -> Option<&mut Menu> {
        if depth == 0 {
            Some(self)
        } else {
            self.child.as_deref_mut()?.at_depth_mut(depth - 1)
        }
    }
    pub fn depth(&self) -> usize {
        self.child.as_ref().map_or(0, |c| 1 + c.depth())
    }
    /// Bounding box of the whole open menu chain.
    pub fn extent(&self) -> Rect {
        match &self.child {
            Some(child) => paint::union(self.bounds, child.extent()),
            None => self.bounds,
        }
    }
    fn selectable(&self, index: usize) -> bool {
        self.item(index).is_some_and(|i| i.enabled)
    }
    /// Keyboard focus step (+1/-1), skipping separators and disabled items.
    pub fn step(&mut self, forward: bool) {
        let n = self.entries.len();
        if n == 0 {
            return;
        }
        let start = self.focus.unwrap_or(if forward { n - 1 } else { 0 });
        for k in 1..=n {
            let i = if forward {
                (start + k) % n
            } else {
                (start + n - k % n) % n
            };
            if self.selectable(i) {
                self.focus = Some(i);
                return;
            }
        }
    }
    pub fn focused_command(&self) -> Option<Command> {
        let item = self.item(self.focus?)?;
        item.enabled.then_some(item.command)
    }
}
fn draw_icon(c: &mut Canvas<'_>, icon: Icon, x: i32, y: i32, color: [u8; 4]) {
    match icon {
        Icon::Glyph(mask) => paint::glyph(c, x, y, 16, mask, color),
        Icon::App(app) => crate::apps::draw_icon(c, x, y, app, 16),
    }
}
/// Paint the menu chain with its own shadow and Windows 11 flyout surface.
pub fn draw(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    menu: &Menu,
    palette: &Palette,
    pointer: (i32, i32),
) {
    if menu.style == Style::Win10 {
        draw_win10(c, fonts, menu, palette, pointer);
        if let Some(child) = &menu.child {
            draw(c, fonts, child, palette, pointer);
        }
        return;
    }
    let b = menu.bounds;
    let radius = menu.style.radius();
    paint::shadow(c, b, radius, palette.dark);
    paint::framed(c, b, radius, palette.menu_border, palette.menu);
    let style = menu.style.text();
    let open_child = menu.child.as_ref().map(|_| ());
    for (i, row, separator) in menu.slots() {
        if separator {
            let inset = if menu.style == Style::Fluent { 0 } else { 4 };
            paint::rect(
                c,
                b.x() + inset,
                row.y() + row.height() as i32 / 2,
                b.width() - 2 * inset as u32,
                1,
                palette.menu_border,
            );
            continue;
        }
        let Some(item) = menu.item(i) else {
            continue;
        };
        let hovered = row.contains(pointer.0, pointer.1);
        let focused = menu.focus == Some(i);
        let submenu_open = open_child.is_some() && menu.focus == Some(i);
        if item.enabled && (hovered || focused || submenu_open) {
            paint::rounded(c, row, 4, palette.subtle_hover);
        }
        let color = if item.enabled {
            palette.text
        } else {
            palette.text_disabled
        };
        let icon_y = row.y() + (row.height() as i32 - 16) / 2;
        let icon_x = row.x() + if menu.style == Style::Fluent { 12 } else { 8 };
        match item.check {
            Check::Checked => paint::glyph(
                c,
                icon_x,
                icon_y,
                16,
                crate::shell_mask!("shell-check", 16),
                color,
            ),
            Check::Radio => paint::circle(c, icon_x + 8, icon_y + 8, 3, color),
            Check::None => {
                if let Some(icon) = item.icon {
                    draw_icon(c, icon, icon_x, icon_y, color);
                }
            }
        }
        let right_pad = if matches!(item.command, Command::Submenu(_)) {
            36
        } else {
            12
        };
        let label = menu.label_rect(i).unwrap_or(row);
        paint::text_row(
            c,
            fonts,
            label.x(),
            label.y(),
            label.height(),
            &item.label,
            style,
            color,
            label.width(),
        );
        if let Some(detail) = item.detail {
            paint::text_right(
                c,
                fonts,
                row.right() - right_pad,
                row.y(),
                row.height(),
                detail,
                TextStyle::Regular12,
                palette.text_secondary,
            );
        }
        if matches!(item.command, Command::Submenu(_)) {
            paint::glyph(
                c,
                row.right() - 24,
                row.y() + (row.height() as i32 - 12) / 2,
                12,
                crate::shell_mask!("shell-chevron-right-small", 12),
                color,
            );
        }
        if focused && !hovered {
            paint::outline(c, row, palette.focus, 1);
        }
    }
    if let Some(child) = &menu.child {
        draw(c, fonts, child, palette, pointer);
    }
}
/// Windows 10 menu colours, sampled from 100% captures of its light and
/// dark menus: frame, panel, hot plate, rule, text, disabled text. Contrast
/// themes keep their system colours.
fn win10_ink(palette: &Palette) -> [[u8; 4]; 6] {
    let grey = |v: u8| [v, v, v, 255];
    if paint::contrast().is_some() {
        [
            palette.menu_border,
            palette.menu,
            palette.subtle_hover,
            palette.menu_border,
            palette.text,
            palette.text_disabled,
        ]
    } else if palette.dark {
        [
            grey(160),
            grey(43),
            grey(65),
            grey(128),
            grey(255),
            grey(109),
        ]
    } else {
        [
            grey(160),
            grey(242),
            grey(217),
            grey(145),
            grey(0),
            grey(109),
        ]
    }
}
/// The classic-menu setting's Windows 10 menu: square frame with a short
/// drop shadow, rules 11 px short of the frame, keyboard focus and hover
/// both shown as the hot plate (disabled rows too, under the pointer).
fn draw_win10(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    menu: &Menu,
    palette: &Palette,
    pointer: (i32, i32),
) {
    let [frame, panel, hot, rule, text, disabled] = win10_ink(palette);
    let b = menu.bounds;
    if paint::contrast().is_none() {
        for (i, a) in [72u8, 46, 22, 8].into_iter().enumerate() {
            let i = i as i32;
            paint::rect(
                c,
                b.right() + i,
                b.y() + 4,
                1,
                b.height().saturating_sub(4) + i as u32,
                [0, 0, 0, a],
            );
            paint::rect(
                c,
                b.x() + 4,
                b.bottom() + i,
                b.width().saturating_sub(4) + i as u32,
                1,
                [0, 0, 0, a],
            );
        }
    }
    paint::framed(c, b, 0, frame, panel);
    let style = menu.style.text();
    for (i, row, separator) in menu.slots() {
        if separator {
            paint::rect(
                c,
                b.x() + 11,
                row.y() + 3,
                b.width().saturating_sub(22),
                1,
                rule,
            );
            continue;
        }
        let Some(item) = menu.item(i) else {
            continue;
        };
        let hovered = row.contains(pointer.0, pointer.1);
        if hovered || (item.enabled && menu.focus == Some(i)) {
            paint::fill(c, row, hot);
        }
        let color = if item.enabled { text } else { disabled };
        let icon_y = row.y() + (row.height() as i32 - 16) / 2;
        let icon_x = b.x() + 11;
        match item.check {
            Check::Checked => paint::glyph(
                c,
                icon_x,
                icon_y,
                16,
                crate::shell_mask!("shell-check", 16),
                color,
            ),
            Check::Radio => paint::circle(c, icon_x + 8, icon_y + 8, 3, color),
            Check::None => {
                if let Some(icon) = item.icon {
                    draw_icon(c, icon, icon_x, icon_y, color);
                }
            }
        }
        let label = menu.label_rect(i).unwrap_or(row);
        paint::text_row(
            c,
            fonts,
            label.x(),
            label.y(),
            label.height(),
            &item.label,
            style,
            color,
            label.width(),
        );
        if let Some(detail) = item.detail {
            paint::text_right(
                c,
                fonts,
                b.right() - 20,
                row.y(),
                row.height(),
                detail,
                TextStyle::Regular12,
                color,
            );
        }
        if matches!(item.command, Command::Submenu(_)) {
            win10_chevron(
                c,
                b.right() - 23,
                row.y() + (row.height() as i32 - 14) / 2,
                color,
            );
        }
    }
}
/// Windows 10's submenu arrow: a thin chevron, 7x12 ink in a 14 px box at
/// logical (x, y), stroked at the canvas density.
fn win10_chevron(c: &mut Canvas<'_>, x: i32, y: i32, color: [u8; 4]) {
    let scale = c.scale();
    let s = scale.percent() as f32 / 100.0;
    let side = scale.len(14) as i32;
    let (ox, oy) = (scale.px(x), scale.px(y));
    let points = [(4.5f32, 1.5f32), (10.5, 7.0), (4.5, 12.5)];
    let half = 0.5 * s;
    let Some(clip) = c.clip_rect() else {
        return;
    };
    for py in 0..side {
        for px in 0..side {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let mut d = f32::MAX;
            for pair in points.windows(2) {
                let (ax, ay) = (pair[0].0 * s, pair[0].1 * s);
                let (bx, by) = (pair[1].0 * s, pair[1].1 * s);
                let (dx, dy) = (bx - ax, by - ay);
                let t = (((fx - ax) * dx + (fy - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let (cx, cy) = (ax + t * dx - fx, ay + t * dy - fy);
                d = d.min(sqrt(cx * cx + cy * cy));
            }
            let coverage = (half + 0.5 - d).clamp(0.0, 1.0);
            let a = (coverage * 255.0 + 0.5) as u8;
            let (tx, ty) = (ox + px, oy + py);
            if a > 0 && clip.contains(tx, ty) {
                let _ = c.pixel(tx, ty, paint::alpha(color, a));
            }
        }
    }
}
/// Newton square root (no_std has no float sqrt without libm).
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = if v > 1.0 { v } else { 1.0 };
    for _ in 0..16 {
        x = 0.5 * (x + v / x);
    }
    x
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn sample() -> Menu {
        Menu::new(
            vec![
                Entry::Item(Item::new("View", Command::Submenu(1))),
                Entry::Item(Item::new("Refresh", Command::Refresh)),
                Entry::Separator,
                Entry::Item(Item::new("New", Command::Nothing).disabled()),
                Entry::Item(Item::new("Display settings", Command::Settings(13))),
            ],
            Style::Fluent,
            7,
        )
    }
    #[test]
    fn windows_10_menus_use_the_measured_metrics() {
        let fonts = FontAtlas::windows_reference().unwrap();
        let area = Rect::new(0, 0, 1280, 672).unwrap();
        let mut m = sample();
        m.style = Style::Win10;
        m.place(&fonts, Anchor::Point(100, 100), area);
        let b = m.bounds;
        let rows: Vec<Rect> = m.rows().filter_map(|(_, r)| r).collect();
        // Rows 3 px in from the sides, 5 px from the top, 22 px tall; the
        // separator after Refresh is 7 px.
        assert_eq!((rows[0].x() - b.x(), rows[0].y() - b.y()), (3, 5));
        assert_eq!(rows[0].width(), b.width() - 6);
        assert_eq!(rows[0].height(), 22);
        assert_eq!(rows[2].y(), rows[1].bottom() + 7);
        assert_eq!(b.bottom(), rows[3].bottom() + 5);
        // The submenu overlaps the frame by 5 px, first row level with View.
        let mut sub = sample();
        sub.style = Style::Win10;
        sub.place(&fonts, Anchor::Beside(rows[0]), area);
        assert_eq!(sub.bounds.x(), b.right() - 6);
        assert_eq!(sub.rows().find_map(|(_, r)| r).unwrap().y(), rows[0].y());
    }
    #[test]
    fn placement_flips_to_fit_and_rows_match_hits() {
        let fonts = FontAtlas::windows_reference().unwrap();
        let area = Rect::new(0, 0, 1280, 672).unwrap();
        let mut m = sample();
        m.place(&fonts, Anchor::Point(1270, 660), area);
        assert_eq!(area.intersection(m.bounds), Some(m.bounds));
        assert!(m.bounds.right() <= 1270 && m.bounds.bottom() <= 660);
        let mut seen = 0;
        for (i, row) in m.rows() {
            if let Some(r) = row {
                assert_eq!(m.hit(r.x() + 2, r.y() + 2), MenuHit::Item(0, i));
                assert_eq!(m.hit(r.right() - 1, r.bottom() - 1), MenuHit::Item(0, i));
                assert_eq!(area.intersection(r), Some(r));
                seen += 1;
            } else {
                assert!(matches!(m.entries[i], Entry::Separator));
            }
        }
        assert_eq!(seen, 4);
        assert_eq!(m.hit(m.bounds.x() + 1, m.bounds.y() + 1), MenuHit::Inside);
        assert_eq!(m.hit(m.bounds.x() - 1, m.bounds.y()), MenuHit::Outside);
        let mut bar = sample();
        let target = Rect::new(600, 676, 40, 40).unwrap();
        bar.place(
            &fonts,
            Anchor::Above(target),
            Rect::new(0, 0, 1280, 720).unwrap(),
        );
        assert_eq!(bar.bounds.bottom(), target.y() - 12);
    }
    #[test]
    fn keyboard_focus_skips_separators_and_disabled_items() {
        let mut m = sample();
        m.step(true);
        assert_eq!(m.focus, Some(0));
        m.step(true);
        assert_eq!(m.focus, Some(1));
        m.step(true);
        assert_eq!(m.focus, Some(4), "separator and disabled New are skipped");
        m.step(true);
        assert_eq!(m.focus, Some(0));
        m.step(false);
        assert_eq!(m.focus, Some(4));
        assert_eq!(m.focused_command(), Some(Command::Settings(13)));
    }
}
