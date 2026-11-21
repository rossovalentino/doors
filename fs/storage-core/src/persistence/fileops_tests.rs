//! The file-operation engine and Recycle Bin on the real redo journal and
//! ext2 backend: persistence across remount, change notification, disk
//! full, and a power cut at every device write followed by journal
//! recovery, engine recovery and resume.
use super::*;
use crate::persistence::fileops::OpVolume;
use crate::storage::{Journal, WorkingSet};
use door_file_ops::{Bin, Engine, Mode, Policy, RecycleOutcome, State as OpState};
use std::string::String;
use std::vec::Vec;

type Disk = crate::storage::tests::Disk;
const U: u32 = 1001;

fn ops<'v, 'a, F: Vfs>(v: &'v mut PersistentVolume<'a, F>) -> OpVolume<'v, 'a, F> {
    OpVolume {
        volume: v,
        user: user(U),
        system: system(),
    }
}
fn write(v: &mut PersistentVolume<'_, impl Vfs>, path: &str, data: &[u8]) {
    let (file, _) = v
        .open(
            user(U),
            Path::Native(path),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&file, 0, data).unwrap();
    v.close(file).unwrap();
}
/// The user's tree below `root` with contents, as (relative path, data).
fn tree(v: &mut PersistentVolume<'_, impl Vfs>, root: &str) -> Vec<(String, Option<Vec<u8>>)> {
    let mut out = Vec::new();
    let mut stack = std::vec![String::from(root)];
    while let Some(dir) = stack.pop() {
        let mut after = 0;
        while let Some(e) = v
            .list(user(U), Path::Native(&dir), Context::root(), after)
            .unwrap()
        {
            after = e.info.object;
            let name = String::from_utf16(e.name.units()).unwrap();
            let path = std::format!("{dir}/{name}");
            if e.info.directory {
                out.push((path.clone(), None));
                stack.push(path);
            } else {
                let (file, _) = v
                    .open(
                        user(U),
                        Path::Native(&path),
                        Context::root(),
                        OpenOptions {
                            access: Access::READ,
                            share: Access::SHARE_ALL,
                            disposition: Disposition::OpenExisting,
                        },
                    )
                    .unwrap();
                let mut data = std::vec![0; e.info.bytes as usize];
                v.read(&file, 0, &mut data).unwrap();
                v.close(file).unwrap();
                out.push((path, Some(data)));
            }
        }
    }
    out.sort();
    out
}
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 13 % 251) as u8).collect()
}
/// Journaled volume with /users/1001/{src/{a.txt, tree/{b.bin, sub/c.txt}}, dst}.
fn baseline() -> Disk {
    let (home, log) = partitions();
    let mut disk = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v = PersistentVolume::initialize(
        Ext2::mount(journal).unwrap(),
        &mut state,
        &mut catalog,
        system(),
    )
    .unwrap();
    v.mkdir(
        system(),
        Path::Native("/users/1001"),
        Context::root(),
        Some(Acl::owner_full(U)),
    )
    .unwrap();
    for dir in ["src", "src/tree", "src/tree/sub", "dst"] {
        v.mkdir(
            user(U),
            Path::Native(&std::format!("/users/1001/{dir}")),
            Context::root(),
            None,
        )
        .unwrap();
    }
    write(&mut v, "/users/1001/src/a.txt", b"alpha");
    write(&mut v, "/users/1001/src/tree/b.bin", &pattern(9000));
    write(&mut v, "/users/1001/src/tree/sub/c.txt", b"gamma");
    v.commit().unwrap();
    disk
}
/// Mount `disk` (recovering the journal) and run `body`.
fn mounted<T>(
    disk: &mut Disk,
    body: impl FnOnce(&mut PersistentVolume<'_, Ext2<Journal<'_, &mut Disk>>>) -> T,
) -> T {
    let (home, log) = partitions();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (mut journal, _) = Journal::open(disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    v.set_clock(133_000_000_000_000_000);
    body(&mut v)
}
fn try_start(
    e: &mut Engine,
    o: &mut impl door_file_ops::Volume,
    mode: Mode,
) -> door_file_ops::Result<()> {
    let dst = if mode == Mode::Delete {
        ""
    } else {
        "/users/1001/dst"
    };
    e.begin(o, 5, U, mode, Policy::Ask, dst)?;
    e.add(o, 5, U, "/users/1001/src/a.txt")?;
    e.add(o, 5, U, "/users/1001/src/tree")
}
fn start(e: &mut Engine, o: &mut impl door_file_ops::Volume, mode: Mode) {
    try_start(e, o, mode).unwrap();
}
fn finish(e: &mut Engine, o: &mut impl door_file_ops::Volume) -> door_file_ops::Progress {
    for _ in 0..500 {
        let p = e.step(o, 5, U).unwrap();
        if !matches!(p.state, OpState::Running | OpState::Scanning) {
            return p;
        }
    }
    panic!("unfinished");
}

#[test]
fn copy_recycle_restore_survive_remount_with_change_notifications() {
    let mut disk = baseline();
    let src_tree = mounted(&mut disk, |v| tree(v, "/users/1001/src"));
    mounted(&mut disk, |v| {
        let dst = v
            .watch_target(user(U), Path::Native("/users/1001/dst"))
            .unwrap();
        // Another user cannot subscribe to (or learn about) this folder.
        assert_eq!(
            v.watch_target(user(1002), Path::Native("/users/1001/dst")),
            Err(Error::AccessDenied)
        );
        let seen = v.change_sequence();
        let mut e = Box::new(Engine::new());
        e.step_bytes = 4096;
        let mut o = ops(v);
        start(&mut e, &mut o, Mode::Copy);
        let p = finish(&mut e, &mut o);
        assert_eq!(p.state, OpState::Done);
        assert_eq!((p.items_done, p.bytes_done), (5, 9010));
        e.dismiss(&mut o, 5, U).unwrap();
        let mut hits = 0;
        v.changes_since(seen, |c| hits += usize::from(c == Change::Folder(dst)));
        assert!(hits > 0, "destination folder change observed");
        let bin = Bin {
            uid: U,
            limit: 1 << 20,
        };
        let mut o = ops(v);
        let RecycleOutcome::Recycled(id) = bin.recycle(&mut o, "/users/1001/src/tree").unwrap()
        else {
            panic!()
        };
        assert_eq!(bin.usage(&mut o).unwrap().items, 1);
        let _ = id;
    });
    // Durable across a remount: the copy, the recycled tree and its record.
    mounted(&mut disk, |v| {
        let copied: Vec<_> = tree(v, "/users/1001/dst")
            .into_iter()
            .map(|(p, d)| (p.replace("/users/1001/dst", ""), d))
            .collect();
        let original: Vec<_> = src_tree
            .iter()
            .map(|(p, d)| (p.replace("/users/1001/src", ""), d.clone()))
            .collect();
        assert_eq!(copied, original);
        assert!(
            tree(v, "/users/1001/src")
                .iter()
                .all(|(p, _)| p.ends_with("a.txt"))
        );
        // The bin is hidden; the item keeps its original path and time.
        let bin_info = v
            .metadata(
                user(U),
                Path::Native("/users/1001/$Recycle.Bin"),
                Context::root(),
            )
            .unwrap();
        assert_eq!(bin_info.attributes & 2, 2);
        assert!(
            v.metadata(
                user(1002),
                Path::Native("/users/1001/$Recycle.Bin"),
                Context::root()
            )
            .is_err()
        );
        let bin = Bin {
            uid: U,
            limit: 1 << 20,
        };
        let mut o = ops(v);
        let info = bin.next(&mut o, 0).unwrap().unwrap();
        assert_eq!(info.original.as_str(), "/users/1001/src/tree");
        assert_eq!(info.deleted, 133_000_000_000_000_000);
        assert_eq!((info.items, info.bytes, info.directory), (4, 9005, true));
        bin.restore(&mut o, info.id).unwrap();
    });
    mounted(&mut disk, |v| {
        assert_eq!(tree(v, "/users/1001/src"), src_tree);
        let mut o = ops(v);
        assert_eq!(
            Bin {
                uid: U,
                limit: 1 << 20
            }
            .usage(&mut o)
            .unwrap()
            .items,
            0
        );
    });
}

#[test]
fn disk_full_copy_fails_without_a_partial_file_and_the_owner_stays_usable() {
    let mut disk = baseline();
    mounted(&mut disk, |v| {
        write(v, "/users/1001/src/huge.bin", &pattern(40_000));
        v.commit().unwrap();
        let mut e = Box::new(Engine::new());
        e.step_bytes = 16384;
        let mut o = ops(v);
        e.begin(&mut o, 9, U, Mode::Copy, Policy::Ask, "/users/1001/dst")
            .unwrap();
        e.add(&mut o, 9, U, "/users/1001/src/huge.bin").unwrap();
        let mut last = None;
        for _ in 0..50 {
            let p = e.step(&mut o, 9, U).unwrap();
            last = Some(p);
            if !matches!(p.state, OpState::Running | OpState::Scanning) {
                break;
            }
        }
        let p = last.unwrap();
        assert_eq!(p.state, OpState::Failed);
        assert_eq!(p.last_error, Some(door_file_ops::Error::NoSpace));
        assert!(
            tree(v, "/users/1001/dst").is_empty(),
            "no partial or temporary file"
        );
        // Still usable: an ordinary mutation commits.
        write(v, "/users/1001/dst/ok.txt", b"ok");
        v.commit().unwrap();
    });
    mounted(&mut disk, |v| {
        assert_eq!(tree(v, "/users/1001/dst").len(), 1);
    });
}

/// Power cut at every device write of a copy, move and delete, with and
/// without a torn sector; journal recovery, engine recovery and resume end
/// in exactly the uninterrupted result.
#[test]
fn power_cut_at_every_write_recovers_and_resumes_identically() {
    for mode in [Mode::Copy, Mode::Move, Mode::Delete] {
        let base = baseline();
        let mut clean = base.clone();
        let expected = mounted(&mut clean, |v| {
            let mut e = Box::new(Engine::new());
            e.step_bytes = 4096;
            let mut o = ops(v);
            start(&mut e, &mut o, mode);
            assert_eq!(finish(&mut e, &mut o).state, OpState::Done);
            (tree(v, "/users/1001"), v.usage())
        });
        let mut cut = 0;
        loop {
            let mut disk = base.clone();
            let torn = cut % 2 == 1;
            disk.power_cut_after(Some(cut), torn);
            let completed = mounted(&mut disk, |v| {
                let mut e = Box::new(Engine::new());
                e.step_bytes = 4096;
                let mut o = ops(v);
                if let Err(err) = try_start(&mut e, &mut o, mode) {
                    assert!(err.fatal(), "{mode:?} cut {cut}: {err:?}");
                    return false;
                }
                for _ in 0..500 {
                    match e.step(&mut o, 5, U) {
                        Ok(p) if p.state == OpState::Done => return true,
                        Ok(_) => (),
                        Err(err) => {
                            assert!(err.fatal(), "{mode:?} cut {cut}: {err:?}");
                            return false;
                        }
                    }
                }
                panic!("unfinished");
            });
            disk.power_cut_after(None, false);
            if completed {
                break;
            }
            let state = mounted(&mut disk, |v| {
                let mut e = Box::new(Engine::new());
                e.step_bytes = 4096;
                let mut o = ops(v);
                e.recover(&mut o).unwrap();
                match e.query(&mut o, 5, U) {
                    // Cut before the operation record became durable.
                    Err(door_file_ops::Error::NotFound) => {
                        start(&mut e, &mut o, mode);
                    }
                    Ok(p) if p.state == OpState::Paused => e.resume(&mut o, 5, U).unwrap(),
                    Ok(p) => assert!(
                        matches!(p.state, OpState::Done | OpState::Scanning),
                        "{mode:?} cut {cut}: {p:?}"
                    ),
                    Err(err) => panic!("{mode:?} cut {cut}: {err:?}"),
                }
                let p = finish(&mut e, &mut o);
                assert_eq!(p.state, OpState::Done, "{mode:?} cut {cut}");
                (tree(v, "/users/1001"), v.usage())
            });
            assert_eq!(state, expected, "{mode:?} cut {cut}");
            cut += 1;
            assert!(cut < 5000);
        }
        assert!(cut > 10, "{mode:?}: the run spans many device writes");
    }
}

#[test]
fn credential_folder_is_created_once_and_private_to_the_session_service() {
    let session = unsafe { Subject::authenticated(SESSION_UID, 0, 2).unwrap() };
    let mut disk = baseline();
    mounted(&mut disk, |v| {
        assert_eq!(v.ensure_credentials(user(U)), Err(Error::AccessDenied));
        assert_eq!(v.ensure_credentials(system()), Ok(true));
        assert_eq!(v.ensure_credentials(system()), Ok(false));
        v.commit().unwrap();
    });
    // Durable, and idempotent on every later mount (format migration).
    mounted(&mut disk, |v| {
        assert_eq!(v.ensure_credentials(system()), Ok(false));
        let path = std::format!("{CREDENTIALS_FOLDER}/1001.cred");
        for denied in [user(U), user(1002)] {
            assert_eq!(
                v.metadata(denied, Path::Native(CREDENTIALS_FOLDER), Context::root())
                    .err(),
                Some(Error::AccessDenied)
            );
            assert!(
                v.open(
                    denied,
                    Path::Native(&path),
                    Context::root(),
                    options(Disposition::CreateNew)
                )
                .is_err()
            );
            assert!(
                v.list(denied, Path::Native(CREDENTIALS_FOLDER), Context::root(), 0)
                    .is_err()
            );
        }
        let (file, created) = v
            .open(
                session,
                Path::Native(&path),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        assert!(created);
        v.write(&file, 0, b"argon2id record").unwrap();
        v.close(file).unwrap();
        v.commit().unwrap();
        assert!(
            v.list(
                session,
                Path::Native(CREDENTIALS_FOLDER),
                Context::root(),
                0
            )
            .unwrap()
            .is_some()
        );
        // A user cannot read the record even knowing its name.
        assert!(
            v.open(
                user(U),
                Path::Native(&path),
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    share: Access::SHARE_ALL,
                    disposition: Disposition::OpenExisting,
                },
            )
            .is_err()
        );
    });
}

#[test]
fn change_log_reports_folder_and_settings_changes_and_overflow() {
    let mut disk = baseline();
    mounted(&mut disk, |v| {
        let src = v
            .watch_target(user(U), Path::Native("/users/1001/src"))
            .unwrap();
        let dst = v
            .watch_target(user(U), Path::Native("/users/1001/dst"))
            .unwrap();
        let seen = v.change_sequence();
        let collect = |v: &PersistentVolume<'_, _>, seen| {
            let mut out = Vec::new();
            let complete = v.changes_since(seen, |c| out.push(c));
            (complete, out)
        };
        write(v, "/users/1001/dst/n.txt", b"x");
        v.rename_path(
            user(U),
            Path::Native("/users/1001/src/a.txt"),
            Path::Native("/users/1001/dst/a.txt"),
            Context::root(),
        )
        .unwrap();
        v.setting_set(user(U), U, Setting::Theme, 1).unwrap();
        let (complete, changes) = collect(v, seen);
        assert!(complete);
        assert!(changes.contains(&Change::Folder(dst)));
        assert!(changes.contains(&Change::Folder(src)));
        assert!(changes.contains(&Change::Settings(U)));
        // A change inside a subfolder also reaches the parent folder's
        // subscribers (its last-write time changed).
        let home = v
            .watch_target(user(U), Path::Native("/users/1001"))
            .unwrap();
        assert!(changes.contains(&Change::Folder(home)));
        // Consecutive identical changes coalesce; nothing new is empty.
        let now = v.change_sequence();
        assert_eq!(collect(v, now), (true, Vec::new()));
        // More than the ring holds: the subscriber must rescan.
        let (file, _) = v
            .open(
                user(U),
                Path::Native("/users/1001/dst/n.txt"),
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        for i in 0..40u32 {
            v.write(&file, 0, b"y").unwrap();
            v.setting_set(user(U), U, Setting::Theme, i % 3).unwrap();
        }
        v.close(file).unwrap();
        assert!(!collect(v, now).0);
        // Removal is visible to the folder's subscription as "gone".
        assert!(v.object_alive(dst));
        assert_eq!(v.watch_allowed(user(1002), dst), Err(Error::AccessDenied));
        v.watch_allowed(user(U), dst).unwrap();
    });
}
