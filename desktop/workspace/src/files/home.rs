//! File Explorer Home (Windows 11): the default landing location with three
//! collapsible sections — Quick access (known folders plus folders pinned
//! with "Pin to Quick access"), Favorites ("Add to Favorites") and Recent
//! (files opened through Explorer, Notes and Preview, from
//! `door_workspace::recent`). Items are real Storage objects listed with
//! their full paths (like search results), so selection, the details pane,
//! the command bar, context menus and keyboard navigation work on them.
//! Collapsed sections, pins and favorites persist in the views file.
use super::modern::paint::{HOVER, MUTED, TEXT, label, line, pinned};
use super::*;
use door_workspace::recent::{self, Recent};

pub(crate) const HOME_PAGE: &str = "::Home";
pub(super) const SECTIONS: [&str; 3] = ["Quick access", "Favorites", "Recent"];
/// Section header targets: 510 + section.
pub(super) const HEADER: u32 = 510;
pub(super) const QUICK: u8 = 0;
pub(super) const FAVORITES: u8 = 1;
pub(super) const RECENT: u8 = 2;
const HEADER_H: i32 = 32;
const ROW_H: i32 = 32;
/// Quick access tiles (E6): Windows lays 270x64 items at a 284x60 pitch
/// (405x96 px at 426x90 at 150 %), their margins overlapping; Door keeps
/// the pitch with a 270x56 cell, so neighbours never share a pixel.
const TILE: (u32, u32) = (270, 56);
const TILE_GAP: (u32, u32) = (14, 4);
/// Folder icon of a tile (Windows 72 px at 150 %) and its text column.
const TILE_ICON: u32 = 48;
const TILE_TEXT: i32 = 8 + TILE_ICON as i32 + 11;
/// Section headers, list rows and tiles (two lines) hold 14 px text and
/// grow with its line box at the applied text size.
fn header_h() -> i32 {
    HEADER_H + xl::grow()
}
fn row_h() -> i32 {
    ROW_H + xl::grow()
}
fn tile_h() -> u32 {
    TILE.1 + 2 * xl::grow() as u32
}

/// Windows' Home empty-state wording.
pub(super) const EMPTY_FAVORITES: &str = "After you've favorited some files, we'll show them here.";
pub(super) const EMPTY_RECENT: &str =
    "After you've opened some files, we'll show the most recent ones here.";

pub(super) struct HomeLayout {
    /// Window-space bounds (scroll applied) of each section header.
    pub(super) headers: [Bounds; 3],
    /// Per entry index: its cell, when its section is expanded.
    pub(super) cells: Vec<Option<Bounds>>,
    /// List-style sections: (section, column header row y).
    pub(super) columns: Vec<(u8, i32)>,
    /// Empty-state rows.
    pub(super) empty: Vec<(u8, Bounds)>,
    /// Content height before scrolling.
    pub(super) height: i32,
}

fn entry_named(name: &str, info: door_service_client::FileInfo) -> DirEntry {
    let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
    let mut len = 0;
    for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
        *o = u;
        len += 1;
    }
    DirEntry {
        times: Default::default(),
        cookie: info.object,
        name: units,
        name_len: len,
        info,
    }
}

impl Files {
    pub(super) fn home_list_mode(&self) -> bool {
        matches!(
            self.view,
            ViewMode::Details | ViewMode::List | ViewMode::Small | ViewMode::Content
        )
    }
    /// Known folders (always pinned) then the user's pins.
    pub(super) fn quick_access(&self) -> Vec<(String, String, bool)> {
        let mut out: Vec<(String, String, bool)> = PINNED_ORDER
            .iter()
            .map(|i| {
                (
                    format!("{}/{}", self.home, FOLDERS[*i].0),
                    String::from(FOLDERS[*i].0),
                    false,
                )
            })
            .collect();
        for path in &self.prefs.pins {
            let name = path.rsplit('/').next().unwrap_or(path).into();
            out.push((path.clone(), name, true));
        }
        out
    }
    /// Some(true) when `path` is a user pin, Some(false) when it is a known
    /// folder (always pinned), None when it is not pinned.
    pub(super) fn pinned(&self, path: &str) -> Option<bool> {
        self.quick_access()
            .into_iter()
            .find(|(p, _, _)| p == path)
            .map(|(_, _, user)| user)
    }
    pub(super) fn toggle_pin(&mut self, path: &str, storage: &mut Storage) {
        match self.pinned(path) {
            Some(true) => self.prefs.pins.retain(|p| p != path),
            Some(false) => return,
            None => {
                if self.prefs.pins.len() >= prefs::MAX_PINS {
                    self.message = format!(
                        "Quick access holds up to {} pinned folders.",
                        prefs::MAX_PINS
                    );
                    return;
                }
                self.prefs.pins.push(path.into());
            }
        }
        self.prefs_dirty = true;
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES PIN path={path} pinned={}\n",
            self.pinned(path).is_some()
        ));
        if self.home_view {
            let _ = self.show_home(storage);
        }
    }
    pub(super) fn favorite(&self, path: &str) -> bool {
        self.prefs.favorites.iter().any(|p| p == path)
    }
    pub(super) fn toggle_favorite(&mut self, path: &str, storage: &mut Storage) {
        if self.favorite(path) {
            self.prefs.favorites.retain(|p| p != path);
        } else if self.prefs.favorites.len() >= prefs::MAX_FAVORITES {
            self.message = format!("Favorites holds up to {} files.", prefs::MAX_FAVORITES);
            return;
        } else {
            self.prefs.favorites.insert(0, path.into());
        }
        self.prefs_dirty = true;
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES FAVORITE path={path} on={}\n",
            self.favorite(path)
        ));
        if self.home_view {
            let _ = self.show_home(storage);
        }
    }
    /// Home section of a listed entry, when Home is shown.
    pub(super) fn home_section(&self, index: usize) -> Option<u8> {
        self.home_view
            .then(|| self.home_sections.get(index).copied())
            .flatten()
    }
    pub(super) fn go_home(&mut self, storage: &mut Storage) -> Result<(), Error> {
        self.push_history(HOME_PAGE);
        self.show_home(storage)
    }
    /// Lists Home: each item is checked with Storage, so deleted or moved
    /// items do not appear (Recent keeps them until removed or reopened).
    pub(super) fn show_home(&mut self, storage: &mut Storage) -> Result<(), Error> {
        // Other apps record into the same list; merge before showing it.
        let _ = self.recent.save(storage, &self.home);
        if self.recent.persistent() || self.recent.items().is_empty() {
            self.recent = Recent::load(storage, &self.home);
        }
        let mut entries = Vec::new();
        let mut paths = Vec::new();
        let mut sections = Vec::new();
        let mut add = |storage: &mut Storage, path: &str, name: &str, section: u8| {
            if paths.len() >= 127
                || paths
                    .iter()
                    .zip(&sections)
                    .any(|(p, s): (&String, &u8)| p == path && *s == section)
            {
                return;
            }
            if let Ok(info) = storage.metadata(path) {
                entries.push(entry_named(name, info));
                paths.push(String::from(path));
                sections.push(section);
            }
        };
        for (path, name, _) in self.quick_access() {
            add(storage, &path, &name, QUICK);
        }
        for path in self.prefs.favorites.clone() {
            add(
                storage,
                &path,
                path.rsplit('/').next().unwrap_or(""),
                FAVORITES,
            );
        }
        let recent: Vec<String> = self.recent.items().iter().map(|i| i.path.clone()).collect();
        for path in recent {
            add(
                storage,
                &path,
                path.rsplit('/').next().unwrap_or(""),
                RECENT,
            );
        }
        let key = String::from(HOME_PAGE);
        self.install(self.home.clone(), Vec::new());
        self.home_view = true;
        self.home_scroll = 0;
        self.entries = entries;
        self.result_paths = paths;
        self.home_sections = sections;
        self.view = ViewMode::Details;
        self.recall_view(&key);
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES HOME quick={} favorites={} recent={}\n",
            self.home_sections.iter().filter(|s| **s == QUICK).count(),
            self.home_sections
                .iter()
                .filter(|s| **s == FAVORITES)
                .count(),
            self.home_sections.iter().filter(|s| **s == RECENT).count()
        ));
        Ok(())
    }
    pub(super) fn home_collapsed(&self, section: u8) -> bool {
        self.prefs.home_collapsed & (1 << section) != 0
    }
    pub(super) fn toggle_home_section(&mut self, section: u8) {
        self.prefs.home_collapsed ^= 1 << section;
        self.prefs_dirty = true;
        self.home_scroll = self.home_scroll.min(self.home_scroll_max());
        if self
            .selected
            .and_then(|i| self.home_section(i))
            .is_some_and(|s| self.home_collapsed(s))
        {
            self.selected = None;
            self.detail = None;
        }
    }
    pub(super) fn home_layout(&self) -> HomeLayout {
        let content = self.content_layout();
        let x = self.layout.sidebar + 16;
        let width = content
            .width
            .saturating_sub(self.layout.sidebar as u32 + 32)
            .max(1);
        let mut y = content_top() + 8 - self.home_scroll;
        let top = y;
        let mut headers = [Bounds {
            x,
            y,
            width,
            height: header_h() as u32,
        }; 3];
        let mut cells = alloc::vec![None; self.entries.len()];
        let mut columns = Vec::new();
        let mut empty = Vec::new();
        let list = self.home_list_mode();
        let shown: Vec<usize> = self.shown().map(|(i, _)| i).collect();
        for section in [QUICK, FAVORITES, RECENT] {
            headers[section as usize] = Bounds {
                x: x - 6,
                y,
                width: width + 6,
                height: header_h() as u32,
            };
            y += header_h() + 4;
            if self.home_collapsed(section) {
                y += 6;
                continue;
            }
            let items: Vec<usize> = shown
                .iter()
                .copied()
                .filter(|i| self.home_sections.get(*i) == Some(&section))
                .collect();
            if items.is_empty() {
                if section != QUICK {
                    empty.push((
                        section,
                        Bounds {
                            x,
                            y,
                            width,
                            height: (40 + xl::grow()) as u32,
                        },
                    ));
                    y += 44 + xl::grow();
                }
                y += 12;
                continue;
            }
            if section == QUICK || !list {
                let (gx, gy) = TILE_GAP;
                let cols = ((width + gx) / (TILE.0 + gx)).max(1) as usize;
                let w = ((width + gx) / cols as u32).saturating_sub(gx).min(TILE.0);
                let pitch = tile_h() as i32 + gy as i32;
                for (n, i) in items.iter().enumerate() {
                    cells[*i] = Some(Bounds {
                        x: x + (n % cols) as i32 * (w + gx) as i32,
                        y: y + (n / cols) as i32 * pitch,
                        width: w,
                        height: tile_h(),
                    });
                }
                y += items.len().div_ceil(cols) as i32 * pitch;
            } else {
                columns.push((section, y));
                y += 28 + xl::grow();
                for i in &items {
                    cells[*i] = Some(Bounds {
                        x,
                        y,
                        width,
                        height: row_h() as u32 - 2,
                    });
                    y += row_h();
                }
            }
            y += 16;
        }
        HomeLayout {
            headers,
            cells,
            columns,
            empty,
            height: y - top,
        }
    }
    pub(super) fn home_scroll_max(&self) -> i32 {
        let view = self.layout.height as i32 - xl::status_height() as i32 - content_top();
        (self.home_layout().height + 8 - view).max(0)
    }
    /// Scrolls so the item at shown `position` is fully visible.
    pub(super) fn reveal_home(&mut self, position: usize) {
        let Some(index) = self.shown().nth(position).map(|(i, _)| i) else {
            return;
        };
        let Some(cell) = self.home_layout().cells.get(index).copied().flatten() else {
            return;
        };
        let bottom = self.layout.height as i32 - xl::status_height() as i32;
        // The first item of a section brings its header into view too.
        let section = self.home_sections.get(index).copied();
        let first = self
            .shown()
            .find(|(i, _)| self.home_sections.get(*i).copied() == section)
            .map(|(i, _)| i)
            == Some(index);
        let top = cell.y - if first { header_h() + 4 } else { 0 };
        if top < content_top() + 8 {
            self.home_scroll -= content_top() + 8 - top;
        } else if cell.y + cell.height as i32 > bottom - 4 {
            self.home_scroll += cell.y + cell.height as i32 - (bottom - 4);
        }
        self.home_scroll = self.home_scroll.clamp(0, self.home_scroll_max());
    }
    /// Entry under the pointer on Home.
    pub(super) fn home_item_at(&self, x: i32, y: i32) -> Option<usize> {
        let l = self.home_layout();
        l.cells
            .iter()
            .position(|c| c.is_some_and(|c| c.contains(x, y)))
    }
    pub(super) fn home_target(&self, x: i32, y: i32) -> Option<u32> {
        let l = self.home_layout();
        if let Some(s) = l.headers.iter().position(|h| h.contains(x, y)) {
            return Some(HEADER + s as u32);
        }
        if let Some(i) = l
            .cells
            .iter()
            .position(|c| c.is_some_and(|c| c.contains(x, y)))
        {
            return Some(100 + i as u32);
        }
        (x >= self.layout.sidebar).then_some(9998)
    }
    pub(super) fn home_summary(&self) -> Vec<(&'static str, String)> {
        let count = |s: u8| self.home_sections.iter().filter(|x| **x == s).count();
        alloc::vec![
            ("Quick access", format!("{} folders", count(QUICK))),
            ("Favorites", format!("{} items", count(FAVORITES))),
            ("Recent", format!("{} files", count(RECENT))),
        ]
    }
    pub(super) fn draw_home(&self, v: &mut ui::View<'_>) {
        let l = self.home_layout();
        let dark = v.palette.dark;
        for (s, header) in l.headers.iter().enumerate() {
            if v.hover.is_some_and(|(x, y)| header.contains(x, y)) {
                v.rounded(header.x, header.y, header.width, header.height, 4, HOVER);
            }
            let open = !self.home_collapsed(s as u8);
            let mask = self.chevron(v.scale, open);
            v.mask_pixels(
                header.x + 6,
                header.y + (header.height as i32 - 12) / 2,
                12,
                &mask,
                MUTED,
                None,
            );
            v.text_clip_centered(
                header.x + 26,
                (header.y, header.height),
                header.width.saturating_sub(32),
                SECTIONS[s],
                TextStyle::Semibold14,
                TEXT,
            );
        }
        for (section, y) in &l.columns {
            let x = self.layout.sidebar + 16;
            let name_w = self.home_name_width();
            let h = 28 + xl::grow();
            label(v, x + 34, *y, name_w, h as u32, "Name", MUTED);
            label(
                v,
                x + 34 + name_w as i32 + 16,
                *y,
                220,
                h as u32,
                "Location",
                MUTED,
            );
            line(
                v,
                x,
                y + h - 1,
                self.content_layout()
                    .width
                    .saturating_sub(self.layout.sidebar as u32 + 32),
            );
            let _ = section;
        }
        for (section, b) in &l.empty {
            label(
                v,
                b.x + 8,
                b.y,
                b.width.saturating_sub(16),
                b.height,
                if *section == FAVORITES {
                    EMPTY_FAVORITES
                } else {
                    EMPTY_RECENT
                },
                MUTED,
            );
        }
        for (i, cell) in l.cells.iter().enumerate() {
            let Some(cell) = cell else { continue };
            let Some(entry) = self.entries.get(i) else {
                continue;
            };
            let section = self.home_sections.get(i).copied().unwrap_or(RECENT);
            self.item_fill(v, *cell, self.is_selected(i));
            let name = ui::display_ascii(&Self::name(entry));
            let icon = if entry.info.directory {
                Self::folder_icon(&name)
            } else {
                Icon::for_file_name(&name)
            };
            let location = self
                .result_paths
                .get(i)
                .map(|p| {
                    let parent = p
                        .rsplit_once('/')
                        .map_or("/", |(a, _)| if a.is_empty() { "/" } else { a });
                    ui::display_ascii(&explorer_path::display(parent))
                })
                .unwrap_or_default();
            if cell.height >= tile_h() {
                let g = xl::grow();
                v.sized_icon(
                    cell.x + 8,
                    cell.y + (cell.height as i32 - TILE_ICON as i32) / 2,
                    TILE_ICON,
                    icon,
                );
                // Name, then "Stored locally" (or the location), then the
                // filled pin on the third line under them (E6).
                let text_w = cell.width.saturating_sub(TILE_TEXT as u32 + 8);
                label(
                    v,
                    cell.x + TILE_TEXT,
                    cell.y + 3,
                    text_w,
                    (18 + g) as u32,
                    &name,
                    TEXT,
                );
                let sub = if section == QUICK {
                    String::from("Stored locally")
                } else {
                    location
                };
                label(
                    v,
                    cell.x + TILE_TEXT,
                    cell.y + 21 + g,
                    text_w,
                    (17 + g) as u32,
                    &sub,
                    MUTED,
                );
                if section == QUICK {
                    pinned(v, cell.x + TILE_TEXT, cell.y + 41 + 2 * g, MUTED);
                }
            } else {
                v.compact_icon(
                    cell.x + 10,
                    cell.y + (cell.height as i32 - 16) / 2,
                    icon,
                    TEXT,
                );
                let name_w = self.home_name_width();
                label(v, cell.x + 34, cell.y, name_w, cell.height, &name, TEXT);
                label(
                    v,
                    cell.x + 34 + name_w as i32 + 16,
                    cell.y,
                    cell.width.saturating_sub(name_w + 60),
                    cell.height,
                    &location,
                    MUTED,
                );
            }
        }
        let max = self.home_scroll_max();
        if max > 0 {
            let track = v.height.saturating_sub(content_top() as u32 + 36) as i32;
            let total = track + max;
            let thumb = (track * track / total.max(1)).max(20);
            let pos = (track - thumb) * self.home_scroll / max;
            v.rounded(
                self.content_layout().width as i32 - 6,
                content_top() + 6 + pos,
                3,
                thumb as u32,
                1,
                if dark {
                    [120, 120, 120, 255]
                } else {
                    [185, 185, 185, 255]
                },
            );
        }
    }
    fn home_name_width(&self) -> u32 {
        (self
            .content_layout()
            .width
            .saturating_sub(self.layout.sidebar as u32 + 80)
            / 2)
        .clamp(120, 420)
    }
}

/// Recording opens from Explorer.
impl Files {
    pub(super) fn record_open(&mut self, path: &str, folder: bool) {
        self.recent.record(
            path,
            if folder {
                recent::Kind::Folder
            } else {
                recent::Kind::File
            },
        );
    }
    /// Saves recent opens (coalesced per tick).
    pub(super) fn save_recent(&mut self, storage: &mut Storage) -> bool {
        if self.recent.dirty() && !self.modal() {
            let _ = self.recent.save(storage, &self.home);
        }
        false
    }
}
