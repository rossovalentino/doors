//! Observed permanent deletion: authority, lifetime and durable old/new recovery.

#[test]
fn deferred_last_close_no_space_retires_owner_and_remount_reclaims_pending_file() {
    let mut disk = crate::storage::tests::ext2_disk();
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let info;
    {
        let fs = NoSpaceUnlink(Ext2::mount(&mut disk).unwrap());
        let mut v = PersistentVolume::initialize(fs, &mut state, &mut catalog, system()).unwrap();
        v.mkdir(
            system(),
            Path::Native("/users/1001"),
            Context::root(),
            Some(Acl::owner_full(1001)),
        )
        .unwrap();
        let p = parent(&mut v);
        let (file, original) = source(&mut v);
        info = original;
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        // Persist the actual ext2 catalog while its compatible open stays live.
        // Journal pending/remount durability is independently covered below.
        v.flush().unwrap();
        assert_eq!(
            v.close(file),
            Err(Error::Storage(crate::storage::Error::NoSpace))
        );
        assert_eq!(v.flush(), Err(Error::Poisoned));
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::Poisoned)
        );
    }
    for _ in 0..2 {
        let mut v =
            PersistentVolume::mount(Ext2::mount(&mut disk).unwrap(), &mut state, &mut catalog)
                .unwrap();
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::NotFound)
        );
        assert!(!v.state.nodes.iter().any(|node| node.id == info.object));
        v.flush().unwrap();
    }
}

struct NoSpaceUnlink<F>(F);
impl<F: Vfs> Vfs for NoSpaceUnlink<F> {
    fn mutation_uncertain(&self) -> bool {
        false
    }
    fn open(&mut self, name: &str) -> crate::storage::Result<crate::storage::FileId> {
        self.0.open(name)
    }
    fn metadata(
        &mut self,
        id: crate::storage::FileId,
    ) -> crate::storage::Result<crate::storage::Metadata> {
        self.0.metadata(id)
    }
    fn read(
        &mut self,
        id: crate::storage::FileId,
        offset: u64,
        out: &mut [u8],
    ) -> crate::storage::Result<usize> {
        self.0.read(id, offset, out)
    }
    fn create(&mut self, name: &str) -> crate::storage::Result<crate::storage::FileId> {
        self.0.create(name)
    }
    fn write(
        &mut self,
        id: crate::storage::FileId,
        offset: u64,
        bytes: &[u8],
    ) -> crate::storage::Result<usize> {
        self.0.write(id, offset, bytes)
    }
    fn truncate(&mut self, id: crate::storage::FileId, bytes: u64) -> crate::storage::Result<()> {
        self.0.truncate(id, bytes)
    }
    fn unlink(&mut self, _name: &str, _id: crate::storage::FileId) -> crate::storage::Result<()> {
        Err(crate::storage::Error::NoSpace)
    }
    fn flush(&mut self) -> crate::storage::Result<()> {
        self.0.flush()
    }
}
#[test]
fn even_preflight_unlink_no_space_poisons_already_changed_namespace() {
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let fs = NoSpaceUnlink(Ext2::mount(crate::storage::tests::ext2_disk()).unwrap());
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut catalog, system()).unwrap();
    v.mkdir(
        system(),
        Path::Native("/users/1001"),
        Context::root(),
        Some(Acl::owner_full(1001)),
    )
    .unwrap();
    let p = parent(&mut v);
    let (file, info) = source(&mut v);
    v.close(file).unwrap();
    v.flush().unwrap();
    assert_eq!(
        v.remove_child_checked(user(1001), p, info.object, info.version),
        Err(Error::Storage(crate::storage::Error::NoSpace))
    );
    assert_eq!(
        v.metadata(user(1001), SOURCE, Context::root()),
        Err(Error::Poisoned)
    );
    assert_eq!(v.flush(), Err(Error::Poisoned));
    assert_eq!(
        v.mkdir(
            user(1001),
            Path::Native("/users/1001/unrelated"),
            Context::root(),
            None
        ),
        Err(Error::Poisoned)
    );
    let mut name = [0; 19];
    assert!(
        v.fs.0
            .open(super::super::namespace::backing_name(
                info.object,
                &mut name
            ))
            .is_ok()
    );
}

use super::*;
use crate::storage::{Checkpoint, Journal, WorkingSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
const SOURCE: Path<'_> = Path::Native("/users/1001/source.bin");
type Disk = crate::storage::tests::Disk;
type Volume<'a> = PersistentVolume<'a, Ext2<Journal<'a, &'a mut Disk>>>;
fn parent<F: Vfs>(v: &mut PersistentVolume<'_, F>) -> u64 {
    v.metadata(user(1001), Path::Native("/users/1001"), Context::root())
        .unwrap()
        .object
}
fn source<F: Vfs>(v: &mut PersistentVolume<'_, F>) -> (FileHandle, FileInfo) {
    let (file, _) = v
        .open(
            user(1001),
            SOURCE,
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&file, 0, b"binary\0\xff retained").unwrap();
    let info = v.info(&file).unwrap();
    (file, info)
}
#[test]
fn checked_remove_rejects_stale_and_sharing_then_keeps_open_stream_until_last_close() {
    with_volume(|v| {
        let p = parent(v);
        let (created, original) = source(v);
        v.close(created).unwrap();
        // Keep READ/WRITE without a captured DELETE grant so another open may
        // validly deny delete sharing; sharing is checked in both directions.
        let (file, _) = v
            .open(
                user(1001),
                SOURCE,
                Context::root(),
                OpenOptions {
                    access: Access::READ.union(Access::WRITE),
                    share: Access::SHARE_ALL,
                    disposition: Disposition::OpenExisting,
                },
            )
            .unwrap();
        let revision = v.revision().unwrap();
        assert_eq!(
            v.remove_child_checked(user(1001), p, original.object, original.version + 1),
            Err(Error::Changed)
        );
        assert_eq!(v.revision().unwrap(), revision);
        let (reader, _) = v
            .open(
                user(1001),
                SOURCE,
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    share: Access::READ.union(Access::WRITE),
                    disposition: Disposition::OpenExisting,
                },
            )
            .unwrap();
        assert_eq!(
            v.remove_child_checked(user(1001), p, original.object, original.version),
            Err(Error::SharingViolation)
        );
        v.close(reader).unwrap();
        // Namespace ABA must invalidate the original observation too.
        v.rename_path(
            user(1001),
            SOURCE,
            Path::Native("/users/1001/away.bin"),
            Context::root(),
        )
        .unwrap();
        v.rename_path(
            user(1001),
            Path::Native("/users/1001/away.bin"),
            SOURCE,
            Context::root(),
        )
        .unwrap();
        let live = v.info(&file).unwrap();
        assert_eq!(live.version, original.version + 2);
        assert_eq!(
            v.remove_child_checked(user(1001), p, original.object, original.version),
            Err(Error::Changed)
        );
        let revision = v.revision().unwrap();
        v.check_remove_child(user(1001), p, live.object, live.version)
            .unwrap();
        assert_eq!(v.revision().unwrap(), revision);
        v.remove_child_checked(user(1001), p, live.object, live.version)
            .unwrap();
        assert_eq!(v.revision().unwrap(), revision + 1);
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::DeletePending)
        );
        assert!(matches!(
            v.list(user(1001), Path::Native("/users/1001"), Context::root(), 0),
            Ok(None)
        ));
        let mut bytes = [0; 32];
        let n = v.read(&file, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"binary\0\xff retained");
        assert_eq!(
            v.remove_child_checked(user(1001), p, live.object, live.version),
            Err(Error::DeletePending)
        );
        v.close(file).unwrap();
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::NotFound)
        );
        assert!(!v.state.nodes.iter().any(|node| node.id == live.object));
        let (replacement, replaced) = source(v);
        assert_ne!(replaced.object, original.object);
        assert_eq!(
            v.remove_child_checked(user(1001), p, original.object, live.version),
            Err(Error::NotFound)
        );
        assert_eq!(v.info(&replacement).unwrap(), replaced);
        v.close(replacement).unwrap();
    });
}
#[test]
fn checked_remove_requires_current_ancestry_acl_binding_and_nonwrapping_revision() {
    with_volume(|v| {
        let p = parent(v);
        let (file, info) = source(v);
        let revision = v.revision().unwrap();
        for (parent, object, version, error) in [
            (0, info.object, info.version, Error::InvalidParameter),
            (p, 0, info.version, Error::InvalidParameter),
            (p, info.object, 0, Error::InvalidParameter),
            (1, info.object, info.version, Error::Changed),
            (1, 2, 1, Error::AccessDenied),
        ] {
            assert_eq!(
                v.remove_child_checked(user(1001), parent, object, version),
                Err(error)
            );
        }
        assert_eq!(
            v.remove_child_checked(user(1002), p, info.object, info.version),
            Err(Error::AccessDenied)
        );
        assert_eq!(v.revision().unwrap(), revision);
        let pi = v.state.nodes.iter().position(|node| node.id == p).unwrap();
        let old_parent = v.state.nodes[pi].parent;
        for bad in [p, u64::MAX] {
            v.state.nodes[pi].parent = bad;
            assert_eq!(
                v.check_remove_child(user(1001), p, info.object, info.version),
                Err(Error::Corrupt)
            );
        }
        v.state.nodes[pi].parent = old_parent;
        v.state.nodes[pi].pending = true;
        assert_eq!(
            v.check_remove_child(user(1001), p, info.object, info.version),
            Err(Error::DeletePending)
        );
        v.state.nodes[pi].pending = false;
        let acl = v.state.nodes[pi].acl;
        v.state.nodes[pi].acl = Acl::empty(1001, 0);
        assert_eq!(
            v.check_remove_child(user(1001), p, info.object, info.version),
            Err(Error::AccessDenied)
        );
        v.state.nodes[pi].acl = acl;
        v.state.revision = u64::MAX;
        assert_eq!(
            v.remove_child_checked(user(1001), p, info.object, info.version),
            Err(Error::NoSpace)
        );
        assert_eq!(v.info(&file).unwrap(), info);
        v.state.revision = revision;
        v.set_acl(user(1001), &file, Acl::empty(1001, 0)).unwrap();
        let live = v.info(&file).unwrap();
        assert_eq!(
            v.remove_child_checked(user(1001), p, live.object, live.version),
            Err(Error::AccessDenied)
        );
        v.close(file).unwrap();
    });
}
#[test]
fn checked_remove_reauthorizes_parent_actually_moved_under_denied_ancestor() {
    with_volume(|v| {
        let mut acl = Acl::owner_full(1001);
        acl.push(Ace {
            principal: Principal::User(1),
            deny: false,
            access: Access::ALL,
            inherit: true,
        })
        .unwrap();
        let p = v
            .mkdir(
                system(),
                Path::Native("/tmp/listed"),
                Context::root(),
                Some(acl),
            )
            .unwrap();
        v.mkdir(
            system(),
            Path::Native("/tmp/locked"),
            Context::root(),
            Some(Acl::owner_full(1)),
        )
        .unwrap();
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/tmp/listed/item"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let info = v.info(&file).unwrap();
        v.rename_path(
            system(),
            Path::Native("/tmp/listed"),
            Path::Native("/tmp/locked/listed"),
            Context::root(),
        )
        .unwrap();
        assert_eq!(v.info(&file).unwrap(), info);
        assert_eq!(
            v.remove_child_checked(user(1001), p, info.object, info.version),
            Err(Error::AccessDenied)
        );
        v.rename_path(
            system(),
            Path::Native("/tmp/locked/listed"),
            Path::Native("/tmp/listed"),
            Context::root(),
        )
        .unwrap();
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        v.close(file).unwrap();
    });
}
#[test]
fn nonempty_and_pending_child_folders_are_never_removed() {
    with_volume(|v| {
        let p = parent(v);
        let folder = v
            .mkdir(
                user(1001),
                Path::Native("/users/1001/folder"),
                Context::root(),
                None,
            )
            .unwrap();
        let observed = v
            .metadata(
                user(1001),
                Path::Native("/users/1001/folder"),
                Context::root(),
            )
            .unwrap();
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/folder/item"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let item = v.info(&file).unwrap();
        assert_eq!(
            v.remove_child_checked(user(1001), p, folder, observed.version),
            Err(Error::InvalidParameter)
        );
        v.remove_child_checked(user(1001), folder, item.object, item.version)
            .unwrap();
        assert!(matches!(
            v.list(
                user(1001),
                Path::Native("/users/1001/folder"),
                Context::root(),
                0
            ),
            Ok(None)
        ));
        assert_eq!(
            v.remove_child_checked(user(1001), p, folder, observed.version),
            Err(Error::InvalidParameter)
        );
        v.close(file).unwrap();
        v.remove_child_checked(user(1001), p, folder, observed.version)
            .unwrap();
        assert_eq!(
            v.metadata(
                user(1001),
                Path::Native("/users/1001/folder"),
                Context::root()
            ),
            Err(Error::NotFound)
        );
    });
}
fn mounted<T>(disk: &mut Disk, test: impl FnOnce(&mut Volume<'_>) -> T) -> T {
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (home, log) = partitions();
    let (mut journal, _) = Journal::open(disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    test(&mut v)
}
fn baseline() -> (Disk, FileInfo, u64) {
    let mut disk = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (home, log) = partitions();
    let info;
    let p;
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
        p = parent(&mut v);
        let (file, original) = source(&mut v);
        info = original;
        v.close(file).unwrap();
        v.commit().unwrap();
    }
    (disk, info, p)
}
#[test]
fn checked_remove_recovers_whole_old_or_absent_binding_at_every_journal_boundary() {
    let (baseline, info, p) = baseline();
    for cut in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::FirstHomeFlushed,
        Checkpoint::HomeFlushed,
        Checkpoint::CheckpointFlushed,
    ] {
        let mut disk = baseline.clone();
        assert!(
            catch_unwind(AssertUnwindSafe(|| mounted(&mut disk, |v| {
                v.remove_child_checked(user(1001), p, info.object, info.version)
                    .unwrap();
                v.commit_with(|point| {
                    if point == cut {
                        panic!("power cut")
                    }
                })
                .unwrap();
            })))
            .is_err()
        );
        for _ in 0..2 {
            mounted(&mut disk, |v| {
                if cut == Checkpoint::PrepareFlushed {
                    assert_eq!(v.metadata(user(1001), SOURCE, Context::root()), Ok(info));
                    let (file, _) = v
                        .open(
                            user(1001),
                            SOURCE,
                            Context::root(),
                            options(Disposition::OpenExisting),
                        )
                        .unwrap();
                    let mut bytes = [0; 32];
                    let n = v.read(&file, 0, &mut bytes).unwrap();
                    assert_eq!(&bytes[..n], b"binary\0\xff retained");
                    v.close(file).unwrap();
                } else {
                    assert_eq!(
                        v.metadata(user(1001), SOURCE, Context::root()),
                        Err(Error::NotFound)
                    );
                    let mut name = [0; 19];
                    assert_eq!(
                        v.fs.open(super::super::namespace::backing_name(
                            info.object,
                            &mut name
                        )),
                        Err(crate::storage::Error::NotFound)
                    );
                }
                v.commit().unwrap();
            });
        }
    }
}
#[test]
fn checked_remove_commit_failure_poison_prevents_unrelated_publication() {
    let (mut disk, info, p) = baseline();
    let failure = disk.fail_writes();
    mounted(&mut disk, |v| {
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        failure.set(true);
        assert!(v.commit().is_err());
        assert_eq!(
            v.check_remove_child(user(1001), p, info.object, info.version),
            Err(Error::Poisoned)
        );
        assert_eq!(
            v.mkdir(
                user(1001),
                Path::Native("/users/1001/unrelated"),
                Context::root(),
                None
            ),
            Err(Error::Poisoned)
        );
        assert_eq!(v.commit(), Err(Error::Poisoned));
        failure.set(false);
    });
    mounted(&mut disk, |v| {
        assert_eq!(v.metadata(user(1001), SOURCE, Context::root()), Ok(info));
        v.commit().unwrap();
    });
}
#[test]
fn durable_pending_delete_reclaims_after_remount_without_open_instances() {
    let (mut disk, info, p) = baseline();
    mounted(&mut disk, |v| {
        let (_file, _) = v
            .open(
                user(1001),
                SOURCE,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        v.commit().unwrap();
        assert!(
            v.state
                .nodes
                .iter()
                .any(|node| node.id == info.object && node.pending)
        );
        // Model power loss while the handle remains open; no close is issued.
    });
    mounted(&mut disk, |v| {
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::NotFound)
        );
        assert!(!v.state.nodes.iter().any(|node| node.id == info.object));
        v.commit().unwrap();
    });
}

#[test]
fn checked_remove_final_generation_isolation_preserves_target() {
    use crate::storage::BlockDevice;
    let (mut disk, info, p) = baseline();
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
        v.setting_set(user(1001), 1001, Setting::Theme, 1).unwrap();
        v.check_remove_child(user(1001), p, info.object, info.version)
            .unwrap();
        // This is the service's unrelated-staging prelude. It acknowledges its
        // final durable generation, then retires; the removal cannot start.
        v.commit().unwrap();
        assert_eq!(
            v.remove_child_checked(user(1001), p, info.object, info.version),
            Err(Error::Poisoned)
        );
    });
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let (journal, recovered) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    assert_eq!(recovered.generation, u64::MAX);
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(1));
    assert_eq!(v.metadata(user(1001), SOURCE, Context::root()), Ok(info));
    assert_payload(&mut v, SOURCE, b"binary\0\xff retained");
}

fn assert_payload<F: Vfs>(v: &mut PersistentVolume<'_, F>, path: Path<'_>, expected: &[u8]) {
    let (file, _) = v
        .open(
            user(1001),
            path,
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    let mut bytes = [0; 4096];
    let count = v.read(&file, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], expected);
    v.close(file).unwrap();
}

#[test]
fn checked_remove_last_close_revision_exhaustion_retires_and_remount_reclaims() {
    let (mut disk, info, p) = baseline();
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                SOURCE,
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        v.state.revision = u64::MAX - 1;
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        assert_eq!(v.revision(), Ok(u64::MAX));
        v.commit().unwrap();
        assert_eq!(v.close(file), Err(Error::NoSpace));
        assert_eq!(v.commit(), Err(Error::Poisoned));
        assert_eq!(v.revision(), Err(Error::Poisoned));
    });
    for _ in 0..2 {
        mounted(&mut disk, |v| {
            assert_eq!(
                v.metadata(user(1001), SOURCE, Context::root()),
                Err(Error::NotFound)
            );
            assert_eq!(v.revision(), Ok(u64::MAX));
            assert!(!v.state.nodes.iter().any(|node| node.id == info.object));
            v.commit().unwrap();
        });
    }
}

// Decode allocation state directly from the independently retained home image.
// Ext2 mount additionally checks bitmap/counter consistency and graph aliases.
fn allocation(disk: &mut Disk) -> (u32, u32, [u8; 4096], [u8; 4096]) {
    use crate::storage::BlockDevice;
    let mut superblock = [0; 4096];
    let mut group = [0; 4096];
    disk.read_sectors(0, &mut superblock).unwrap();
    disk.read_sectors(8, &mut group).unwrap();
    let get = |bytes: &[u8], offset: usize| {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    };
    let mut blocks = [0; 4096];
    let mut inodes = [0; 4096];
    disk.read_sectors(u64::from(get(&group, 0)) * 8, &mut blocks)
        .unwrap();
    disk.read_sectors(u64::from(get(&group, 4)) * 8, &mut inodes)
        .unwrap();
    (
        get(&superblock, 1036),
        get(&superblock, 1040),
        blocks,
        inodes,
    )
}

#[test]
fn checked_remove_failed_commit_preserves_isolated_neighbor_payload_and_allocation() {
    const NEIGHBOR: Path<'_> = Path::Native("/users/1001/neighbor.bin");
    let (mut disk, info, p) = baseline();
    let mut neighbor = None;
    mounted(&mut disk, |v| {
        let (file, _) = v
            .open(
                user(1001),
                NEIGHBOR,
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&file, 0, &[0x91; 4096]).unwrap();
        neighbor = Some(v.info(&file).unwrap());
        v.close(file).unwrap();
        // Service prelude commits an unrelated dirty payload before removal.
        v.check_remove_child(user(1001), p, info.object, info.version)
            .unwrap();
        v.commit().unwrap();
    });
    let before = allocation(&mut disk);
    let failure = disk.fail_writes();
    mounted(&mut disk, |v| {
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        failure.set(true);
        assert!(v.commit().is_err());
        assert_eq!(v.commit(), Err(Error::Poisoned));
        failure.set(false);
    });
    for _ in 0..2 {
        mounted(&mut disk, |v| {
            assert_eq!(v.metadata(user(1001), SOURCE, Context::root()), Ok(info));
            assert_eq!(
                v.metadata(user(1001), NEIGHBOR, Context::root()),
                Ok(neighbor.unwrap())
            );
            assert_payload(v, SOURCE, b"binary\0\xff retained");
            assert_payload(v, NEIGHBOR, &[0x91; 4096]);
            v.commit().unwrap();
        });
        assert_eq!(allocation(&mut disk), before);
    }
    mounted(&mut disk, |v| {
        v.remove_child_checked(user(1001), p, info.object, info.version)
            .unwrap();
        v.commit().unwrap();
    });
    let after = allocation(&mut disk);
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1 + 1);
    assert_eq!(
        before
            .2
            .iter()
            .zip(after.2)
            .map(|(a, b)| (a ^ b).count_ones())
            .sum::<u32>(),
        1
    );
    assert_eq!(
        before
            .3
            .iter()
            .zip(after.3)
            .map(|(a, b)| (a ^ b).count_ones())
            .sum::<u32>(),
        1
    );
    mounted(&mut disk, |v| {
        assert_eq!(
            v.metadata(user(1001), SOURCE, Context::root()),
            Err(Error::NotFound)
        );
        assert_eq!(
            v.metadata(user(1001), NEIGHBOR, Context::root()),
            Ok(neighbor.unwrap())
        );
        assert_payload(v, NEIGHBOR, &[0x91; 4096]);
        v.commit().unwrap();
    });
    assert_eq!(allocation(&mut disk), after);
}
