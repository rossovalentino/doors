//! The three-way sync planner. Each item is compared on three sides: the
//! local folder now, the server now, and the index (both sides at the last
//! successful sync). A side "changed" when it differs from the index; the
//! rules then follow OneDrive's:
//!
//! - one side changed: that side's version goes to the other;
//! - both changed (or both new with different contents): conflict, both
//!   kept: the local file moves aside as `name-DEVICE.ext` and uploads
//!   under that name, the server's version downloads;
//! - deleted on one side and unchanged on the other: deleted there too;
//!   deleted on one side and changed on the other: the change wins and
//!   the item comes back;
//! - never synced and on one side only: copied to the other side.
//!
//! An index that was lost reads as empty: then nothing is ever deleted, and
//! files on both sides are compared byte for byte (`Compare`) before a
//! conflict copy is made. Server names Windows cannot hold, and names
//! that differ only in case, are left alone and reported.
use crate::{index::Known, xml::Entry};
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

/// A local item, from a walk of the sync folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub path: String,
    pub folder: bool,
    pub size: u64,
    /// Change stamp: the file's Storage version (bumped by every write).
    pub stamp: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// A server name Windows forbids (`<>:"/\|?*`, a trailing dot or
    /// space, a device name such as CON or LPT1).
    WindowsName,
    /// A second server name equal to another but for case.
    CaseClash,
    /// A file on one side and a folder of the same name on the other.
    KindClash,
    /// A server file larger than this PC's volume holds (`max_file`).
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    MakeLocal(String),
    MakeRemote(String),
    Upload(String),
    Download(String),
    /// Both sides hold a file the index does not know, with the same size:
    /// download to compare; equal bytes only record it, different bytes
    /// become a conflict with `copy` (named in advance like a conflict's).
    Compare { path: String, copy: String },
    /// Both sides changed: keep both (see the module comment).
    Conflict {
        path: String,
        copy: String,
    },
    /// Both sides agree; only the index entry is (re)written.
    Record(String),
    /// Both sides deleted: the index entry goes.
    Forget(String),
    DeleteLocal {
        path: String,
        folder: bool,
    },
    DeleteRemote {
        path: String,
        folder: bool,
    },
    Skip {
        path: String,
        reason: Skip,
    },
}
impl Action {
    pub fn path(&self) -> &str {
        match self {
            Action::MakeLocal(p)
            | Action::MakeRemote(p)
            | Action::Upload(p)
            | Action::Download(p)
            | Action::Record(p)
            | Action::Forget(p) => p,
            Action::Conflict { path, .. }
            | Action::Compare { path, .. }
            | Action::DeleteLocal { path, .. }
            | Action::DeleteRemote { path, .. }
            | Action::Skip { path, .. } => path,
        }
    }
    fn rank(&self) -> u8 {
        match self {
            Action::Skip { .. } => 0,
            Action::MakeLocal(_) | Action::MakeRemote(_) => 1,
            Action::Record(_) | Action::Forget(_) => 2,
            Action::Compare { .. } | Action::Conflict { .. } => 3,
            Action::Upload(_) | Action::Download(_) => 4,
            Action::DeleteLocal { .. } | Action::DeleteRemote { .. } => 5,
        }
    }
}

/// The server's identity of an item's version: its entity tag, or its
/// size and time where a server sends no tag.
pub fn remote_tag(e: &Entry) -> String {
    if e.etag.is_empty() {
        format!("{}-{}", e.length, e.modified.unwrap_or(0))
    } else {
        e.etag.clone()
    }
}
/// A name Windows can store (one path segment).
pub fn windows_name(name: &str) -> bool {
    const DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    if name.is_empty()
        || name.ends_with('.')
        || name.ends_with(' ')
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").trim_end();
    let upper = stem.to_ascii_uppercase();
    let numbered = |prefix: &str| {
        upper
            .strip_prefix(prefix)
            .is_some_and(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0")
    };
    !(DEVICES.contains(&upper.as_str()) || numbered("COM") || numbered("LPT"))
}
/// Files NoneDrive never syncs, as OneDrive: Explorer's folder settings,
/// thumbnail caches and Office / LibreOffice lock files.
pub fn ignored(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "desktop.ini"
        || lower == "thumbs.db"
        || lower.starts_with("~$")
        || (lower.starts_with(".~lock.") && lower.ends_with('#'))
}
fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn depth(path: &str) -> usize {
    path.matches('/').count()
}
/// `parent/child` relation (strict).
fn under(path: &str, folder: &str) -> bool {
    path.len() > folder.len()
        && path.as_bytes()[folder.len()] == b'/'
        && path[..folder.len()].eq_ignore_ascii_case(folder)
}
/// `name-DEVICE.ext`, numbered if that is taken (case-insensitively).
pub fn conflict_name(path: &str, device: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let (folder, name) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{folder}{stem}-{device}{ext}")
        } else {
            format!("{folder}{stem}-{device}-{n}{ext}")
        };
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[derive(Default)]
struct Slot<'a> {
    local: Option<&'a Local>,
    remote: Option<&'a Entry>,
    known: Option<&'a Known>,
}

/// Every action that brings the two sides together, in execution order:
/// skips (reported), folder creations (parents first), index updates,
/// comparisons and conflicts, transfers, then deletions (children first,
/// a folder's contents with it). Server files over `max_file` bytes are
/// skipped and reported, and nothing else is done with their names (a
/// skipped file is never taken for a deleted one).
pub fn plan(
    local: &[Local],
    remote: &[Entry],
    known: &[Known],
    device: &str,
    max_file: u64,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut slots: BTreeMap<String, Slot> = BTreeMap::new();
    let mut blocked: alloc::collections::BTreeSet<String> = Default::default();
    for l in local {
        if l.path.is_empty() || ignored(name_of(&l.path)) {
            continue;
        }
        slots.entry(l.path.to_lowercase()).or_default().local = Some(l);
    }
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    for r in remote {
        if r.path.is_empty() || ignored(name_of(&r.path)) {
            continue;
        }
        if !r.path.split('/').all(windows_name) {
            actions.push(Action::Skip {
                path: r.path.clone(),
                reason: Skip::WindowsName,
            });
            continue;
        }
        let key = r.path.to_lowercase();
        if seen.insert(key.clone(), &r.path).is_some() {
            actions.push(Action::Skip {
                path: r.path.clone(),
                reason: Skip::CaseClash,
            });
            continue;
        }
        if !r.folder && r.length > max_file {
            actions.push(Action::Skip {
                path: r.path.clone(),
                reason: Skip::TooLarge,
            });
            blocked.insert(key.clone());
        }
        slots.entry(key).or_default().remote = Some(r);
    }
    for k in known {
        slots.entry(k.path.to_lowercase()).or_default().known = Some(k);
    }
    // Every path either side has (conflict copies must not collide).
    let taken = |p: &str| slots.contains_key(&p.to_lowercase());
    for (key, slot) in &slots {
        if blocked.contains(key) {
            continue;
        }
        if let Some(a) = decide(slot, device, &taken) {
            actions.push(a);
        }
    }
    reconcile_folders(&mut actions);
    actions.sort_by(|a, b| {
        a.rank().cmp(&b.rank()).then_with(|| {
            let (da, db) = (depth(a.path()), depth(b.path()));
            if a.rank() == 5 {
                db.cmp(&da)
            } else {
                da.cmp(&db)
            }
            .then_with(|| a.path().cmp(b.path()))
        })
    });
    actions
}

fn decide(s: &Slot, device: &str, taken: &dyn Fn(&str) -> bool) -> Option<Action> {
    let local_changed = |l: &Local, k: &Known| {
        l.folder != k.folder || (!l.folder && (l.size != k.size || l.stamp != k.stamp))
    };
    let remote_changed =
        |r: &Entry, k: &Known| r.folder != k.folder || (!r.folder && remote_tag(r) != k.etag);
    Some(match (s.local, s.remote, s.known) {
        (Some(l), Some(r), known) => {
            let path = r.path.clone();
            if l.folder != r.folder {
                return Some(Action::Skip {
                    path,
                    reason: Skip::KindClash,
                });
            }
            if l.folder {
                return known.is_none().then_some(Action::Record(path));
            }
            match known {
                Some(k) => match (local_changed(l, k), remote_changed(r, k)) {
                    (false, false) => return None,
                    (true, false) => Action::Upload(path),
                    (false, true) => Action::Download(path),
                    (true, true) => Action::Conflict {
                        copy: conflict_name(&l.path, device, taken),
                        path,
                    },
                },
                None if l.size == r.length => Action::Compare {
                    copy: conflict_name(&l.path, device, taken),
                    path,
                },
                None => Action::Conflict {
                    copy: conflict_name(&l.path, device, taken),
                    path,
                },
            }
        }
        (Some(l), None, Some(k)) => {
            // Deleted on the server.
            if local_changed(l, k) {
                if l.folder {
                    Action::MakeRemote(l.path.clone())
                } else {
                    Action::Upload(l.path.clone())
                }
            } else {
                Action::DeleteLocal {
                    path: l.path.clone(),
                    folder: l.folder,
                }
            }
        }
        (Some(l), None, None) => {
            if l.folder {
                Action::MakeRemote(l.path.clone())
            } else {
                Action::Upload(l.path.clone())
            }
        }
        (None, Some(r), Some(k)) => {
            // Deleted locally.
            if remote_changed(r, k) {
                if r.folder {
                    Action::MakeLocal(r.path.clone())
                } else {
                    Action::Download(r.path.clone())
                }
            } else {
                Action::DeleteRemote {
                    path: r.path.clone(),
                    folder: r.folder,
                }
            }
        }
        (None, Some(r), None) => {
            if r.folder {
                Action::MakeLocal(r.path.clone())
            } else {
                Action::Download(r.path.clone())
            }
        }
        (None, None, Some(k)) => Action::Forget(k.path.clone()),
        (None, None, None) => return None,
    })
}

/// A folder deleted on one side whose contents changed on the other comes
/// back (its changed contents need it); a deleted folder's own contents
/// go with it rather than one by one.
fn reconcile_folders(actions: &mut Vec<Action>) {
    let deleted_local: Vec<String> = folders(actions, true);
    let deleted_remote: Vec<String> = folders(actions, false);
    // Contents that keep a local folder: anything downloaded or created in
    // it; a remote one: anything uploaded or created there.
    let keeps_local = |f: &str| {
        actions.iter().any(|a| {
            matches!(
                a,
                Action::Download(_)
                    | Action::MakeLocal(_)
                    | Action::Compare { .. }
                    | Action::Conflict { .. }
                    | Action::Upload(_)
                    | Action::MakeRemote(_)
                    | Action::Skip { .. }
            ) && under(a.path(), f)
        })
    };
    let revived_local: Vec<String> = deleted_local
        .iter()
        .filter(|f| keeps_local(f))
        .cloned()
        .collect();
    let revived_remote: Vec<String> = deleted_remote
        .iter()
        .filter(|f| keeps_local(f))
        .cloned()
        .collect();
    for a in actions.iter_mut() {
        match a {
            // Kept locally: the server must have it again.
            Action::DeleteLocal { path, folder: true } if revived_local.contains(path) => {
                *a = Action::MakeRemote(path.clone());
            }
            Action::DeleteRemote { path, folder: true } if revived_remote.contains(path) => {
                *a = Action::MakeLocal(path.clone());
            }
            _ => (),
        }
    }
    // Contents of a folder still being deleted go with it; their index
    // entries are dropped by the folder's deletion.
    let gone_local: Vec<String> = folders(actions, true);
    let gone_remote: Vec<String> = folders(actions, false);
    actions.retain(|a| match a {
        Action::DeleteLocal { path, .. } => !gone_local.iter().any(|f| under(path, f)),
        Action::DeleteRemote { path, .. } => !gone_remote.iter().any(|f| under(path, f)),
        _ => true,
    });
}
fn folders(actions: &[Action], local: bool) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::DeleteLocal { path, folder: true } if local => Some(path.clone()),
            Action::DeleteRemote { path, folder: true } if !local => Some(path.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn l(path: &str, size: u64, stamp: u64) -> Local {
        Local {
            path: path.into(),
            folder: false,
            size,
            stamp,
        }
    }
    fn ld(path: &str) -> Local {
        Local {
            path: path.into(),
            folder: true,
            size: 0,
            stamp: 0,
        }
    }
    fn r(path: &str, etag: &str, length: u64) -> Entry {
        Entry {
            path: path.into(),
            folder: false,
            etag: etag.into(),
            length,
            modified: None,
        }
    }
    fn rd(path: &str) -> Entry {
        Entry {
            path: path.into(),
            folder: true,
            etag: "\"d\"".into(),
            length: 0,
            modified: None,
        }
    }
    fn k(path: &str, size: u64, stamp: u64, etag: &str) -> Known {
        Known {
            path: path.into(),
            folder: false,
            size,
            stamp,
            etag: etag.into(),
        }
    }
    fn kd(path: &str) -> Known {
        Known {
            path: path.into(),
            folder: true,
            size: 0,
            stamp: 0,
            etag: String::new(),
        }
    }
    #[test]
    fn one_side_changed_goes_to_the_other() {
        let known = [k("a.txt", 1, 10, "\"1\""), k("b.txt", 2, 20, "\"2\"")];
        let local = [l("a.txt", 5, 11), l("b.txt", 2, 20), l("c.txt", 3, 30)];
        let remote = [
            r("a.txt", "\"1\"", 1),
            r("b.txt", "\"2b\"", 9),
            r("d.txt", "\"4\"", 4),
        ];
        let p = plan(&local, &remote, &known, "PC", u64::MAX);
        assert_eq!(
            p,
            vec![
                Action::Upload("a.txt".into()),
                Action::Download("b.txt".into()),
                Action::Upload("c.txt".into()),
                Action::Download("d.txt".into()),
            ]
        );
    }
    fn sorted(mut v: Vec<Action>) -> Vec<Action> {
        v.sort_by(|a, b| a.path().cmp(b.path()));
        v
    }
    #[test]
    fn both_changed_keeps_both() {
        let known = [k("Docs/report.docx", 1, 10, "\"1\"")];
        let local = [ld("Docs"), l("Docs/report.docx", 2, 11)];
        let remote = [rd("Docs"), r("Docs/report.docx", "\"2\"", 3)];
        let p = plan(&local, &remote, &known, "LAPTOP", u64::MAX);
        assert_eq!(
            p,
            vec![
                Action::Record("Docs".into()),
                Action::Conflict {
                    path: "Docs/report.docx".into(),
                    copy: "Docs/report-LAPTOP.docx".into()
                }
            ]
        );
        // The copy name avoids existing names.
        let local = [
            l("a.txt", 2, 11),
            l("a-PC.txt", 1, 1),
            l(".profile", 1, 1),
        ];
        let remote = [r("a.txt", "\"2\"", 3), r(".profile", "\"p\"", 9)];
        let p = plan(&local, &remote, &[k("a.txt", 1, 10, "\"1\"")], "PC", u64::MAX);
        assert!(p.contains(&Action::Conflict {
            path: "a.txt".into(),
            copy: "a-PC-2.txt".into()
        }));
        // Never synced, different sizes: a conflict; same size: compare.
        assert!(p.contains(&Action::Conflict {
            path: ".profile".into(),
            copy: ".profile-PC".into()
        }));
        let p = plan(&[l("x", 5, 1)], &[r("x", "\"e\"", 5)], &[], "PC", u64::MAX);
        assert_eq!(
            p,
            vec![Action::Compare {
                path: "x".into(),
                copy: "x-PC".into()
            }]
        );
    }
    #[test]
    fn deletions_follow_unless_the_other_side_changed() {
        let known = [
            k("gone-here.txt", 1, 1, "\"1\""),
            k("gone-there.txt", 2, 2, "\"2\""),
            k("edited-there.txt", 3, 3, "\"3\""),
            k("edited-here.txt", 4, 4, "\"4\""),
            k("both.txt", 5, 5, "\"5\""),
        ];
        let local = [l("gone-there.txt", 2, 2), l("edited-here.txt", 9, 9)];
        let remote = [
            r("gone-here.txt", "\"1\"", 1),
            r("edited-there.txt", "\"3b\"", 3),
        ];
        let p = sorted(plan(&local, &remote, &known, "PC", u64::MAX));
        assert_eq!(
            p,
            vec![
                Action::Forget("both.txt".into()),
                Action::Upload("edited-here.txt".into()),
                Action::Download("edited-there.txt".into()),
                Action::DeleteRemote {
                    path: "gone-here.txt".into(),
                    folder: false
                },
                Action::DeleteLocal {
                    path: "gone-there.txt".into(),
                    folder: false
                },
            ]
        );
        // A lost index never deletes anything.
        let p = plan(&local, &remote, &[], "PC", u64::MAX);
        assert!(!p.iter().any(|a| matches!(
            a,
            Action::DeleteLocal { .. } | Action::DeleteRemote { .. }
        )));
    }
    #[test]
    fn folders_are_created_parents_first_and_deleted_whole() {
        let p = plan(
            &[ld("A"), ld("A/B"), l("A/B/f.txt", 1, 1)],
            &[],
            &[],
            "PC",
        );
        assert_eq!(
            p,
            vec![
                Action::MakeRemote("A".into()),
                Action::MakeRemote("A/B".into()),
                Action::Upload("A/B/f.txt".into()),
            ]
        );
        // Deleted locally, unchanged on the server: one recursive delete.
        let known = [kd("A"), kd("A/B"), k("A/B/f.txt", 1, 1, "\"f\"")];
        let remote = [rd("A"), rd("A/B"), r("A/B/f.txt", "\"f\"", 1)];
        let p = plan(&[], &remote, &known, "PC", u64::MAX);
        assert_eq!(
            p,
            vec![Action::DeleteRemote {
                path: "A".into(),
                folder: true
            }]
        );
        // Deleted locally while a file in it changed on the server: the
        // folder comes back with the file.
        let remote = [rd("A"), rd("A/B"), r("A/B/f.txt", "\"f2\"", 2)];
        let p = plan(&[], &remote, &known, "PC", u64::MAX);
        assert_eq!(
            p,
            vec![
                Action::MakeLocal("A".into()),
                Action::MakeLocal("A/B".into()),
                Action::Download("A/B/f.txt".into()),
            ]
        );
    }
    #[test]
    fn unusable_names_are_skipped_and_reported() {
        let remote = [
            r("CON.txt", "\"1\"", 1),
            r("a:b", "\"2\"", 1),
            r("ok.", "\"3\"", 1),
            r("Same.txt", "\"4\"", 1),
            r("same.TXT", "\"5\"", 1),
            r("desktop.ini", "\"6\"", 1),
            r("~$draft.docx", "\"7\"", 1),
        ];
        let p = plan(&[], &remote, &[], "PC", u64::MAX);
        let skipped: Vec<(&str, Skip)> = p
            .iter()
            .filter_map(|a| match a {
                Action::Skip { path, reason } => Some((path.as_str(), *reason)),
                _ => None,
            })
            .collect();
        assert_eq!(
            skipped,
            vec![
                ("CON.txt", Skip::WindowsName),
                ("a:b", Skip::WindowsName),
                ("ok.", Skip::WindowsName),
                ("same.TXT", Skip::CaseClash),
            ]
        );
        assert!(p.contains(&Action::Download("Same.txt".into())));
        assert_eq!(p.len(), 5);
        assert!(windows_name("COM10") && windows_name("console.log"));
        assert!(!windows_name("lpt3.txt") && !windows_name("nul"));
        // A file here and a folder there.
        let p = plan(&[l("x", 1, 1)], &[rd("x")], &[], "PC", u64::MAX);
        assert_eq!(
            p,
            vec![Action::Skip {
                path: "x".into(),
                reason: Skip::KindClash
            }]
        );
    }
    #[test]
    fn files_too_large_for_the_volume_are_skipped_never_deleted() {
        let known = [kd("A"), k("A/big.iso", 1, 1, "\"b\""), k("A/s.txt", 1, 1, "\"s\"")];
        let remote = [rd("A"), r("A/big.iso", "\"b2\"", 9_000_000), r("A/s.txt", "\"s\"", 1)];
        // The folder was deleted here: the server keeps the big file, so the
        // folder is not deleted there and comes back here.
        let p = plan(&[], &remote, &known, "PC", 4_243_456);
        assert_eq!(
            p,
            vec![
                Action::Skip {
                    path: "A/big.iso".into(),
                    reason: Skip::TooLarge
                },
                Action::MakeLocal("A".into()),
                Action::DeleteRemote {
                    path: "A/s.txt".into(),
                    folder: false
                },
            ]
        );
    }
    #[test]
    fn servers_without_etags_use_size_and_time() {
        let mut e = r("a", "", 7);
        e.modified = Some(100);
        assert_eq!(remote_tag(&e), "7-100");
        let known = [k("a", 7, 1, "7-100")];
        assert!(plan(&[l("a", 7, 1)], &[e.clone()], &known, "PC", u64::MAX).is_empty());
        e.modified = Some(101);
        assert_eq!(
            plan(&[l("a", 7, 1)], &[e], &known, "PC", u64::MAX),
            vec![Action::Download("a".into())]
        );
    }
}
