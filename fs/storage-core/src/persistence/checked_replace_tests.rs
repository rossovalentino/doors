use super::*;
use crate::storage::{Checkpoint, Journal, WorkingSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

type Volume<'a> = PersistentVolume<'a, Ext2<Journal<'a, &'a mut crate::storage::tests::Disk>>>;
const PATH: Path<'_> = Path::Native("/users/1001/document.txt");
fn mounted<T>(
    disk: &mut crate::storage::tests::Disk,
    test: impl FnOnce(&mut Volume<'_>) -> T,
) -> T {
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let (home, log) = partitions();
    let (mut journal, _) = Journal::open(disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut volume =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    test(&mut volume)
}
fn baseline() -> (crate::storage::tests::Disk, FileInfo) {
    baseline_with(false)
}
fn baseline_with(production: bool) -> (crate::storage::tests::Disk, FileInfo) {
    let mut disk = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = if production {
        State::boxed_production()
    } else {
        State::boxed()
    };
    let mut catalog = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let (home, log) = partitions();
    let original;
    {
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
            Some(Acl::owner_full(1001)),
        )
        .unwrap();
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&file, 0, &[0x71; REPLACE_BYTES]).unwrap();
        original = v.info(&file).unwrap();
        v.close(file).unwrap();
        v.commit().unwrap();
    }
    (disk, original)
}
fn read(v: &mut Volume<'_>, file: &FileHandle, expected: &[u8]) {
    let mut bytes = [0; REPLACE_BYTES + 1];
    let n = v.read(file, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], expected);
}
#[test]
fn checked_replace_commits_max_shrink_empty_and_regrow_with_one_version_and_stable_identity() {
    let (mut disk, original) = baseline();
    let mut last = original;
    for payload in [
        &[0x23; REPLACE_BYTES][..],
        &b"binary\0\xff\n"[..],
        &b""[..],
        &[0x61; REPLACE_BYTES][..],
    ] {
        mounted(&mut disk, |v| {
            let (file, _) = v
                .open(
                    user(1001),
                    PATH,
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            let revision = v.revision().unwrap();
            let saved = v
                .replace_checked(&file, last.object, last.version, payload)
                .unwrap();
            assert_eq!(saved.object, original.object);
            assert_eq!(saved.backing, original.backing);
            assert_eq!(saved.version, last.version + 1);
            assert_eq!(saved.bytes, payload.len() as u64);
            assert_eq!(v.revision().unwrap(), revision + 1);
            assert_eq!(v.info(&file).unwrap(), saved);
            read(v, &file, payload);
            v.close(file).unwrap();
            last = saved;
        });
        mounted(&mut disk, |v| {
            let (file, _) = v
                .open(
                    user(1001),
                    PATH,
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            assert_eq!(v.info(&file).unwrap(), last);
            read(v, &file, payload);
            v.close(file).unwrap();
            v.commit().unwrap();
        });
    }
}
#[test]
fn checked_replace_validation_and_stale_writer_never_mutate_or_commit() {
    let (mut disk, original) = baseline();
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        let (reader, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    ..options(Disposition::OpenExisting)
                },
            )
            .unwrap();
        let revision = v.revision().unwrap();
        for (object, version, len, error) in [
            (0, original.version, 1, Error::InvalidParameter),
            (original.object, 0, 1, Error::InvalidParameter),
            (original.object + 1, original.version, 1, Error::Changed),
            (original.object, original.version + 1, 1, Error::Changed),
            (
                original.object,
                original.version,
                REPLACE_BYTES + 1,
                Error::InvalidParameter,
            ),
        ] {
            assert_eq!(v.check_replace(&file, object, version, len), Err(error));
        }
        assert_eq!(
            v.replace_checked(&reader, original.object, original.version, b"bad"),
            Err(Error::AccessDenied)
        );
        assert_eq!(v.revision().unwrap(), revision);
        assert_eq!(v.info(&file).unwrap(), original);
        v.write(&file, 0, b"other").unwrap();
        let other = v.info(&file).unwrap();
        assert_eq!(
            v.replace_checked(&file, original.object, original.version, b"stale"),
            Err(Error::Changed)
        );
        assert_eq!(v.info(&file).unwrap(), other);
        let mut actual = [0; 5];
        v.read(&file, 0, &mut actual).unwrap();
        assert_eq!(&actual, b"other");
        // ACL changes advance the observed version, but captured handle grants
        // retain the same semantics as every other file operation.
        v.set_acl(user(1001), &file, Acl::empty(1001, 0)).unwrap();
        let denied = v.info(&file).unwrap();
        assert_eq!(
            v.replace_checked(&file, other.object, other.version, b"stale"),
            Err(Error::Changed)
        );
        assert_eq!(
            v.replace_checked(&file, denied.object, denied.version, b"captured")
                .unwrap()
                .version,
            denied.version + 1
        );
        assert!(matches!(
            v.open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting)
            ),
            Err(Error::AccessDenied)
        ));
        v.close(reader).unwrap();
        v.close(file).unwrap();
        v.commit().unwrap();
    });
}
#[test]
fn checked_replace_rejects_pending_oversize_and_exhausted_counters() {
    let (mut disk, original) = baseline();
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        v.write(&file, REPLACE_BYTES as u64, b"x").unwrap();
        let big = v.info(&file).unwrap();
        assert_eq!(
            v.replace_checked(&file, big.object, big.version, b"small"),
            Err(Error::InvalidParameter)
        );
        v.truncate(&file, REPLACE_BYTES as u64).unwrap();
        let at = v
            .state
            .nodes
            .iter()
            .position(|n| n.id == original.object)
            .unwrap();
        let valid = v.state.nodes[at].version;
        v.state.nodes[at].version = u64::MAX;
        assert_eq!(
            v.check_replace(&file, original.object, u64::MAX, 0),
            Err(Error::NoSpace)
        );
        v.state.nodes[at].version = valid;
        let revision = v.state.revision;
        v.state.revision = u64::MAX;
        assert_eq!(
            v.check_replace(&file, original.object, valid, 0),
            Err(Error::NoSpace)
        );
        v.state.revision = revision;
        v.remove_path(user(1001), PATH, Context::root()).unwrap();
        assert_eq!(
            v.check_replace(&file, original.object, valid, 0),
            Err(Error::DeletePending)
        );
        v.close(file).unwrap();
        v.commit().unwrap();
    });
}
#[test]
fn checked_replace_recovers_complete_old_or_new_at_every_commit_boundary_twice() {
    let (baseline, original) = baseline();
    for checkpoint in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::FirstHomeFlushed,
        Checkpoint::HomeFlushed,
        Checkpoint::CheckpointFlushed,
    ] {
        let mut disk = baseline.clone();
        assert!(
            catch_unwind(AssertUnwindSafe(|| mounted(&mut disk, |v| {
                let (file, _) = v
                    .open(
                        user(1001),
                        PATH,
                        Context::root(),
                        options(Disposition::OpenExisting),
                    )
                    .unwrap();
                v.replace_checked_with(
                    &file,
                    original.object,
                    original.version,
                    b"short\0\xff\n",
                    |p| {
                        if p == checkpoint {
                            panic!("power cut")
                        }
                    },
                )
                .unwrap();
            })))
            .is_err()
        );
        for _ in 0..2 {
            mounted(&mut disk, |v| {
                let (file, _) = v
                    .open(
                        user(1001),
                        PATH,
                        Context::root(),
                        options(Disposition::OpenExisting),
                    )
                    .unwrap();
                let current = v.info(&file).unwrap();
                assert_eq!(current.backing, original.backing);
                assert_eq!(current.object, original.object);
                if checkpoint == Checkpoint::PrepareFlushed {
                    assert_eq!(current, original);
                    read(v, &file, &[0x71; REPLACE_BYTES]);
                } else {
                    assert_eq!(current.version, original.version + 1);
                    read(v, &file, b"short\0\xff\n");
                }
                v.close(file).unwrap();
                v.commit().unwrap();
            });
        }
    }
}
#[test]
fn checked_replace_failed_io_poisons_until_remount_and_preserves_durable_bytes() {
    let (mut disk, original) = baseline();
    let failure = disk.fail_writes();
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        failure.set(true);
        assert!(
            v.replace_checked(&file, original.object, original.version, b"new")
                .is_err()
        );
        assert_eq!(v.info(&file), Err(Error::Poisoned));
        assert_eq!(v.commit(), Err(Error::Poisoned));
        failure.set(false);
    });
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        assert_eq!(v.info(&file).unwrap(), original);
        read(v, &file, &[0x71; REPLACE_BYTES]);
        v.close(file).unwrap();
        v.commit().unwrap();
    });
}

#[test]
fn production_replace_precommits_unrelated_staging_without_publishing_candidate() {
    let (mut disk, original) = baseline_with(true);
    assert!(
        catch_unwind(AssertUnwindSafe(|| mounted(&mut disk, |v| {
            assert_eq!(v.state.node_capacity(), 256);
            let (file, _) = v
                .open(
                    user(1001),
                    PATH,
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            v.setting_set(user(1001), 1001, Setting::Theme, 1).unwrap();
            v.replace_checked_with(
                &file,
                original.object,
                original.version,
                &[0x42; REPLACE_BYTES],
                |p| {
                    if p == Checkpoint::PrepareFlushed {
                        panic!("candidate not committed")
                    }
                },
            )
            .unwrap();
        })))
        .is_err()
    );
    mounted(&mut disk, |v| {
        assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(1));
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        assert_eq!(v.info(&file).unwrap(), original);
        read(v, &file, &[0x71; REPLACE_BYTES]);
        let saved = v
            .replace_checked(
                &file,
                original.object,
                original.version,
                &[0x42; REPLACE_BYTES],
            )
            .unwrap();
        assert_eq!(saved.version, original.version + 1);
        read(v, &file, &[0x42; REPLACE_BYTES]);
        v.close(file).unwrap();
        v.commit().unwrap();
    });
    mounted(&mut disk, |v| {
        assert_eq!(v.state.node_capacity(), 256);
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        read(v, &file, &[0x42; REPLACE_BYTES]);
        v.close(file).unwrap();
        v.commit().unwrap();
    });
}
#[test]
fn replacement_commit_io_failure_poisons_and_recovery_resolves_durable_marker() {
    let (baseline, original) = baseline();
    for point in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::HomeFlushed,
    ] {
        let mut disk = baseline.clone();
        let failure = disk.fail_writes();
        mounted(&mut disk, |v| {
            let (file, _) = v
                .open(
                    user(1001),
                    PATH,
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            assert!(
                v.replace_checked_with(
                    &file,
                    original.object,
                    original.version,
                    b"replacement",
                    |p| if p == point {
                        failure.set(true)
                    }
                )
                .is_err()
            );
            assert_eq!(v.info(&file), Err(Error::Poisoned));
            assert_eq!(v.commit(), Err(Error::Poisoned));
            failure.set(false);
        });
        mounted(&mut disk, |v| {
            let (file, _) = v
                .open(
                    user(1001),
                    PATH,
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            // The adapter persists one sector before reporting ShortIo. A
            // failed header write may therefore contain a valid durable marker;
            // only recovery can determine which complete version survived.
            let info = v.info(&file).unwrap();
            if info == original {
                read(v, &file, &[0x71; REPLACE_BYTES]);
            } else {
                assert_eq!(
                    info,
                    FileInfo {
                        version: original.version + 1,
                        bytes: 11,
                        ..original
                    }
                );
                read(v, &file, b"replacement");
            }
            v.close(file).unwrap();
            v.commit().unwrap();
        });
    }
}

#[test]
fn replacement_no_space_retires_owner_and_recovers_empty_document() {
    let (mut disk, original) = baseline();
    let empty = mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        let empty = v
            .replace_checked(&file, original.object, original.version, &[])
            .unwrap();
        let (filler, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/filler"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        for page in 0..64 {
            match v.write(&filler, page * 4096, &[0x5a; 4096]) {
                Ok(4096) => (),
                Err(Error::Storage(crate::storage::Error::NoSpace)) => break,
                result => panic!("unexpected filler result {result:?}"),
            }
        }
        v.commit().unwrap();
        assert_eq!(
            v.replace_checked(&file, empty.object, empty.version, &[0x23; REPLACE_BYTES]),
            Err(Error::Storage(crate::storage::Error::NoSpace))
        );
        assert_eq!(v.info(&file), Err(Error::Poisoned));
        assert_eq!(v.commit(), Err(Error::Poisoned));
        empty
    });
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        assert_eq!(v.info(&file).unwrap(), empty);
        read(v, &file, &[]);
        v.close(file).unwrap();
        v.commit().unwrap();
    });
}
#[test]
fn final_generation_isolation_commit_never_publishes_candidate() {
    use crate::storage::BlockDevice;
    let (mut disk, original) = baseline();
    let (home, log) = partitions();
    let mut header = [0; 4096];
    disk.read_sectors(log.first, &mut header).unwrap();
    header[24..32].copy_from_slice(&(u64::MAX - 1).to_le_bytes());
    header[120..124].fill(0);
    let crc = crate::storage::crc32(&header);
    header[120..124].copy_from_slice(&crc.to_le_bytes());
    disk.write_sectors(log.first, &header).unwrap();
    disk.write_sectors(log.first + 8, &header).unwrap();
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                PATH,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        v.setting_set(user(1001), 1001, Setting::Theme, 1).unwrap();
        assert_eq!(
            v.replace_checked(&file, original.object, original.version, b"never published"),
            Err(Error::Poisoned)
        );
    });
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (journal, recovered) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    assert_eq!(recovered.generation, u64::MAX);
    // No further writable generation exists, but the retired checkpoint remains
    // independently readable without manufacturing a new transaction.
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(1));
    let (file, _) = v
        .open(
            user(1001),
            PATH,
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    assert_eq!(v.info(&file).unwrap(), original);
    read(&mut v, &file, &[0x71; REPLACE_BYTES]);
}
