//! Settings' semantic tree for door.accessibility, described from the same
//! geometry `Settings::target` hit-tests: Back, the search box and its
//! suggestions, the navigation rail, the breadcrumb heading, every card item
//! of the page layout (`settings_catalog::PageLayout`) including run-time
//! list entries and InfoBars, the open dialog, and the open combo list or
//! menu. Keyboard focus follows the app's own model: the dialog's control,
//! the search box, the open popup's highlighted option, the focused card
//! item, or else the selected rail item (arrow keys move the rail then, as
//! in Windows).
use super::*;
use door_workspace::semantics::{Actions, Role, Semantics, States};

fn rect(r: catalog::Rect, scroll: i32) -> (i32, i32, u32, u32) {
    (r.x, r.y - scroll, r.w, r.h)
}
/// Title and description of the card row an item belongs to.
fn row_text(
    layout: &catalog::PageLayout,
    item: &catalog::Item,
) -> Option<(&'static str, &'static str)> {
    if let Action::Expand(id) = item.action {
        return layout.blocks.iter().find_map(|p| match p.block {
            catalog::Block::Expander(e) if e.id == id => {
                Some((e.header.title, e.header.description))
            }
            _ => None,
        });
    }
    let (cx, cy) = (
        item.rect.x + item.rect.w as i32 / 2,
        item.rect.y + item.rect.h as i32 / 2,
    );
    layout
        .blocks
        .iter()
        .flat_map(|p| p.rows.iter())
        .find(|(_, r)| r.contains(cx, cy))
        .map(|(row, _)| (row.title, row.description))
}
/// The title of the setting an item changes (event log, accessible name).
pub(super) fn row_title(
    layout: &catalog::PageLayout,
    item: &catalog::Item,
) -> Option<&'static str> {
    row_text(layout, item)
        .map(|(title, _)| title)
        .filter(|t| !t.is_empty())
}
impl Settings {
    pub fn semantics(&self, s: &mut Semantics) {
        let state = self.state();
        let popup_open = self.popup.is_some();
        let modal = self.dialog.is_some();
        s.control(Role::Button, "Back", self.layout.back(), Some(BACK))
            .state(States::DISABLED, self.history.is_empty() || modal);
        if let Some(b) = self.layout.search() {
            s.control(
                Role::Edit,
                "Find a setting",
                (b.x, b.y, b.w, b.h),
                Some(SEARCH),
            )
            .states(States::EDITABLE)
            .actions(Actions::FOCUS | Actions::SET_VALUE)
            .value(&self.query)
            .focused(self.search_focus && !popup_open && !modal);
            if self.suggestions_open()
                && let Some((sx, sy, sw, pitch)) = navigation::suggestion_box(self.layout)
            {
                let hits: Vec<Hit> = self.suggestions().collect();
                s.begin(
                    Role::List,
                    "Suggestions",
                    (sx, sy, sw, (hits.len().max(1) as i32 * pitch + 8) as u32),
                );
                for (i, hit) in hits.iter().enumerate() {
                    let (title, secondary) = navigation::suggestion_text(*hit);
                    s.control(
                        Role::ListItem,
                        title,
                        (sx, sy + 4 + i as i32 * pitch, sw, pitch as u32),
                        Some(SUGGESTION + i as u32),
                    )
                    .description(secondary)
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, i == self.suggestion)
                    .position(i, hits.len());
                }
                s.end();
            }
        }
        // Navigation rail (selection follows the page, as WinUI NavigationView).
        let rail = self.rail();
        let items: Vec<Page> = self.items().collect();
        let selected = self.page.category();
        let nothing_focused = !self.search_focus && !popup_open && !modal && self.focus.is_none();
        s.begin(
            Role::Navigation,
            "Navigation",
            (
                Layout::NAV_X,
                rail.top,
                Layout::NAV_W,
                (rail.bottom() - rail.top).max(0) as u32,
            ),
        );
        for (row, page) in items.iter().enumerate() {
            let y = rail.row(row);
            let r = (Layout::NAV_X, y, Layout::NAV_W, Layout::nav_row() as u32);
            let hidden = y + Layout::nav_row() <= rail.top || y >= rail.bottom();
            let current = *page == selected;
            s.control(
                Role::ListItem,
                page.nav_title(),
                r,
                Some(100 + *page as u32),
            )
            .states(States::SELECTABLE)
            .state(States::SELECTED, current)
            .state(States::OFFSCREEN, hidden)
            .position(row, items.len())
            .focused(nothing_focused && current);
        }
        s.end();
        // Breadcrumb: each ancestor is its own link; the heading is the
        // current page's title alone (Windows: "Storage, link", "Disks &
        // volumes, heading").
        let mut heading_x = Layout::MAIN;
        let (page, crumbs) = self.heading_hit.get();
        if page == Some(self.page) {
            for (i, crumb) in crumbs.iter().enumerate() {
                if let Some((p, hx, hw)) = crumb {
                    s.control(
                        Role::Hyperlink,
                        p.nav_title(),
                        (*hx, HEADING_Y, *hw, super::heading_height() as u32),
                        Some(ANCESTOR + i as u32),
                    );
                    heading_x =
                        hx + *hw as i32 + 2 * navigation::HEADING_GAP + navigation::HEADING_CHEVRON;
                }
            }
        }
        s.text(
            Role::Heading,
            self.page.nav_title(),
            (
                heading_x,
                HEADING_Y,
                self.layout
                    .main_width()
                    .saturating_sub((heading_x - Layout::MAIN).max(0) as u32),
                super::heading_height() as u32,
            ),
        );
        let layout = self.page_layout();
        let scroll = self.scroll.value as i32;
        let count = layout.items.len();
        s.begin(
            Role::Pane,
            self.page.nav_title(),
            (
                Layout::MAIN,
                Layout::content_top(),
                self.layout.main_width(),
                self.layout
                    .height
                    .saturating_sub(Layout::content_top() as u32),
            ),
        );
        // InfoBars and list entries without an action are read as text.
        for placed in &layout.blocks {
            match placed.block {
                catalog::Block::Info(text) => {
                    s.text(Role::InfoBar, text, rect(placed.rect, scroll));
                }
                catalog::Block::List(list) if list.entry_action().is_none() => {
                    for index in 0..self.admin.count(list) {
                        if let Some((title, description, _)) = self.admin.entry(list, index) {
                            s.text(
                                Role::Text,
                                &format!("{title}, {description}"),
                                rect(catalog::list_entry(placed.rect, index), scroll),
                            );
                        }
                    }
                }
                _ => (),
            }
        }
        for (index, item) in layout.items.iter().enumerate() {
            let r = rect(item.rect, scroll);
            let (title, description) = row_text(&layout, item).unwrap_or(("", ""));
            let open_popup = matches!((self.popup, item.action), (Some((_, PopupKind::Choice(c))), Action::Combo(d)) if c == d);
            let focused = !self.search_focus && !popup_open && !modal && self.focus == Some(index);
            let target = Some(ITEM + index as u32);
            let control = match item.action {
                Action::Navigate(page) => {
                    let name = if title.is_empty() {
                        page.nav_title()
                    } else {
                        title
                    };
                    s.control(Role::Button, name, r, target)
                        .description(description)
                }
                Action::Toggle(t) => s
                    .control(Role::ToggleSwitch, title, r, target)
                    .states(States::CHECKABLE)
                    .state(States::CHECKED, t.value(state))
                    .state(States::DISABLED, !t.enabled())
                    .actions(Actions::INVOKE | Actions::TOGGLE)
                    .description(description),
                Action::Combo(c) => s
                    .control(Role::ComboBox, title, r, target)
                    .states(States::EXPANDABLE | States::HAS_POPUP)
                    .state(States::EXPANDED, open_popup)
                    .state(States::DISABLED, !c.enabled())
                    .actions(Actions::INVOKE | Actions::EXPAND)
                    .value(c.label(c.current(state), state))
                    .description(description),
                Action::Expand(id) => s
                    .control(Role::Expander, title, r, target)
                    .states(States::EXPANDABLE)
                    .state(States::EXPANDED, id < 64 && self.open & (1u64 << id) != 0)
                    .actions(Actions::INVOKE | Actions::EXPAND | Actions::COLLAPSE)
                    .description(description),
                Action::Slider(_) => s
                    .control(
                        Role::Slider,
                        if title.is_empty() { "Text size" } else { title },
                        r,
                        target,
                    )
                    .states(States::HORIZONTAL)
                    .actions(Actions::NONE)
                    .range(
                        i64::from(catalog::TEXT_MIN),
                        i64::from(catalog::TEXT_MAX),
                        i64::from(self.text_pick),
                    ),
                Action::Theme(i) => s
                    .control(Role::ListItem, &format!("Theme {}", i + 1), r, target)
                    .states(States::SELECTABLE),
                Action::Background(i) => s
                    .control(Role::ListItem, &format!("Background {}", i + 1), r, target)
                    .states(States::SELECTABLE),
                Action::SolidColor(i) => s
                    .control(
                        Role::ListItem,
                        &format!(
                            "Background color {} of {}",
                            i + 1,
                            door_workspace::settings_catalog::SOLID_PALETTE.len()
                        ),
                        r,
                        target,
                    )
                    .states(States::SELECTABLE),
                Action::Accent(i) => s
                    .control(
                        Role::ListItem,
                        door_design_tokens::accent::WINDOWS_COLOR_NAMES[i],
                        r,
                        target,
                    )
                    .states(States::SELECTABLE),
                Action::RecentAccent(_) => s
                    .control(Role::ListItem, "Recent color", r, target)
                    .states(States::SELECTABLE),
                Action::Command(command) => {
                    // A card button is named by its label and described by
                    // the card's title ("Go to, Go to a key").
                    let label = layout
                        .blocks
                        .iter()
                        .flat_map(|p| p.rows.iter())
                        .find_map(|(row, _)| match row.control {
                            catalog::Control::Command(label, c) if c == command => Some(label),
                            _ => None,
                        })
                        .unwrap_or(command.label());
                    s.control(Role::Button, label, r, target)
                        .state(States::DISABLED, !self.command_enabled(command))
                        .description(title)
                }
                Action::AppMenu(_) | Action::KeyboardMenu(_) => s
                    .control(Role::Button, "More options", r, target)
                    .states(States::HAS_POPUP)
                    .description(title),
                Action::KeyboardAdd => s
                    .control(Role::Button, "Add a keyboard", r, target)
                    .states(States::HAS_POPUP),
                Action::ListOpen(list, entry) => {
                    let (name, description, _) = self.admin.entry(list, entry).unwrap_or_default();
                    // A VPN connection's Advanced options button.
                    let label = if list == List::VpnConnections {
                        "Advanced options"
                    } else {
                        name.as_str()
                    };
                    s.control(Role::Button, label, r, target)
                        .description(&if list == List::VpnConnections {
                            name.clone()
                        } else {
                            description
                        })
                        .position(entry, self.admin.count(list))
                }
                Action::ListEdit(list, entry) => {
                    let (name, description, _) = self.admin.entry(list, entry).unwrap_or_default();
                    let up = self.admin.vpns.get(entry).is_some_and(|v| v.2);
                    let label = match list {
                        List::VpnConnections if up => "Disconnect",
                        List::VpnConnections => "Connect",
                        List::FileTypes => "Choose",
                        _ => "Edit",
                    };
                    s.control(Role::Button, label, r, target)
                        .description(&format!("{name}, {description}"))
                        .position(entry, self.admin.count(list))
                }
                Action::ListRemove(list, entry) => {
                    let (name, _, _) = self.admin.entry(list, entry).unwrap_or_default();
                    s.control(Role::Button, "Remove", r, target)
                        .description(&name)
                        .position(entry, self.admin.count(list))
                }
            };
            control
                .state(
                    States::OFFSCREEN,
                    r.1 + r.3 as i32 <= Layout::content_top() || r.1 >= self.layout.height as i32,
                )
                .state(States::DISABLED, modal)
                .position(index, count)
                .focused(focused);
        }
        s.end();
        // The open dialog: its fields, the type combo and its buttons.
        if let Some(d) = &self.dialog {
            let g = d.geometry(self.layout.width, self.layout.height);
            s.begin(
                Role::Dialog,
                d.title,
                (g.rect.x, g.rect.y, g.rect.w, g.rect.h),
            );
            for (i, label) in d.kind.fields().iter().enumerate() {
                if i == 1 && d.kind.has_type() {
                    let k = g.kind;
                    s.control(
                        Role::ComboBox,
                        "Type",
                        (k.x, k.y, k.w, k.h),
                        Some(dialog::TYPE),
                    )
                    .states(States::EXPANDABLE | States::HAS_POPUP)
                    .state(States::EXPANDED, popup_open)
                    .actions(Actions::INVOKE | Actions::EXPAND)
                    .value(door_workspace::registry_value::label(d.value_kind))
                    .focused(!popup_open && d.focus == dialog::Focus::Type);
                }
                let f = g.fields[i];
                let fixed = i == 0 && d.fixed_name;
                s.control(
                    Role::Edit,
                    label,
                    (f.x, f.y, f.w, f.h),
                    Some(dialog::FIELD + i as u32),
                )
                .states(if fixed {
                    States::READ_ONLY
                } else {
                    States::EDITABLE
                })
                .actions(Actions::FOCUS | Actions::SET_VALUE)
                .value(d.fields[i].text())
                .focused(!popup_open && d.focus == dialog::Focus::Field(i));
            }
            if !d.error.is_empty() {
                s.text(
                    Role::Status,
                    &d.error,
                    (g.rect.x + 24, g.error, g.rect.w - 48, 20),
                );
            }
            for (i, label) in d.kind.buttons().iter().enumerate() {
                let b = g.buttons[i];
                s.control(
                    Role::Button,
                    label,
                    (b.x, b.y, b.w, b.h),
                    Some(dialog::BUTTON + i as u32),
                )
                .focused(!popup_open && d.focus == dialog::Focus::Button(i));
            }
            s.end();
        }
        // The open combo list or menu: its highlighted option has focus.
        if let Some((popup, kind)) = &self.popup {
            let owned: Vec<String> = match kind {
                PopupKind::DefaultApp(i) => self
                    .admin
                    .file_type_choices(*i)
                    .into_iter()
                    .map(|(_, name)| name)
                    .collect(),
                _ => Vec::new(),
            };
            let labels: Vec<&str> = match kind {
                PopupKind::DefaultApp(_) => owned.iter().map(String::as_str).collect(),
                PopupKind::Choice(choice) => (0..choice.count(state))
                    .map(|i| choice.label(i, state))
                    .collect(),
                PopupKind::AppMenu(app) => self.app_menu_labels(*app),
                PopupKind::ConfirmUninstall(_) => pages::CONFIRM_UNINSTALL.to_vec(),
                PopupKind::KeyboardMenu(_) => catalog::KEYBOARD_MENU.to_vec(),
                PopupKind::AddKeyboard => catalog::addable(state)
                    .map(|m| catalog::input_label(m.id))
                    .collect(),
                PopupKind::DialogType => dialog::Dialog::type_labels(),
            };
            let (role, item) = match kind {
                PopupKind::Choice(_) | PopupKind::DialogType => (Role::List, Role::ListItem),
                _ => (Role::Menu, Role::MenuItem),
            };
            let current = match kind {
                PopupKind::Choice(choice) => Some(choice.current(state)),
                PopupKind::DialogType => self.dialog.as_ref().map(dialog::Dialog::type_index),
                _ => None,
            };
            s.begin(
                role,
                "",
                (popup.rect.x, popup.rect.y, popup.rect.w, popup.rect.h),
            );
            for (i, label) in labels.iter().enumerate().take(popup.count) {
                let o = popup.option(i);
                let c = s
                    .control(
                        item,
                        label,
                        (o.x, o.y, o.w, o.h),
                        Some(POPUP_OPTION + i as u32),
                    )
                    .state(States::DISABLED, !popup.enabled(i))
                    .position(i, popup.count.min(labels.len()))
                    .focused(i == popup.selected);
                if item == Role::ListItem {
                    c.states(States::SELECTABLE)
                        .state(States::SELECTED, current == Some(i));
                }
            }
            s.end();
        }
    }
}
