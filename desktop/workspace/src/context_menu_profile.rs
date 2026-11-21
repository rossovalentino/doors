//! Settings > Personalization > Context menu > "Customize": the user's own
//! layout of each context menu. A profile never lists fixed menus; it is a
//! set of per-context preferences (hidden commands, an order with group
//! breaks, which commands sit in the icon row) applied on top of whatever
//! the clicked context actually offers right now (`apply`). Commands a
//! context does not offer at the moment (Open file location outside search,
//! Paste without a clipboard provider, Empty Recycle Bin in a normal
//! folder) simply do not appear; commands the user never placed keep their
//! default place next to the neighbour they followed.
//!
//! Saved by Settings as the profile file `.door-context-menu` and read by
//! File Explorer when a menu opens (bounded text; a damaged file is ignored).
use crate::context_menu::{Action, Entry, Look, Menu, Style, Subject};
use alloc::{format, string::String, vec::Vec};

pub const FILE: &str = ".door-context-menu";
const MAGIC: &str = "DOORCTX1";
pub const MAX_BYTES: usize = 8 * 1024;
/// Most commands one context lists (bounds the profile and the editor).
pub const MAX_ITEMS: usize = 32;
/// Most icon-row buttons (the row must still fit the menu).
pub const MAX_ROW: usize = 6;
/// The tail every compact menu ends with; it stays reachable whatever the
/// layout, so the customizer can always be found again.
pub const FIXED: &str = "customize";

/// Where the user right-clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    File,
    Folder,
    Multiple,
    Background,
    Drive,
    Navigation,
    RecycleItem,
}
impl Context {
    pub const ALL: [Self; 7] = [
        Self::File,
        Self::Folder,
        Self::Multiple,
        Self::Background,
        Self::Drive,
        Self::Navigation,
        Self::RecycleItem,
    ];
    /// The context of an Explorer menu subject (tags keep their own menu).
    pub fn of(subject: Subject) -> Option<Self> {
        Some(match subject {
            Subject::File => Self::File,
            Subject::Folder => Self::Folder,
            Subject::Multiple => Self::Multiple,
            Subject::Background => Self::Background,
            Subject::Drive | Subject::ThisPc => Self::Drive,
            Subject::TreeFolder | Subject::TreePin | Subject::RecycleBin => Self::Navigation,
            Subject::RecycleItem => Self::RecycleItem,
            Subject::Tag(_) | Subject::TagChip(_) => return None,
        })
    }
    /// A representative subject for previews and the editor's item list.
    pub fn subject(self) -> Subject {
        match self {
            Self::File => Subject::File,
            Self::Folder => Subject::Folder,
            Self::Multiple => Subject::Multiple,
            Self::Background => Subject::Background,
            Self::Drive => Subject::Drive,
            Self::Navigation => Subject::TreePin,
            Self::RecycleItem => Subject::RecycleItem,
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Multiple => "multiple",
            Self::Background => "background",
            Self::Drive => "drive",
            Self::Navigation => "navigation",
            Self::RecycleItem => "recycle",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "A file",
            Self::Folder => "A folder",
            Self::Multiple => "Several items",
            Self::Background => "Empty space in a folder",
            Self::Drive => "A drive",
            Self::Navigation => "The navigation pane",
            Self::RecycleItem => "A Recycle Bin item",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::File => "Documents, pictures and other files",
            Self::Folder => "A folder in the file list",
            Self::Multiple => "Two or more selected items",
            Self::Background => "Blank space beside the items",
            Self::Drive => "Local Disk and other drives",
            Self::Navigation => "Folders and pins on the left",
            Self::RecycleItem => "Something you deleted",
        }
    }
}

/// One step of a context's order: a command or a group break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Item(String),
    Separator,
}

/// The user's layout of one context. Empty means Windows' default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// Commands (and group breaks) in the order the user placed them.
    pub order: Vec<Token>,
    pub hidden: Vec<String>,
    /// Commands shown as icon-row buttons; `None` keeps the default row.
    pub row: Option<Vec<String>>,
}
impl Layout {
    pub fn is_default(&self) -> bool {
        self.order.is_empty() && self.hidden.is_empty() && self.row.is_none()
    }
    pub fn is_hidden(&self, key: &str) -> bool {
        self.hidden.iter().any(|k| k == key)
    }
}

/// Every context's layout plus the appearance shared by all menus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub layouts: [Layout; 7],
    pub look: Look,
}

/// The compact menu width holding its icon row in `look`.
fn cm_width(menu: &Menu, look: Look) -> u32 {
    crate::context_menu::metrics::MODERN_WIDTH.max(look.row_width(menu.icons.len()))
}
/// The stable key of a command (`None` for separators and commands the
/// profile never moves: radio choices inside View/Sort and dynamic rows).
pub fn key(entry: &Entry) -> Option<&'static str> {
    if entry.separator {
        return None;
    }
    Some(match entry.action {
        Action::Cut => "cut",
        Action::Copy => "copy",
        Action::Paste => "paste",
        Action::Rename => "rename",
        Action::Share => "share",
        Action::Delete => "delete",
        Action::Open => "open",
        Action::OpenNewTab => "open-new-tab",
        Action::OpenNewWindow => "open-new-window",
        Action::PinQuickAccess => "pin",
        Action::CopyPath => "copy-path",
        Action::Compress => "compress",
        Action::ExtractAll => "extract",
        Action::Convert => "convert",
        Action::Properties => "properties",
        Action::ShowMore => "show-more",
        Action::OpenTerminal => "terminal",
        Action::Refresh => "refresh",
        Action::NewFolder => "new-folder",
        Action::NewTextDocument => "new-text",
        Action::Expand => "expand",
        Action::Collapse => "collapse",
        Action::Customize => FIXED,
        Action::Favorite => "favorite",
        Action::RemoveRecent => "remove-recent",
        Action::OpenLocation => "open-location",
        Action::Restore => "restore",
        Action::RestoreAll => "restore-all",
        Action::EmptyRecycleBin => "empty-bin",
        Action::Submenu => match entry.label.as_str() {
            "View" => "view",
            "Sort by" => "sort",
            "New" => "new",
            "Manage file" => "manage-file",
            "Open with" => "open-with",
            _ => return None,
        },
        _ => return None,
    })
}
/// Commands that may be icon-row buttons: single actions, no flyouts.
pub fn row_capable(key: &str) -> bool {
    matches!(
        key,
        "cut"
            | "copy"
            | "paste"
            | "rename"
            | "share"
            | "delete"
            | "properties"
            | "pin"
            | "copy-path"
            | "refresh"
            | "terminal"
            | "open-new-tab"
            | "open-new-window"
            | "restore"
            | "empty-bin"
    )
}

impl Profile {
    pub fn layout(&self, context: Context) -> &Layout {
        &self.layouts[context.index()]
    }
    pub fn layout_mut(&mut self, context: Context) -> &mut Layout {
        &mut self.layouts[context.index()]
    }
    pub fn is_default(&self) -> bool {
        self.layouts.iter().all(Layout::is_default) && self.look == Look::default()
    }
    /// Applies the user's layout for `subject` to the menu its context just
    /// produced. Classic ("Show more options") menus stay as Windows lists
    /// them; the appearance applies to every compact menu.
    pub fn apply(&self, subject: Subject, menu: &mut Menu) {
        if menu.style != Style::Modern {
            return;
        }
        menu.look = self.look;
        menu.width = cm_width(menu, self.look);
        let Some(context) = Context::of(subject) else {
            return;
        };
        let layout = self.layout(context);
        if layout.is_default() {
            return;
        }
        // Everything this context offers right now, in default order, with
        // its default group breaks.
        let default_row: Vec<Entry> = core::mem::take(&mut menu.icons);
        // `Some(Some(e))` a command, `Some(None)` a group break, `None` a
        // command the icon row took.
        let mut list: Vec<Option<Option<Entry>>> = core::mem::take(&mut menu.entries)
            .into_iter()
            .map(|e| Some((!e.separator).then_some(e)))
            .collect();
        let hidden = |e: &Entry| key(e).is_some_and(|k| k != FIXED && layout.is_hidden(k));
        // The icon row: the user's buttons in their order, else the default.
        let mut row: Vec<Entry> = Vec::new();
        let mut spare: Vec<Entry> = Vec::new();
        match &layout.row {
            Some(wanted) => {
                let mut pool: Vec<Entry> = default_row;
                for slot in list.iter_mut() {
                    let take = slot
                        .as_ref()
                        .and_then(|s| s.as_ref())
                        .and_then(key)
                        .is_some_and(|k| wanted.iter().any(|w| w == k) && row_capable(k));
                    if take {
                        pool.extend(slot.take().flatten());
                    }
                }
                for w in wanted.iter().take(MAX_ROW) {
                    if let Some(i) = pool.iter().position(|e| key(e) == Some(w.as_str())) {
                        row.push(pool.remove(i));
                    }
                }
                // Default row buttons the user moved out of the row join the
                // list, where the order below places them.
                spare = pool;
            }
            None => row = default_row,
        }
        row.retain(|e| !hidden(e));
        // The list in default order (spare row buttons after the main group).
        let mut items: Vec<Option<Entry>> = list.into_iter().flatten().collect();
        if !spare.is_empty() {
            let at = items
                .iter()
                .position(Option::is_none)
                .unwrap_or(items.len());
            for (i, e) in spare.into_iter().enumerate() {
                items.insert(at + i, Some(e));
            }
        }
        let defaults: Vec<Option<String>> = items
            .iter()
            .map(|slot| slot.as_ref().and_then(key).map(String::from))
            .collect();
        let mut out: Vec<Option<Entry>> = Vec::new();
        if layout.order.is_empty() {
            out = items;
        } else {
            let mut taken = alloc::vec![false; items.len()];
            for token in &layout.order {
                match token {
                    Token::Separator => out.push(None),
                    Token::Item(k) => {
                        if let Some(i) = (0..items.len())
                            .find(|&i| !taken[i] && defaults[i].as_deref() == Some(k.as_str()))
                        {
                            taken[i] = true;
                            out.push(items[i].take());
                        }
                    }
                }
            }
            // Commands the user never placed (offered only in some
            // situations, or new) follow the default neighbour they
            // followed; unkeyed rows keep their default position.
            for i in 0..items.len() {
                if taken[i] || items[i].is_none() {
                    continue;
                }
                let anchor = (0..i).rev().find_map(|j| {
                    let k = defaults[j].as_deref()?;
                    out.iter()
                        .position(|slot| slot.as_ref().and_then(key) == Some(k))
                });
                let entry = items[i].take();
                match anchor {
                    Some(p) => out.insert(p + 1, entry),
                    None => out.insert(0, entry),
                }
            }
        }
        // Drop hidden commands and tidy the group breaks.
        let mut entries: Vec<Entry> = Vec::new();
        let mut pending = false;
        for slot in out {
            match slot {
                None => pending = !entries.is_empty(),
                Some(e) if hidden(&e) => (),
                Some(e) => {
                    if pending {
                        entries.push(Entry::separator());
                        pending = false;
                    }
                    entries.push(e);
                }
            }
        }
        // "Customize menu" always closes the menu.
        if let Some(i) = entries.iter().position(|e| key(e) == Some(FIXED))
            && i + 1 != entries.len()
        {
            let fixed = entries.remove(i);
            while entries.last().is_some_and(|e| e.separator) {
                entries.pop();
            }
            if !entries.is_empty() {
                entries.push(Entry::separator());
            }
            entries.push(fixed);
        }
        while entries.first().is_some_and(|e| e.separator) {
            entries.remove(0);
        }
        menu.icons = row;
        menu.entries = entries;
        let needed = menu.look.row_width(menu.icons.len());
        menu.width = menu.width.max(needed);
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = format!("{MAGIC}\n");
        let flag = |b: bool| u8::from(b);
        out.push_str(&format!(
            "look {} {} {} {}\n",
            flag(self.look.labels),
            flag(self.look.compact),
            flag(self.look.plain),
            flag(self.look.no_shortcuts)
        ));
        for context in Context::ALL {
            let l = self.layout(context);
            if l.is_default() {
                continue;
            }
            let order: Vec<&str> = l
                .order
                .iter()
                .map(|t| match t {
                    Token::Item(k) => k.as_str(),
                    Token::Separator => "|",
                })
                .collect();
            out.push_str(&format!("order {} {}\n", context.key(), order.join(",")));
            out.push_str(&format!("hide {} {}\n", context.key(), l.hidden.join(",")));
            if let Some(row) = &l.row {
                out.push_str(&format!("row {} {}\n", context.key(), row.join(",")));
            }
        }
        out.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        if bytes.len() > MAX_BYTES {
            return Err("context menu profile exceeds its limit");
        }
        let text = core::str::from_utf8(bytes).map_err(|_| "context menu profile is not text")?;
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err("unknown context menu profile format");
        }
        let mut profile = Self::default();
        let keys = |list: &str| -> Result<Vec<String>, &'static str> {
            let list: Vec<String> = list
                .split(',')
                .filter(|k| !k.is_empty())
                .map(String::from)
                .collect();
            if list.len() > MAX_ITEMS * 2
                || list.iter().any(|k| {
                    k.len() > 24
                        || !k
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b == b'-' || b == b'|')
                })
            {
                return Err("malformed context menu command list");
            }
            Ok(list)
        };
        for line in lines {
            let mut parts = line.splitn(3, ' ');
            let kind = parts.next().unwrap_or("");
            if kind == "look" {
                let flags: Vec<&str> = line[5.min(line.len())..].split(' ').collect();
                let on = |i: usize| flags.get(i) == Some(&"1");
                profile.look = Look {
                    labels: on(0),
                    compact: on(1),
                    plain: on(2),
                    no_shortcuts: on(3),
                    win10: false,
                };
                continue;
            }
            let context = parts
                .next()
                .and_then(|k| Context::ALL.into_iter().find(|c| c.key() == k))
                .ok_or("unknown context menu context")?;
            let list = keys(parts.next().unwrap_or(""))?;
            let layout = profile.layout_mut(context);
            match kind {
                "order" => {
                    layout.order = list
                        .into_iter()
                        .map(|k| {
                            if k == "|" {
                                Token::Separator
                            } else {
                                Token::Item(k)
                            }
                        })
                        .collect()
                }
                "hide" => layout.hidden = list.into_iter().filter(|k| k != "|").collect(),
                "row" => {
                    layout.row = Some(
                        list.into_iter()
                            .filter(|k| k != "|")
                            .take(MAX_ROW)
                            .collect(),
                    )
                }
                _ => return Err("unknown context menu preference"),
            }
        }
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_menu::{Gates, modern};
    use door_service_protocol::desktop::Controls;
    fn keys(entries: &[Entry]) -> Vec<&'static str> {
        entries
            .iter()
            .map(|e| {
                if e.separator {
                    "|"
                } else {
                    key(e).unwrap_or("?")
                }
            })
            .collect()
    }
    fn menu(subject: Subject) -> Menu {
        modern(subject, Gates::default(), Controls::DEFAULT)
    }
    #[test]
    fn a_default_profile_leaves_every_context_menu_as_windows_builds_it() {
        let p = Profile::default();
        for context in Context::ALL {
            let mut m = menu(context.subject());
            let before = m.clone();
            p.apply(context.subject(), &mut m);
            assert_eq!(m, before, "{context:?}");
        }
    }
    #[test]
    fn layouts_follow_the_context_and_skip_what_it_does_not_offer() {
        let mut p = Profile::default();
        // Files: Properties last, Open hidden, Rename and Delete as the only
        // row buttons, Copy back in the list after Properties.
        *p.layout_mut(Context::File) = Layout {
            order: alloc::vec![
                Token::Item("manage-file".into()),
                Token::Separator,
                Token::Item("properties".into()),
                Token::Item("copy".into()),
                // Not offered for a plain file: silently skipped.
                Token::Item("empty-bin".into()),
            ],
            hidden: alloc::vec!["open".into(), "share".into()],
            row: Some(alloc::vec!["rename".into(), "delete".into()]),
        };
        let mut m = menu(Subject::File);
        p.apply(Subject::File, &mut m);
        assert_eq!(keys(&m.icons), ["rename", "delete"]);
        let list = keys(&m.entries);
        assert_eq!(&list[..3], ["manage-file", "|", "properties"]);
        let at = |k: &str| list.iter().position(|x| *x == k).unwrap();
        assert!(at("copy") > at("properties"));
        assert!(!list.contains(&"open") && !list.contains(&"share"));
        // Cut left the row and was never placed: it follows its default
        // neighbour; the tail still ends with Customize menu.
        assert!(list.contains(&"cut"));
        assert_eq!(*list.last().unwrap(), FIXED);
        assert!(!list.windows(2).any(|w| w == ["|", "|"]));
        // A folder keeps its own (default) menu.
        let mut folder = menu(Subject::Folder);
        let before = folder.clone();
        p.apply(Subject::Folder, &mut folder);
        assert_eq!(folder, before);
        // Situational commands appear only where the context offers them:
        // Open file location exists only in search results.
        let gates = Gates {
            located: true,
            ..Gates::default()
        };
        let mut located = modern(Subject::File, gates, Controls::DEFAULT);
        p.apply(Subject::File, &mut located);
        let list = keys(&located.entries);
        assert!(list.contains(&"open-location"));
    }
    #[test]
    fn customize_menu_cannot_be_hidden_and_the_profile_round_trips() {
        let mut p = Profile {
            look: Look {
                labels: true,
                compact: true,
                plain: false,
                no_shortcuts: true,
                win10: false,
            },
            ..Profile::default()
        };
        *p.layout_mut(Context::Background) = Layout {
            order: alloc::vec![Token::Item(FIXED.into()), Token::Item("refresh".into())],
            hidden: alloc::vec![FIXED.into(), "view".into()],
            row: Some(alloc::vec!["refresh".into(), "paste".into()]),
        };
        let mut m = menu(Subject::Background);
        p.apply(Subject::Background, &mut m);
        assert_eq!(*keys(&m.entries).last().unwrap(), FIXED);
        assert!(!keys(&m.entries).contains(&"view"));
        assert_eq!(keys(&m.icons), ["refresh", "paste"]);
        assert!(m.look.labels && m.width >= m.look.row_width(2));
        let back = Profile::decode(&p.encode()).unwrap();
        assert_eq!(back, p);
        assert!(Profile::decode(b"DOORCTX1\norder nowhere x\n").is_err());
        assert!(Profile::decode(b"OTHER\n").is_err());
        assert_eq!(Profile::decode(b"").unwrap(), Profile::default());
    }
}
