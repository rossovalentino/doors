//! File Explorer's semantic tree for door.accessibility and its pane focus
//! model. Everything is described from the geometry `modern_target`
//! hit-tests (tabs, navigation buttons, address and search boxes, command
//! bar, navigation tree, items view, Home sections, status view buttons)
//! plus the open context menu or delete dialog.
//!
//! Keyboard: Tab and Shift+Tab move between the items view and the
//! navigation pane (File Explorer's pane cycle); in the pane, Up/Down move
//! through the tree, Right/Left expand or collapse, Enter opens and Escape
//! returns to the items. Focus otherwise follows the existing model: the
//! address or search editor, the rename box, the menu's highlighted
//! command, the dialog's focused button, or the selected item.
use super::*;
use door_workspace::semantics::{self as sem, Actions, Role, Semantics, States};

fn b(r: Bounds) -> (i32, i32, u32, u32) {
    (r.x, r.y, r.width, r.height)
}
/// Command bar names by target id (icon-only commands are named as their
/// tool tips are in Windows).
fn command_name(id: u32, text: &'static str) -> &'static str {
    match id {
        8 => "New",
        44 => "Cut",
        45 => "Copy",
        46 => "Paste",
        33 => "Rename",
        47 => "Share",
        48 => "Delete",
        5 => "Sort",
        6 => "View",
        7 => "See more",
        910 => "Details",
        49 => "Empty Recycle Bin",
        50 => "Restore",
        _ => text,
    }
}
impl Files {
    /// Rows of the navigation tree a keyboard can stop on.
    fn navigation_stops(rows: &[door_workspace::explorer_tree::Row]) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.node != door_workspace::explorer_tree::Node::Separator)
            .map(|(i, _)| i)
            .collect()
    }
    /// Pane focus keys; None when the key is not one of them here.
    pub(super) fn navigation_key(
        &mut self,
        code: u32,
        storage: &mut Storage,
    ) -> Result<Option<bool>, Error> {
        let scan = code & 511;
        let m = code >> 9;
        if self.editor != 0
            || self.modal()
            || self.xmenu.is_some()
            || m & (door_abi::graphics::CONTROL | door_abi::graphics::ALT) != 0
        {
            return Ok(None);
        }
        let rows = self.rail_rows();
        let stops = Self::navigation_stops(&rows);
        if scan == 15 {
            self.nav_focus = match self.nav_focus {
                Some(_) => None,
                None => self.rail_selected(&rows).or(stops.first().copied()),
            };
            if self.nav_focus.is_none() && self.selected.is_none() {
                let first = self.shown().next().map(|(i, _)| i);
                self.selected = first;
            }
            return Ok(Some(true));
        }
        let Some(at) = self.nav_focus.filter(|i| *i < rows.len()) else {
            self.nav_focus = None;
            return Ok(None);
        };
        let position = stops.iter().position(|i| *i == at).unwrap_or(0);
        match scan {
            0x148 => self.nav_focus = Some(stops[position.saturating_sub(1)]),
            0x150 => self.nav_focus = stops.get(position + 1).or(stops.last()).copied(),
            0x147 => self.nav_focus = stops.first().copied(),
            0x14f => self.nav_focus = stops.last().copied(),
            0x14d if rows[at].expandable && !rows[at].expanded => self.toggle_rail_row(storage, at),
            0x14d => self.nav_focus = stops.get(position + 1).copied().or(Some(at)),
            0x14b if rows[at].expanded => self.toggle_rail_row(storage, at),
            28 | 0x11c | 57 => self.open_rail_row(storage, at)?,
            1 => self.nav_focus = None,
            _ => return Ok(None),
        }
        Ok(Some(true))
    }
    /// The navigation pane's keyboard focus ring (WinUI focus visual: two
    /// pixels in the text color around the row).
    pub(super) fn draw_navigation_focus(&self, v: &mut ui::View<'_>) {
        let Some(at) = self.nav_focus else { return };
        let rows = self.rail_rows();
        let Some(r) = self.rail_row_bounds(&rows).get(at).copied() else {
            return;
        };
        let y = r.y - self.rail_offset;
        if y < content_top() || y + r.height as i32 > self.layout.height as i32 {
            return;
        }
        let color = if v.palette.dark {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        };
        let (x, w, h) = (r.x, r.width, r.height);
        v.rect(x, y, w, 2, color);
        v.rect(x, y + h as i32 - 2, w, 2, color);
        v.rect(x, y, 2, h, color);
        v.rect(x + w as i32 - 2, y, 2, h, color);
    }
    pub fn semantics(&self, s: &mut Semantics, fonts: &FontAtlas) {
        if self.removal.dialog.open() {
            let (title, body, buttons) = self.removal_texts();
            sem::dialog(
                s,
                &self.dialog_layout(),
                &self.removal.dialog,
                &title,
                &body,
                buttons,
                3000,
            );
            return;
        }
        if self.security_open() {
            self.security_semantics(s);
            return;
        }
        if self.picker_open() {
            self.picker_semantics(s);
            return;
        }
        // The progress dialog (non-modal), or the modal conflict / prompt.
        self.operations_semantics(s);
        if self.modal() {
            return;
        }
        let menu_open = self.xmenu.is_some();
        let free = !menu_open && self.editor == 0;
        // Tabs.
        let tabs = self.visible_tabs();
        if !tabs.is_empty() {
            let first = self.tab_bounds(tabs.start);
            s.begin(
                Role::TabList,
                "Tabs",
                (first.x, first.y, self.caption_width(), first.height),
            );
            for i in tabs.clone() {
                let r = self.tab_bounds(i);
                s.control(Role::Tab, self.tab_label(i), b(r), Some(800 + i as u32))
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, i == self.active_tab)
                    .actions(Actions::INVOKE | Actions::SELECT)
                    .position(i, self.tab_count());
                let close = Bounds {
                    x: r.x + r.width as i32 - 26,
                    y: r.y,
                    width: 26,
                    height: r.height,
                };
                s.control(Role::Button, "Close tab", b(close), Some(850 + i as u32));
            }
            s.end();
            for (r, id) in self.caption_controls() {
                let name = match id {
                    897 => "Scroll tabs left",
                    898 => "Scroll tabs right",
                    _ => "New tab",
                };
                s.control(Role::Button, name, b(r), Some(id));
            }
        }
        // Navigation buttons, address and search.
        for (index, (id, name)) in [(20, "Back"), (21, "Forward"), (22, "Up"), (2, "Refresh")]
            .into_iter()
            .enumerate()
        {
            if let Some(r) = ExplorerLayout::navigation(index) {
                let disabled = match id {
                    20 => self.history_index == 0,
                    21 => self.history_index + 1 >= self.history.len(),
                    _ => false,
                };
                s.control(Role::Button, name, b(r), Some(id))
                    .state(States::DISABLED, disabled);
            }
        }
        let address = if self.editor == 1 {
            String::from(self.edit.text())
        } else {
            explorer_path::display(&self.cwd)
        };
        s.control(Role::Edit, "Address", b(self.layout.address()), Some(30))
            .states(States::EDITABLE)
            .actions(Actions::INVOKE | Actions::FOCUS | Actions::SET_VALUE)
            .value(&address)
            .focused(!menu_open && self.editor == 1);
        let query = if self.editor == 2 {
            String::from(self.edit.text())
        } else {
            self.search.clone()
        };
        s.control(
            Role::Edit,
            &format!("Search {}", self.title()),
            b(self.layout.search()),
            Some(31),
        )
        .states(States::EDITABLE)
        .actions(Actions::INVOKE | Actions::FOCUS | Actions::SET_VALUE)
        .value(&query)
        .focused(!menu_open && self.editor == 2);
        // Command bar.
        let commands = self.command_controls();
        if let Some((first, ..)) = commands.first() {
            s.begin(
                Role::ToolBar,
                "Command bar",
                (
                    first.x,
                    first.y,
                    self.layout.width.saturating_sub(first.x as u32),
                    first.height,
                ),
            );
            for (r, id, _, text, enabled) in commands {
                let c = s
                    .control(Role::Button, command_name(id, text), b(r), Some(id))
                    .state(States::DISABLED, !enabled);
                let c = if matches!(id, 5..=8) {
                    c.states(States::HAS_POPUP)
                } else {
                    c
                };
                if id == 910 {
                    c.states(States::CHECKABLE)
                        .state(States::CHECKED, self.details_visible)
                        .actions(Actions::INVOKE | Actions::TOGGLE);
                }
            }
            s.end();
        }
        // Navigation pane (tree).
        let rows = self.rail_rows();
        let selected_row = self.rail_selected(&rows);
        let bounds = self.rail_row_bounds(&rows);
        s.begin(
            Role::Tree,
            "Navigation Pane",
            (
                0,
                content_top(),
                self.layout.sidebar as u32,
                self.layout
                    .height
                    .saturating_sub(content_top() as u32 + xl::status_height()),
            ),
        );
        for (i, (row, r)) in rows.iter().zip(&bounds).enumerate() {
            if row.node == door_workspace::explorer_tree::Node::Separator {
                continue;
            }
            let y = r.y - self.rail_offset;
            let c = s
                .control(
                    Role::TreeItem,
                    &row.label,
                    (r.x, y, r.width, r.height),
                    Some(1100 + i as u32),
                )
                .states(States::SELECTABLE)
                .state(States::SELECTED, selected_row == Some(i))
                .state(
                    States::OFFSCREEN,
                    y + (r.height as i32) <= content_top()
                        || y >= self.layout.height as i32 - xl::status_height() as i32,
                )
                .actions(Actions::INVOKE | Actions::SELECT)
                .level(u16::from(row.depth) + 1)
                .focused(free && self.nav_focus == Some(i));
            if row.expandable {
                c.states(States::EXPANDABLE)
                    .state(States::EXPANDED, row.expanded)
                    .actions(
                        Actions::INVOKE | Actions::SELECT | Actions::EXPAND | Actions::COLLAPSE,
                    );
            }
        }
        s.end();
        // Items view.
        let items_focus = free && self.nav_focus.is_none();
        let l = self.content_layout();
        let area = (
            self.layout.sidebar,
            content_top(),
            l.width.saturating_sub(self.layout.sidebar as u32),
            self.layout
                .height
                .saturating_sub(content_top() as u32 + xl::status_height()),
        );
        if self.home_view {
            let home = self.home_layout();
            s.begin(Role::List, "Items View", area)
                .focused(items_focus && self.selected.is_none());
            let total = self.entries.len();
            for (section, header) in home.headers.iter().enumerate() {
                let open = !self.home_collapsed(section as u8);
                s.control(
                    Role::Expander,
                    home::SECTIONS[section],
                    b(*header),
                    Some(home::HEADER + section as u32),
                )
                .states(States::EXPANDABLE)
                .state(States::EXPANDED, open)
                .actions(Actions::INVOKE | Actions::EXPAND | Actions::COLLAPSE);
                for (i, entry) in self.entries.iter().enumerate() {
                    if self.home_section(i) != Some(section as u8) {
                        continue;
                    }
                    let Some(Some(cell)) = home.cells.get(i) else {
                        continue;
                    };
                    s.control(
                        Role::ListItem,
                        &Self::name(entry),
                        b(*cell),
                        Some(100 + i as u32),
                    )
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, self.selected == Some(i))
                    .actions(Actions::INVOKE | Actions::SELECT)
                    .position(i, total)
                    .focused(items_focus && self.selected == Some(i));
                }
            }
            s.end();
        } else if self.overview && self.search.is_empty() {
            s.begin(Role::List, "Items View", area)
                .focused(items_focus && self.drives().is_empty());
            s.control(
                Role::Expander,
                "Devices and drives",
                b(self.drives_header()),
                Some(502),
            )
            .states(States::EXPANDABLE)
            .state(States::EXPANDED, self.drives_expanded)
            .actions(Actions::INVOKE | Actions::EXPAND | Actions::COLLAPSE);
            let drives = self.drives();
            for (row, slot) in (self.offset..drives.len()).enumerate() {
                if let Some(cell) = self.item_cell(row) {
                    s.control(
                        Role::ListItem,
                        drives[slot].label,
                        b(cell),
                        Some(500 + slot as u32),
                    )
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, self.overview_selected == slot)
                    .actions(Actions::INVOKE | Actions::SELECT)
                    .position(slot, drives.len())
                    .focused(items_focus && self.overview_selected == slot);
                }
            }
            s.end();
        } else {
            let total = self.shown().count();
            let focus_visible = self.selected.is_some_and(|sel| {
                self.shown()
                    .skip(self.offset)
                    .take(l.capacity(self.view))
                    .any(|(i, _)| i == sel)
            });
            s.begin(Role::List, "Items View", area)
                .focused(items_focus && !focus_visible);
            for (row, (index, entry)) in self
                .shown()
                .skip(self.offset)
                .take(l.capacity(self.view))
                .enumerate()
            {
                let Some(cell) = self.item_cell(row) else {
                    continue;
                };
                let marked = self.marked.contains(&entry.info.object);
                s.control(
                    Role::ListItem,
                    &Self::name(entry),
                    b(cell),
                    Some(100 + index as u32),
                )
                .states(States::SELECTABLE)
                .state(States::SELECTED, self.selected == Some(index) || marked)
                .actions(Actions::INVOKE | Actions::SELECT)
                .position(self.offset + row, total)
                .focused(items_focus && self.selected == Some(index));
            }
            s.end();
        }
        // Rename box over the item being renamed.
        if self.editor == 3
            && let Some(r) = self.rename_bounds()
        {
            s.control(Role::Edit, "Rename", b(r), Some(34))
                .states(States::EDITABLE)
                .actions(Actions::FOCUS | Actions::SET_VALUE)
                .value(self.edit.text())
                .focused(!menu_open);
        }
        // Status bar view buttons.
        for (icons, id, name) in [(false, 40, "Details"), (true, 41, "Large Icons")] {
            let on = matches!(
                (icons, self.view_mode()),
                (false, ViewMode::Details) | (true, ViewMode::Large)
            );
            s.control(
                Role::Button,
                name,
                b(self.layout.status_view(icons)),
                Some(id),
            )
            .states(States::CHECKABLE)
            .state(States::CHECKED, on);
        }
        if let Some(m) = &self.xmenu {
            sem::context_menu(s, m, &|x, y| self.modern_target(x, y, fonts));
        }
    }
}
