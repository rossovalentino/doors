//! Preview's semantic tree for door.accessibility. Every control comes from
//! `Preview::actions`, the list hit testing searches, so the tree names and
//! places exactly what a pointer can reach: caption tabs, the navigation
//! pane, the toolbar, library items, page buttons, the info panel and the
//! crop or notice controls.
use super::*;
use door_app_ui::fluent::Rect as R;
use door_workspace::semantics::{Actions, Role, Semantics, States};

fn r4(r: R) -> (i32, i32, u32, u32) {
    (r.x, r.y, r.w, r.h)
}
fn file_name(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or(path)
}
impl Preview {
    /// The toolbar and other command names (by target id).
    fn command_name(&self, id: u32, r: R) -> &'static str {
        match id {
            200 => "Back",
            201 => "Forward",
            2 => "Zoom out",
            3 if r.w > 36 => "Zoom level",
            3 => "Fit to window",
            4 => "Zoom in",
            5 => "Rotate",
            205 => "Pan",
            206 => "Crop",
            6 => "Convert",
            207 => "Edit",
            7 => "Save as",
            1 if r.y >= 48 => "Open file",
            1 => "New tab",
            209 => "Version history",
            202 => "File information",
            203 => "Copy",
            204 => "Print",
            9 => "Close",
            210 => "Apply",
            211 => "Cancel",
            920 => "Previous",
            921 => "Next",
            922 => "Close",
            422 => "Previous page",
            423 => "Next page",
            _ => "",
        }
    }
    pub(crate) fn describe(&self, s: &mut Semantics) {
        let actions = self.actions();
        let tab_name = |i: usize| -> String {
            self.tabs
                .get(i)
                .map_or(String::from("Home"), |t| String::from(file_name(&t.path)))
        };
        // Caption tabs (y above the toolbar band).
        let tabs: Vec<(u32, R)> = actions
            .iter()
            .copied()
            .filter(|(id, r)| (100..400).contains(id) && r.y < 48)
            .collect();
        if let Some((_, first)) = tabs.first() {
            s.begin(
                Role::TabList,
                "Tabs",
                (first.x, first.y, self.width, first.h),
            );
            let count = self.tabs.len().max(1);
            for (id, r) in &tabs {
                if *id < 300 {
                    let i = (*id - 100) as usize;
                    s.control(Role::Tab, &tab_name(i), r4(*r), Some(*id))
                        .states(States::SELECTABLE)
                        .state(States::SELECTED, i == self.active)
                        .actions(Actions::INVOKE | Actions::SELECT)
                        .position(i, count);
                } else {
                    s.control(Role::Button, "Close tab", r4(*r), Some(*id));
                }
            }
            s.end();
        }
        // Navigation pane.
        let nav: Vec<(u32, R)> = actions
            .iter()
            .copied()
            .filter(|(id, _)| (400..426).contains(id))
            .collect();
        if let Some((_, first)) = nav.first() {
            s.begin(
                Role::Navigation,
                "Navigation",
                (
                    first.x,
                    first.y,
                    first.w,
                    self.height.saturating_sub(first.y as u32),
                ),
            );
            for (id, r) in &nav {
                if *id >= 424 {
                    let g = (*id - 424) as usize;
                    if let Some((label, ..)) = presentation::GROUPS.get(g) {
                        s.control(Role::ListItem, label, r4(*r), Some(*id))
                            .states(States::EXPANDABLE);
                    }
                    continue;
                }
                let i = (*id - 400) as usize;
                let label = presentation::NAV.get(i).map_or("Settings", |n| n.0);
                s.control(Role::ListItem, label, r4(*r), Some(*id))
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, self.section as usize == i);
            }
            s.end();
        }
        // Information pane: its heading and the visible rows (paint layout).
        if let (Some(p), Some(t)) = (self.panel(), self.selected()) {
            let info = self.info_layout(p);
            s.text(Role::Heading, "Information", r4(info.heading));
            let rows = self.info_rows(t);
            let page = self.info_page.min(self.info_pages() - 1);
            for (i, (name, value)) in rows
                .iter()
                .skip(page * info.count)
                .take(info.count)
                .enumerate()
            {
                let y = info.rows_top + i as i32 * info.pitch;
                s.text(
                    Role::Text,
                    &format!("{name}: {value}"),
                    (p.x + 20, y, p.w.saturating_sub(40), info.row_h),
                );
            }
        }
        // Everything else, in hit-test order.
        let library = self.library_paths();
        for (id, r) in actions {
            if (100..426).contains(&id) && (r.y < 48 || id >= 400) || id == 426 {
                continue;
            }
            if (100..300).contains(&id) {
                let i = (id - 100) as usize;
                s.control(Role::ListItem, &tab_name(i), r4(r), Some(id))
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, i == self.active);
                continue;
            }
            if (600..700).contains(&id) {
                let name = library
                    .get(self.section_scroll + (id - 600) as usize)
                    .map_or("", |p| file_name(p));
                s.control(Role::ListItem, name, r4(r), Some(id));
                continue;
            }
            let name = self.command_name(id, r);
            s.control(Role::Button, name, r4(r), Some(id));
        }
    }
}
