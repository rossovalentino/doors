//! Contents of the shell's menus. Only actions with a working backend are
//! enabled; everything else Windows shows is present but disabled.
extern crate alloc;
use crate::apps::App;
use crate::menu::{Check, Command, Entry, Icon, Item, Menu, Style};
use crate::search::page;
use alloc::vec;
pub const DESKTOP: u32 = 1;
pub const TASKBAR: u32 = 2;
pub const JUMP_LIST: u32 = 3;
pub const POWER: u32 = 4;
pub const USER: u32 = 5;
pub const START_VIEW: u32 = 6;
pub const START_ITEM: u32 = 7;
pub const QUICK_LINK: u32 = 8;
pub const SUB_VIEW: u8 = 1;
pub const SUB_SORT: u8 = 2;
pub const SUB_POWER: u8 = 3;
fn item(label: &str, command: Command) -> Entry {
    Entry::Item(Item::new(label, command))
}
fn with(entry: Item) -> Entry {
    Entry::Item(entry)
}
/// The desktop menu under the classic-menu setting: Windows 10's menu.
pub fn desktop_win10() -> Menu {
    let mut menu = desktop(true, true);
    menu.style = Style::Win10;
    menu
}
/// Windows 11 desktop context menu (or the classic one). `more_options`
/// is Personalization > Context menu > Show more options (26H2).
pub fn desktop(classic: bool, more_options: bool) -> Menu {
    if classic {
        return Menu::new(
            vec![
                item("View", Command::Submenu(SUB_VIEW)),
                item("Sort by", Command::Submenu(SUB_SORT)),
                item("Refresh", Command::Refresh),
                Entry::Separator,
                with(Item::new("Paste", Command::Nothing).disabled()),
                with(Item::new("Paste shortcut", Command::Nothing).disabled()),
                Entry::Separator,
                with(Item::new("New", Command::Nothing).disabled()),
                Entry::Separator,
                with(
                    Item::new("Display settings", Command::Settings(page::DISPLAY))
                        .glyph(crate::shell_mask!("shell-display", 16)),
                ),
                with(
                    Item::new("Personalize", Command::Settings(page::BACKGROUND))
                        .glyph(crate::shell_mask!("shell-personalize", 16)),
                ),
            ],
            Style::Classic,
            DESKTOP,
        )
        .min_width(220);
    }
    let mut entries = vec![
        with(
            Item::new("View", Command::Submenu(SUB_VIEW))
                .glyph(crate::shell_mask!("shell-view", 16)),
        ),
        with(
            Item::new("Sort by", Command::Submenu(SUB_SORT))
                .glyph(crate::shell_mask!("shell-sort", 16)),
        ),
        with(Item::new("Refresh", Command::Refresh).glyph(crate::shell_mask!("shell-refresh", 16))),
        Entry::Separator,
        with(
            Item::new("New", Command::Nothing)
                .glyph(crate::shell_mask!("shell-new", 16))
                .disabled(),
        ),
        Entry::Separator,
        with(
            Item::new("Display settings", Command::Settings(page::DISPLAY))
                .glyph(crate::shell_mask!("shell-display", 16)),
        ),
        with(
            Item::new("Personalize", Command::Settings(page::BACKGROUND))
                .glyph(crate::shell_mask!("shell-personalize", 16)),
        ),
        Entry::Separator,
        with(
            Item::new("Open in Terminal", Command::Launch(App::Terminal))
                .icon(Icon::App(App::Terminal)),
        ),
    ];
    if more_options {
        entries.push(Entry::Separator);
        entries.push(with(
            Item::new("Show more options", Command::ShowMoreOptions)
                .glyph(crate::shell_mask!("shell-more-options", 16))
                .detail("Shift+F10"),
        ));
    }
    Menu::new(entries, Style::Fluent, DESKTOP).min_width(260)
}
/// `Command::RecycleBin` actions.
pub const BIN_OPEN: u8 = 0;
pub const BIN_EMPTY: u8 = 1;
pub const BIN_PROPERTIES: u8 = 2;
/// The desktop Recycle Bin's context menu (Windows 11): Open, Empty Recycle
/// Bin (disabled while the bin is empty) and Properties. File Explorer
/// carries each out (it asks before emptying).
pub fn recycle_bin(full: bool) -> Menu {
    let empty = Item::new("Empty Recycle Bin", Command::RecycleBin(BIN_EMPTY))
        .glyph(crate::shell_mask!("explorer-delete", 16));
    Menu::new(
        vec![
            with(Item::new("Open", Command::RecycleBin(BIN_OPEN))),
            with(if full { empty } else { empty.disabled() }),
            Entry::Separator,
            with(
                Item::new("Properties", Command::RecycleBin(BIN_PROPERTIES))
                    .glyph(crate::shell_mask!("explorer-properties", 16)),
            ),
        ],
        Style::Fluent,
        DESKTOP,
    )
    .min_width(220)
}
pub fn desktop_view(style: Style, icons_shown: bool) -> Menu {
    Menu::new(
        vec![
            with(Item::new("Large icons", Command::Nothing).disabled()),
            with(Item::new("Medium icons", Command::Nothing).checked(Check::Radio)),
            with(Item::new("Small icons", Command::Nothing).disabled()),
            Entry::Separator,
            with(
                Item::new("Auto arrange icons", Command::Nothing)
                    .checked(Check::Checked)
                    .disabled(),
            ),
            with(
                Item::new("Align icons to grid", Command::Nothing)
                    .checked(Check::Checked)
                    .disabled(),
            ),
            Entry::Separator,
            with(
                Item::new("Show desktop icons", Command::ToggleDesktopIcons).checked(
                    if icons_shown {
                        Check::Checked
                    } else {
                        Check::None
                    },
                ),
            ),
        ],
        style,
        DESKTOP,
    )
}
pub fn desktop_sort(style: Style, by_name: bool) -> Menu {
    Menu::new(
        vec![
            with(Item::new("Name", Command::SortByName).checked(if by_name {
                Check::Radio
            } else {
                Check::None
            })),
            with(Item::new("Size", Command::Nothing).disabled()),
            with(Item::new("Item type", Command::Nothing).disabled()),
            with(Item::new("Date modified", Command::Nothing).disabled()),
        ],
        style,
        DESKTOP,
    )
}
/// Right-click on empty taskbar space.
pub fn taskbar() -> Menu {
    Menu::new(
        vec![
            with(
                Item::new("Task Manager", Command::Launch(App::Monitor))
                    .icon(Icon::App(App::Monitor)),
            ),
            with(
                Item::new("Taskbar settings", Command::Settings(page::TASKBAR))
                    .glyph(crate::shell_mask!("shell-settings", 16)),
            ),
        ],
        Style::Fluent,
        TASKBAR,
    )
    .min_width(220)
}
/// Jump list (jump-list-lite) for an extra taskbar button: a registered app
/// group the desktop shell published, or a Windows program's windows. Pin
/// and new-window entries need the shell (`shell`); closing is the server's.
pub fn extra_jump_list(
    label: &str,
    key: u32,
    pinned: bool,
    windows: usize,
    shell: bool,
    end_task: bool,
) -> Menu {
    use door_service_protocol::shell::command;
    let mut entries = alloc::vec::Vec::new();
    if shell {
        entries.push(with(
            Item::new(label, Command::Shell(command::LAUNCH_NEW, key))
                .glyph(crate::shell_mask!("explorer-window", 16)),
        ));
        entries.push(with(if pinned {
            Item::new(
                "Unpin from taskbar",
                Command::Shell(command::UNPIN_TASKBAR, key),
            )
            .glyph(crate::shell_mask!("shell-unpin", 16))
        } else {
            Item::new("Pin to taskbar", Command::Shell(command::PIN_TASKBAR, key))
                .glyph(crate::shell_mask!("shell-pin", 16))
        }));
    }
    if windows > 0 && end_task {
        entries.push(with(
            Item::new("End task", Command::EndTaskExtra(key))
                .glyph(crate::shell_mask!("shell-close", 16)),
        ));
    }
    if windows > 0 {
        entries.push(with(
            Item::new(
                if windows > 1 {
                    "Close all windows"
                } else {
                    "Close window"
                },
                Command::CloseExtra(key),
            )
            .glyph(crate::shell_mask!("shell-close", 16)),
        ));
    }
    Menu::new(entries, Style::Fluent, JUMP_LIST).min_width(256)
}
/// Jump list for a taskbar app button.
pub fn jump_list(app: App, pinned: bool, windows: u8, end_task: bool) -> Menu {
    let mut entries = vec![
        with(Item::new(app.label(), Command::NewWindow(app)).icon(Icon::App(app))),
        if pinned {
            with(
                Item::new("Unpin from taskbar", Command::UnpinTaskbar(app))
                    .glyph(crate::shell_mask!("shell-unpin", 16)),
            )
        } else {
            with(
                Item::new("Pin to taskbar", Command::PinTaskbar(app))
                    .glyph(crate::shell_mask!("shell-pin", 16)),
            )
        },
    ];
    if windows > 0 && end_task {
        entries.push(with(
            Item::new("End task", Command::EndTask(app))
                .glyph(crate::shell_mask!("shell-close", 16)),
        ));
    }
    if windows > 0 {
        entries.push(with(
            Item::new(
                if windows > 1 {
                    "Close all windows"
                } else {
                    "Close window"
                },
                Command::CloseWindows(app),
            )
            .glyph(crate::shell_mask!("shell-close", 16)),
        ));
    }
    Menu::new(entries, Style::Fluent, JUMP_LIST).min_width(256)
}
pub fn power() -> Menu {
    Menu::new(
        vec![
            with(
                Item::new("Sleep", Command::Nothing)
                    .glyph(crate::shell_mask!("shell-sleep", 16))
                    .detail("Not supported")
                    .disabled(),
            ),
            with(
                Item::new("Shut down", Command::Shutdown)
                    .glyph(crate::shell_mask!("shell-power", 16)),
            ),
            with(
                Item::new("Restart", Command::Restart)
                    .glyph(crate::shell_mask!("shell-restart", 16)),
            ),
        ],
        Style::Fluent,
        POWER,
    )
    .min_width(200)
}
pub fn user() -> Menu {
    Menu::new(
        vec![
            with(
                Item::new("Change account settings", Command::Settings(page::ACCOUNTS))
                    .glyph(crate::shell_mask!("shell-settings", 16)),
            ),
            with(Item::new("Lock", Command::Lock).glyph(crate::shell_mask!("shell-lock", 16))),
            with(Item::new("Sign out", Command::SignOut)),
        ],
        Style::Fluent,
        USER,
    )
    .min_width(240)
}
pub fn start_view(current: crate::start::View) -> Menu {
    Menu::new(
        crate::start::View::ALL
            .into_iter()
            .map(|view| {
                with(
                    Item::new(view.label(), Command::StartView(view.index())).checked(
                        if view == current {
                            Check::Radio
                        } else {
                            Check::None
                        },
                    ),
                )
            })
            .collect(),
        Style::Fluent,
        START_VIEW,
    )
    .min_width(160)
}
/// Context menu for an app in Start (`pinned_index` when in Pinned).
pub fn start_item(
    app: App,
    pinned_index: Option<usize>,
    start_pinned: bool,
    taskbar_pinned: bool,
) -> Menu {
    let mut entries = vec![if start_pinned {
        with(
            Item::new("Unpin from Start", Command::UnpinStart(app))
                .glyph(crate::shell_mask!("shell-unpin", 16)),
        )
    } else {
        with(
            Item::new("Pin to Start", Command::PinStart(app))
                .glyph(crate::shell_mask!("shell-pin", 16)),
        )
    }];
    if pinned_index.is_some_and(|i| i > 0) {
        entries.push(with(Item::new("Move to front", Command::MoveToFront(app))));
    }
    entries.push(if taskbar_pinned {
        with(
            Item::new("Unpin from taskbar", Command::UnpinTaskbar(app))
                .glyph(crate::shell_mask!("shell-unpin", 16)),
        )
    } else {
        with(
            Item::new("Pin to taskbar", Command::PinTaskbar(app))
                .glyph(crate::shell_mask!("shell-pin", 16)),
        )
    });
    Menu::new(entries, Style::Fluent, START_ITEM).min_width(220)
}
/// Start > Recent item menu (Windows 11 recommended-file menu).
pub fn recent_item(index: usize) -> Menu {
    let index = index.min(u8::MAX as usize) as u8;
    Menu::new(
        vec![
            with(
                Item::new("Open file location", Command::RecentLocation(index))
                    .glyph(crate::shell_mask!("shell-folder", 16)),
            ),
            with(
                Item::new("Remove from list", Command::RecentRemove(index))
                    .glyph(crate::shell_mask!("shell-close", 16)),
            ),
        ],
        Style::Fluent,
        START_ITEM,
    )
    .min_width(220)
}
/// Win+X quick-link menu (right-click Start), the 26H2 list with real
/// destinations: legacy tools open their unified Settings pages (Mobility
/// Center's brightness/volume/power tiles are System > Power; Computer
/// Management is System > Advanced). "Terminal (Admin)" asks User Account
/// Control, then opens the elevated Terminal.
pub fn quick_link() -> Menu {
    Menu::new(
        vec![
            item("Installed apps", Command::Settings(page::INSTALLED_APPS)),
            item("Mobility Center", Command::Settings(page::POWER)),
            item("Power Options", Command::Settings(page::POWER)),
            item("Event Viewer", Command::Settings(page::EVENT_VIEWER)),
            item("System", Command::Settings(page::ABOUT)),
            item("Device Manager", Command::Settings(page::DEVICES_DRIVERS)),
            item(
                "Network Connections",
                Command::Settings(page::NETWORK_ADMINISTRATION),
            ),
            item("Disk Management", Command::Settings(page::DISK_MANAGEMENT)),
            item(
                "Computer Management",
                Command::Settings(page::SYSTEM_ADVANCED),
            ),
            item("Terminal", Command::Launch(App::Terminal)),
            item("Terminal (Admin)", Command::AdminTerminal),
            Entry::Separator,
            item("Task Manager", Command::Launch(App::Monitor)),
            item("Settings", Command::Launch(App::Settings)),
            item("File Explorer", Command::Launch(App::Files)),
            item("Search", Command::Search),
            item("Run", Command::Run),
            Entry::Separator,
            item("Shut down or sign out", Command::Submenu(SUB_POWER)),
            item("Desktop", Command::ShowDesktop),
        ],
        Style::Fluent,
        QUICK_LINK,
    )
    .min_width(240)
}
pub fn quick_link_power() -> Menu {
    Menu::new(
        vec![
            with(Item::new("Sign out", Command::SignOut)),
            with(
                Item::new("Sleep", Command::Nothing)
                    .detail("Not supported")
                    .disabled(),
            ),
            item("Shut down", Command::Shutdown),
            item("Restart", Command::Restart),
        ],
        Style::Fluent,
        QUICK_LINK,
    )
    .min_width(200)
}
/// The submenu an item opens, given the parent tag and shell state.
pub fn submenu(id: u8, style: Style, icons_shown: bool, sorted: bool) -> Option<Menu> {
    match id {
        SUB_VIEW => Some(desktop_view(style, icons_shown)),
        SUB_SORT => Some(desktop_sort(style, sorted)),
        SUB_POWER => Some(quick_link_power()),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycle_bin_menu_opens_empties_only_a_full_bin_and_shows_properties() {
        let commands = |m: Menu| -> alloc::vec::Vec<(Command, bool)> {
            m.entries
                .iter()
                .filter_map(|e| match e {
                    Entry::Item(i) => Some((i.command, i.enabled)),
                    Entry::Separator => None,
                })
                .collect()
        };
        for full in [false, true] {
            assert_eq!(
                commands(recycle_bin(full)),
                [
                    (Command::RecycleBin(BIN_OPEN), true),
                    (Command::RecycleBin(BIN_EMPTY), full),
                    (Command::RecycleBin(BIN_PROPERTIES), true),
                ]
            );
        }
    }
    #[test]
    fn menus_only_enable_working_commands() {
        let hidden = desktop(false, false);
        assert!(
            !hidden
                .entries
                .iter()
                .any(|e| matches!(e, Entry::Item(i) if i.command == Command::ShowMoreOptions))
        );
        assert!(
            matches!(hidden.entries.last(), Some(Entry::Item(_))),
            "no dangling separator"
        );
        for menu in [
            desktop(false, true),
            desktop(true, true),
            taskbar(),
            power(),
            user(),
            quick_link(),
            quick_link_power(),
            jump_list(App::Notes, true, 2, true),
        ] {
            for entry in &menu.entries {
                if let Entry::Item(item) = entry
                    && item.enabled
                {
                    assert_ne!(
                        item.command,
                        Command::Nothing,
                        "{} has no backend",
                        item.label
                    );
                }
            }
        }
        let j = jump_list(App::Files, false, 0, true);
        assert_eq!(j.entries.len(), 2);
        assert!(
            matches!(&j.entries[1], Entry::Item(i) if i.command == Command::PinTaskbar(App::Files))
        );
        assert!(
            matches!(&jump_list(App::Files, true, 3, false).entries[2], Entry::Item(i) if i.label == "Close all windows")
        );
    }
}
