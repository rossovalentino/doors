//! Windows 11 Explorer menus: the compact context menu (icon row + commands),
//! the classic menu behind "Show more options", and flyout dropdowns
//! (address-bar chevrons, command-bar Sort/View/New, history). The model,
//! geometry, keyboard and pointer behavior live here so painting and hit
//! testing share one layout; the app maps actions to real operations and
//! glyphs to its icon set. Commands without a provider are present but
//! disabled with the reason shown on hover, never silently inert.
extern crate alloc;
use crate::explorer_layout::Bounds;
use alloc::{string::String, vec::Vec};
use door_service_protocol::desktop::Controls;

/// Reasons shown for commands that have no provider on this system.
pub const NO_CLIPBOARD: &str = "No shared clipboard is available on this system.";
pub const NO_SHARE: &str = "No sharing provider is installed.";
pub const PINS_FIXED: &str = "Quick access pins cannot be changed in this version.";
pub const PIN_KNOWN: &str = "Known folders always stay in Quick access.";
pub const PIN_FOLDERS: &str = "Only folders can be pinned to Quick access.";
pub const TERMINAL_HOME: &str = "Terminal can only start in your home folder.";
pub const RENAME_DRIVE: &str = "Volume labels cannot be changed.";
pub const RENAME_TREE: &str = "Rename this folder in the file list.";
pub const MULTI_RENAME: &str = "Select a single item to rename it.";
pub const MULTI_DELETE: &str = "Delete one item at a time in this version.";
pub const BIN_EMPTY: &str = "The Recycle Bin is empty.";
pub const SELECT_FIRST: &str = "Select a file or folder first.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cut,
    Copy,
    Paste,
    Rename,
    Share,
    Delete,
    Open,
    OpenNewTab,
    OpenNewWindow,
    PinQuickAccess,
    CopyPath,
    Compress,
    Properties,
    ShowMore,
    OpenTerminal,
    Refresh,
    NewFolder,
    NewTextDocument,
    /// View mode index (`ViewMode::ALL` order).
    View(u8),
    /// Sort column index (`SortKey::ALL` order).
    Sort(u8),
    Ascending,
    Descending,
    Expand,
    Collapse,
    /// Row of a location list the caller keeps beside the menu.
    Pick(u16),
    RenameTag(u8),
    DeleteTag(u8),
    RemoveTag(u8),
    FilterTag(u8),
    SelectAll,
    SelectNone,
    InvertSelection,
    /// 26H2: Settings > Personalization > Context menu.
    Customize,
    /// Home: "Add to Favorites" / "Remove from Favorites".
    Favorite,
    /// Home > Recent: "Remove from list".
    RemoveRecent,
    /// "Open file location": the parent folder with the item selected.
    OpenLocation,
    /// Opens the entry's children.
    Submenu,
    /// Recycle Bin: put the selected items back where they were deleted.
    Restore,
    /// Doors extra, View > Show > Folder sizes (Details' Size column).
    FolderSizes,
    /// Recycle Bin: "Restore all items".
    RestoreAll,
    /// Archives: "Extract All..." into a folder named after the archive.
    ExtractAll,
    /// Files Preview converts: open them there (Convert list, password).
    Convert,
    /// Recycle Bin: delete every item permanently (confirmed).
    EmptyRecycleBin,
    /// "Open with" > an app, by its row in the list the caller keeps.
    OpenWith(u8),
    /// "Open with" > "Choose another app" (the app picker).
    ChooseApp,
    /// Programs and batch files: User Account Control, then run elevated.
    RunAsAdministrator,
    /// Windows Installer packages: msiexec /i, /f and /x.
    Install,
    Repair,
    Uninstall,
    /// Doors tags: the Tags tab with its tag editor for the item.
    AddTag,
}

/// Leading glyph role; the app maps it to its own icon assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    None,
    /// The glyph that belongs to the entry's action.
    Action,
    Folder,
    Drive,
    ThisPc,
    Network,
    Home,
    /// Pinned known folder by `FOLDERS` index.
    Known(u8),
    /// A user tag's color dot.
    Tag(u8),
    /// A view mode's own glyph.
    ViewMode(u8),
    /// An app's icon by `door_service_protocol::shell::icon` id.
    App(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub shortcut: &'static str,
    pub action: Action,
    pub glyph: Glyph,
    pub enabled: bool,
    /// Why a disabled command is unavailable (shown on hover).
    pub reason: &'static str,
    /// Radio bullet for the current View/Sort choice.
    pub checked: bool,
    pub separator: bool,
    pub children: Vec<Entry>,
}
impl Entry {
    pub fn item(label: &str, action: Action) -> Self {
        Self {
            label: label.into(),
            shortcut: "",
            action,
            glyph: Glyph::Action,
            enabled: true,
            reason: "",
            checked: false,
            separator: false,
            children: Vec::new(),
        }
    }
    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            glyph: Glyph::None,
            ..Self::item("", Action::Submenu)
        }
    }
    pub fn submenu(label: &str, glyph: Glyph, children: Vec<Entry>) -> Self {
        Self {
            glyph,
            children,
            ..Self::item(label, Action::Submenu)
        }
    }
    pub fn shortcut(mut self, keys: &'static str) -> Self {
        self.shortcut = keys;
        self
    }
    pub fn glyph(mut self, glyph: Glyph) -> Self {
        self.glyph = glyph;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    /// `Some(reason)` disables the entry and records why.
    pub fn gate(mut self, unavailable: Option<&'static str>) -> Self {
        if let Some(reason) = unavailable {
            self.enabled = false;
            self.reason = reason;
        }
        self
    }
    fn focusable(&self) -> bool {
        self.enabled && !self.separator
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Windows 11 compact menu: optional icon row, 14 px commands.
    Modern,
    /// "Show more options": the full legacy command set, 12 px rows.
    Classic,
    /// Dropdown list under a breadcrumb chevron or command-bar button.
    Flyout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu {
    pub style: Style,
    pub width: u32,
    /// Compact icon row (Cut, Copy, Rename, Share, Delete as applicable).
    pub icons: Vec<Entry>,
    pub entries: Vec<Entry>,
    /// The user's appearance (`context_menu_profile`); default is Windows'.
    pub look: Look,
}
/// Settings > Personalization > Context menu > Customize: how compact menus
/// look. The default is the Windows 11 26H2 menu, whose icon row names each
/// button under its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// Text under each icon-row button.
    pub labels: bool,
    /// Tighter command rows (26 px pitch instead of 32).
    pub compact: bool,
    /// No icons beside commands.
    pub plain: bool,
    /// No keyboard shortcuts beside commands.
    pub no_shortcuts: bool,
    /// The classic-menu setting: Windows 10's flat menu (square panel,
    /// 22 px rows, grey hot-tracking) rather than 26H2's styling of the
    /// legacy menu behind "Show more options".
    pub win10: bool,
}
impl Default for Look {
    fn default() -> Self {
        Self {
            labels: true,
            compact: false,
            plain: false,
            no_shortcuts: false,
            win10: false,
        }
    }
}
impl Look {
    /// Icon-row button size and pitch.
    pub fn button(self) -> (u32, u32, i32) {
        if self.labels {
            (LABELLED_BUTTON_W, LABELLED_BUTTON_H, LABELLED_PITCH)
        } else {
            (ICON_BUTTON, ICON_BUTTON, ICON_PITCH)
        }
    }
    /// Height of the icon row block (buttons plus their rule).
    pub fn icon_block(self) -> i32 {
        self.button().1 as i32 + ICON_BLOCK - ICON_BUTTON as i32
    }
    /// Narrowest menu holding `buttons` icon-row buttons.
    pub fn row_width(self, buttons: usize) -> u32 {
        let (w, _, pitch) = self.button();
        if buttons == 0 {
            return 0;
        }
        16 + (buttons as i32 - 1) as u32 * pitch as u32 + w
    }
}

/// Geometry constants, logical pixels at 96 DPI.
pub mod metrics {
    pub const MODERN_WIDTH: u32 = 272;
    pub const CLASSIC_WIDTH: u32 = 264;
    pub const FLYOUT_MIN: u32 = 200;
    pub const FLYOUT_MAX: u32 = 320;
    /// Panel padding above the first and below the last row.
    pub const PAD: i32 = 4;
    pub const ROW: i32 = 32;
    pub const CLASSIC_ROW: i32 = 26;
    pub const SEPARATOR: i32 = 9;
    /// Icon row: 32 px square buttons on a 40 px pitch, then a rule.
    pub const ICON_BUTTON: u32 = 32;
    pub const ICON_PITCH: i32 = 40;
    pub const ICON_BLOCK: i32 = 41;
    /// Icon-row buttons with labels (`Look::labels`): icon over a 12 px
    /// caption.
    pub const LABELLED_BUTTON_W: u32 = 56;
    pub const LABELLED_BUTTON_H: u32 = 50;
    pub const LABELLED_PITCH: i32 = 58;
    /// Compact command rows (`Look::compact`).
    pub const COMPACT_ROW: i32 = 26;
    pub const RADIUS: u32 = 8;
    /// Windows 10 menus (`Look::win10`), measured from a 100% capture:
    /// 22 px rows, 7 px separators (rule 3 px down), a 1 px border plus 4 px
    /// above the first and below the last row, hot plates inset 3 px from
    /// each side, submenus overlapping their parent by 5 px.
    pub const WIN10_ROW: i32 = 22;
    pub const WIN10_SEPARATOR: i32 = 7;
    pub const WIN10_PAD: i32 = 5;
    pub const WIN10_INSET: i32 = 3;
    pub const WIN10_OVERLAP: i32 = 5;
}
use metrics::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panel {
    pub bounds: Bounds,
    /// Icon-row buttons (root modern panel only).
    pub icons: Vec<Bounds>,
    /// Hairline under (or above, when opened upward) the icon row.
    pub icon_rule: Option<i32>,
    /// Row plate per entry; `None` for separators and scrolled-out rows.
    pub rows: Vec<Option<Bounds>>,
    /// Separator line positions.
    pub separators: Vec<i32>,
    pub upward: bool,
    pub scroll: usize,
    pub more_above: bool,
    pub more_below: bool,
}

/// Row pitch: rows hold one label (12 px in the classic menu, 14 px
/// otherwise) and grow with its line box at the applied text size;
/// separators and the icon row keep their size.
fn pitch(style: Style, entry: &Entry, look: Look) -> i32 {
    if entry.separator {
        if win10(style, look) {
            WIN10_SEPARATOR
        } else {
            SEPARATOR
        }
    } else {
        row(style, look)
    }
}
/// A Windows 10 menu (the classic-menu setting).
pub fn win10(style: Style, look: Look) -> bool {
    style == Style::Classic && look.win10
}
fn row(style: Style, look: Look) -> i32 {
    if win10(style, look) {
        crate::palette::text_grow_i(WIN10_ROW, door_graphics_text::TextStyle::Regular12)
    } else if style != Style::Classic && look.compact {
        crate::palette::text_grow_i(COMPACT_ROW, door_graphics_text::TextStyle::Regular14)
    } else if style == Style::Classic {
        crate::palette::text_grow_i(CLASSIC_ROW, door_graphics_text::TextStyle::Regular12)
    } else {
        crate::palette::text_grow_i(ROW, door_graphics_text::TextStyle::Regular14)
    }
}
/// Panel width at the applied text size: the label column widens with the
/// text (icon, padding and chevron columns, 72 px, keep their size).
pub fn grown_width(width: u32) -> u32 {
    let text = width.saturating_sub(72);
    width - text + crate::palette::text_len(text)
}

/// Lays out `entries` in a panel whose top-left is placed at `anchor`, kept
/// inside `window`; it opens upward (Windows keeps the icon row next to the
/// pointer) when it does not fit below. Tall flyouts scroll by whole rows.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    style: Style,
    width: u32,
    icons: usize,
    entries: &[Entry],
    anchor: (i32, i32),
    window: (u32, u32),
    scroll: usize,
    look: Look,
) -> Panel {
    let (ww, wh) = (window.0 as i32, window.1 as i32);
    let width = width.min(window.0.max(1)) as i32;
    let icon_block = if icons > 0 { look.icon_block() } else { 0 };
    let flat = win10(style, look);
    let pad = if flat { WIN10_PAD } else { PAD };
    let full: i32 =
        entries.iter().map(|e| pitch(style, e, look)).sum::<i32>() + icon_block + pad * 2;
    let limit = (wh - 8).max(row(style, look) + pad * 2);
    let scrolling = full > limit;
    let scroll = if scrolling {
        scroll.min(entries.len().saturating_sub(1))
    } else {
        0
    };
    let height = full.min(limit);
    let below = anchor.1 + height <= wh;
    let upward = !below && anchor.1 - height >= 0;
    let y = if below {
        anchor.1
    } else if upward {
        anchor.1 - height
    } else {
        (wh - height).max(0)
    };
    let x = if anchor.0 + width <= ww {
        anchor.0
    } else {
        (anchor.0 - width).max(0).min((ww - width).max(0))
    };
    let bounds = Bounds {
        x,
        y,
        width: width as u32,
        height: height as u32,
    };
    // Upward compact menus put the icon row at the bottom, next to the pointer.
    let icons_at_bottom = upward && style == Style::Modern;
    let mut top = y + pad + if icons_at_bottom { 0 } else { icon_block };
    let bottom = y + height - pad - if icons_at_bottom { icon_block } else { 0 };
    let mut rows = Vec::with_capacity(entries.len());
    let mut separators = Vec::new();
    let mut more_below = false;
    for (i, entry) in entries.iter().enumerate() {
        let h = pitch(style, entry, look);
        if i < scroll {
            rows.push(None);
            continue;
        }
        if top + h > bottom {
            more_below = true;
            rows.push(None);
            continue;
        }
        if entry.separator {
            separators.push(top + h / 2);
            rows.push(None);
        } else if flat {
            rows.push(Some(Bounds {
                x: x + WIN10_INSET,
                y: top,
                width: (width - 2 * WIN10_INSET).max(0) as u32,
                height: h as u32,
            }));
        } else {
            rows.push(Some(Bounds {
                x: x + 4,
                y: top + 2,
                width: (width - 8).max(0) as u32,
                height: (h - 4) as u32,
            }));
        }
        top += h;
    }
    let (button_w, button_h, button_pitch) = look.button();
    let (icon_top, icon_rule) = if icons == 0 {
        (0, None)
    } else if icons_at_bottom {
        (
            bottom + look.icon_block() - 4 - button_h as i32,
            Some(bottom + 4),
        )
    } else {
        (y + PAD, Some(y + PAD + button_h as i32 + 4))
    };
    let icons = (0..icons)
        .map(|i| Bounds {
            x: x + 8 + i as i32 * button_pitch,
            y: icon_top,
            width: button_w,
            height: button_h,
        })
        .collect();
    Panel {
        bounds,
        icons,
        icon_rule,
        rows,
        separators,
        upward,
        scroll,
        more_above: scroll > 0,
        more_below,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    None,
    Icon(usize),
    Row(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Outside,
    /// Inside a panel but not on a command (padding, separators).
    Chrome,
    Icon(usize),
    Row(usize),
    SubRow(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing to do (still open, or a disabled/chrome press).
    Stay,
    Dismiss,
    /// The chosen command; the menu has closed. `index` is its position in
    /// the panel it came from (root entries, icons, or the open submenu).
    Invoke(Action, Source, usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Icon,
    Root,
    Submenu,
}

#[derive(Clone, Debug)]
pub struct MenuState {
    pub menu: Menu,
    pub anchor: (i32, i32),
    pub window: (u32, u32),
    pub focus: Focus,
    /// Open submenu: (parent entry, focused child, scroll).
    pub sub: Option<(usize, Option<usize>, usize)>,
    pub scroll: usize,
}
impl MenuState {
    /// Keyboard-opened menus focus their first command, as Windows does.
    pub fn open(menu: Menu, anchor: (i32, i32), window: (u32, u32), keyboard: bool) -> Self {
        let mut state = Self {
            menu,
            anchor,
            window,
            focus: Focus::None,
            sub: None,
            scroll: 0,
        };
        if keyboard {
            state.focus = state.next_row(None, true).map_or(Focus::None, Focus::Row);
        }
        state
    }
    pub fn root(&self) -> Panel {
        panel(
            self.menu.style,
            grown_width(self.menu.width),
            self.menu.icons.len(),
            &self.menu.entries,
            self.anchor,
            self.window,
            self.scroll,
            self.menu.look,
        )
    }
    pub fn submenu(&self) -> Option<Panel> {
        let (parent, _, scroll) = self.sub?;
        let root = self.root();
        let row = root.rows.get(parent).copied().flatten()?;
        let children = &self.menu.entries.get(parent)?.children;
        let width = grown_width(submenu_width(self.menu.style, children));
        // Windows 10 submenus overlap the parent's border and line their
        // first row up with the parent row.
        let (overlap, lift) = if win10(self.menu.style, self.menu.look) {
            (WIN10_OVERLAP + 1, WIN10_PAD)
        } else {
            (4, PAD + 2)
        };
        let right = root.bounds.x + root.bounds.width as i32 - overlap;
        let room = self.window.0 as i32 - right;
        let x = if room >= width as i32 {
            right
        } else {
            (root.bounds.x + overlap - width as i32).max(0)
        };
        let mut p = panel(
            self.menu.style,
            width,
            0,
            children,
            (x, row.y - lift),
            self.window,
            scroll,
            self.menu.look,
        );
        // A submenu never jumps upward past its parent row; clamp instead.
        if p.upward {
            let top = (self.window.1 as i32 - p.bounds.height as i32).max(0);
            p = panel(
                self.menu.style,
                width,
                0,
                children,
                (x, top),
                self.window,
                scroll,
                self.menu.look,
            );
        }
        Some(p)
    }
    pub fn hit(&self, x: i32, y: i32) -> Hit {
        if let Some(sub) = self.submenu()
            && sub.bounds.contains(x, y)
        {
            return sub
                .rows
                .iter()
                .position(|r| r.is_some_and(|r| r.contains(x, y)))
                .map_or(Hit::Chrome, Hit::SubRow);
        }
        let root = self.root();
        if !root.bounds.contains(x, y) {
            return Hit::Outside;
        }
        if let Some(i) = root.icons.iter().position(|r| r.contains(x, y)) {
            return Hit::Icon(i);
        }
        root.rows
            .iter()
            .position(|r| r.is_some_and(|r| r.contains(x, y)))
            .map_or(Hit::Chrome, Hit::Row)
    }
    /// Entry under the pointer (for disabled-reason tooltips).
    pub fn entry_at(&self, x: i32, y: i32) -> Option<(&Entry, Bounds)> {
        match self.hit(x, y) {
            Hit::Icon(i) => Some((self.menu.icons.get(i)?, self.root().icons[i])),
            Hit::Row(i) => Some((self.menu.entries.get(i)?, self.root().rows[i]?)),
            Hit::SubRow(i) => {
                let (parent, _, _) = self.sub?;
                Some((
                    self.menu.entries.get(parent)?.children.get(i)?,
                    self.submenu()?.rows[i]?,
                ))
            }
            _ => None,
        }
    }
    fn next_row(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let entries = &self.menu.entries;
        let n = entries.len();
        (1..=n)
            .map(|step| match from {
                None if forward => step - 1,
                None => n - step,
                Some(i) if forward => (i + step) % n,
                Some(i) => (i + n - step % n) % n,
            })
            .find(|i| entries[*i].focusable())
    }
    fn next_child(&self, parent: usize, from: Option<usize>, forward: bool) -> Option<usize> {
        let children = &self.menu.entries.get(parent)?.children;
        let n = children.len();
        (1..=n)
            .map(|step| match from {
                None if forward => step - 1,
                None => n - step,
                Some(i) if forward => (i + step) % n,
                Some(i) => (i + n - step % n) % n,
            })
            .find(|i| children[*i].focusable())
    }
    /// Keeps a focused row of a scrolling flyout visible.
    fn reveal(&mut self) {
        if let Focus::Row(i) = self.focus {
            if i < self.scroll {
                self.scroll = i;
            }
            while self.root().rows.get(i).is_some_and(|r| r.is_none()) && self.scroll < i {
                self.scroll += 1;
            }
        }
    }
    /// Pointer motion: focus follows enabled commands and opens submenus.
    /// Returns whether anything visible changed.
    pub fn hover(&mut self, x: i32, y: i32) -> bool {
        let before = (self.focus, self.sub);
        match self.hit(x, y) {
            Hit::Row(i) => {
                let entry = &self.menu.entries[i];
                if entry.focusable() {
                    self.focus = Focus::Row(i);
                    if !entry.children.is_empty() {
                        if self.sub.is_none_or(|(p, _, _)| p != i) {
                            self.sub = Some((i, None, 0));
                        }
                    } else {
                        self.sub = None;
                    }
                }
            }
            Hit::Icon(i) => {
                if self.menu.icons[i].focusable() {
                    self.focus = Focus::Icon(i);
                }
                self.sub = None;
            }
            Hit::SubRow(i) => {
                if let Some((parent, focus, scroll)) = self.sub {
                    let ok = self.menu.entries[parent].children[i].focusable();
                    self.sub = Some((parent, if ok { Some(i) } else { focus }, scroll));
                    self.focus = Focus::Row(parent);
                }
            }
            Hit::Chrome | Hit::Outside => (),
        }
        before != (self.focus, self.sub)
    }
    pub fn click(&mut self, x: i32, y: i32) -> Outcome {
        match self.hit(x, y) {
            Hit::Outside => Outcome::Dismiss,
            Hit::Chrome => Outcome::Stay,
            Hit::Icon(i) => self.choose(Source::Icon, i),
            Hit::Row(i) => {
                if !self.menu.entries[i].children.is_empty() {
                    if self.menu.entries[i].enabled {
                        self.focus = Focus::Row(i);
                        self.sub = Some((i, None, 0));
                    }
                    return Outcome::Stay;
                }
                self.choose(Source::Root, i)
            }
            Hit::SubRow(i) => self.choose(Source::Submenu, i),
        }
    }
    fn choose(&mut self, source: Source, index: usize) -> Outcome {
        let entry = match source {
            Source::Icon => self.menu.icons.get(index),
            Source::Root => self.menu.entries.get(index),
            Source::Submenu => self
                .sub
                .and_then(|(p, _, _)| self.menu.entries.get(p))
                .and_then(|e| e.children.get(index)),
        };
        match entry {
            Some(e) if e.focusable() && e.children.is_empty() => {
                Outcome::Invoke(e.action, source, index)
            }
            _ => Outcome::Stay,
        }
    }
    /// Windows menu keys: arrows (wrapping), Home/End, Tab between the icon
    /// row and the list, Right/Enter into submenus, Left/Esc out of them.
    pub fn key(&mut self, scan: u32) -> Outcome {
        const UP: u32 = 0x148;
        const DOWN: u32 = 0x150;
        const LEFT: u32 = 0x14b;
        const RIGHT: u32 = 0x14d;
        if let Some((parent, focus, scroll)) = self.sub {
            if matches!(scan, UP | DOWN | 0x147 | 0x14f | 28 | 0x11c | RIGHT) {
                let next = match scan {
                    UP => self.next_child(parent, focus, false),
                    DOWN => self.next_child(parent, focus, true),
                    0x147 => self.next_child(parent, None, true),
                    0x14f => self.next_child(parent, None, false),
                    _ => match focus {
                        Some(i) => return self.choose(Source::Submenu, i),
                        None => self.next_child(parent, None, true),
                    },
                };
                self.sub = Some((parent, next.or(focus), scroll));
                return Outcome::Stay;
            }
            if matches!(scan, LEFT | 1) {
                self.sub = None;
                self.focus = Focus::Row(parent);
                return Outcome::Stay;
            }
            return Outcome::Stay;
        }
        match scan {
            1 => return Outcome::Dismiss,
            UP | DOWN => {
                let from = match self.focus {
                    Focus::Row(i) => Some(i),
                    _ => None,
                };
                if let Some(i) = self.next_row(from, scan == DOWN) {
                    self.focus = Focus::Row(i);
                }
            }
            0x147 => self.focus = self.next_row(None, true).map_or(self.focus, Focus::Row),
            0x14f => self.focus = self.next_row(None, false).map_or(self.focus, Focus::Row),
            15 => {
                self.focus = match self.focus {
                    Focus::Icon(_) => self.next_row(None, true).map_or(self.focus, Focus::Row),
                    _ => self
                        .menu
                        .icons
                        .iter()
                        .position(Entry::focusable)
                        .map_or(self.focus, Focus::Icon),
                }
            }
            LEFT | RIGHT if matches!(self.focus, Focus::Icon(_)) => {
                let Focus::Icon(i) = self.focus else {
                    unreachable!()
                };
                let n = self.menu.icons.len();
                let step = |k: usize| {
                    if scan == RIGHT {
                        (i + k) % n
                    } else {
                        (i + n - k % n) % n
                    }
                };
                if let Some(next) = (1..=n).map(step).find(|j| self.menu.icons[*j].focusable()) {
                    self.focus = Focus::Icon(next);
                }
            }
            RIGHT | 28 | 0x11c => match self.focus {
                Focus::Row(i) if !self.menu.entries[i].children.is_empty() => {
                    let first = self.next_child(i, None, true);
                    self.sub = Some((i, first, 0));
                }
                Focus::Row(i) if scan != RIGHT => return self.choose(Source::Root, i),
                Focus::Icon(i) if scan != RIGHT => return self.choose(Source::Icon, i),
                _ => (),
            },
            _ => (),
        }
        self.reveal();
        Outcome::Stay
    }
    /// Wheel over a scrolling flyout moves it by whole rows.
    pub fn wheel(&mut self, notches: i32) -> bool {
        let root = self.root();
        if !root.more_above && !root.more_below {
            return false;
        }
        let before = self.scroll;
        if notches > 0 {
            self.scroll = self.scroll.saturating_sub(notches as usize * 3);
        } else if root.more_below {
            self.scroll = (self.scroll + notches.unsigned_abs() as usize * 3)
                .min(self.menu.entries.len().saturating_sub(1));
        }
        before != self.scroll
    }
}

fn submenu_width(style: Style, children: &[Entry]) -> u32 {
    match style {
        Style::Classic => 248,
        _ => {
            if children.iter().any(|c| !c.shortcut.is_empty()) {
                300
            } else {
                220
            }
        }
    }
}

/// What a context menu was opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    File,
    Folder,
    Multiple,
    Drive,
    ThisPc,
    /// The folder's empty background.
    Background,
    /// A folder row of the navigation tree.
    TreeFolder,
    /// A pinned Quick access row of the navigation tree.
    TreePin,
    /// A user tag in the navigation pane ("Your tags").
    Tag(u8),
    /// A tag chip on the selected item in the details pane.
    TagChip(u8),
    /// An item listed in the Recycle Bin.
    RecycleItem,
    /// The Recycle Bin row of the navigation pane.
    RecycleBin,
}

/// Per-command availability: `None` enabled, `Some(reason)` disabled.
#[derive(Clone, Copy, Debug)]
pub struct Gates {
    pub cut: Option<&'static str>,
    pub copy: Option<&'static str>,
    pub rename: Option<&'static str>,
    pub delete: Option<&'static str>,
    pub terminal: Option<&'static str>,
    pub compress: Option<&'static str>,
    pub new_window: Option<&'static str>,
    pub paste: Option<&'static str>,
    /// Tree rows: Some(expanded) when the row has a disclosure chevron.
    pub expanded: Option<bool>,
    pub view: u8,
    pub sort: u8,
    pub descending: bool,
    /// The background belongs to This PC (no New, Paste or Terminal).
    pub overview: bool,
    /// Settings > Context menu > File actions (Manage file, Send to).
    pub file_actions: bool,
    /// Quick access: None not a folder; Some(false) pin; Some(true) pinned.
    pub pinned: Option<bool>,
    /// Why pinning or unpinning is unavailable (known folders, drives).
    pub pin_reason: Option<&'static str>,
    /// Favorites (files): None n/a; Some(on).
    pub favorite: Option<bool>,
    /// The item is listed in Home > Recent.
    pub recent: bool,
    /// The item is shown away from its folder (Home, search results).
    pub located: bool,
    /// The file is an archive "Extract All..." handles: Some(reason)
    /// while another extraction runs; None otherwise.
    pub extract: Option<Option<&'static str>>,
    /// The file has conversions in Preview ("Convert...").
    pub convert: bool,
    /// The background is the Recycle Bin (no New, Paste or Terminal).
    pub recycle: bool,
    /// "Empty Recycle Bin" availability.
    pub empty_bin: Option<&'static str>,
    /// A program or batch file: "Run as administrator".
    pub elevate: bool,
    /// A Windows Installer package: "Install", "Repair", "Uninstall".
    pub installer: bool,
    /// A single file or folder can take Doors tags ("Add tag").
    pub tags: bool,
}
impl Default for Gates {
    fn default() -> Self {
        Self {
            cut: Some(NO_CLIPBOARD),
            copy: Some(NO_CLIPBOARD),
            rename: None,
            delete: None,
            terminal: Some(TERMINAL_HOME),
            compress: None,
            new_window: None,
            paste: Some(NO_CLIPBOARD),
            expanded: None,
            view: 5,
            sort: 0,
            descending: false,
            overview: false,
            file_actions: true,
            pinned: None,
            pin_reason: Some(PIN_FOLDERS),
            favorite: None,
            recent: false,
            located: false,
            extract: None,
            convert: false,
            recycle: false,
            empty_bin: Some(BIN_EMPTY),
            elevate: false,
            installer: false,
            tags: false,
        }
    }
}

pub const VIEW_LABELS: [(&str, &str); 8] = [
    ("Extra large icons", "Ctrl+Shift+1"),
    ("Large icons", "Ctrl+Shift+2"),
    ("Medium icons", "Ctrl+Shift+3"),
    ("Small icons", "Ctrl+Shift+4"),
    ("List", "Ctrl+Shift+5"),
    ("Details", "Ctrl+Shift+6"),
    ("Tiles", "Ctrl+Shift+7"),
    ("Content", "Ctrl+Shift+8"),
];
/// "Sort by" in `SortKey::ALL` order.
pub const SORT_LABELS: [&str; 4] = ["Name", "Date modified", "Type", "Size"];

/// The command-bar View dropdown: the eight layouts, then "Show" with the
/// Doors extra "Folder sizes" (`folder_sizes` checked).
pub fn view_menu(view: u8, folder_sizes: bool) -> Vec<Entry> {
    let mut out = view_entries(view);
    out.push(Entry::separator());
    out.push(Entry::submenu(
        "Show",
        Glyph::None,
        alloc::vec![
            Entry::item("Folder sizes", Action::FolderSizes)
                .glyph(Glyph::None)
                .checked(folder_sizes)
        ],
    ));
    out
}
pub fn view_entries(view: u8) -> Vec<Entry> {
    VIEW_LABELS
        .iter()
        .enumerate()
        .map(|(i, (label, keys))| {
            Entry::item(label, Action::View(i as u8))
                .shortcut(keys)
                .glyph(Glyph::ViewMode(i as u8))
                .checked(view == i as u8)
        })
        .collect()
}
pub fn sort_entries(sort: u8, descending: bool) -> Vec<Entry> {
    let mut out: Vec<Entry> = SORT_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            Entry::item(label, Action::Sort(i as u8))
                .glyph(Glyph::None)
                .checked(sort == i as u8)
        })
        .collect();
    out.push(Entry::separator());
    out.push(
        Entry::item("Ascending", Action::Ascending)
            .glyph(Glyph::None)
            .checked(!descending),
    );
    out.push(
        Entry::item("Descending", Action::Descending)
            .glyph(Glyph::None)
            .checked(descending),
    );
    out
}
pub fn new_entries() -> Vec<Entry> {
    alloc::vec![
        Entry::item("Folder", Action::NewFolder)
            .shortcut("Ctrl+Shift+N")
            .glyph(Glyph::Folder),
        Entry::item("Text Document", Action::NewTextDocument),
    ]
}

/// The Windows 11 26H2 compact menu for `subject`. 26H2 declutters the main
/// group: Compress to ZIP file and Copy as path move into a "Manage file"
/// flyout (hidden with Settings > Context menu > File actions), "Show more
/// options" can be hidden, and "Customize menu" closes every menu.
/// `controls` also keeps the icon-row and Properties-placement choices.
pub fn modern(subject: Subject, gates: Gates, controls: Controls) -> Menu {
    let mut icons = Vec::new();
    if matches!(
        subject,
        Subject::File
            | Subject::Folder
            | Subject::Multiple
            | Subject::TreeFolder
            | Subject::TreePin
    ) {
        icons = alloc::vec![
            Entry::item("Cut", Action::Cut)
                .shortcut("Ctrl+X")
                .gate(gates.cut),
            Entry::item("Copy", Action::Copy)
                .shortcut("Ctrl+C")
                .gate(gates.copy),
            Entry::item("Rename", Action::Rename)
                .shortcut("F2")
                .gate(gates.rename),
            Entry::item("Share", Action::Share).gate(Some(NO_SHARE)),
            Entry::item("Delete", Action::Delete)
                .shortcut("Delete")
                .gate(gates.delete),
        ];
    } else if subject == Subject::RecycleItem {
        icons = alloc::vec![
            Entry::item("Cut", Action::Cut)
                .shortcut("Ctrl+X")
                .gate(gates.cut),
            Entry::item("Delete", Action::Delete)
                .shortcut("Delete")
                .gate(gates.delete),
        ];
    }
    let file_actions = gates.file_actions && !controls.has(Controls::CONTEXT_HIDE_FILE_ACTIONS);
    let manage = || {
        Entry::submenu(
            "Manage file",
            Glyph::Action,
            alloc::vec![
                Entry::item("Compress to ZIP file", Action::Compress).gate(gates.compress),
                Entry::item("Copy as path", Action::CopyPath)
                    .shortcut("Ctrl+Shift+C")
                    .gate(gates.copy),
            ],
        )
    };
    let new_tab = || Entry::item("Open in new tab", Action::OpenNewTab);
    let new_window =
        || Entry::item("Open in new window", Action::OpenNewWindow).gate(gates.new_window);
    let pinned = gates.pinned == Some(true) || subject == Subject::TreePin;
    let pin = || {
        Entry::item(
            if pinned {
                "Unpin from Quick access"
            } else {
                "Pin to Quick access"
            },
            Action::PinQuickAccess,
        )
        .gate(gates.pin_reason)
    };
    let open = || Entry::item("Open", Action::Open).shortcut("Enter");
    // The main group; Properties closes it unless Settings moves it first.
    let mut main = match subject {
        Subject::File => {
            // A package's verbs replace Open, as its file type registers
            // them (Install is the default).
            let mut items = if gates.installer {
                alloc::vec![
                    Entry::item("Install", Action::Install).shortcut("Enter"),
                    Entry::item("Repair", Action::Repair),
                    Entry::item("Uninstall", Action::Uninstall),
                ]
            } else {
                alloc::vec![open()]
            };
            if gates.elevate {
                items.push(Entry::item(
                    "Run as administrator",
                    Action::RunAsAdministrator,
                ));
            }
            if let Some(gate) = gates.extract {
                items.push(Entry::item("Extract All...", Action::ExtractAll).gate(gate));
            }
            if gates.convert {
                items.push(Entry::item("Convert...", Action::Convert));
            }
            if gates.located {
                items.push(Entry::item("Open file location", Action::OpenLocation));
            }
            if let Some(on) = gates.favorite {
                items.push(Entry::item(
                    if on {
                        "Remove from Favorites"
                    } else {
                        "Add to Favorites"
                    },
                    Action::Favorite,
                ));
            }
            if gates.tags {
                items.push(Entry::item("Add tag", Action::AddTag));
            }
            items
        }
        Subject::Folder | Subject::TreeFolder | Subject::TreePin => {
            let mut items = alloc::vec![new_tab(), new_window(), pin()];
            if gates.located {
                items.insert(0, Entry::item("Open file location", Action::OpenLocation));
            }
            if gates.tags && subject == Subject::Folder {
                items.push(Entry::item("Add tag", Action::AddTag));
            }
            items
        }
        Subject::Drive | Subject::ThisPc => alloc::vec![open(), new_tab(), new_window(), pin()],
        Subject::RecycleItem => alloc::vec![Entry::item("Restore", Action::Restore)],
        Subject::RecycleBin => alloc::vec![
            open(),
            new_tab(),
            new_window(),
            Entry::item("Empty Recycle Bin", Action::EmptyRecycleBin).gate(gates.empty_bin),
        ],
        Subject::Multiple | Subject::Background | Subject::Tag(_) | Subject::TagChip(_) => {
            Vec::new()
        }
    };
    if file_actions
        && matches!(
            subject,
            Subject::File
                | Subject::Folder
                | Subject::Multiple
                | Subject::TreeFolder
                | Subject::TreePin
        )
    {
        main.push(manage());
    }
    let properties = Entry::item("Properties", Action::Properties).shortcut("Alt+Enter");
    let mut entries = Vec::new();
    let group = |entries: &mut Vec<Entry>, mut items: Vec<Entry>| {
        if items.is_empty() {
            return;
        }
        if !entries.is_empty() {
            entries.push(Entry::separator());
        }
        entries.append(&mut items);
    };
    match subject {
        Subject::Background => {
            group(
                &mut entries,
                alloc::vec![
                    Entry::submenu(
                        "View",
                        Glyph::ViewMode(gates.view),
                        view_entries(gates.view)
                    ),
                    Entry::submenu(
                        "Sort by",
                        Glyph::Action,
                        sort_entries(gates.sort, gates.descending)
                    ),
                    Entry::item("Refresh", Action::Refresh).shortcut("F5"),
                ],
            );
            if gates.recycle {
                group(
                    &mut entries,
                    alloc::vec![
                        Entry::item("Empty Recycle Bin", Action::EmptyRecycleBin)
                            .gate(gates.empty_bin),
                        Entry::item("Restore all items", Action::RestoreAll).gate(gates.empty_bin),
                    ],
                );
            }
            if !gates.overview && !gates.recycle {
                group(
                    &mut entries,
                    alloc::vec![
                        Entry::item("Paste", Action::Paste)
                            .shortcut("Ctrl+V")
                            .gate(gates.paste)
                    ],
                );
                group(
                    &mut entries,
                    alloc::vec![Entry::submenu("New", Glyph::Action, new_entries())],
                );
            }
            group(&mut entries, alloc::vec![properties]);
            if !gates.overview && !gates.recycle {
                group(
                    &mut entries,
                    alloc::vec![
                        Entry::item("Open in Terminal", Action::OpenTerminal).gate(gates.terminal)
                    ],
                );
            }
        }
        Subject::Tag(i) => group(
            &mut entries,
            alloc::vec![
                Entry::item("Show items with this tag", Action::FilterTag(i)).glyph(Glyph::Tag(i)),
                Entry::item("Rename tag", Action::RenameTag(i)),
                Entry::item("Delete tag", Action::DeleteTag(i)),
            ],
        ),
        Subject::TagChip(i) => group(
            &mut entries,
            alloc::vec![
                Entry::item("Remove tag", Action::RemoveTag(i)),
                Entry::item("Show items with this tag", Action::FilterTag(i)).glyph(Glyph::Tag(i)),
            ],
        ),
        _ => {
            if controls.properties_bottom() || subject == Subject::RecycleItem {
                main.push(properties);
            } else {
                main.insert(0, properties);
            }
            group(&mut entries, main);
            if gates.recent {
                group(
                    &mut entries,
                    alloc::vec![Entry::item("Remove from list", Action::RemoveRecent)],
                );
            }
            if !controls.inline_actions() {
                // Settings > Context menu: the compact actions as labelled rows.
                group(&mut entries, core::mem::take(&mut icons));
            }
            if matches!(
                subject,
                Subject::Folder | Subject::TreeFolder | Subject::TreePin | Subject::Drive
            ) {
                group(
                    &mut entries,
                    alloc::vec![
                        Entry::item("Open in Terminal", Action::OpenTerminal).gate(gates.terminal)
                    ],
                );
            }
        }
    }
    if !matches!(subject, Subject::Tag(_) | Subject::TagChip(_)) {
        let mut tail = Vec::new();
        if !controls.has(Controls::CONTEXT_HIDE_MORE_OPTIONS) {
            tail.push(Entry::item("Show more options", Action::ShowMore).shortcut("Shift+F10"));
        }
        tail.push(Entry::item("Customize menu", Action::Customize));
        group(&mut entries, tail);
    }
    Menu {
        style: Style::Modern,
        width: MODERN_WIDTH.max(Look::default().row_width(icons.len())),
        icons,
        entries,
        look: Look::default(),
    }
}

/// The legacy full menu ("Show more options" or the classic-menu setting).
pub fn classic(subject: Subject, gates: Gates) -> Menu {
    let mut e = Vec::new();
    let item = |label: &str, action| Entry::item(label, action).glyph(Glyph::None);
    match subject {
        Subject::Background => {
            e.push(Entry::submenu(
                "View",
                Glyph::None,
                view_entries(gates.view),
            ));
            e.push(Entry::submenu(
                "Sort by",
                Glyph::None,
                sort_entries(gates.sort, gates.descending),
            ));
            e.push(item("Refresh", Action::Refresh));
            e.push(Entry::separator());
            e.push(
                item("Paste", Action::Paste)
                    .shortcut("Ctrl+V")
                    .gate(gates.paste),
            );
            e.push(Entry::separator());
            e.push(
                item("Open in Terminal", Action::OpenTerminal)
                    .glyph(Glyph::Action)
                    .gate(gates.terminal),
            );
            e.push(Entry::separator());
            e.push(Entry::submenu("New", Glyph::None, new_entries()));
            e.push(Entry::separator());
            e.push(item("Properties", Action::Properties));
        }
        Subject::Tag(_) | Subject::TagChip(_) => return modern(subject, gates, Controls::DEFAULT),
        Subject::RecycleItem => {
            e.push(item("Restore", Action::Restore));
            e.push(Entry::separator());
            e.push(item("Cut", Action::Cut).gate(gates.cut));
            e.push(Entry::separator());
            e.push(item("Delete", Action::Delete).gate(gates.delete));
            e.push(Entry::separator());
            e.push(item("Properties", Action::Properties));
        }
        Subject::RecycleBin => {
            e.push(item("Open", Action::Open));
            e.push(item("Open in new tab", Action::OpenNewTab));
            e.push(item("Open in new window", Action::OpenNewWindow).gate(gates.new_window));
            e.push(Entry::separator());
            e.push(item("Empty Recycle Bin", Action::EmptyRecycleBin).gate(gates.empty_bin));
        }
        _ => {
            if let Some(open) = gates.expanded {
                e.push(item(
                    if open { "Collapse" } else { "Expand" },
                    if open {
                        Action::Collapse
                    } else {
                        Action::Expand
                    },
                ));
            }
            if subject == Subject::File && gates.installer {
                e.push(item("Install", Action::Install));
                e.push(item("Repair", Action::Repair));
                e.push(item("Uninstall", Action::Uninstall));
            } else if matches!(subject, Subject::File | Subject::Drive | Subject::Folder) {
                e.push(item("Open", Action::Open));
            }
            if subject == Subject::File && gates.elevate {
                e.push(item("Run as administrator", Action::RunAsAdministrator));
            }
            if !matches!(subject, Subject::File | Subject::Multiple) {
                e.push(item("Open in new tab", Action::OpenNewTab));
                e.push(item("Open in new window", Action::OpenNewWindow).gate(gates.new_window));
                let pinned = gates.pinned == Some(true) || subject == Subject::TreePin;
                e.push(
                    item(
                        if pinned {
                            "Unpin from Quick access"
                        } else {
                            "Pin to Quick access"
                        },
                        Action::PinQuickAccess,
                    )
                    .gate(gates.pin_reason),
                );
            }
            if matches!(
                subject,
                Subject::Folder | Subject::TreeFolder | Subject::TreePin
            ) {
                e.push(
                    item("Open in Terminal", Action::OpenTerminal)
                        .glyph(Glyph::Action)
                        .gate(gates.terminal),
                );
            }
            if !matches!(subject, Subject::Drive | Subject::ThisPc) {
                if gates.file_actions {
                    e.push(Entry::separator());
                    e.push(Entry::submenu(
                        "Send to",
                        Glyph::None,
                        alloc::vec![
                            Entry::item("Compressed (zipped) folder", Action::Compress)
                                .glyph(Glyph::Action)
                                .gate(gates.compress)
                        ],
                    ));
                }
                e.push(Entry::separator());
                e.push(item("Cut", Action::Cut).gate(gates.cut));
                e.push(item("Copy", Action::Copy).gate(gates.copy));
                e.push(Entry::separator());
                e.push(item("Delete", Action::Delete).gate(gates.delete));
                e.push(item("Rename", Action::Rename).gate(gates.rename));
            }
            e.push(Entry::separator());
            e.push(item("Properties", Action::Properties));
        }
    }
    Menu {
        style: Style::Classic,
        width: CLASSIC_WIDTH,
        icons: Vec::new(),
        entries: e,
        look: Look::default(),
    }
}

/// A dropdown flyout of locations or choices.
pub fn flyout(entries: Vec<Entry>, width: u32) -> Menu {
    Menu {
        style: Style::Flyout,
        width: width.clamp(FLYOUT_MIN, FLYOUT_MAX),
        icons: Vec::new(),
        entries,
        look: Look::default(),
    }
}

/// One app of a file's "Open with" submenu: name, `shell::icon` id and, for
/// an app that cannot open the file, why (the row is disabled).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWithApp {
    pub name: String,
    pub icon: u16,
    pub unavailable: Option<&'static str>,
}
/// Windows 11's "Open with" submenu: the apps for the file's type (the
/// default first, `Action::OpenWith(row)`), a separator, then "Choose
/// another app" (`Action::ChooseApp`).
pub fn open_with(apps: &[OpenWithApp]) -> Entry {
    let mut children: Vec<Entry> = apps
        .iter()
        .enumerate()
        .take(usize::from(u8::MAX))
        .map(|(i, a)| {
            Entry::item(&a.name, Action::OpenWith(i as u8))
                .glyph(Glyph::App(a.icon))
                .gate(a.unavailable)
        })
        .collect();
    if !children.is_empty() {
        children.push(Entry::separator());
    }
    children.push(Entry::item("Choose another app", Action::ChooseApp).glyph(Glyph::None));
    Entry::submenu("Open with", Glyph::Action, children)
}
/// Put the "Open with" submenu right after the menu's "Open" command (as
/// both the compact and the classic file menus show it); a menu without
/// "Open" is left unchanged.
pub fn insert_open_with(menu: &mut Menu, entry: Entry) {
    if let Some(at) = menu
        .entries
        .iter()
        .position(|e| e.action == Action::Open && !e.separator)
    {
        let mut entry = entry;
        if menu.style == Style::Classic {
            entry.glyph = Glyph::None;
        }
        menu.entries.insert(at + 1, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gates() -> Gates {
        Gates::default()
    }
    fn labels(m: &Menu) -> Vec<&str> {
        m.entries
            .iter()
            .filter(|e| !e.separator)
            .map(|e| e.label.as_str())
            .collect()
    }
    #[test]
    fn menus_change_with_the_subject_and_unavailable_commands_say_why() {
        let c = Controls::DEFAULT;
        let file = modern(Subject::File, gates(), c);
        assert_eq!(
            file.icons
                .iter()
                .map(|e| e.label.as_str())
                .collect::<Vec<_>>(),
            ["Cut", "Copy", "Rename", "Share", "Delete"]
        );
        // 26H2: Compress and Copy as path live in the "Manage file" flyout.
        assert_eq!(
            labels(&file),
            [
                "Open",
                "Manage file",
                "Properties",
                "Show more options",
                "Customize menu"
            ]
        );
        let manage = file
            .entries
            .iter()
            .find(|e| e.label == "Manage file")
            .unwrap();
        assert_eq!(
            manage.children.iter().map(|e| e.action).collect::<Vec<_>>(),
            [Action::Compress, Action::CopyPath]
        );
        let folder = modern(Subject::Folder, gates(), c);
        assert!(labels(&folder).contains(&"Open in new tab"));
        assert!(labels(&folder).contains(&"Open in Terminal"));
        let background = modern(Subject::Background, gates(), c);
        assert!(background.icons.is_empty());
        assert_eq!(
            labels(&background),
            [
                "View",
                "Sort by",
                "Refresh",
                "Paste",
                "New",
                "Properties",
                "Open in Terminal",
                "Show more options",
                "Customize menu"
            ]
        );
        let pc = modern(
            Subject::Background,
            Gates {
                overview: true,
                ..gates()
            },
            c,
        );
        assert_eq!(
            labels(&pc),
            [
                "View",
                "Sort by",
                "Refresh",
                "Properties",
                "Show more options",
                "Customize menu"
            ]
        );
        // Separators only between groups, never doubled or trailing.
        for menu in [&background, &pc] {
            for pair in menu.entries.windows(2) {
                assert!(!(pair[0].separator && pair[1].separator));
            }
            assert!(!menu.entries[0].separator);
        }
        let drive = modern(Subject::Drive, gates(), c);
        assert!(drive.icons.is_empty() && labels(&drive)[0] == "Open");
        for menu in [
            file,
            folder,
            background,
            drive,
            modern(Subject::Multiple, gates(), c),
        ] {
            for e in menu.icons.iter().chain(&menu.entries) {
                assert!(
                    e.separator || e.enabled || !e.reason.is_empty(),
                    "{}",
                    e.label
                );
            }
            let n = menu.entries.len();
            assert_eq!(
                menu.entries[n - 1].action,
                Action::Customize,
                "Customize menu is last"
            );
            assert_eq!(menu.entries[n - 2].action, Action::ShowMore);
        }
        // Settings > Context menu (26H2): hide file actions and Show more options.
        let lean = c
            .with(Controls::CONTEXT_HIDE_FILE_ACTIONS, true)
            .with(Controls::CONTEXT_HIDE_MORE_OPTIONS, true);
        let file = modern(Subject::File, gates(), lean);
        assert_eq!(labels(&file), ["Open", "Properties", "Customize menu"]);
        assert!(
            !labels(&classic(
                Subject::Folder,
                Gates {
                    file_actions: false,
                    ..gates()
                }
            ))
            .contains(&"Send to")
        );
        // Settings: text rows instead of icons, Properties first.
        let rows = modern(Subject::File, gates(), Controls::from_bits(0).unwrap());
        assert!(rows.icons.is_empty());
        assert_eq!(rows.entries[0].action, Action::Properties);
        assert!(labels(&rows).contains(&"Cut"));
        let classic = classic(Subject::Folder, gates());
        assert!(
            labels(&classic).contains(&"Send to") && labels(&classic).ends_with(&["Properties"])
        );
    }
    #[test]
    fn this_pc_keyboard_tail_is_properties_show_more_customize() {
        // tools/xtask controls scenario: Shift+F10 on This PC, Up reaches
        // Customize menu, Up x3 reaches Properties.
        for gates in [
            Gates {
                overview: true,
                ..gates()
            },
            gates(),
        ] {
            for subject in [Subject::Background, Subject::Drive] {
                let mut s = MenuState::open(
                    modern(subject, gates, Controls::DEFAULT),
                    (10, 10),
                    (900, 700),
                    true,
                );
                let focused = |s: &MenuState| match s.focus {
                    Focus::Row(i) => s.menu.entries[i].action,
                    _ => panic!(),
                };
                s.key(0x148);
                assert_eq!(focused(&s), Action::Customize);
                s.key(0x148);
                assert_eq!(focused(&s), Action::ShowMore);
                s.key(0x148);
                assert_eq!(focused(&s), Action::Properties, "{subject:?}");
            }
        }
    }
    #[test]
    fn geometry_flips_up_keeps_icons_by_the_pointer_and_stays_inside() {
        let m = modern(Subject::File, gates(), Controls::DEFAULT);
        for (anchor, window) in [
            ((10, 10), (800, 600)),
            ((790, 590), (800, 600)),
            ((400, 500), (800, 600)),
            ((0, 0), (300, 200)),
        ] {
            let s = MenuState::open(m.clone(), anchor, window, false);
            let p = s.root();
            assert!(p.bounds.x >= 0 && p.bounds.y >= 0);
            assert!(p.bounds.x + p.bounds.width as i32 <= window.0 as i32);
            assert!(p.bounds.y + p.bounds.height as i32 <= window.1 as i32);
            let rows: Vec<_> = p.rows.iter().flatten().collect();
            for (i, r) in rows.iter().enumerate() {
                assert!(p.bounds.contains(r.x, r.y));
                for other in &rows[..i] {
                    assert!(!other.contains(r.x, r.y));
                }
            }
            for icon in &p.icons {
                assert!(rows.iter().all(|r| !r.contains(icon.x, icon.y)));
            }
            if p.upward {
                assert!(p.icons[0].y > rows[0].y, "icon row moves to the bottom");
            }
        }
    }
    #[test]
    fn keyboard_wraps_skips_disabled_and_enters_submenus() {
        let m = modern(Subject::Background, gates(), Controls::DEFAULT);
        let mut s = MenuState::open(m, (10, 10), (800, 600), true);
        assert_eq!(s.focus, Focus::Row(0));
        assert_eq!(s.key(0x148), Outcome::Stay);
        assert_eq!(
            s.focus,
            Focus::Row(s.menu.entries.len() - 1),
            "Up wraps to Customize menu"
        );
        // Up skips separators and the disabled Terminal row (not home).
        s.key(0x148);
        s.key(0x148);
        let Focus::Row(i) = s.focus else { panic!() };
        assert_eq!(s.menu.entries[i].action, Action::Properties);
        s.key(0x147);
        assert_eq!(s.key(0x14d), Outcome::Stay);
        let (parent, child, _) = s.sub.unwrap();
        assert_eq!((parent, child), (0, Some(0)));
        s.key(0x150);
        assert_eq!(
            s.key(28),
            Outcome::Invoke(Action::View(1), Source::Submenu, 1)
        );
        s.key(0x14b);
        assert!(s.sub.is_none());
        // A menu without an icon row keeps Tab in the list.
        s.key(15);
        assert!(matches!(s.focus, Focus::Row(_)));
        // The icon row is reached with Tab and walked with Left/Right,
        // skipping the disabled clipboard commands.
        let mut f = MenuState::open(
            modern(Subject::File, gates(), Controls::DEFAULT),
            (10, 10),
            (800, 600),
            true,
        );
        f.key(15);
        assert_eq!(f.focus, Focus::Icon(2), "Rename is the first enabled icon");
        f.key(0x14d);
        assert_eq!(f.focus, Focus::Icon(4), "Share is skipped");
        assert_eq!(f.key(28), Outcome::Invoke(Action::Delete, Source::Icon, 4));
        assert_eq!(s.key(1), Outcome::Dismiss);
    }
    #[test]
    fn pointer_targets_match_painted_rows_and_disabled_rows_never_invoke() {
        let m = modern(Subject::File, gates(), Controls::DEFAULT);
        let mut s = MenuState::open(m, (100, 100), (900, 700), false);
        let p = s.root();
        for (i, r) in p.rows.iter().enumerate() {
            let Some(r) = r else { continue };
            assert_eq!(s.hit(r.x + 3, r.y + 3), Hit::Row(i));
        }
        let cut = p.icons[0];
        assert_eq!(s.click(cut.x + 4, cut.y + 4), Outcome::Stay, "no clipboard");
        assert_eq!(
            s.entry_at(cut.x + 4, cut.y + 4).unwrap().0.reason,
            NO_CLIPBOARD
        );
        let open = p.rows[0].unwrap();
        assert_eq!(
            s.click(open.x + 4, open.y + 4),
            Outcome::Invoke(Action::Open, Source::Root, 0)
        );
        assert_eq!(s.click(0, 0), Outcome::Dismiss);
    }
    #[test]
    fn long_flyouts_scroll_by_rows_and_reveal_keyboard_focus() {
        let entries: Vec<_> = (0..60)
            .map(|i| Entry::item("Folder", Action::Pick(i)).glyph(Glyph::Folder))
            .collect();
        let mut s = MenuState::open(flyout(entries, 240), (20, 40), (600, 400), true);
        let p = s.root();
        assert!(p.more_below && !p.more_above);
        assert!(p.bounds.y + p.bounds.height as i32 <= 400);
        for _ in 0..30 {
            s.key(0x150);
        }
        let Focus::Row(i) = s.focus else { panic!() };
        assert_eq!(i, 30);
        assert!(s.root().rows[30].is_some(), "focused row is visible");
        assert!(s.wheel(1));
    }
}

#[cfg(test)]
mod file_operation_tests {
    use super::*;
    fn labels(m: &Menu) -> Vec<&str> {
        m.entries
            .iter()
            .filter(|e| !e.separator)
            .map(|e| e.label.as_str())
            .collect()
    }
    #[test]
    fn clipboard_commands_follow_their_gates_and_the_bin_has_its_own_menus() {
        let c = Controls::DEFAULT;
        let open = Gates {
            cut: None,
            copy: None,
            paste: None,
            ..Gates::default()
        };
        let file = modern(Subject::File, open, c);
        assert!(file.icons[..2].iter().all(|e| e.enabled), "Cut and Copy");
        assert_eq!(file.icons[4].shortcut, "Delete", "Delete recycles");
        let background = modern(Subject::Background, open, c);
        assert!(
            background
                .entries
                .iter()
                .any(|e| e.action == Action::Paste && e.enabled)
        );
        // Recycle Bin items: Restore is first (Shift+F10, Enter restores).
        let item = modern(Subject::RecycleItem, Gates::default(), c);
        assert_eq!(item.entries[0].action, Action::Restore);
        let mut s = MenuState::open(item.clone(), (10, 10), (800, 600), true);
        assert_eq!(s.key(28), Outcome::Invoke(Action::Restore, Source::Root, 0));
        assert_eq!(
            item.icons
                .iter()
                .map(|e| e.label.as_str())
                .collect::<Vec<_>>(),
            ["Cut", "Delete"]
        );
        let bin = Gates {
            recycle: true,
            empty_bin: None,
            ..Gates::default()
        };
        let back_menu = modern(Subject::Background, bin, c);
        let back = labels(&back_menu);
        assert!(back.contains(&"Empty Recycle Bin") && back.contains(&"Restore all items"));
        assert!(!back.contains(&"Paste") && !back.contains(&"New"));
        let row = modern(Subject::RecycleBin, Gates::default(), c);
        let empty = row
            .entries
            .iter()
            .find(|e| e.action == Action::EmptyRecycleBin)
            .unwrap();
        assert!(!empty.enabled && empty.reason == BIN_EMPTY);
        assert_eq!(
            classic(Subject::RecycleItem, Gates::default()).entries[0].action,
            Action::Restore
        );
    }
}

#[cfg(test)]
mod open_with_tests {
    use super::*;
    fn apps() -> Vec<OpenWithApp> {
        alloc::vec![
            OpenWithApp {
                name: "Notes".into(),
                icon: 2,
                unavailable: None,
            },
            OpenWithApp {
                name: "Terminal".into(),
                icon: 4,
                unavailable: Some("Unavailable: Terminal can't open a file yet."),
            },
        ]
    }
    #[test]
    fn open_with_follows_open_in_compact_and_classic_file_menus() {
        let gates = Gates {
            cut: None,
            copy: None,
            ..Gates::default()
        };
        let mut menu = modern(Subject::File, gates, Controls::DEFAULT);
        insert_open_with(&mut menu, open_with(&apps()));
        let labels: Vec<&str> = menu
            .entries
            .iter()
            .filter(|e| !e.separator)
            .map(|e| e.label.as_str())
            .collect();
        assert_eq!(labels[..2], ["Open", "Open with"]);
        let sub = &menu.entries[1];
        assert_eq!(sub.action, Action::Submenu);
        assert_eq!(sub.glyph, Glyph::Action);
        let rows: Vec<(&str, Action, bool)> = sub
            .children
            .iter()
            .filter(|e| !e.separator)
            .map(|e| (e.label.as_str(), e.action, e.enabled))
            .collect();
        assert_eq!(
            rows,
            [
                ("Notes", Action::OpenWith(0), true),
                ("Terminal", Action::OpenWith(1), false),
                ("Choose another app", Action::ChooseApp, true),
            ]
        );
        assert_eq!(sub.children[0].glyph, Glyph::App(2));
        assert!(sub.children[1].reason.starts_with("Unavailable:"));
        assert!(sub.children[2].separator);
        let mut classic = classic(Subject::File, gates);
        insert_open_with(&mut classic, open_with(&[]));
        let at = classic
            .entries
            .iter()
            .position(|e| e.label == "Open with")
            .unwrap();
        assert_eq!(classic.entries[at - 1].action, Action::Open);
        assert_eq!(classic.entries[at].glyph, Glyph::None);
        let only: Vec<Action> = classic.entries[at]
            .children
            .iter()
            .map(|e| e.action)
            .collect();
        assert_eq!(
            only,
            [Action::ChooseApp],
            "no apps: only Choose another app"
        );
        // Folders have no Open command in the compact menu: unchanged.
        let mut folder = modern(Subject::Folder, gates, Controls::DEFAULT);
        let before = folder.clone();
        insert_open_with(&mut folder, open_with(&apps()));
        assert_eq!(folder, before);
    }
    #[test]
    fn the_open_with_submenu_opens_and_invokes_by_keyboard() {
        let gates = Gates::default();
        let mut menu = modern(Subject::File, gates, Controls::DEFAULT);
        insert_open_with(&mut menu, open_with(&apps()));
        let mut state = MenuState::open(menu, (100, 100), (1280, 800), true);
        // Down to "Open with", Right opens it, Enter picks Notes.
        state.key(0x150);
        state.key(0x14d);
        assert!(state.sub.is_some());
        assert_eq!(
            state.key(28),
            Outcome::Invoke(Action::OpenWith(0), Source::Submenu, 0)
        );
    }
    #[test]
    fn the_classic_setting_lays_out_windows_10_menus() {
        let mut menu = classic(Subject::Background, Gates::default());
        menu.look.win10 = true;
        let mut state = MenuState::open(menu, (100, 100), (1280, 800), false);
        let p = state.root();
        let b = p.bounds;
        let first = p.rows[0].unwrap();
        // Rows: 3 px in from the frame's sides, 5 px from its top, 22 px.
        assert_eq!((first.x - b.x, first.y - b.y, first.height), (3, 5, 22));
        assert_eq!(first.width, b.width - 6);
        // View, Sort by, Refresh, then a 7 px separator with its rule 3 px
        // down; the frame ends 5 px under the last row.
        let refresh = p.rows[2].unwrap();
        assert_eq!(p.separators[0], refresh.y + 22 + 3);
        assert_eq!(p.rows[4].unwrap().y, refresh.y + 22 + 7);
        let last = p.rows.iter().rev().find_map(|r| *r).unwrap();
        assert_eq!(b.y + b.height as i32, last.y + 22 + 5);
        // View's submenu overlaps the frame by 5 px, first row level with it.
        state.sub = Some((0, None, 0));
        let sub = state.submenu().unwrap();
        assert_eq!(sub.bounds.x, b.x + b.width as i32 - 6);
        assert_eq!(sub.rows[0].unwrap().y, first.y);
        // "Show more options" keeps 26H2's styling of the legacy menu.
        let legacy = MenuState::open(
            classic(Subject::Background, Gates::default()),
            (100, 100),
            (1280, 800),
            false,
        );
        let lp = legacy.root();
        assert_eq!(lp.rows[0].unwrap().x - lp.bounds.x, 4);
    }
}
