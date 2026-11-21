//! Taskbar groups the shell publishes (door.shell PUBLISH, kind TASKBAR):
//! pinned registrations first, then running groups, one per registration
//! (several windows and processes of one program share a button). Catalog
//! apps are not published: the window server already shows their buttons
//! from its own pins and process slots, and they never move.
extern crate alloc;
use crate::registry::{ADHOC_KEY, Database};
use alloc::{string::String, vec::Vec};
use door_service_protocol::shell::{self as s, entry, flag, icon};

/// One authorized window as WINDOWS reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: u64,
    pub pid: u64,
    pub app: u32,
    pub flags: u32,
    pub progress_state: u8,
    pub progress: u16,
    pub title: String,
}
impl Window {
    pub fn from_record(w: &s::Window<'_>) -> Self {
        Self {
            id: w.id,
            pid: w.pid,
            app: w.app,
            flags: w.flags,
            progress_state: w.progress_state,
            progress: w.progress,
            title: w.title.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub key: u32,
    pub icon: u16,
    pub flags: u16,
    pub progress_state: u8,
    pub progress: u16,
    /// Every window of the group (the first `GROUP_WINDOWS` are published).
    pub ids: Vec<u64>,
    pub label: String,
}
impl Group {
    pub fn entry(&self, generation: u16, index: u8, count: u8) -> s::Entry<'_> {
        let mut ids = [0; s::GROUP_WINDOWS];
        for (slot, id) in ids.iter_mut().zip(&self.ids) {
            *slot = *id;
        }
        s::Entry {
            kind: s::kind::TASKBAR,
            generation,
            index,
            count,
            key: self.key,
            icon: self.icon,
            flags: self.flags,
            progress_state: self.progress_state,
            progress: self.progress,
            windows: self.ids.len().min(255) as u8,
            ids,
            label: s::clip(&self.label, s::LABEL_BYTES),
        }
    }
}
/// Build the published groups. `path_of(pid)` is Init's record of the
/// executable a process was admitted from (never the app's own claim).
pub fn taskbar(
    db: &Database,
    windows: &[Window],
    path_of: impl Fn(u64) -> Option<String>,
) -> Vec<Group> {
    let mut groups: Vec<Group> = db
        .taskbar_pins
        .iter()
        .filter_map(|key| db.get(*key))
        .map(|r| Group {
            key: r.key,
            icon: r.icon,
            flags: entry::PINNED,
            progress_state: 0,
            progress: 0,
            ids: Vec::new(),
            label: r.name.clone(),
        })
        .collect();
    for w in windows {
        if w.flags & (flag::SKIP_TASKBAR | flag::OTHER_DESKTOP) != 0 {
            continue;
        }
        // Native windows: the window server draws the catalog apps' own
        // buttons; a bundled app with a taskbar registration (Calculator,
        // Clock) gets a group here like any program.
        let registration = if w.app != 0 {
            let command = u8::try_from(w.app).ok();
            match command.and_then(|c| db.by_command(c)) {
                Some(r)
                    if r.taskbar
                        && command.is_some_and(|c| {
                            door_service_protocol::application::desktop_index(c).is_none()
                        }) =>
                {
                    Some(r.clone())
                }
                _ => continue,
            }
        } else if w.flags & flag::WIN32 == 0 {
            continue;
        } else {
            path_of(w.pid).and_then(|p| db.by_path(&p).cloned())
        };
        if registration.as_ref().is_some_and(|r| !r.taskbar) {
            continue;
        }
        let (key, group_icon, label) = match &registration {
            Some(r) => (r.key, r.icon, r.name.clone()),
            None => (
                ADHOC_KEY | (w.pid as u32 & 0x0fff_ffff),
                icon::WINDOWS_APP,
                w.title.clone(),
            ),
        };
        let i = match groups.iter().position(|g| g.key == key) {
            Some(i) => i,
            None => {
                groups.push(Group {
                    key,
                    icon: group_icon,
                    flags: 0,
                    progress_state: 0,
                    progress: 0,
                    ids: Vec::new(),
                    label: if label.is_empty() {
                        String::from("Windows app")
                    } else {
                        label
                    },
                });
                groups.len() - 1
            }
        };
        let g = &mut groups[i];
        g.ids.push(w.id);
        g.flags |= entry::RUNNING;
        if w.flags & flag::ACTIVE != 0 {
            g.flags |= entry::ACTIVE;
        }
        if w.flags & flag::ATTENTION != 0 {
            g.flags |= entry::ATTENTION;
        }
        if g.progress_state == 0 && w.progress_state != 0 {
            g.progress_state = w.progress_state;
            g.progress = w.progress;
        }
    }
    groups.truncate(s::MAX_ENTRIES);
    groups
}
/// Search catalog the shell publishes (kind CATALOG): the registered
/// programs the window server does not already list (Windows executables).
pub fn catalog(db: &Database) -> Vec<(u32, u16, u16, String)> {
    db.executables()
        .take(s::MAX_ENTRIES)
        .map(|r| {
            let mut flags = 0;
            if r.listed {
                flags |= entry::LISTED;
            }
            if db.start_pins.contains(&r.key) {
                flags |= entry::START_PINNED;
            }
            if db.taskbar_pins.contains(&r.key) {
                flags |= entry::PINNED;
            }
            (r.key, r.icon, flags, r.name.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::executable_key;
    fn win(id: u64, pid: u64, flags: u32, title: &str) -> Window {
        Window {
            id,
            pid,
            app: 0,
            flags,
            progress_state: 0,
            progress: 0,
            title: title.into(),
        }
    }
    #[test]
    fn windows_group_per_registration_with_pins_first() {
        let mut db = Database::builtin();
        db.discover(
            "/system/bin",
            ["wingui-editor.exe", "wingui-f3.exe"].into_iter(),
        );
        let f3 = executable_key("/system/bin/wingui-f3.exe");
        let editor = executable_key("/system/bin/wingui-editor.exe");
        db.set_taskbar_pin(f3, true);
        let mut attention = win(12, 41, flag::WIN32 | flag::ATTENTION, "two.txt");
        attention.progress_state = s::progress::NORMAL;
        attention.progress = 300;
        let windows = [
            win(11, 40, flag::WIN32 | flag::ACTIVE, "one.txt - Editor"),
            attention,
            win(13, 40, flag::WIN32 | flag::SKIP_TASKBAR, "Save As"),
            win(14, 50, flag::WIN32, "Untracked"),
            Window {
                app: 2,
                ..win(15, 60, 0, "Notes")
            },
            win(16, 70, 0, "Native test client"),
        ];
        let path = |pid| match pid {
            40 | 41 => Some(String::from("/system/bin/wingui-editor.exe")),
            _ => None,
        };
        let g = taskbar(&db, &windows, path);
        let keys: Vec<u32> = g.iter().map(|g| g.key).collect();
        assert_eq!(keys, [f3, editor, ADHOC_KEY | 50]);
        assert_eq!(g[0].flags, entry::PINNED);
        assert!(g[0].ids.is_empty());
        assert_eq!(
            g[1].ids,
            [11, 12],
            "two processes of one program share a button"
        );
        assert_eq!(
            g[1].flags,
            entry::RUNNING | entry::ACTIVE | entry::ATTENTION
        );
        assert_eq!((g[1].progress_state, g[1].progress), (1, 300));
        assert_eq!(g[2].label, "Untracked");
        let e = g[1].entry(4, 1, 3);
        assert_eq!((e.windows, e.ids[..2].to_vec()), (2, alloc::vec![11, 12]));
        let mut out = [0; door_service_protocol::BODY_BYTES];
        assert!(e.encode(&mut out).is_ok());
        let c = catalog(&db);
        assert_eq!(c.len(), 2);
        assert!(
            c.iter()
                .any(|(k, _, f, _)| *k == f3 && f & entry::PINNED != 0)
        );
    }
    #[test]
    fn running_inbox_apps_get_buttons_but_catalog_apps_and_tools_do_not() {
        let db = Database::builtin();
        let windows = [
            Window {
                app: 15,
                ..win(21, 80, flag::ACTIVE, "Clock")
            },
            Window {
                app: 14,
                ..win(22, 81, 0, "Calculator")
            },
            Window {
                app: 2,
                ..win(23, 82, 0, "Notes")
            },
            Window {
                app: 12,
                ..win(24, 83, 0, "Doors UI Gallery")
            },
        ];
        let g = taskbar(&db, &windows, |_| None);
        let labels: Vec<&str> = g.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(labels, ["Clock", "Calculator"]);
        assert_eq!((g[0].icon, g[0].ids.as_slice()), (icon::CLOCK, &[21][..]));
        assert_eq!(g[0].flags, entry::RUNNING | entry::ACTIVE);
        assert_eq!(g[1].icon, icon::CALCULATOR);
    }
}
