//! Explorer navigation tree: pinned folders, This PC with its drives and
//! lazily listed subfolders. Rows are identified by keys derived from their
//! position in the hierarchy, so the same filesystem object can appear under
//! two parents (a pinned folder and its drive) with independent expansion,
//! while a single parent never lists a child twice. Fixed drives are listed
//! once, under This PC: Windows puts only removable drives at the root. Children are replaced, never appended, on each (re)load.
extern crate alloc;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::{String, ToString},
    vec::Vec,
};

/// Rows beyond this are not built; deep expansions stay bounded.
pub const MAX_ROWS: usize = 200;
/// Indentation per tree level, in logical pixels.
/// Windows 11 26H2 at 150 %: 12 px per level (chevrons, icons and labels).
pub const INDENT: i32 = 8;
/// Windows 11 26H2 tree rows: a 48 px pitch at 150 % with a 46 px hover
/// plate, and the group separator (a 1 px rule) centred in a row slot.
pub const ROW_PITCH: i32 = 32;
pub const ROW_HEIGHT: u32 = 30;
pub const SEPARATOR_PITCH: i32 = 32;
/// Row pitch at the applied Accessibility > Text size: a tree row holds one
/// 14 px label and grows with its line box (`ROW_PITCH` at 100%).
pub fn row_pitch() -> i32 {
    ROW_PITCH + crate::explorer_layout::grow()
}
/// Row height at the applied text size (`ROW_HEIGHT` at 100%).
pub fn row_height() -> u32 {
    ROW_HEIGHT + crate::explorer_layout::grow() as u32
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// File Explorer Home (Quick access, Favorites, Recent).
    Home,
    /// Index into the caller's pinned-folder list.
    Pin(usize),
    Separator,
    ThisPc,
    /// Index into the caller's drive list; `alias` is the root shortcut.
    Drive {
        slot: usize,
        alias: bool,
    },
    Folder {
        path: String,
    },
    Network,
    /// NoneDrive's folder (Settings > System > Advanced), under Home as
    /// Windows shows OneDrive.
    NoneDrive {
        path: String,
    },
    /// The Linux environment's root, after Network (no distributions yet).
    Linux,
    /// The Recycle Bin (`::RecycleBin`), after Network as with "Show all
    /// folders".
    RecycleBin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub node: Node,
    pub depth: u8,
    pub label: String,
    pub expandable: bool,
    pub expanded: bool,
}
impl Row {
    pub fn pitch(&self) -> i32 {
        if self.node == Node::Separator {
            SEPARATOR_PITCH
        } else {
            row_pitch()
        }
    }
    /// The filesystem path this row opens, if it is a directory location.
    pub fn path<'a>(
        &'a self,
        pins: &'a [String],
        drives: &'a [(&'a str, &'a str)],
    ) -> Option<&'a str> {
        match &self.node {
            Node::Pin(i) => pins.get(*i).map(String::as_str),
            Node::Drive { slot, .. } => drives.get(*slot).map(|d| d.1),
            Node::Folder { path } | Node::NoneDrive { path } => Some(path),
            _ => None,
        }
    }
}

/// Listed subfolders of one directory: (display name, path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Children {
    Loaded(Vec<(String, String)>),
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    expanded: BTreeSet<String>,
    children: BTreeMap<String, Children>,
    /// Folder options > "Show all folders": the pane also lists the Recycle
    /// Bin after Network. Off by default, as in Windows 11, where the bin
    /// lives on the desktop.
    pub show_all_folders: bool,
    /// Optional features turned on (`preferences::features`): NoneDrive's
    /// folder, and the Linux environment.
    pub nonedrive: Option<String>,
    pub linux: bool,
}
impl Tree {
    pub fn new() -> Self {
        let mut tree = Self::default();
        tree.expanded.insert("pc".into());
        tree
    }
    pub fn is_expanded(&self, key: &str) -> bool {
        self.expanded.contains(key)
    }
    /// Expanded row keys, for remembering the tree between sessions.
    pub fn expanded_keys(&self) -> Vec<String> {
        self.expanded.iter().cloned().collect()
    }
    /// Restores remembered expansion; listings load on demand.
    pub fn set_expanded(&mut self, keys: &[String]) {
        if !keys.is_empty() {
            self.expanded = keys.iter().cloned().collect();
        }
    }
    /// Toggles a row; returns the path whose children must be (re)listed
    /// when the row opens. Collapsing forgets nothing but the open state.
    pub fn toggle(
        &mut self,
        row: &Row,
        pins: &[String],
        drives: &[(&str, &str)],
    ) -> Option<String> {
        if !row.expandable {
            return None;
        }
        if self.expanded.remove(&row.key) {
            return None;
        }
        self.expanded.insert(row.key.clone());
        row.path(pins, drives).map(String::from)
    }
    /// Installs a fresh listing for `path`, replacing any earlier one.
    /// Duplicate names (case-sensitive) within one listing are dropped.
    pub fn set_children(&mut self, path: &str, listed: Result<Vec<(String, String)>, ()>) {
        let entry = match listed {
            Ok(mut folders) => {
                let mut seen = BTreeSet::new();
                folders.retain(|(name, _)| seen.insert(name.clone()));
                Children::Loaded(folders)
            }
            Err(()) => Children::Failed,
        };
        self.children.insert(path.into(), entry);
    }
    pub fn children(&self, path: &str) -> Option<&Children> {
        self.children.get(path)
    }
    /// Drops cached listings (refresh); open rows re-list on the next build.
    pub fn invalidate(&mut self) {
        self.children.clear();
    }
    /// Paths of expanded rows whose listing is missing.
    pub fn missing(&self, rows: &[Row], pins: &[String], drives: &[(&str, &str)]) -> Vec<String> {
        rows.iter()
            .filter(|r| r.expanded)
            .filter_map(|r| r.path(pins, drives))
            .filter(|p| !self.children.contains_key(*p))
            .map(String::from)
            .collect()
    }
    fn push_folders(&self, rows: &mut Vec<Row>, parent: &str, path: &str, depth: u8) {
        let Some(Children::Loaded(folders)) = self.children.get(path) else {
            return;
        };
        for (name, child) in folders {
            if rows.len() >= MAX_ROWS {
                return;
            }
            let key = format!("{parent}/{name}");
            let expanded = self.expanded.contains(&key);
            // Unlisted folders may have subfolders; listed ones show a
            // chevron only when they do.
            let expandable = match self.children.get(child) {
                Some(Children::Loaded(f)) => !f.is_empty(),
                Some(Children::Failed) => false,
                None => true,
            };
            rows.push(Row {
                key: key.clone(),
                node: Node::Folder {
                    path: child.clone(),
                },
                depth,
                label: name.clone(),
                expandable,
                expanded: expanded && expandable,
            });
            if expanded && depth < 8 {
                self.push_folders(rows, &key, child, depth + 1);
            }
        }
    }
    /// Builds the visible rows: pins, separator, This PC (+ drives), the root
    /// drive shortcuts, then Network.
    pub fn rows(&self, pins: &[String], pin_labels: &[&str], drives: &[(&str, &str)]) -> Vec<Row> {
        let mut rows = Vec::new();
        // Home first, as in Windows 11, then the Quick access pins.
        rows.push(Row {
            key: "home".into(),
            node: Node::Home,
            depth: 0,
            label: "Home".into(),
            expandable: false,
            expanded: false,
        });
        if let Some(path) = &self.nonedrive {
            let key = String::from("nonedrive");
            let expanded = self.expanded.contains(&key);
            let expandable = match self.children.get(path) {
                Some(Children::Loaded(f)) => !f.is_empty(),
                Some(Children::Failed) => false,
                None => true,
            };
            rows.push(Row {
                key: key.clone(),
                node: Node::NoneDrive { path: path.clone() },
                depth: 0,
                label: "NoneDrive".into(),
                expandable,
                expanded: expanded && expandable,
            });
            if expanded && expandable {
                self.push_folders(&mut rows, &key, path, 1);
            }
        }
        rows.push(Row {
            key: "sep:home".into(),
            node: Node::Separator,
            depth: 0,
            label: String::new(),
            expandable: false,
            expanded: false,
        });
        for (i, label) in pin_labels.iter().enumerate().take(pins.len()) {
            rows.push(Row {
                key: format!("pin:{i}"),
                node: Node::Pin(i),
                depth: 0,
                label: label.to_string(),
                expandable: false,
                expanded: false,
            });
        }
        rows.push(Row {
            key: "sep:0".into(),
            node: Node::Separator,
            depth: 0,
            label: String::new(),
            expandable: false,
            expanded: false,
        });
        let pc_open = self.expanded.contains("pc");
        rows.push(Row {
            key: "pc".into(),
            node: Node::ThisPc,
            depth: 0,
            label: "This PC".into(),
            expandable: !drives.is_empty(),
            expanded: pc_open && !drives.is_empty(),
        });
        let drive = |rows: &mut Vec<Row>, key: String, slot: usize, alias: bool, depth: u8| {
            let (label, path) = drives[slot];
            let expanded = self.expanded.contains(&key);
            let expandable = !matches!(
                self.children.get(path),
                Some(Children::Loaded(f)) if f.is_empty()
            ) && !matches!(self.children.get(path), Some(Children::Failed));
            rows.push(Row {
                key: key.clone(),
                node: Node::Drive { slot, alias },
                depth,
                label: label.into(),
                expandable,
                expanded: expanded && expandable,
            });
            if expanded {
                self.push_folders(rows, &key, path, depth + 1);
            }
        };
        if pc_open {
            for slot in 0..drives.len() {
                drive(&mut rows, format!("pc/d:{slot}"), slot, false, 1);
            }
        }
        rows.push(Row {
            key: "net".into(),
            node: Node::Network,
            depth: 0,
            label: "Network".into(),
            expandable: false,
            expanded: false,
        });
        if self.linux {
            rows.push(Row {
                key: "linux".into(),
                node: Node::Linux,
                depth: 0,
                label: "Linux".into(),
                expandable: false,
                expanded: false,
            });
        }
        if self.show_all_folders {
            rows.truncate(MAX_ROWS - 1);
            rows.push(Row {
                key: "bin".into(),
                node: Node::RecycleBin,
                depth: 0,
                label: "Recycle Bin".into(),
                expandable: false,
                expanded: false,
            });
        } else {
            rows.truncate(MAX_ROWS);
        }
        rows
    }
    /// Top of each row relative to the tree's first row.
    pub fn offsets(rows: &[Row]) -> Vec<i32> {
        let mut y = 0;
        rows.iter()
            .map(|r| {
                let top = y;
                y += r.pitch();
                top
            })
            .collect()
    }
    pub fn height(rows: &[Row]) -> i32 {
        rows.iter().map(Row::pitch).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn pins() -> Vec<String> {
        vec!["/users/1/Desktop".into(), "/users/1/Downloads".into()]
    }
    const LABELS: [&str; 2] = ["Desktop", "Downloads"];
    const DRIVES: [(&str, &str); 1] = [("Local Disk (C:)", "/compat")];
    fn listing() -> Result<Vec<(String, String)>, ()> {
        Ok(vec![
            ("Users".into(), "/users".into()),
            ("Windows".into(), "/compat/Windows".into()),
        ])
    }
    #[test]
    fn this_pc_lists_drives_once_without_a_root_duplicate() {
        let tree = Tree::new();
        let rows = tree.rows(&pins(), &LABELS, &DRIVES);
        let labels: Vec<_> = rows.iter().map(|r| (r.label.as_str(), r.depth)).collect();
        assert_eq!(
            labels,
            [
                ("Home", 0),
                ("", 0),
                ("Desktop", 0),
                ("Downloads", 0),
                ("", 0),
                ("This PC", 0),
                ("Local Disk (C:)", 1),
                ("Network", 0),
            ]
        );
        // Folder options > "Show all folders" adds the Recycle Bin.
        let mut all = Tree::new();
        all.show_all_folders = true;
        let rows_all = all.rows(&pins(), &LABELS, &DRIVES);
        assert_eq!(rows_all.len(), 9);
        assert_eq!(rows_all[8].node, Node::RecycleBin);
        let p = pins();
        assert_eq!(rows_all[8].path(&p, &DRIVES), None);
        assert_eq!(
            rows[0].node,
            Node::Home,
            "Home leads the pane, as in Windows 11"
        );
        assert!(rows[5].expanded && rows[5].expandable);
        assert!(!rows[7].expandable, "no network provider, no chevron");
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.node, Node::Drive { alias: true, .. })),
            "fixed drives are not repeated at the root"
        );
    }
    #[test]
    fn repeated_expand_refresh_and_collapse_never_duplicate_children() {
        let mut tree = Tree::new();
        let (p, d) = (pins(), DRIVES);
        for _ in 0..10 {
            let rows = tree.rows(&p, &LABELS, &d);
            let drive = rows.iter().find(|r| r.key == "pc/d:0").unwrap().clone();
            if let Some(path) = tree.toggle(&drive, &p, &d) {
                assert_eq!(path, "/compat");
                // A listing that repeats a name is still installed once.
                let mut twice = listing().unwrap();
                twice.push(("Users".into(), "/users".into()));
                tree.set_children(&path, Ok(twice));
            }
            tree.set_children("/compat", listing());
            let rows = tree.rows(&p, &LABELS, &d);
            let users = rows.iter().filter(|r| r.key == "pc/d:0/Users").count();
            assert!(users <= 1);
            let unique: BTreeSet<_> = rows.iter().map(|r| r.key.clone()).collect();
            assert_eq!(unique.len(), rows.len(), "row keys are unique");
        }
        // The drive is listed once, so its folders are too.
        let rows = tree.rows(&p, &LABELS, &d);
        assert!(rows.iter().all(|r| r.key != "d:0"));
        assert_eq!(
            rows.iter().filter(|r| r.label == "Users").count(),
            usize::from(tree.is_expanded("pc/d:0"))
        );
        // An even number of toggles left the drive collapsed: open it again
        // so invalidation leaves an expanded listing to fetch.
        if !tree.is_expanded("pc/d:0") {
            let rows = tree.rows(&p, &LABELS, &d);
            let drive = rows.iter().find(|r| r.key == "pc/d:0").unwrap().clone();
            assert_eq!(tree.toggle(&drive, &p, &d).as_deref(), Some("/compat"));
        }
        tree.invalidate();
        assert!(!tree.missing(&tree.rows(&p, &LABELS, &d), &p, &d).is_empty());
    }
    #[test]
    fn failed_or_empty_listings_drop_the_chevron() {
        let mut tree = Tree::new();
        tree.set_children("/compat", Err(()));
        let rows = tree.rows(&pins(), &LABELS, &DRIVES);
        assert!(
            rows.iter()
                .filter(|r| matches!(r.node, Node::Drive { .. }))
                .all(|r| !r.expandable)
        );
        tree.set_children("/compat", Ok(Vec::new()));
        let rows = tree.rows(&pins(), &LABELS, &DRIVES);
        assert!(
            rows.iter()
                .filter(|r| matches!(r.node, Node::Drive { .. }))
                .all(|r| !r.expandable)
        );
        // Home, two pins, This PC, its drive and Network (no Recycle Bin).
        assert_eq!(Tree::height(&rows), 6 * ROW_PITCH + 2 * SEPARATOR_PITCH);
        // 26H2 metrics: 32 px rows with a 30 px plate, separators in a slot.
        assert_eq!((row_pitch(), row_height(), SEPARATOR_PITCH), (32, 30, 32));
    }
}
