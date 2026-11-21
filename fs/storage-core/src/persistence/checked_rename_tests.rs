//! Observed directory entries are optimistic identities, never authorization.
use super::*;
#[test]
fn checked_rename_reauthorizes_a_parent_actually_moved_beneath_an_unreadable_ancestor() {
    with_volume(|v| {
        let mut shared = Acl::owner_full(1001);
        shared
            .push(Ace {
                principal: Principal::User(1),
                deny: false,
                access: Access::ALL,
                inherit: true,
            })
            .unwrap();
        v.mkdir(
            system(),
            Path::Native("/tmp/shared"),
            Context::root(),
            Some(shared),
        )
        .unwrap();
        v.mkdir(
            system(),
            Path::Native("/tmp/locked"),
            Context::root(),
            Some(Acl::owner_full(1)),
        )
        .unwrap();
        let parent = v
            .mkdir(
                user(1001),
                Path::Native("/tmp/shared/listed"),
                Context::root(),
                None,
            )
            .unwrap();
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/tmp/shared/listed/source.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&file, 0, b"original bytes").unwrap();
        let observed = v.info(&file).unwrap();
        v.rename_path(
            system(),
            Path::Native("/tmp/shared/listed"),
            Path::Native("/tmp/locked/listed"),
            Context::root(),
        )
        .unwrap();
        assert_eq!(v.info(&file).unwrap(), observed);
        assert_eq!(
            v.metadata(
                user(1001),
                Path::Native("/tmp/locked/listed/source.txt"),
                Context::root()
            ),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                observed.object,
                observed.version,
                &wide("wrong.txt")
            ),
            Err(Error::AccessDenied)
        );
        assert_eq!(v.info(&file).unwrap(), observed);
        v.rename_path(
            system(),
            Path::Native("/tmp/locked/listed"),
            Path::Native("/tmp/shared/listed"),
            Context::root(),
        )
        .unwrap();
        v.rename_child_checked(
            user(1001),
            parent,
            observed.object,
            observed.version,
            &wide("renamed.txt"),
        )
        .unwrap();
        assert_eq!(
            v.metadata(
                user(1001),
                Path::Native("/tmp/shared/listed/renamed.txt"),
                Context::root()
            )
            .unwrap()
            .object,
            observed.object
        );
        v.close(file).unwrap();
    });
}
#[test]
fn committed_checked_rename_survives_remount_and_failed_commit_poison_recovers_previous_name() {
    use crate::storage::{Journal, WorkingSet};
    let mut disk = journal_disk();
    let failure = disk.fail_writes();
    let (home, log) = partitions();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    let original;
    let parent;
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
        parent = v
            .metadata(user(1001), Path::Native("/users/1001"), Context::root())
            .unwrap()
            .object;
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&file, 0, b"binary\0\xff preserved").unwrap();
        original = v.info(&file).unwrap();
        v.close(file).unwrap();
        v.commit().unwrap();
        v.rename_child_checked(
            user(1001),
            parent,
            original.object,
            original.version,
            &wide("旅行😀.txt"),
        )
        .unwrap();
        v.commit().unwrap();
    }
    {
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut v =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        let info = v
            .metadata(
                user(1001),
                Path::Native("/users/1001/旅行😀.txt"),
                Context::root(),
            )
            .unwrap();
        assert_eq!(info.object, original.object);
        assert_eq!(info.version, original.version + 1);
        assert_eq!(
            v.metadata(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root()
            ),
            Err(Error::NotFound)
        );
        v.rename_child_checked(
            user(1001),
            parent,
            info.object,
            info.version,
            &wide("failed.txt"),
        )
        .unwrap();
        failure.set(true);
        assert!(v.commit().is_err());
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                info.object,
                info.version + 1,
                &wide("retry.txt")
            ),
            Err(Error::Poisoned)
        );
        assert_eq!(
            v.metadata(
                user(1001),
                Path::Native("/users/1001/failed.txt"),
                Context::root()
            ),
            Err(Error::Poisoned)
        );
        failure.set(false);
    }
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert_eq!(
        v.metadata(
            user(1001),
            Path::Native("/users/1001/failed.txt"),
            Context::root()
        ),
        Err(Error::NotFound)
    );
    let (file, _) = v
        .open(
            user(1001),
            Path::Native("/users/1001/旅行😀.txt"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    assert_eq!(v.info(&file).unwrap().object, original.object);
    let mut bytes = [0; 32];
    let n = v.read(&file, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"binary\0\xff preserved");
    v.close(file).unwrap();
    v.commit().unwrap();
}
fn metadata(
    v: &mut PersistentVolume<'_, Ext2<crate::storage::tests::Disk>>,
    path: &str,
) -> FileInfo {
    v.metadata(user(1001), Path::Native(path), Context::root())
        .unwrap()
}
#[test]
fn checked_rename_preserves_identity_bytes_and_lossless_units_rejects_stale_and_collision() {
    with_volume(|v| {
        let parent = metadata(v, "/users/1001").object;
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&file, 0, b"exact binary\0\xff").unwrap();
        let original = v.info(&file).unwrap();
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version,
                &wide("source.txt")
            ),
            Ok(())
        );
        assert_eq!(v.info(&file).unwrap(), original);
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version + 1,
                &wide("other.txt")
            ),
            Err(Error::Changed)
        );
        assert!(
            v.rename_child_checked(
                user(1001),
                parent + 1,
                original.object,
                original.version,
                &wide("other.txt")
            )
            .is_err()
        );
        let (collision, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/Existing.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version,
                &wide("EXISTING.TXT")
            ),
            Err(Error::AlreadyExists)
        );
        assert_eq!(v.info(&file).unwrap(), original);
        for name in [&[][..], &[46], &[46, 46], &[47], &[0], &[b'a' as u16; 49]] {
            assert!(
                v.rename_child_checked(user(1001), parent, original.object, original.version, name)
                    .is_err()
            );
        }
        // The core namespace deliberately retains unpaired UTF-16 units. The
        // UI must reject editing them instead of substituting another pathname.
        let raw = [0xd800, b'.' as u16, b'x' as u16];
        v.rename_child_checked(user(1001), parent, original.object, original.version, &raw)
            .unwrap();
        let changed = v.info(&file).unwrap();
        assert_eq!(changed.object, original.object);
        assert_eq!(changed.version, original.version + 1);
        let found = v
            .list(user(1001), Path::Native("/users/1001"), Context::root(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(found.info.object, original.object);
        assert_eq!(found.name.units(), raw);
        let mut bytes = [0; 32];
        let n = v.read(&file, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..n], b"exact binary\0\xff");
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version,
                &wide("source.txt")
            ),
            Err(Error::Changed)
        );
        v.rename_child_checked(
            user(1001),
            parent,
            original.object,
            changed.version,
            &wide("Source.txt"),
        )
        .unwrap();
        assert_eq!(v.info(&file).unwrap().version, original.version + 2);
        v.close(collision).unwrap();
        v.close(file).unwrap();
    });
}
#[test]
fn checked_rename_rechecks_ancestry_acl_sharing_pending_and_counter_limits() {
    with_volume(|v| {
        let parent = metadata(v, "/users/1001").object;
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root(),
                OpenOptions {
                    access: Access::READ.union(Access::WRITE),
                    ..options(Disposition::CreateNew)
                },
            )
            .unwrap();
        let original = v.info(&file).unwrap();
        let rename = |v: &mut PersistentVolume<'_, Ext2<crate::storage::tests::Disk>>| {
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version,
                &wide("renamed.txt"),
            )
        };
        assert_eq!(
            v.rename_child_checked(
                user(1002),
                parent,
                original.object,
                original.version,
                &wide("renamed.txt")
            ),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            v.rename_child_checked(system(), 1, 2, 1, &wide("protected")),
            Err(Error::AccessDenied)
        );
        let (blocker, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    share: Access::READ.union(Access::WRITE),
                    disposition: Disposition::OpenExisting,
                },
            )
            .unwrap();
        assert_eq!(rename(v), Err(Error::SharingViolation));
        v.close(blocker).unwrap();
        let pi = v.state.nodes.iter().position(|n| n.id == parent).unwrap();
        let fi = v
            .state
            .nodes
            .iter()
            .position(|n| n.id == original.object)
            .unwrap();
        let acl = v.state.nodes[pi].acl;
        v.state.nodes[pi].acl = Acl::empty(1001, 0);
        assert_eq!(rename(v), Err(Error::AccessDenied));
        let mut traverse_only = Acl::empty(1001, 0);
        traverse_only
            .push(Ace {
                principal: Principal::User(1001),
                deny: false,
                access: Access::TRAVERSE,
                inherit: false,
            })
            .unwrap();
        v.state.nodes[pi].acl = traverse_only;
        assert_eq!(rename(v), Err(Error::AccessDenied));
        v.state.nodes[pi].acl = acl;
        v.state.nodes[pi].pending = true;
        assert_eq!(rename(v), Err(Error::DeletePending));
        v.state.nodes[pi].pending = false;
        let grandparent = v.state.nodes[pi].parent;
        v.state.nodes[pi].parent = parent;
        assert_eq!(rename(v), Err(Error::Corrupt));
        v.state.nodes[pi].parent = u64::MAX;
        assert_eq!(rename(v), Err(Error::Corrupt));
        v.state.nodes[pi].parent = grandparent;
        v.state.nodes[fi].pending = true;
        assert_eq!(rename(v), Err(Error::DeletePending));
        v.state.nodes[fi].pending = false;
        v.state.nodes[fi].version = u64::MAX;
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                u64::MAX,
                &wide("renamed.txt")
            ),
            Err(Error::NoSpace)
        );
        assert_eq!(v.info(&file).unwrap().version, u64::MAX);
        v.state.nodes[fi].version = original.version;
        rename(v).unwrap();
        v.close(file).unwrap();
    });
}
#[test]
fn old_namespace_and_policy_mutations_invalidate_observed_version_including_aba() {
    with_volume(|v| {
        let parent = metadata(v, "/users/1001").object;
        let (file, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/source.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let original = v.info(&file).unwrap();
        v.rename_path(
            user(1001),
            Path::Native("/users/1001/source.txt"),
            Path::Native("/users/1001/away.txt"),
            Context::root(),
        )
        .unwrap();
        v.rename(
            user(1001),
            &file,
            Path::Native("/users/1001/source.txt"),
            Context::root(),
            false,
        )
        .unwrap();
        assert_eq!(v.info(&file).unwrap().version, original.version + 2);
        assert_eq!(
            v.rename_child_checked(
                user(1001),
                parent,
                original.object,
                original.version,
                &wide("wrong.txt")
            ),
            Err(Error::Changed)
        );
        let before = v.info(&file).unwrap().version;
        v.rename(
            user(1001),
            &file,
            Path::Native("/users/1001/Source.txt"),
            Context::root(),
            false,
        )
        .unwrap();
        assert_eq!(v.info(&file).unwrap().version, before + 1);
        v.set_attributes(&file, 0x22).unwrap();
        assert_eq!(v.info(&file).unwrap().version, before + 2);
        v.set_attributes(&file, 0x22).unwrap();
        assert_eq!(v.info(&file).unwrap().version, before + 2);
        let mut acl = Acl::owner_full(1001);
        acl.push(Ace {
            principal: Principal::User(1002),
            deny: false,
            access: Access::READ,
            inherit: false,
        })
        .unwrap();
        v.set_acl(user(1001), &file, acl).unwrap();
        assert_eq!(v.info(&file).unwrap().version, before + 3);
        v.set_acl(user(1001), &file, acl).unwrap();
        assert_eq!(v.info(&file).unwrap().version, before + 3);
        v.close(file).unwrap();
    });
}
