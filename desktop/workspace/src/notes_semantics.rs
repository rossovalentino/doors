//! Notes' semantic tree for door.accessibility, described from the geometry
//! `Notes::target` hit-tests: the document tabs, the menu bar and its open
//! menu, the formatting bar, the document (an edit with its leading text and
//! selection) and the save prompt. Focus follows the app: the prompt's
//! focused button, the open menu's selected command, else the document.
use super::*;
use door_workspace::semantics::{self as sem, Actions, Role, Semantics, States};

fn b(r: door_workspace::explorer_layout::Bounds) -> (i32, i32, u32, u32) {
    (r.x, r.y, r.width, r.height)
}
impl Notes {
    pub fn semantics(&self, s: &mut Semantics) {
        if self.modal() {
            let body = if !self.current.prompt_error.is_empty() {
                self.current.prompt_error.clone()
            } else {
                String::from("Your changes will be lost from this window if you don't save.")
            };
            sem::dialog(
                s,
                &self.dialog_layout(),
                &self.current.prompt.dialog,
                &format!("Save changes to {}?", self.file_name()),
                &body,
                &["Save", "Don't save", "Cancel"],
                3000,
            );
            return;
        }
        let strip = self.chrome();
        let range = strip.range();
        if let Some(first) = strip.tab(range.start) {
            s.begin(
                Role::TabList,
                "Tabs",
                (first.x, first.y, self.caption_width(), first.height),
            );
            for i in range.clone() {
                let Some(r) = strip.tab(i) else { continue };
                let name: String = if i == self.active {
                    String::from(self.file_name())
                } else {
                    self.tabs
                        .get(i)
                        .and_then(|t| t.as_ref())
                        .map_or(String::from("Untitled"), |d| {
                            String::from(d.path.rsplit('/').next().unwrap_or("Untitled"))
                        })
                };
                s.control(Role::Tab, &name, b(r), Some(500 + i as u32))
                    .states(States::SELECTABLE)
                    .state(States::SELECTED, i == self.active)
                    .actions(Actions::INVOKE | Actions::SELECT)
                    .position(i, self.tabs.len());
                if let Some(c) = strip.close(i) {
                    s.control(Role::Button, "Close tab", b(c), Some(600 + i as u32));
                }
            }
            s.end();
        }
        if let Some(p) = strip.plus() {
            s.control(Role::Button, "Add new tab", b(p), Some(10));
        }
        let bar = self.menu_bar();
        s.begin(
            Role::MenuBar,
            "Application",
            (
                bar[0].x,
                bar[0].y,
                (bar[2].x + bar[2].width as i32 - bar[0].x).max(0) as u32,
                bar[0].height,
            ),
        );
        for (i, (r, id)) in bar.iter().zip([1u32, 40, 41]).enumerate() {
            s.control(Role::MenuItem, MENU_LABELS[i], b(*r), Some(id))
                .states(States::HAS_POPUP | States::EXPANDABLE)
                .state(
                    States::EXPANDED,
                    self.menu.open() && usize::from(self.menu_kind) == i,
                )
                .actions(Actions::INVOKE | Actions::EXPAND);
        }
        s.end();
        let tools: Vec<(usize, door_workspace::explorer_layout::Bounds)> = Tool::ALL
            .iter()
            .enumerate()
            .filter_map(|(i, t)| strip.tool(*t).map(|r| (i, r)))
            .collect();
        if let (Some((_, first)), Some((_, last))) = (tools.first(), tools.last()) {
            let width = (last.x + last.width as i32 - first.x) as u32;
            s.begin(
                Role::ToolBar,
                "Formatting",
                (first.x, first.y, width, first.height),
            );
            for &(i, r) in &tools {
                let tool = Tool::ALL[i];
                let control = s.control(Role::Button, tool.name(), b(r), Some(50 + i as u32));
                if tool.dropdown() {
                    control
                        .states(States::HAS_POPUP | States::EXPANDABLE)
                        .actions(Actions::INVOKE | Actions::EXPAND);
                }
            }
            s.end();
        }
        s.control(Role::Button, "What's new", b(strip.whats_new()), Some(42));
        s.control(Role::Button, "Settings", b(strip.settings()), Some(43));
        // The document: an edit whose value is its leading text.
        let doc = &self.current.document;
        let mut end = doc.len().min(2048);
        while end > 0 && end < doc.len() && doc.byte(end).is_some_and(|c| c & 0xc0 == 0x80) {
            end -= 1;
        }
        let text = doc.text(0..end);
        let sel = doc.selection();
        let menu = self.menu.open();
        let edit = s
            .control(
                Role::Edit,
                "Text editor",
                b(strip.body(self.status_bar)),
                None,
            )
            .states(States::FOCUSABLE | States::EDITABLE | States::MULTILINE)
            .actions(Actions::FOCUS)
            .value(&text)
            .state(States::VALUE_TRUNCATED, end < doc.len())
            .focused(!menu);
        if sel.end <= end {
            edit.selection(doc.anchor(), doc.caret());
        }
        if let Some(toggle) = strip.format_toggle().filter(|_| self.status_bar) {
            let label = if self.markdown {
                "Formatted"
            } else {
                "Plain text"
            };
            s.control(Role::Button, label, b(toggle), Some(44));
        }
        if self.status_bar && !self.current.message.is_empty() {
            s.text(
                Role::Status,
                &self.current.message,
                (0, self.size.1 as i32 - 32, self.size.0, 32),
            );
        }
        if menu {
            sem::popup_menu(
                s,
                &self.menu,
                self.menu_items(),
                match self.menu_kind {
                    3 => "Headings",
                    4 => "Lists",
                    5 => "Table",
                    kind => MENU_LABELS.get(usize::from(kind)).copied().unwrap_or(""),
                },
            );
        }
    }
}
