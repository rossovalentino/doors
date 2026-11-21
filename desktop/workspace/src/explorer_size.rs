//! Bounded, cancellable recursive folder size. The UI advances it a few
//! listing calls per tick; results are cached by the root's (object, version).
//! Each filesystem object is counted once (hard links, aliases such as the
//! C:\Users junction), so cycles cannot recurse forever.
extern crate alloc;
use alloc::{collections::BTreeSet, string::String, vec::Vec};

/// Hard bounds: entries visited and directory depth.
pub const MAX_ENTRIES: u64 = 20_000;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Calculating,
    Complete,
    /// Stopped at a bound; totals are a lower limit.
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Totals {
    /// Logical bytes of regular files (allocation size is not reported by
    /// the volume, so "size on disk" stays unavailable).
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
    /// Subfolders that could not be listed (denied, removed, I/O).
    pub unreadable: u64,
    pub state: State,
}

/// One child reported by a directory listing.
#[derive(Clone, Debug)]
pub struct Child {
    pub path: String,
    pub object: u64,
    pub directory: bool,
    pub bytes: u64,
}

pub struct Scan {
    root: u64,
    pending: Vec<(String, usize)>,
    seen: BTreeSet<u64>,
    totals: Totals,
}
impl Scan {
    pub fn new(path: String, root_object: u64) -> Self {
        let mut seen = BTreeSet::new();
        seen.insert(root_object);
        Self {
            root: root_object,
            pending: alloc::vec![(path, 0)],
            seen,
            totals: Totals {
                bytes: 0,
                files: 0,
                folders: 0,
                unreadable: 0,
                state: State::Calculating,
            },
        }
    }
    pub fn root(&self) -> u64 {
        self.root
    }
    pub fn totals(&self) -> Totals {
        self.totals
    }
    /// The next directory to list, if the scan is still running. Not an
    /// iterator: the scan also ends on cancellation and reports totals.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<(String, usize)> {
        if self.totals.state != State::Calculating {
            return None;
        }
        let next = self.pending.pop();
        if next.is_none() {
            self.totals.state = State::Complete;
        }
        next
    }
    /// A listing stopped at its per-directory bound: totals become a lower limit.
    pub fn mark_partial(&mut self) {
        self.totals.state = State::Partial;
    }
    /// Records one directory's listing (`Err` when it could not be read).
    pub fn record(&mut self, depth: usize, listing: Result<Vec<Child>, ()>) {
        let Ok(children) = listing else {
            self.totals.unreadable += 1;
            return;
        };
        for child in children {
            if self.totals.files + self.totals.folders >= MAX_ENTRIES {
                self.totals.state = State::Partial;
                return;
            }
            if !self.seen.insert(child.object) {
                continue;
            }
            if child.directory {
                self.totals.folders += 1;
                if depth + 1 >= MAX_DEPTH {
                    self.totals.state = State::Partial;
                } else {
                    self.pending.push((child.path, depth + 1));
                }
            } else {
                self.totals.files += 1;
                self.totals.bytes = self.totals.bytes.saturating_add(child.bytes);
            }
        }
        if self.pending.is_empty() && self.totals.state == State::Calculating {
            self.totals.state = State::Complete;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::ToString, vec};
    fn dir(path: &str, object: u64) -> Child {
        Child {
            path: path.into(),
            object,
            directory: true,
            bytes: 0,
        }
    }
    fn file(object: u64, bytes: u64) -> Child {
        Child {
            path: format!("/f{object}"),
            object,
            directory: false,
            bytes,
        }
    }
    #[test]
    fn totals_count_each_object_once_and_finish() {
        let mut scan = Scan::new("/root".into(), 1);
        assert_eq!(scan.next(), Some(("/root".to_string(), 0)));
        // A hard link (object 5) and an alias back to the root (object 1).
        scan.record(
            0,
            Ok(vec![
                file(5, 100),
                file(5, 100),
                dir("/root/a", 2),
                dir("/root/self", 1),
            ]),
        );
        assert_eq!(scan.next(), Some(("/root/a".to_string(), 1)));
        scan.record(1, Ok(vec![file(6, 50), dir("/root/a/b", 3)]));
        let (path, depth) = scan.next().unwrap();
        scan.record(depth, Err(()));
        assert_eq!(path, "/root/a/b");
        assert_eq!(scan.next(), None);
        let t = scan.totals();
        assert_eq!((t.bytes, t.files, t.folders, t.unreadable), (150, 2, 2, 1));
        assert_eq!(t.state, State::Complete);
    }
    #[test]
    fn bounds_stop_as_partial() {
        let mut scan = Scan::new("/".into(), 0);
        scan.next();
        let many: Vec<Child> = (1..=MAX_ENTRIES + 10).map(|o| file(o, 1)).collect();
        scan.record(0, Ok(many));
        assert_eq!(scan.totals().state, State::Partial);
        assert_eq!(scan.totals().files, MAX_ENTRIES);
        assert_eq!(scan.next(), None, "a stopped scan lists nothing more");
        let mut deep = Scan::new("/".into(), 0);
        for depth in 0..MAX_DEPTH {
            let Some((_, d)) = deep.next() else { break };
            assert_eq!(d, depth);
            deep.record(d, Ok(vec![dir("/x", depth as u64 + 1)]));
        }
        assert_eq!(deep.totals().state, State::Partial);
    }
}
