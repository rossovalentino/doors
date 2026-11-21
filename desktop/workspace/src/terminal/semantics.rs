//! Terminal's semantic tree for door.accessibility, from the geometry
//! `Terminal::target` hit-tests: the caption tab strip (one tab per
//! session, its close button, New tab) and, on the console page, the text
//! area as a terminal document whose value is the last output lines and the
//! prompt (Windows Terminal exposes its TermControl the same way). Focus is
//! on the console's input whenever the console page shows.
use super::*;
use door_workspace::semantics::{Actions, Role, Semantics, States};
use door_workspace::terminal_model::{CONTENT_TOP, Page};

fn b(r: door_workspace::explorer_layout::Bounds) -> (i32, i32, u32, u32) {
    (r.x, r.y, r.width, r.height)
}
impl Terminal {
    pub fn semantics(&self, s: &mut Semantics) {
        let (count, active) = (self.state.tabs.len(), self.state.active);
        let strip = self.layout.strip(count, active);
        let range = strip.range();
        if let Some(first) = strip.tab(range.start) {
            s.begin(
                Role::TabList,
                "Tabs",
                (first.x, first.y, self.layout.width, first.height),
            );
            for i in range.clone() {
                let Some(r) = strip.tab(i) else { continue };
                s.control(
                    Role::Tab,
                    &self.state.tabs[i].title,
                    b(r),
                    Some(super::TAB + i as u32),
                )
                .states(States::SELECTABLE)
                .state(States::SELECTED, i == active)
                .actions(Actions::INVOKE | Actions::SELECT)
                .position(i, count);
                if let Some(c) = strip.close(i) {
                    s.control(
                        Role::Button,
                        "Close tab",
                        b(c),
                        Some(super::TAB_CLOSE + i as u32),
                    );
                }
            }
            s.end();
        }
        if let Some((plus, profiles)) = self.new_tab_buttons() {
            s.control(Role::Button, "New tab", b(plus), Some(30));
            if let Some(d) = profiles {
                s.control(Role::Button, "Open a profile", b(d), Some(20));
            }
        }
        // Navigation rail: the rows `Terminal::target` answers for.
        let rail = self.layout.rail();
        let rows = if rail > 0 {
            super::rail_rows().len()
        } else {
            0
        };
        let pitch = super::rail_pitch() as u32;
        s.begin(
            Role::List,
            "Navigation",
            (
                0,
                CAPTION as i32,
                rail,
                self.layout.height.saturating_sub(CAPTION),
            ),
        );
        for (id, top) in super::rail_rows().into_iter().take(rows) {
            let selected = matches!(
                (id, self.state.page),
                (1, Page::Home) | (2, Page::Settings) | (3, Page::About) | (20, Page::Profiles)
            ) || (self.state.page == Page::Console
                && (10..=13).contains(&id)
                && self.state.tabs[self.state.active].profile == Profile::ALL[(id - 10) as usize]);
            s.control(
                Role::ListItem,
                super::rail_label(id),
                (0, top, rail, pitch),
                Some(id),
            )
            .states(States::SELECTABLE)
            .state(States::SELECTED, selected)
            .actions(Actions::INVOKE | Actions::SELECT);
        }
        s.end();
        let left = self.layout.rail() as i32;
        let area = (
            left,
            CONTENT_TOP,
            self.layout.width.saturating_sub(left as u32),
            self.layout.height.saturating_sub(CONTENT_TOP as u32),
        );
        if self.state.page != Page::Console {
            let name = match self.state.page {
                Page::Settings => "Settings",
                Page::About => "About",
                Page::Profiles => "Profiles",
                Page::Home => "Home",
                Page::Console => "",
            };
            s.text(
                Role::Heading,
                name,
                (
                    left,
                    CONTENT_TOP,
                    area.2,
                    40 + super::grow_by(TextStyle::Semibold28) as u32,
                ),
            );
            if self.state.page == Page::Home {
                for (id, r) in self.home_layout().targets {
                    let name = match id {
                        50..=53 => {
                            alloc::format!("New {} tab", Profile::ALL[(id - 50) as usize].name())
                        }
                        super::SESSION..=super::SESSION_LAST => alloc::format!(
                            "Switch to {}",
                            self.state.tabs[(id - super::SESSION) as usize].title
                        ),
                        super::SESSION_CLOSE..=super::SESSION_CLOSE_LAST => alloc::format!(
                            "Close {}",
                            self.state.tabs[(id - super::SESSION_CLOSE) as usize].title
                        ),
                        _ => alloc::format!(
                            "Open Doorsh in {}",
                            self.shown_path(&self.recent[(id - 80) as usize])
                        ),
                    };
                    s.control(Role::Button, &name, b(r), Some(id));
                }
            }
            if self.state.page == Page::Settings {
                for (id, top, height) in super::settings_cards() {
                    let name = match id {
                        40 => "Color scheme",
                        41 => "Text size",
                        42 => "Command history",
                        _ => "Background opacity",
                    };
                    let x = left + 28;
                    s.control(
                        Role::Button,
                        name,
                        (
                            x,
                            top,
                            self.layout.width.saturating_sub(left as u32 + 56),
                            height as u32,
                        ),
                        Some(id),
                    );
                }
                let (_, save) = super::settings_footer(self.layout.height);
                let x = self.layout.width as i32 - 148;
                s.control(
                    Role::Button,
                    "Save",
                    (x, save.y, save.width, save.height),
                    Some(43),
                );
            }
            return;
        }
        let right = self.layout.width as i32;
        for (offset, id, name) in [
            (
                204,
                38,
                if self.fullscreen {
                    "Exit full screen"
                } else {
                    "Full screen"
                },
            ),
            (164, 31, "Find"),
            (
                124,
                36,
                if self.layout.rail_hidden {
                    "Show navigation pane"
                } else {
                    "Hide navigation pane"
                },
            ),
            (84, 32, "Clear"),
            (44, 37, "More options"),
        ] {
            s.control(Role::Button, name, (right - offset, 68, 36, 30), Some(id));
        }
        let tab = &self.state.tabs[active];
        // The newest lines (at most 24) and the prompt line being typed.
        let start = tab.lines.len().saturating_sub(24);
        let mut text = String::new();
        for line in tab.lines.iter().skip(start) {
            text.push_str(line.text());
            text.push('\n');
        }
        text.push_str(tab.input.text());
        // Keep the newest 2000 bytes (the value limit cuts from the end).
        if text.len() > 2000 {
            let mut cut = text.len() - 2000;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text = String::from(&text[cut..]);
        }
        s.control(Role::Document, "Terminal", area, None)
            .states(States::FOCUSABLE | States::READ_ONLY | States::MULTILINE)
            .actions(Actions::FOCUS)
            .value(&text)
            .focused(!self.search_focus && !self.actions_open);
    }
}
