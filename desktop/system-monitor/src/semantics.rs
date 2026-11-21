//! Task Manager's semantic tree for door.accessibility, built from `hits`
//! geometry (the same rectangles paint and hit testing read): search box,
//! navigation, page commands, the page's table (column headers and realized
//! rows, each with its position in the whole set), performance tiles,
//! settings controls, the open menu and the Run new task dialog.
use super::*;
use door_workspace::semantics::{Actions, Role, Semantics, States};

fn r4(r: R) -> (i32, i32, u32, u32) {
    (r.x, r.y, r.w, r.h)
}
impl Monitor {
    pub(crate) fn describe(&self, s: &mut Semantics) {
        if let Some(dialog) = &self.dialog {
            let [field, ok, cancel] = self.dialog_controls();
            s.begin(
                Role::Dialog,
                "Create new task",
                (0, 0, self.width, self.height),
            )
            .states(States::MODAL);
            s.control(Role::Edit, "Open", r4(field), Some(ID_FIELD))
                .states(States::EDITABLE)
                .actions(Actions::FOCUS | Actions::SET_VALUE)
                .value(dialog.input.text())
                .focused(true);
            if !dialog.error.is_empty() {
                s.text(
                    Role::Text,
                    &dialog.error,
                    (field.x, field.y + field.h as i32 + 4, field.w, 20),
                );
            }
            s.control(Role::Button, "OK", r4(ok), Some(ID_OK))
                .states(States::DEFAULT);
            s.control(Role::Button, "Cancel", r4(cancel), Some(ID_CANCEL));
            s.end();
            return;
        }
        if let Some(r) = self.search() {
            s.control(
                Role::Edit,
                "Type a name, publisher, or PID to search",
                r4(r),
                Some(ID_SEARCH),
            )
            .states(States::EDITABLE)
            .actions(Actions::INVOKE | Actions::FOCUS | Actions::SET_VALUE)
            .value(self.search.text())
            .focused(self.searching && self.menu.is_none());
        }
        s.control(
            Role::Button,
            "Open Navigation",
            r4(HAMBURGER),
            Some(ID_HAMBURGER),
        );
        s.begin(
            Role::Navigation,
            "Navigation",
            (0, NAV_TOP, 48, self.height.saturating_sub(NAV_TOP as u32)),
        );
        for (i, name) in PAGE_NAMES.iter().enumerate() {
            if let Some(r) = self.nav_visible(i) {
                s.control(Role::ListItem, name, r4(r), Some(ID_NAV + i as u32))
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, self.page == i as u32)
                    .position(i, PAGE_NAMES.len());
            }
        }
        s.end();
        s.text(
            Role::Heading,
            PAGE_NAMES[self.page as usize],
            (
                self.content_x() + 24,
                HEADER_Y,
                text_w(300),
                header_band() as u32,
            ),
        );
        let commands = self.command_rects();
        if let Some((_, first, _)) = commands.first() {
            s.begin(
                Role::ToolBar,
                "Commands",
                (
                    first.x,
                    first.y,
                    self.width.saturating_sub(first.x as u32),
                    first.h,
                ),
            );
            for (c, r, _) in &commands {
                s.control(Role::Button, c.label, r4(*r), Some(c.id))
                    .state(States::DISABLED, !c.enabled)
                    .description(c.reason);
            }
            s.end();
        }
        match self.page {
            PERFORMANCE => {
                s.begin(
                    Role::List,
                    "Performance",
                    (
                        self.content_x(),
                        table_y(),
                        self.tiles_width(),
                        3 * (76 + tile_grow()) as u32,
                    ),
                );
                for i in 0..3 {
                    let (name, value) = self.resource_summary(i);
                    s.control(Role::ListItem, name, r4(self.tile(i)), Some(ID_TILE + i))
                        .states(States::SELECTABLE)
                        .value(&value)
                        .position(i as usize, 3);
                }
                s.end();
            }
            SETTINGS => {
                let [start, speed, on_top, grouping] = self.settings_controls();
                s.control(
                    Role::ComboBox,
                    "Default start page",
                    r4(start.1),
                    Some(start.0),
                )
                .states(States::EXPANDABLE | States::HAS_POPUP)
                .value(PAGE_NAMES[self.default_page as usize]);
                s.control(
                    Role::ComboBox,
                    "Real time update speed",
                    r4(speed.1),
                    Some(speed.0),
                )
                .states(States::EXPANDABLE | States::HAS_POPUP)
                .value(self.speed.label());
                s.control(
                    Role::CheckBox,
                    "Always on top",
                    r4(on_top.1),
                    Some(on_top.0),
                )
                .states(States::CHECKABLE | States::DISABLED)
                .description(ON_TOP_REASON);
                s.control(
                    Role::CheckBox,
                    "Group processes by type",
                    r4(grouping.1),
                    Some(grouping.0),
                )
                .states(States::CHECKABLE)
                .state(States::CHECKED, self.group_by_type)
                .actions(Actions::INVOKE | Actions::TOGGLE);
            }
            STARTUP => (),
            _ => {
                let t = self.table();
                s.begin(Role::List, PAGE_NAMES[self.page as usize], r4(t));
                for (i, c) in self.columns().iter().enumerate() {
                    s.control(
                        Role::Button,
                        c.label,
                        (c.x, t.y, c.w, self.header_height()),
                        Some(ID_COLUMN + i as u32),
                    );
                }
                let rows = self.rows();
                let view = self.list_viewport();
                let total = rows.len();
                let extents = row_extents(&rows);
                for (i, (row, (offset, pitch))) in
                    rows.iter().zip(extents).enumerate().take(ROW_LIMIT)
                {
                    let y = view.y + offset - self.scroll as i32;
                    let top = y.max(view.y);
                    let bottom = (y + pitch).min(view.y + view.h as i32);
                    if bottom <= top {
                        continue;
                    }
                    let name = match row {
                        Row::Group(g, n) => format!("{} ({n})", g.label()),
                        Row::User(uid, n) => format!("{} ({n})", model::user_name(*uid)),
                        Row::Process(p) => {
                            let id = model::identity(p);
                            String::from(if self.page == DETAILS {
                                id.image
                            } else {
                                id.name
                            })
                        }
                        Row::Service(_) | Row::Scm(_) => {
                            self.service_fields(row).map_or(String::new(), |f| f.name)
                        }
                    };
                    let selected = row.key().is_some() && row.key() == self.selected;
                    s.control(
                        Role::ListItem,
                        &name,
                        (view.x, top, view.w, (bottom - top) as u32),
                        Some(ID_ROW + i as u32),
                    )
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, selected)
                    .actions(Actions::INVOKE | Actions::SELECT)
                    .position(i, total)
                    .focused(selected && self.menu.is_none() && !self.searching);
                }
                s.end();
            }
        }
        if let Some((card, entries)) = self.menu_layout() {
            s.begin(Role::Menu, "", r4(card));
            let items = self.menu_entries();
            let count = items
                .iter()
                .filter(|e| matches!(e, MenuEntry::Item(..)))
                .count();
            let mut k = 0;
            for (i, (e, r)) in items.iter().zip(entries).enumerate() {
                if let MenuEntry::Item(label, keys) = e {
                    s.control(Role::MenuItem, label, r4(r), Some(ID_MENU + i as u32))
                        .accelerator(keys.unwrap_or(""))
                        .position(k, count)
                        .focused(k == 0);
                    k += 1;
                }
            }
            s.end();
        }
    }
}
