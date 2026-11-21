//! Recent items (File Explorer Home > Recent, Start > Recent): files opened
//! through Explorer, Notes and Preview, newest first. Doors apps have no wall
//! clock, so entries are ordered by a recency sequence instead of a time.
//! Stored per user in the hidden profile file `.door-recent` (bounded to
//! `MAX` entries, deduplicated by path, with an open counter). Several apps
//! (separate processes) record into it: `save` re-reads the file and replays
//! this session's records, removals and clears onto it, so nobody's opens
//! are lost. A corrupt or unreadable file makes the list session-only and is
//! never overwritten.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_service_client::{Disposition, Error, READ, SHARE_ALL, Storage, WRITE};

pub const FILE: &str = ".door-recent";
pub const MAX: usize = 50;
const MAGIC: &str = "DOORRECENT1";
const MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Door namespace path.
    pub path: String,
    /// Windows-style path ("C:\Users\1001\notes\a.txt").
    pub display: String,
    pub kind: Kind,
    /// How many times it was opened (across apps and sessions).
    pub opens: u32,
    /// Recency sequence: larger is more recent.
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Change {
    Record(String, Kind),
    Remove(String),
    Clear,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recent {
    items: Vec<Item>,
    next: u64,
    /// This session's changes, replayed onto the file on save.
    changes: Vec<Change>,
    writable: bool,
}

/// The profile folder (`/users/<uid>`) that holds `path`, if any.
pub fn profile_of(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/users/")?;
    let end = rest.find('/').unwrap_or(rest.len());
    (end > 0).then(|| &path[..7 + end])
}

impl Recent {
    /// Reads the user's list. A missing file is an empty, writable list; an
    /// unreadable or malformed one is kept as it is and the list becomes
    /// session-only.
    pub fn load(storage: &mut Storage, home: &str) -> Self {
        match read(storage, home) {
            Ok(Some(bytes)) => match Self::decode(&bytes) {
                Ok(mut list) => {
                    list.writable = true;
                    list
                }
                Err(_) => Self::default(),
            },
            Ok(None) => Self {
                writable: true,
                ..Self::default()
            },
            Err(_) => Self::default(),
        }
    }
    /// Whether changes reach the profile file (false: session-only).
    pub fn persistent(&self) -> bool {
        self.writable
    }
    /// Whether there are changes not yet saved.
    pub fn dirty(&self) -> bool {
        !self.changes.is_empty()
    }
    /// Records an open: moves `path` to the top and counts it. Re-opening
    /// the newest item changes nothing visible, so it is not recorded: no
    /// counter bump, no change to save, no disk write.
    pub fn record(&mut self, path: &str, kind: Kind) {
        if path.is_empty() || !path.starts_with('/') || path.len() > 512 || self.newest(path, kind)
        {
            return;
        }
        self.apply(&Change::Record(path.into(), kind));
        self.changes.push(Change::Record(path.into(), kind));
    }
    /// Newest first.
    pub fn items(&self) -> &[Item] {
        &self.items
    }
    /// "Remove from list".
    pub fn remove(&mut self, path: &str) -> bool {
        let had = self.items.iter().any(|i| i.path == path);
        self.apply(&Change::Remove(path.into()));
        self.changes.push(Change::Remove(path.into()));
        had
    }
    /// "Clear recent items".
    pub fn clear(&mut self) {
        self.apply(&Change::Clear);
        self.changes.push(Change::Clear);
    }
    fn newest(&self, path: &str, kind: Kind) -> bool {
        self.items
            .first()
            .is_some_and(|i| i.path == path && i.kind == kind)
    }
    fn apply(&mut self, change: &Change) {
        match change {
            Change::Record(path, kind) if self.newest(path, *kind) => (),
            Change::Record(path, kind) => {
                let opens = match self.items.iter().position(|i| i.path == *path) {
                    Some(at) => self.items.remove(at).opens.saturating_add(1),
                    None => 1,
                };
                self.next += 1;
                self.items.insert(
                    0,
                    Item {
                        path: path.clone(),
                        display: crate::explorer_path::display(path),
                        kind: *kind,
                        opens,
                        sequence: self.next,
                    },
                );
                self.items.truncate(MAX);
            }
            Change::Remove(path) => self.items.retain(|i| i.path != *path),
            Change::Clear => self.items.clear(),
        }
    }
    /// Writes this session's changes onto the current file contents.
    /// Session-only lists (unreadable file) never write.
    pub fn save(&mut self, storage: &mut Storage, home: &str) -> Result<(), Error> {
        if !self.writable || self.changes.is_empty() {
            return Ok(());
        }
        let path = format!("{home}/{FILE}");
        let file = storage.open(&path, Disposition::OpenAlways, READ | WRITE, SHARE_ALL)?;
        let result = (|| {
            let info = storage.info(&file)?;
            let bytes = read_open(storage, &file, info.bytes)?;
            let mut merged = Self::decode(&bytes).map_err(|_| Error::Malformed)?;
            for change in &self.changes {
                merged.apply(change);
            }
            // Another app may already have recorded the same opens.
            let encoded = merged.encode();
            if encoded != bytes {
                storage.replace_checked(&file, info, &encoded)?;
            }
            Ok(merged)
        })();
        let closed = storage.close(file);
        match result {
            Ok(merged) => {
                closed?;
                self.items = merged.items;
                self.next = merged.next;
                self.changes.clear();
                Ok(())
            }
            Err(error) => {
                if matches!(error, Error::Malformed) {
                    // Someone else's damaged file: keep it, stay in memory.
                    self.writable = false;
                }
                Err(error)
            }
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = format!("{MAGIC} {}\n", self.next);
        for i in &self.items {
            out.push_str(&format!(
                "{} {} {} {}\n",
                i.sequence,
                i.opens,
                if i.kind == Kind::Folder { 'd' } else { 'f' },
                i.path
            ));
        }
        out.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        if bytes.len() > MAX_BYTES {
            return Err("recent list exceeds its limit");
        }
        let text = core::str::from_utf8(bytes).map_err(|_| "recent list is not text")?;
        let mut lines = text.lines();
        let next = lines
            .next()
            .and_then(|l| l.strip_prefix(MAGIC))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .ok_or("unknown recent list format")?;
        let mut list = Self {
            next,
            ..Self::default()
        };
        for line in lines {
            let mut f = line.splitn(4, ' ');
            let sequence = f.next().and_then(|v| v.parse::<u64>().ok());
            let opens = f.next().and_then(|v| v.parse::<u32>().ok());
            let kind = match f.next() {
                Some("f") => Some(Kind::File),
                Some("d") => Some(Kind::Folder),
                _ => None,
            };
            let path = f.next().filter(|p| p.starts_with('/'));
            match (sequence, opens, kind, path) {
                (Some(sequence), Some(opens), Some(kind), Some(path))
                    if sequence <= next && !list.items.iter().any(|i| i.path == path) =>
                {
                    list.items.push(Item {
                        path: path.into(),
                        display: crate::explorer_path::display(path),
                        kind,
                        opens,
                        sequence,
                    })
                }
                _ => return Err("malformed recent item"),
            }
        }
        if list.items.len() > MAX {
            return Err("recent list exceeds its limit");
        }
        list.items.sort_by(|a, b| b.sequence.cmp(&a.sequence));
        Ok(list)
    }
}

fn read(storage: &mut Storage, home: &str) -> Result<Option<Vec<u8>>, Error> {
    let file = match storage.open(
        &format!("{home}/{FILE}"),
        Disposition::OpenExisting,
        READ,
        SHARE_ALL,
    ) {
        Ok(file) => file,
        Err(Error::Service(door_service_protocol::Code::NotFound)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let bytes = storage
        .info(&file)
        .and_then(|info| read_open(storage, &file, info.bytes));
    let closed = storage.close(file);
    let bytes = bytes?;
    closed?;
    Ok(Some(bytes))
}
fn read_open(
    storage: &mut Storage,
    file: &door_service_client::File,
    size: u64,
) -> Result<Vec<u8>, Error> {
    if size > MAX_BYTES as u64 {
        return Err(Error::Malformed);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
    while (bytes.len() as u64) < size {
        let count = (size as usize - bytes.len()).min(chunk.len());
        let n = storage.read(file, bytes.len() as u64, &mut chunk[..count])?;
        if n == 0 {
            return Err(Error::Malformed);
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(bytes)
}

/// One-call hook for apps that open (or save) a file: records it in the
/// owner's profile list. Paths outside a profile folder are not recorded.
pub fn note_opened(storage: &mut Storage, path: &str, kind: Kind) {
    let Some(home) = profile_of(path).map(String::from) else {
        return;
    };
    let mut recent = Recent::load(storage, &home);
    recent.record(path, kind);
    let _ = recent.save(storage, &home);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opens_dedupe_count_and_order_by_sequence() {
        let mut r = Recent {
            writable: true,
            ..Recent::default()
        };
        r.record("/users/1001/a.txt", Kind::File);
        r.record("/users/1001/b.txt", Kind::File);
        r.record("/users/1001/a.txt", Kind::File);
        assert_eq!(r.items()[0].path, "/users/1001/a.txt");
        assert_eq!(r.items()[0].opens, 2);
        assert_eq!(r.items()[0].display, "C:\\Users\\1001\\a.txt");
        assert_eq!(r.items().len(), 2);
        for i in 0..80 {
            r.record(&format!("/users/1001/n{i}"), Kind::File);
        }
        assert_eq!(r.items().len(), MAX);
        assert_eq!(r.items()[0].path, "/users/1001/n79");
        assert!(r.remove("/users/1001/n79"));
        assert!(!r.remove("/users/1001/missing"));
        let back = Recent::decode(&r.encode()).unwrap();
        assert_eq!(back.items(), r.items());
        r.clear();
        assert!(r.items().is_empty() && r.dirty());
    }
    #[test]
    fn another_app_s_opens_survive_a_merge_and_bad_files_are_rejected() {
        // The file already holds Notes' open; Explorer's session replays.
        let mut disk = Recent::default();
        disk.apply(&Change::Record(
            "/users/1001/notes/desktop.txt".into(),
            Kind::File,
        ));
        let mut merged = Recent::decode(&disk.encode()).unwrap();
        let mut explorer = Recent {
            writable: true,
            ..Recent::default()
        };
        explorer.record("/users/1001/photo.jpg", Kind::File);
        explorer.remove("/users/1001/old.txt");
        for change in &explorer.changes {
            merged.apply(change);
        }
        let paths: Vec<_> = merged.items().iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            ["/users/1001/photo.jpg", "/users/1001/notes/desktop.txt"]
        );
        for bad in [
            &b"DOORRECENT2 1\n"[..],
            b"DOORRECENT1 1\n5 1 f /a\n",
            b"DOORRECENT1 3\n1 1 x /a\n",
            b"DOORRECENT1 3\n1 1 f rel\n",
        ] {
            assert!(Recent::decode(bad).is_err());
        }
        assert_eq!(profile_of("/users/1001/notes/a.txt"), Some("/users/1001"));
        assert_eq!(profile_of("/compat/Windows"), None);
        assert_eq!(profile_of("/users/"), None);
    }
    #[test]
    fn reopening_the_newest_item_changes_nothing_and_writes_nothing() {
        // Notes' reload (Ctrl+O, Don't save) re-records the open document,
        // which is already the newest item: `note_opened` must not write.
        let mut disk = Recent::default();
        disk.apply(&Change::Record(
            "/users/1001/notes/a.txt".into(),
            Kind::File,
        ));
        disk.apply(&Change::Record(
            "/users/1001/notes/desktop.txt".into(),
            Kind::File,
        ));
        let bytes = disk.encode();
        let mut notes = Recent::decode(&bytes).unwrap();
        notes.writable = true;
        notes.record("/users/1001/notes/desktop.txt", Kind::File);
        assert!(
            !notes.dirty(),
            "no change, so `save` returns before opening the file"
        );
        assert_eq!(notes.encode(), bytes, "no reorder, no counter bump");
        // A session that recorded it while stale replays onto a file where it
        // is already newest: the merged bytes equal the file, so no write.
        let mut stale = Recent {
            writable: true,
            ..Recent::default()
        };
        stale.record("/users/1001/notes/desktop.txt", Kind::File);
        let mut merged = Recent::decode(&bytes).unwrap();
        for change in &stale.changes {
            merged.apply(change);
        }
        assert_eq!(merged.encode(), bytes);
        // A distinct re-open (another item opened in between) still counts.
        notes.record("/users/1001/notes/a.txt", Kind::File);
        assert!(notes.dirty());
        assert_eq!(notes.items()[0].path, "/users/1001/notes/a.txt");
        assert_eq!(notes.items()[0].opens, 2);
        // The same path as a folder is a different item kind: recorded.
        let mut kinds = Recent::decode(&bytes).unwrap();
        kinds.record("/users/1001/notes/desktop.txt", Kind::Folder);
        assert!(kinds.dirty());
    }
}
