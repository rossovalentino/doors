//! Index-backed file and folder name search over the mounted volumes
//! (Windows 11 26H2 "This PC" search). Doors has no indexing service, so the
//! caller owns an [`Index`] and advances it a bounded number of Storage
//! listing calls at a time; queries are then answered from memory. Shared by
//! Explorer's This PC search and the Start search provider, and independent
//! of any Explorer window. It covers names only (no file contents), counts
//! every object once (the C:\Users junction and hard links), and stops at
//! `MAX_ITEMS` names or `MAX_DEPTH` levels with an honest `Partial` state.
extern crate alloc;
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use door_service_client::{DirEntry, Storage};

/// Mounted volumes: (label, Door namespace root). One persistent volume is
/// mounted today; its compatibility root is presented as C:.
pub const VOLUMES: [(&str, &str); 1] = [("Local Disk (C:)", "/compat")];
pub const MAX_ITEMS: usize = 20_000;
pub const MAX_DEPTH: usize = 32;
/// Explorer's own profile metadata, never offered as user content.
pub const HIDDEN: [&str; 3] = [
    ".door-explorer-tags",
    ".door-explorer-views",
    crate::recent::FILE,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Building,
    Complete,
    /// Stopped at a bound; some names are not indexed.
    Partial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    File,
    Folder,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: HitKind,
    pub name: String,
    /// Door namespace path (what Storage and Explorer open).
    pub path: String,
    /// Windows-style path for display ("C:\Users\1001\notes\a.txt").
    pub display: String,
    pub bytes: u64,
    pub object: u64,
}

/// Case-insensitive ASCII substring match on a UTF-16 name.
pub fn name_matches(name: &[u16], query: &str) -> bool {
    let q = query.trim().as_bytes();
    q.is_empty()
        || name.windows(q.len()).any(|units| {
            units
                .iter()
                .zip(q)
                .all(|(u, c)| *u <= 127 && (*u as u8).eq_ignore_ascii_case(c))
        })
}
fn join(parent: &str, name: &str) -> String {
    if parent == "/compat" && name == "Users" {
        // The existing Users junction resolves to the authenticated /users.
        "/users".into()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

pub struct Index {
    items: Vec<(String, DirEntry)>,
    pending: Vec<(String, usize)>,
    current: Option<(String, usize, u64)>,
    seen: BTreeSet<u64>,
    state: State,
    unreadable: u64,
}
impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}
impl Index {
    /// An index of every mounted volume (`VOLUMES`).
    pub fn new() -> Self {
        let roots: Vec<&str> = VOLUMES.iter().map(|v| v.1).collect();
        Self::for_roots(&roots)
    }
    pub fn for_roots(roots: &[&str]) -> Self {
        Self {
            items: Vec::new(),
            pending: roots.iter().rev().map(|r| (String::from(*r), 0)).collect(),
            current: None,
            seen: BTreeSet::new(),
            state: State::Building,
            unreadable: 0,
        }
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// Folders that could not be listed (denied, removed, I/O).
    pub fn unreadable(&self) -> u64 {
        self.unreadable
    }
    /// Indexed (path, entry) pairs whose name contains `text`.
    pub fn matches<'a>(
        &'a self,
        text: &'a str,
    ) -> impl Iterator<Item = &'a (String, DirEntry)> + 'a {
        self.items
            .iter()
            .filter(move |(_, e)| name_matches(&e.name[..e.name_len], text))
    }
    /// Up to `limit` hits for `text`, in index order (shallow paths first).
    pub fn query(&self, text: &str, limit: usize) -> Vec<Hit> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        self.matches(text)
            .take(limit)
            .map(|(path, e)| Hit {
                kind: if e.info.directory {
                    HitKind::Folder
                } else {
                    HitKind::File
                },
                name: String::from_utf16_lossy(&e.name[..e.name_len]),
                path: path.clone(),
                display: crate::explorer_path::display(path),
                bytes: e.info.bytes,
                object: e.info.object,
            })
            .collect()
    }
    /// Records one listed child of `parent` (exposed for host tests).
    pub fn visit(&mut self, parent: &str, depth: usize, entry: DirEntry) {
        if self.state != State::Building || !self.seen.insert(entry.info.object) {
            return;
        }
        let Ok(name) = String::from_utf16(&entry.name[..entry.name_len]) else {
            return;
        };
        let path = join(parent, &name);
        if entry.info.directory {
            if depth + 1 < MAX_DEPTH {
                self.pending.push((path.clone(), depth + 1));
            } else {
                self.state = State::Partial;
            }
        }
        if !HIDDEN.contains(&name.as_str()) {
            self.items.push((path, entry));
        }
        if self.items.len() >= MAX_ITEMS {
            self.state = State::Partial;
        }
    }
    /// Advances the walk by at most `calls` Storage listing calls (one entry
    /// each). Returns whether names were added or the walk finished.
    pub fn step(&mut self, storage: &mut Storage, calls: usize) -> bool {
        let before = (self.items.len(), self.state);
        for _ in 0..calls {
            if self.state != State::Building {
                break;
            }
            if self.current.is_none() {
                match self.pending.pop() {
                    Some((path, depth)) => self.current = Some((path, depth, 0)),
                    None => {
                        self.state = State::Complete;
                        break;
                    }
                }
            }
            let (path, depth, cookie) = self.current.clone().unwrap();
            match storage.list(&path, cookie) {
                Ok(Some(entry)) => {
                    if let Some(c) = self.current.as_mut() {
                        c.2 = entry.cookie;
                    }
                    self.visit(&path, depth, entry);
                }
                Ok(None) => self.current = None,
                Err(_) => {
                    self.unreadable += 1;
                    self.current = None;
                }
            }
        }
        before != (self.items.len(), self.state)
    }
}

/// One-shot query for callers without a long-lived index: walks the volumes
/// with at most `budget` listing calls, then answers. Returns the hits and
/// whether the walk covered everything (`Complete`) or not.
pub fn query(storage: &mut Storage, text: &str, limit: usize, budget: usize) -> (Vec<Hit>, State) {
    let mut index = Index::new();
    index.step(storage, budget);
    let state = match index.state() {
        State::Building => State::Partial,
        other => other,
    };
    (index.query(text, limit), state)
}

/// What an Explorer window shows when opened from a search hit, the Start
/// menu or "Open in new window". Encoded for any process boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// Open `folder`; if `select` names a child, select and reveal it.
    Location {
        folder: String,
        select: Option<String>,
    },
    /// Open This PC with this search run over every volume.
    Search(String),
    /// A location as typed in Run (`~\`, `C:\...`, `%USERPROFILE%`,
    /// `shell:` folders, profile-relative names), resolved like the
    /// address bar.
    Typed(String),
    /// The desktop Recycle Bin's context menu: open the bin and empty it
    /// (asking first) or show its properties.
    RecycleBin(BinAction),
    /// A Windows program opened `path` with no Windows association (the
    /// window server's OPEN_WITH): its folder with it selected, then its
    /// default app here, or the picker (`choose`: always the picker).
    OpenWith { path: String, choose: bool },
}
/// What the desktop asks of the Recycle Bin besides opening it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinAction {
    Empty,
    Properties,
}
impl Open {
    /// "Open file location": the parent folder with the hit selected.
    pub fn file_location(hit: &Hit) -> Self {
        // C:\Users is the junction /users shown inside C: (/compat).
        let folder = match hit.path.rsplit_once('/') {
            _ if hit.path == "/users" => "/compat".into(),
            Some(("", _)) | None => "/".into(),
            Some((p, _)) => String::from(p),
        };
        Self::Location {
            folder,
            select: Some(hit.name.clone()),
        }
    }
    /// Opening a hit: a folder opens itself, a file opens its location.
    pub fn for_hit(hit: &Hit) -> Self {
        match hit.kind {
            HitKind::Folder => Self::Location {
                folder: hit.path.clone(),
                select: None,
            },
            HitKind::File => Self::file_location(hit),
        }
    }
    pub const MAX_BYTES: usize = 512;
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Self::Location { folder, select } => {
                out.push(b'L');
                out.extend_from_slice(folder.as_bytes());
                if let Some(name) = select {
                    out.push(0);
                    out.extend_from_slice(name.as_bytes());
                }
            }
            Self::Search(text) => {
                out.push(b'S');
                out.extend_from_slice(text.as_bytes());
            }
            Self::Typed(text) => {
                out.push(b'T');
                out.extend_from_slice(text.as_bytes());
            }
            Self::OpenWith { path, choose } => {
                out.push(if *choose { b'A' } else { b'W' });
                out.extend_from_slice(path.as_bytes());
            }
            Self::RecycleBin(action) => {
                out.push(b'B');
                out.push(match action {
                    BinAction::Empty => b'E',
                    BinAction::Properties => b'P',
                });
            }
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > Self::MAX_BYTES {
            return None;
        }
        let (kind, rest) = bytes.split_first()?;
        let text = core::str::from_utf8(rest).ok()?;
        match kind {
            b'L' => {
                let (folder, select) = match text.split_once('\0') {
                    Some((f, s)) if !s.is_empty() && !s.contains(['\0', '/']) => {
                        (f, Some(String::from(s)))
                    }
                    Some(_) => return None,
                    None => (text, None),
                };
                (folder.starts_with('/')
                    || folder == "::ThisPC"
                    || folder == crate::file_operations::RECYCLE_BIN)
                    .then(|| Self::Location {
                        folder: folder.into(),
                        select,
                    })
            }
            b'S' if !text.trim().is_empty() && !text.contains('\0') => {
                Some(Self::Search(text.into()))
            }
            b'T' if !text.trim().is_empty() && !text.contains('\0') => {
                Some(Self::Typed(text.into()))
            }
            b'W' | b'A'
                if text.starts_with('/') && !text.ends_with('/') && !text.contains('\0') =>
            {
                Some(Self::OpenWith {
                    path: text.into(),
                    choose: *kind == b'A',
                })
            }
            b'B' => match text {
                "E" => Some(Self::RecycleBin(BinAction::Empty)),
                "P" => Some(Self::RecycleBin(BinAction::Properties)),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_service_client::FileInfo;
    fn entry(name: &str, object: u64, directory: bool) -> DirEntry {
        let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
        for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
            *o = u;
        }
        DirEntry {
            times: Default::default(),
            cookie: object,
            name: units,
            name_len: name.encode_utf16().count(),
            info: FileInfo {
                attributes: 0,
                bytes: 7,
                object,
                version: 1,
                directory,
            },
        }
    }
    #[test]
    fn index_counts_objects_once_follows_the_users_junction_and_hides_metadata() {
        let mut index = Index::new();
        index.visit("/compat", 0, entry("Users", 2, true));
        index.visit("/compat", 0, entry("Windows", 3, true));
        index.visit("/users", 1, entry("1001", 4, true));
        index.visit("/users/1001", 2, entry("Report.TXT", 5, false));
        index.visit("/users/1001", 2, entry("report-copy-link", 5, false));
        index.visit("/users/1001", 2, entry(".door-explorer-tags", 6, false));
        assert_eq!(index.len(), 4);
        let hits = index.query("report", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/users/1001/Report.TXT");
        assert_eq!(hits[0].display, "C:\\Users\\1001\\Report.TXT");
        assert_eq!(hits[0].kind, HitKind::File);
        assert!(index.query("   ", 10).is_empty());
        assert_eq!(index.query("1001", 10)[0].kind, HitKind::Folder);
        assert_eq!(
            Open::for_hit(&hits[0]),
            Open::Location {
                folder: "/users/1001".into(),
                select: Some("Report.TXT".into())
            }
        );
        let users = &index.query("users", 1)[0];
        assert_eq!(
            Open::file_location(users),
            Open::Location {
                folder: "/compat".into(),
                select: Some("Users".into())
            }
        );
    }
    #[test]
    fn open_with_requests_roundtrip_and_need_a_file_path() {
        for choose in [false, true] {
            let open = Open::OpenWith {
                path: "/users/1/Documents/notes.xyz".into(),
                choose,
            };
            assert_eq!(Open::decode(&open.encode()), Some(open));
        }
        // The window server sends the same bytes (window::open_with).
        assert_eq!(
            Open::decode(b"W/compat/a.log"),
            Some(Open::OpenWith {
                path: "/compat/a.log".into(),
                choose: false
            })
        );
        for bad in [&b"Wrelative.txt"[..], b"A/folder/", b"W/a\0b", b"W"] {
            assert!(Open::decode(bad).is_none());
        }
    }
    #[test]
    fn bounds_stop_as_partial_and_requests_roundtrip() {
        let mut index = Index::for_roots(&["/"]);
        for i in 0..(MAX_ITEMS as u64 + 5) {
            index.visit("/", 0, entry("f", i + 10, false));
        }
        assert_eq!(index.state(), State::Partial);
        assert_eq!(index.len(), MAX_ITEMS);
        let mut deep = Index::for_roots(&["/"]);
        deep.visit("/a", MAX_DEPTH - 1, entry("deeper", 1, true));
        assert_eq!(deep.state(), State::Partial);
        for open in [
            Open::Location {
                folder: "/users/1001/notes".into(),
                select: Some("a b.txt".into()),
            },
            Open::Location {
                folder: "::ThisPC".into(),
                select: None,
            },
            Open::Search("proposal".into()),
            Open::Typed("~\\Downloads".into()),
            Open::Typed("shell:Documents".into()),
            Open::RecycleBin(BinAction::Empty),
            Open::RecycleBin(BinAction::Properties),
        ] {
            assert_eq!(Open::decode(&open.encode()), Some(open));
        }
        for bad in [
            &b""[..],
            b"Lrelative",
            b"Sx\0y",
            b"S  ",
            b"L/a\0b/c",
            b"X/a",
            b"B",
            b"BX",
            b"BEE",
        ] {
            assert_eq!(Open::decode(bad), None, "{bad:?}");
        }
    }
}
