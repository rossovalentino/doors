extern crate std;
#[path = "catalog_v6_tests.rs"]
mod catalog_v6;
#[path = "checked_remove_tests.rs"]
mod checked_remove;
#[path = "checked_rename_tests.rs"]
mod checked_rename;
#[path = "checked_replace_tests.rs"]
mod checked_replace;
#[path = "fileops_tests.rs"]
mod fileops_engine;
#[path = "staged_replace_tests.rs"]
mod staged_replace;
#[path = "timestamps_tests.rs"]
mod timestamps;
use super::*;
use crate::storage::{Ext2, Vfs};
use std::{boxed::Box, vec::Vec};
fn system() -> Subject {
    unsafe { Subject::authenticated(1, 0, 2).unwrap() }
}
fn user(id: u32) -> Subject {
    unsafe { Subject::authenticated(id, 0, 1).unwrap() }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn options(disposition: Disposition) -> OpenOptions {
    OpenOptions {
        access: Access::ALL,
        share: Access::SHARE_ALL,
        disposition,
    }
}
fn with_volume(test: impl FnOnce(&mut PersistentVolume<'_, Ext2<crate::storage::tests::Disk>>)) {
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    volume
        .mkdir(
            system(),
            Path::Native("/users/1001"),
            Context::root(),
            Some(Acl::owner_full(1001)),
        )
        .unwrap();
    test(&mut volume);
}
#[test]
fn desktop_controls_are_checked_per_user_registry_values_and_survive_catalog_roundtrip() {
    with_volume(|volume| {
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::DesktopControls),
            Ok(48)
        );
        assert_eq!(
            volume.setting_get(user(1002), 1001, Setting::DesktopControls),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            volume.setting_set(user(1002), 1001, Setting::DesktopControls, 63),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            volume.setting_set(user(1001), 1001, Setting::DesktopControls, 126 << 25), // text size step past 225%
            Err(Error::InvalidParameter)
        );
        volume
            .setting_set(user(1001), 1001, Setting::DesktopControls, 63)
            .unwrap();
        volume
            .setting_set(user(1002), 1002, Setting::DesktopControls, 0)
            .unwrap();
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::DesktopControls),
            Ok(63)
        );
        assert_eq!(
            volume.setting_get(user(1002), 1002, Setting::DesktopControls),
            Ok(0)
        );
        let root = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &[],
                Access::READ.union(Access::WRITE),
                false,
            )
            .unwrap();
        assert_eq!(
            volume.registry_get(&root, &wide("DesktopControls")),
            Ok(Value::dword(63))
        );
        volume
            .registry_set(&root, &wide("DesktopControls"), Value::dword(126 << 25)) // text size past 225%
            .unwrap();
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::DesktopControls),
            Err(Error::InvalidParameter)
        );
        volume
            .registry_set(
                &root,
                &wide("DesktopControls"),
                Value::new(RegistryType::Binary, &[0; 4]).unwrap(),
            )
            .unwrap();
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::DesktopControls),
            Err(Error::TypeMismatch)
        );
        volume.registry_close(root).unwrap();
        volume
            .setting_set(user(1001), 1001, Setting::DesktopControls, 57)
            .unwrap();
        let mut wire = Box::new([0; CATALOG_BYTES]);
        codec::encode(volume.state, &mut wire[..]).unwrap();
        let mut reopened = State::boxed();
        codec::decode(&wire[..], &mut reopened).unwrap();
        let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
        // Catalog roundtrip exercises the exact existing registry wire format;
        // the state is authenticated again through a mounted volume below.
        let mut buffer = Box::new([0; CATALOG_BYTES]);
        let mounted =
            PersistentVolume::initialize(fs, &mut reopened, &mut buffer, system()).unwrap();
        codec::decode(&wire[..], mounted.state).unwrap();
        assert_eq!(
            mounted.setting_get(user(1001), 1001, Setting::DesktopControls),
            Ok(57)
        );
        assert_eq!(
            mounted.setting_get(user(1002), 1002, Setting::DesktopControls),
            Ok(0)
        );
    });
}
#[test]
fn input_methods_are_a_checked_per_user_preference_word_that_survives_roundtrip() {
    use door_keyboard_layouts::Preferences;
    with_volume(|volume| {
        // The US keyboard until Settings stores a list.
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::InputMethods),
            Ok(1)
        );
        let p = Preferences::new(&[1, 7, 8, 9], 1, false).unwrap();
        volume
            .setting_set(user(1001), 1001, Setting::InputMethods, p.bits())
            .unwrap();
        for bad in [0, 0x11, 0xa, 0x0400_0001] {
            assert_eq!(
                volume.setting_set(user(1001), 1001, Setting::InputMethods, bad),
                Err(Error::InvalidParameter),
                "{bad:#x}"
            );
        }
        assert_eq!(
            volume.setting_set(user(1002), 1001, Setting::InputMethods, 1),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            volume.setting_get(user(1002), 1002, Setting::InputMethods),
            Ok(1)
        );
        let mut wire = Box::new([0; CATALOG_BYTES]);
        codec::encode(volume.state, &mut wire[..]).unwrap();
        let mut reopened = State::boxed();
        let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
        let mut buffer = Box::new([0; CATALOG_BYTES]);
        let mounted =
            PersistentVolume::initialize(fs, &mut reopened, &mut buffer, system()).unwrap();
        codec::decode(&wire[..], mounted.state).unwrap();
        assert_eq!(
            mounted.setting_get(system(), 1001, Setting::InputMethods),
            Ok(p.bits())
        );
    });
}
#[test]
fn shell_layout_is_a_checked_per_user_value_written_by_init_and_survives_roundtrip() {
    use door_service_protocol::desktop::ShellLayout;
    with_volume(|volume| {
        // Windows defaults until the shell persists a change.
        for setting in [Setting::ShellLayout, Setting::ShellOrder] {
            assert_eq!(volume.setting_get(user(1001), 1001, setting), Ok(0));
            assert_eq!(
                volume.setting_get(user(1002), 1001, setting),
                Err(Error::AccessDenied)
            );
        }
        let layout = ShellLayout::new(&[5, 0, 3], &[2, 4, 1, 0], 2, true).unwrap();
        // Init (system subject) writes on behalf of the session user.
        volume
            .setting_set(system(), 1001, Setting::ShellLayout, layout.flags())
            .unwrap();
        volume
            .setting_set(system(), 1001, Setting::ShellOrder, layout.order())
            .unwrap();
        assert_eq!(
            volume.setting_set(user(1002), 1001, Setting::ShellLayout, 0),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            volume.setting_set(system(), 1001, Setting::ShellLayout, 1 << 15),
            Err(Error::InvalidParameter)
        );
        assert_eq!(
            volume.setting_set(system(), 1001, Setting::ShellOrder, 720),
            Err(Error::InvalidParameter)
        );
        // Independent of DesktopControls and of other users.
        assert_eq!(
            volume.setting_get(user(1001), 1001, Setting::DesktopControls),
            Ok(48)
        );
        assert_eq!(
            volume.setting_get(system(), 1002, Setting::ShellLayout),
            Ok(0)
        );
        let mut wire = Box::new([0; CATALOG_BYTES]);
        codec::encode(volume.state, &mut wire[..]).unwrap();
        let mut reopened = State::boxed();
        let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
        let mut buffer = Box::new([0; CATALOG_BYTES]);
        let mounted =
            PersistentVolume::initialize(fs, &mut reopened, &mut buffer, system()).unwrap();
        codec::decode(&wire[..], mounted.state).unwrap();
        let flags = mounted
            .setting_get(system(), 1001, Setting::ShellLayout)
            .unwrap();
        let order = mounted
            .setting_get(user(1001), 1001, Setting::ShellOrder)
            .unwrap();
        let restored = ShellLayout::from_parts(flags, order).unwrap();
        assert_eq!(restored, layout);
        let (pins, n) = restored.taskbar_pins();
        assert_eq!(&pins[..n], &[5, 0, 3]);
    });
}
#[test]
fn production_catalog_survives_more_than_legacy_capacity_and_rejects_tail_corruption() {
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    for index in 0..64 {
        let path = std::format!("/tmp/directory{index}");
        volume
            .mkdir(system(), Path::Native(&path), Context::root(), None)
            .unwrap();
    }
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    assert_eq!(volume.state.node_capacity(), 256);
    assert!(
        volume
            .metadata(system(), Path::Native("/tmp/directory63"), Context::root())
            .unwrap()
            .directory
    );
    let mut wire = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    codec::encode(volume.state, &mut wire[..]).unwrap();
    assert_eq!(u32::from_le_bytes(wire[8..12].try_into().unwrap()), 5);
    wire[CATALOG_BYTES + 224 + 16] = 99;
    let crc = codec::checksum(&wire[..]);
    wire[40..44].copy_from_slice(&crc.to_le_bytes());
    let mut candidate = State::boxed();
    assert_eq!(
        codec::decode(&wire[..], &mut candidate),
        Err(Error::Corrupt)
    );
}
#[test]
fn production_inode_exhaustion_preserves_reads_and_allows_recovery_by_delete() {
    let mut state = State::boxed_production();
    let mut catalog = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let mut volume = PersistentVolume::initialize(
        Ext2::mount(crate::storage::tests::ext2_disk()).unwrap(),
        &mut state,
        &mut catalog,
        system(),
    )
    .unwrap();
    for index in 0..117 {
        let path = std::format!("/tmp/file{index}");
        let (file, _) = volume
            .open(
                system(),
                Path::Native(&path),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        if index == 0 {
            volume.write(&file, 0, b"preserve existing bytes").unwrap();
        }
        volume.close(file).unwrap();
    }
    assert!(matches!(
        volume.open(
            system(),
            Path::Native("/tmp/overflow"),
            Context::root(),
            options(Disposition::CreateNew)
        ),
        Err(Error::Storage(crate::storage::Error::NoSpace))
    ));
    let (file, _) = volume
        .open(
            system(),
            Path::Native("/tmp/file0"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    let mut data = [0; 23];
    assert_eq!(volume.read(&file, 0, &mut data).unwrap(), 23);
    assert_eq!(&data, b"preserve existing bytes");
    volume.close(file).unwrap();
    volume
        .remove_path(system(), Path::Native("/tmp/file1"), Context::root())
        .unwrap();
    let (file, _) = volume
        .open(
            system(),
            Path::Native("/tmp/recovered"),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    volume.write(&file, 0, b"usable again").unwrap();
    volume.close(file).unwrap();
    let fs = volume.into_inner().unwrap();
    let mut volume = PersistentVolume::mount(fs, &mut state, &mut catalog).unwrap();
    assert_eq!(
        volume
            .metadata(system(), Path::Native("/tmp/recovered"), Context::root())
            .unwrap()
            .bytes,
        12
    );
}
#[test]
fn general_paths_enumerate_authorize_rename_truncate_and_delete() {
    with_volume(|v| {
        let home = Path::Native("/users/1001");
        v.mkdir(
            user(1001),
            Path::Native("/users/1001/notes"),
            Context::root(),
            None,
        )
        .unwrap();
        let (f, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/notes/旅行.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&f, 0, b"user selected content").unwrap();
        assert_eq!(
            v.metadata(
                user(1002),
                Path::Native("/users/1001/notes/旅行.txt"),
                Context::root()
            ),
            Err(Error::AccessDenied)
        );
        assert!(matches!(
            v.list(user(1002), home, Context::root(), 0),
            Err(Error::AccessDenied)
        ));
        let dir = v
            .list(user(1001), home, Context::root(), 0)
            .unwrap()
            .unwrap();
        assert_eq!(dir.name.units(), wide("notes"));
        assert!(dir.info.directory);
        let child = v
            .list(
                user(1001),
                Path::Native("/users/1001/notes"),
                Context::root(),
                0,
            )
            .unwrap()
            .unwrap();
        assert_eq!(child.name.units(), wide("旅行.txt"));
        assert_eq!(child.info.bytes, 21);
        assert!(
            v.list(
                user(1001),
                Path::Native("/users/1001/notes"),
                Context::root(),
                child.info.object
            )
            .unwrap()
            .is_none()
        );
        assert!(
            v.rename_path(
                user(1001),
                Path::Native("/users/1001/notes"),
                Path::Native("/users/1001/notes/cycle"),
                Context::root()
            )
            .is_err()
        );
        v.rename_path(
            user(1001),
            Path::Native("/users/1001/notes/旅行.txt"),
            Path::Native("/users/1001/renamed.bin"),
            Context::root(),
        )
        .unwrap();
        v.truncate(&f, 4).unwrap();
        let mut out = [0; 32];
        assert_eq!(v.read(&f, 0, &mut out).unwrap(), 4);
        assert_eq!(&out[..4], b"user");
        v.truncate(&f, 9).unwrap();
        assert_eq!(v.read(&f, 4, &mut out).unwrap(), 5);
        assert_eq!(&out[..5], &[0; 5]);
        v.remove_path(
            user(1001),
            Path::Native("/users/1001/renamed.bin"),
            Context::root(),
        )
        .unwrap();
        assert!(
            v.list(user(1001), home, Context::root(), dir.info.object)
                .unwrap()
                .is_none()
        );
        assert_eq!(v.read(&f, 0, &mut out).unwrap(), 9);
        v.close(f).unwrap();
        v.remove_path(
            user(1001),
            Path::Native("/users/1001/notes"),
            Context::root(),
        )
        .unwrap();
        assert!(
            v.list(user(1001), home, Context::root(), 0)
                .unwrap()
                .is_none()
        );
    });
}
#[test]
fn namespace_identity_sharing_truncate_delete_and_real_ext2_reclamation() {
    with_volume(|v| {
        let path = wide("C:\\Users\\1001\\first.txt");
        let (h, created) = v
            .open(
                user(1001),
                Path::Windows(&path),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        assert!(created);
        let original = v.info(&h).unwrap();
        v.write(&h, 0, &[0x75; 6000]).unwrap();
        assert!(matches!(
            v.open(
                user(1001),
                Path::Windows(&path),
                Context::root(),
                OpenOptions {
                    access: Access::READ,
                    share: Access::READ,
                    disposition: Disposition::OpenExisting
                }
            ),
            Err(Error::SharingViolation)
        ));
        v.rename(
            user(1001),
            &h,
            Path::Native("/users/1001/second.txt"),
            Context::root(),
            false,
        )
        .unwrap();
        assert_eq!(v.info(&h).unwrap().object, original.object);
        let before_truncate = v.info(&h).unwrap().version;
        let (second, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/second.txt"),
                Context::root(),
                options(Disposition::TruncateExisting),
            )
            .unwrap();
        assert_eq!(v.info(&h).unwrap().bytes, 0);
        assert_eq!(v.info(&h).unwrap().version, before_truncate + 1);
        v.delete(&h).unwrap();
        assert!(matches!(
            v.open(
                user(1001),
                Path::Native("/users/1001/second.txt"),
                Context::root(),
                options(Disposition::OpenExisting)
            ),
            Err(Error::DeletePending)
        ));
        v.close(h).unwrap();
        v.write(&second, 0, b"still open").unwrap();
        v.close(second).unwrap();
        let (new, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/second.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let next = v.info(&new).unwrap();
        assert_ne!(original.object, next.object);
        assert_eq!(original.backing.inode, next.backing.inode);
        assert_ne!(original.backing.generation, next.backing.generation);
        v.close(new).unwrap();
        v.flush().unwrap();
    });
}
#[test]
fn security_is_read_by_owner_or_reader_and_changed_by_owner_or_change_acl() {
    with_volume(|v| {
        let file = wide("C:\\Users\\1001\\secret.txt");
        let path = Path::Windows(&file);
        let (h, _) = v
            .open(
                user(1001),
                path,
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let (acl, directory) = v.security(user(1001), path, Context::root()).unwrap();
        assert!(!directory && acl.owner == 1001);
        assert_eq!(acl.entries().len(), 1);
        assert_eq!(v.security_of(user(1001), &h).unwrap().0, acl);
        // The profile's ACL grants nobody else read (nor traverse).
        assert!(v.security(user(1002), path, Context::root()).is_err());
        // The owner shares it with Everyone (read) and keeps full control.
        let mut shared = Acl::owner_full(1001);
        shared
            .push(Ace {
                principal: Principal::Everyone,
                deny: false,
                access: Access::READ,
                inherit: false,
            })
            .unwrap();
        v.set_security(user(1001), path, Context::root(), shared)
            .unwrap();
        assert_eq!(v.security_of(user(1001), &h).unwrap().0, shared);
        // Giving the file away or raising its label is refused.
        let mut given = shared;
        given.owner = 1002;
        assert_eq!(
            v.set_security_of(user(1001), &h, given),
            Err(Error::AccessDenied)
        );
        let mut raised = shared;
        raised.integrity = 2;
        assert_eq!(
            v.set_security(user(1001), path, Context::root(), raised),
            Err(Error::AccessDenied)
        );
        // The system may.
        v.set_security_of(system(), &h, given).unwrap();
        assert_eq!(v.security_of(system(), &h).unwrap().0.owner, 1002);
        // The Users folder itself is readable to a user (Everyone may list it).
        let (users, directory) = v
            .security(system(), Path::Native("/users/1001"), Context::root())
            .unwrap();
        assert!(directory && users.owner == 1001);
        v.close(h).unwrap();
    });
}
#[test]
fn ordered_acl_inheritance_integrity_and_lossless_path_policy() {
    let mut acl = Acl::empty(1001, 0);
    acl.push(Ace {
        principal: Principal::User(1002),
        deny: true,
        access: Access::WRITE,
        inherit: true,
    })
    .unwrap();
    acl.push(Ace {
        principal: Principal::Everyone,
        deny: false,
        access: Access::ALL,
        inherit: true,
    })
    .unwrap();
    assert_eq!(
        acl.check(user(1002), Access::WRITE),
        Err(Error::AccessDenied)
    );
    assert!(acl.check(user(1002), Access::READ).is_ok());
    let mut ordered = Acl::owner_full(1001);
    ordered
        .push(Ace {
            principal: Principal::Everyone,
            deny: true,
            access: Access::ALL,
            inherit: true,
        })
        .unwrap();
    assert!(ordered.check(user(1001), Access::ALL).is_ok());
    with_volume(|v| {
        let mut path = wide("C:\\Users\\1001\\name-");
        path.push(0xd800);
        path.extend(wide(".txt"));
        let (h, _) = v
            .open(
                user(1001),
                Path::Windows(&path),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&h, 0, b"lossless").unwrap();
        v.close(h).unwrap();
        let mut different = path.clone();
        different[19] = 0xfffd;
        assert!(
            v.open(
                user(1001),
                Path::Windows(&different),
                Context::root(),
                options(Disposition::OpenExisting)
            )
            .is_err()
        );
        for bad in [
            "C:\\Users\\1001\\CON.txt",
            "C:\\Users\\1001\\LPT¹.txt",
            "\\\\server\\share",
            "C:\\Users\\1001\\x:stream",
        ] {
            assert!(
                v.open(
                    user(1001),
                    Path::Windows(&wide(bad)),
                    Context::root(),
                    options(Disposition::CreateNew)
                )
                .is_err()
            );
        }
        assert!(matches!(
            v.open(
                user(1001),
                Path::Windows(&wide("C:\\Windows\\evil.dll")),
                Context::root(),
                options(Disposition::CreateNew)
            ),
            Err(Error::AccessDenied)
        ));
    });
}
#[test]
fn registry_typed_views_user_isolation_deletion_notifications_and_settings() {
    with_volume(|v| {
        let p = wide("Software\\Door\\Fixture");
        let k = v
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &p,
                Access::READ.union(Access::WRITE).union(Access::DELETE),
                true,
            )
            .unwrap();
        let observed = v.registry_version(&k).unwrap();
        v.registry_set(&k, &[], Value::dword(7)).unwrap();
        assert!(v.registry_changed(&k, observed).unwrap());
        assert_eq!(
            v.registry_enumerate_value(&k, 0, observed),
            Err(Error::Changed)
        );
        assert_eq!(v.registry_get(&k, &[]).unwrap(), Value::dword(7));
        assert!(matches!(
            v.registry_open(
                user(1002),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &p,
                Access::READ,
                false
            ),
            Err(Error::NotFound)
        ));
        assert!(matches!(
            v.registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits32,
                &p,
                Access::READ,
                false
            ),
            Err(Error::NotFound)
        ));
        assert!(matches!(
            v.registry_open(
                user(1001),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &p,
                Access::WRITE,
                true
            ),
            Err(Error::AccessDenied)
        ));
        assert_eq!(
            Value::new(RegistryType::Dword, &[1, 2]),
            Err(Error::TypeMismatch)
        );
        v.registry_delete_key(&k).unwrap();
        assert_eq!(v.registry_get(&k, &[]), Err(Error::KeyDeleted));
        v.registry_close(k).unwrap();
        // Unset reads as 0 = automatic (host hint, else integer heuristic).
        assert_eq!(
            v.setting_get(user(1001), 1001, Setting::ScalePercent),
            Ok(0)
        );
        for value in [100, 125, 150, 175, 200, 225, 250, 300, 0] {
            v.setting_set(user(1001), 1001, Setting::ScalePercent, value)
                .unwrap();
            assert_eq!(
                v.setting_get(user(1001), 1001, Setting::ScalePercent),
                Ok(value)
            );
        }
        for value in [25, 75, 99, 129, 310, 325, 400] {
            assert_eq!(
                v.setting_set(user(1001), 1001, Setting::ScalePercent, value),
                Err(Error::InvalidParameter),
                "{value}"
            );
        }
        v.setting_set(user(1001), 1001, Setting::ScalePercent, 125)
            .unwrap();
        assert_eq!(
            v.setting_set(user(1001), 1001, Setting::ScalePercent, 129),
            Err(Error::InvalidParameter)
        );
        assert_eq!(
            v.setting_get(user(1001), 1001, Setting::ScalePercent),
            Ok(125)
        );
        assert_eq!(
            v.setting_get(user(1002), 1001, Setting::ScalePercent),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            v.setting_set(user(1002), 1001, Setting::DesktopAppearance, 7),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            v.setting_set(
                user(1001),
                1001,
                Setting::DesktopAppearance,
                door_service_protocol::window::APPEARANCE_VALUES
            ),
            Err(Error::InvalidParameter)
        );
        v.setting_set(user(1001), 1001, Setting::DesktopAppearance, 7)
            .unwrap();
        assert_eq!(
            v.setting_get(user(1001), 1001, Setting::DesktopAppearance),
            Ok(7)
        );
        assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(1));
        assert_eq!(
            v.setting_get(user(1001), 1001, Setting::ScalePercent),
            Ok(125)
        );
        v.flush().unwrap();
    });
}
#[test]
fn namespace_codec_and_ext2_reopen_keep_authority_payload_and_configuration() {
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    v.mkdir(
        system(),
        Path::Native("/users/1001"),
        Context::root(),
        Some(Acl::owner_full(1001)),
    )
    .unwrap();
    let (h, _) = v
        .open(
            user(1001),
            Path::Native("/users/1001/persist"),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&h, 0, b"durable bytes").unwrap();
    let id = v.info(&h).unwrap().object;
    v.setting_set(user(1001), 1001, Setting::Theme, 2).unwrap();
    v.close(h).unwrap();
    let disk = v.into_inner().unwrap().into_inner().unwrap();
    let fs = Ext2::mount(disk).unwrap();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    let (h, _) = v
        .open(
            user(1001),
            Path::Native("/users/1001/persist"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    assert_eq!(v.info(&h).unwrap().object, id);
    let mut bytes = [0; 13];
    assert_eq!(v.read(&h, 0, &mut bytes), Ok(13));
    assert_eq!(&bytes, b"durable bytes");
    assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(2));
    v.close(h).unwrap();
    let fs = v.into_inner().unwrap();
    let disk = fs.into_inner().unwrap();
    Ext2::mount(disk).unwrap();
}

#[test]
fn path_corpus_drive_relative_extended_case_and_ancestor_authority() {
    with_volume(|v| {
        let (h, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/Mixed.txt"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let id = v.info(&h).unwrap().object;
        v.close(h).unwrap();
        let cwd = v
            .change_directory(
                user(1001),
                Path::Windows(&wide("C:\\Users\\1001")),
                Context::root(),
            )
            .unwrap();
        for p in [
            "mixed.TXT",
            "C:mIxEd.txt",
            "C:\\Users\\1001\\Mixed.txt. ",
            "C:\\Users\\1001\\.\\Mixed.txt",
            "\\\\?\\C:\\Users\\1001\\Mixed.txt",
            "\\??\\C:\\Users\\1001\\Mixed.txt",
        ] {
            let (h, _) = v
                .open(
                    user(1001),
                    Path::Windows(&wide(p)),
                    cwd,
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            assert_eq!(v.info(&h).unwrap().object, id);
            v.close(h).unwrap();
        }
        assert!(
            v.change_directory(
                user(1001),
                Path::Windows(&wide("C:\\..\\Windows\\System32")),
                cwd
            )
            .is_ok()
        );
        assert!(matches!(
            v.change_directory(user(1001), Path::Windows(&wide("C:\\..\\system")), cwd),
            Err(Error::NotFound)
        ));
        assert!(matches!(
            v.change_directory(user(1001), Path::Native("/system"), cwd),
            Err(Error::Unsupported)
        ));
        let local = v
            .change_directory(user(1001), Path::Native("/users/1001"), Context::root())
            .unwrap();
        assert!(
            v.open(
                user(1001),
                Path::Windows(&wide("Mixed.txt")),
                local,
                options(Disposition::OpenExisting)
            )
            .is_ok()
        );
        assert!(matches!(
            v.open(
                user(1001),
                Path::Windows(&wide("C:Mixed.txt")),
                local,
                options(Disposition::OpenExisting)
            ),
            Err(Error::NotFound)
        ));
        v.mkdir(
            system(),
            Path::Native("/users/locked"),
            Context::root(),
            Some(Acl::owner_full(1)),
        )
        .unwrap();
        assert!(matches!(
            v.open(
                user(1001),
                Path::Windows(&wide("C:\\Users\\locked\\..\\1001\\Mixed.txt")),
                Context::root(),
                options(Disposition::OpenExisting)
            ),
            Err(Error::AccessDenied)
        ));
    });
}

#[test]
fn replacement_keeps_old_stream_open_and_last_close_reclaims_real_inode() {
    with_volume(|v| {
        let (a, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/a"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        let (b, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/b"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&a, 0, b"source").unwrap();
        v.write(&b, 0, b"target").unwrap();
        let old = v.info(&b).unwrap();
        let source = v.info(&a).unwrap();
        v.rename(
            user(1001),
            &a,
            Path::Native("/users/1001/b"),
            Context::root(),
            true,
        )
        .unwrap();
        let mut data = [0; 6];
        v.read(&b, 0, &mut data).unwrap();
        assert_eq!(&data, b"target");
        assert!(v.info(&b).unwrap().delete_pending);
        let (c, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/b"),
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        assert_eq!(v.info(&c).unwrap().object, source.object);
        v.close(c).unwrap();
        v.close(b).unwrap();
        v.close(a).unwrap();
        v.flush().unwrap();
        assert_eq!(
            v.fs.metadata(old.backing),
            Err(crate::storage::Error::NotFound)
        );
    });
}

#[test]
fn handle_capacity_preflights_creation_and_truncation() {
    with_volume(|v| {
        let (h, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/a"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&h, 0, b"preserve").unwrap();
        let mut handles = Vec::new();
        handles.push(h);
        for _ in 1..super::namespace::OPENS {
            handles.push(
                v.open(
                    user(1001),
                    Path::Native("/users/1001/a"),
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap()
                .0,
            );
        }
        let before = v.revision().unwrap();
        assert!(matches!(
            v.open(
                user(1001),
                Path::Native("/users/1001/a"),
                Context::root(),
                options(Disposition::CreateAlways)
            ),
            Err(Error::NoSpace)
        ));
        assert!(matches!(
            v.open(
                user(1001),
                Path::Native("/users/1001/new"),
                Context::root(),
                options(Disposition::CreateNew)
            ),
            Err(Error::NoSpace)
        ));
        assert_eq!(v.revision().unwrap(), before);
        assert_eq!(v.info(&handles[0]).unwrap().bytes, 8);
        for h in handles {
            v.close(h).unwrap();
        }
    });
}

#[test]
fn merged_registry_and_subkey_versions_are_live_for_bound_objects() {
    with_volume(|v| {
        let p = wide("Software\\Classes\\Door");
        let m = v
            .registry_open(
                system(),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &p,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_set(&m, &wide("Common"), Value::dword(1))
            .unwrap();
        v.registry_set(&m, &wide("Fallback"), Value::dword(8))
            .unwrap();
        let u = v
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &p,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_set(&u, &wide("Common"), Value::dword(2))
            .unwrap();
        let merged = v
            .registry_open(
                user(1001),
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &wide("Door"),
                Access::READ,
                false,
            )
            .unwrap();
        assert_eq!(
            v.registry_get(&merged, &wide("Common")),
            Ok(Value::dword(2))
        );
        assert_eq!(
            v.registry_get(&merged, &wide("Fallback")),
            Ok(Value::dword(8))
        );
        let before = v.registry_version(&u).unwrap();
        let child = v
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Software\\Classes\\Door\\Child"),
                Access::READ.union(Access::DELETE),
                true,
            )
            .unwrap();
        assert!(v.registry_changed(&u, before).unwrap());
        let version = v.registry_version(&u).unwrap();
        assert_eq!(
            v.registry_enumerate_key(&u, 0, version).unwrap(),
            Name::ascii("Child").unwrap()
        );
        v.registry_delete_key(&child).unwrap();
        assert!(v.registry_changed(&u, version).unwrap());
        v.registry_close(child).unwrap();
        v.registry_close(merged).unwrap();
        v.registry_close(u).unwrap();
        v.registry_close(m).unwrap();
        v.flush().unwrap();
    });
}

#[test]
fn catalog_rejects_corrupt_structure_and_migrates_only_supported_schema() {
    with_volume(|v| {
        v.setting_set(user(1001), 1001, Setting::Theme, 2).unwrap();
        v.flush().unwrap();
        let mut encoded = Box::new([0; CATALOG_BYTES]);
        codec::encode(v.state, &mut encoded[..]).unwrap();
        let mut target = State::boxed();
        for offset in [
            0,
            8,
            12,
            16,
            24,
            32,
            40,
            44,
            48,
            128,
            128 + 16,
            128 + 18,
            128 + 152,
            7296,
            10368,
            14976,
            15488,
            16383,
        ] {
            let mut corrupt = encoded.clone();
            corrupt[offset] ^= 0x80;
            assert!(
                codec::decode(&corrupt[..], &mut target).is_err(),
                "offset{offset}"
            );
        }
        // Recompute valid checksums for semantically malformed metadata.
        for (offset, bytes) in [
            (128 + 8, 99u64.to_le_bytes()),
            (128 + 224 + 8, 2u64.to_le_bytes()),
            (128 + 224, 1u64.to_le_bytes()),
        ] {
            let mut corrupt = encoded.clone();
            corrupt[offset..offset + 8].copy_from_slice(&bytes);
            let crc = codec::checksum(&corrupt[..]);
            corrupt[40..44].copy_from_slice(&crc.to_le_bytes());
            assert!(codec::decode(&corrupt[..], &mut target).is_err());
        }
        encoded[14980..14984].copy_from_slice(&1u32.to_le_bytes());
        encoded[14988..14992].copy_from_slice(&0u32.to_le_bytes());
        let crc = codec::checksum(&encoded[..]);
        encoded[40..44].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(codec::decode(&encoded[..], &mut target), Ok(true));
        assert_eq!(target.config.settings[0].theme, 2);
        // Schema 1 stored no scale preference: automatic.
        assert_eq!(target.config.settings[0].scale, 0);
        // Schemas 2/3 stored 100 as the default no UI could choose: it reads
        // as automatic and marks the catalog dirty; other values are kept,
        // and schema 4 keeps an explicit 100.
        for (schema, stored, scale, migrated) in [
            (3u32, 100u32, 0, true),
            (2, 100, 0, true),
            (3, 125, 125, false),
            (4, 100, 100, false),
        ] {
            encoded[14980..14984].copy_from_slice(&schema.to_le_bytes());
            encoded[14988..14992].copy_from_slice(&stored.to_le_bytes());
            let crc = codec::checksum(&encoded[..]);
            encoded[40..44].copy_from_slice(&crc.to_le_bytes());
            assert_eq!(
                codec::decode(&encoded[..], &mut target),
                Ok(migrated),
                "schema {schema}"
            );
            assert_eq!(target.config.settings[0].schema, 4);
            assert_eq!(
                target.config.settings[0].scale, scale,
                "schema {schema} scale {stored}"
            );
        }
        encoded[14980..14984].copy_from_slice(&5u32.to_le_bytes());
        let crc = codec::checksum(&encoded[..]);
        encoded[40..44].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(
            codec::decode(&encoded[..], &mut target),
            Err(Error::Unsupported)
        );
    });
}

fn partitions() -> (crate::storage::Partition, crate::storage::Partition) {
    (
        crate::storage::Partition {
            type_guid: [1; 16],
            id: [2; 16],
            first: 0,
            sectors: 256,
            attributes: 0,
        },
        crate::storage::Partition {
            type_guid: [3; 16],
            id: [4; 16],
            first: 256,
            sectors: 768,
            attributes: 0,
        },
    )
}
fn journal_disk() -> crate::storage::tests::Disk {
    let mut disk = crate::storage::tests::ext2_disk();
    disk.add_empty_tail(1024);
    let (home, log) = partitions();
    let mut header = [0u8; 4096];
    header[..8].copy_from_slice(b"DOORJNL1");
    for (offset, value) in [(8, 1u32), (12, 4096), (112, 512), (116, 4096)] {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    header[32..48].fill(7);
    header[48..64].copy_from_slice(&home.id);
    header[64..80].copy_from_slice(&log.id);
    for (offset, value) in [
        (80, home.first),
        (88, home.sectors),
        (96, log.first),
        (104, log.sectors),
    ] {
        header[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    let crc = crate::storage::crc32(&header);
    header[120..124].copy_from_slice(&crc.to_le_bytes());
    crate::storage::BlockDevice::write_sectors(&mut disk, log.first, &header).unwrap();
    crate::storage::BlockDevice::write_sectors(&mut disk, log.first + 8, &header).unwrap();
    disk
}
#[test]
fn indirect_namespace_commits_recover_at_every_journal_boundary() {
    use crate::storage::{Checkpoint, Journal, WorkingSet};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let (home, log) = partitions();
    let mut baseline = journal_disk();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    {
        let (mut journal, _) = Journal::open(&mut baseline, home, log, [7; 16], &mut work).unwrap();
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
        v.commit().unwrap();
    }
    for checkpoint in [
        Checkpoint::PrepareFlushed,
        Checkpoint::CommitFlushed,
        Checkpoint::FirstHomeFlushed,
        Checkpoint::HomeFlushed,
        Checkpoint::CheckpointFlushed,
    ] {
        let mut disk = baseline.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
            journal.begin().unwrap();
            let mut v =
                PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                    .unwrap();
            let (file, _) = v
                .open(
                    user(1001),
                    Path::Native("/users/1001/arbitrary.elf"),
                    Context::root(),
                    options(Disposition::CreateNew),
                )
                .unwrap();
            v.write(&file, 0, &[0x8d; 13 * 4096 + 19]).unwrap();
            v.close(file).unwrap();
            v.commit_with(|point| {
                if point == checkpoint {
                    panic!("power cut");
                }
            })
            .unwrap();
        }));
        assert!(result.is_err());
        for _ in 0..2 {
            let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
            journal.begin().unwrap();
            let mut v =
                PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                    .unwrap();
            let file = v.open(
                user(1001),
                Path::Native("/users/1001/arbitrary.elf"),
                Context::root(),
                options(Disposition::OpenExisting),
            );
            if checkpoint == Checkpoint::PrepareFlushed {
                assert!(matches!(file, Err(Error::NotFound)));
            } else {
                let (file, _) = file.unwrap();
                assert_eq!(v.info(&file).unwrap().bytes, 13 * 4096 + 19);
                let mut out = [0; 32];
                assert_eq!(v.read(&file, 12 * 4096 - 8, &mut out).unwrap(), 32);
                assert_eq!(out, [0x8d; 32]);
                v.close(file).unwrap();
            }
            v.commit().unwrap();
        }
    }
}
#[test]
fn production_budget_checkpoints_unflushed_writes_beyond_journal_capacity() {
    use crate::storage::{BlockDevice, Journal, WorkingSet};
    let (mut home, mut log) = partitions();
    let mut disk = journal_disk();
    home.sectors = 512 * 8;
    log.first = home.sectors;
    disk.add_empty_tail(log.first + log.sectors);
    let mut page = [0; 4096];
    disk.read_sectors(0, &mut page).unwrap();
    page[1028..1032].copy_from_slice(&512u32.to_le_bytes());
    page[1036..1040].copy_from_slice(&503u32.to_le_bytes());
    disk.write_sectors(0, &page).unwrap();
    disk.read_sectors(8, &mut page).unwrap();
    page[12..14].copy_from_slice(&503u16.to_le_bytes());
    disk.write_sectors(8, &page).unwrap();
    page.fill(255);
    page[..64].fill(0);
    page[0] = 255;
    page[1] = 1;
    disk.write_sectors(16, &page).unwrap();
    disk.read_sectors(256, &mut page).unwrap();
    page[88..96].copy_from_slice(&home.sectors.to_le_bytes());
    page[96..104].copy_from_slice(&log.first.to_le_bytes());
    page[120..124].fill(0);
    let crc = crate::storage::crc32(&page);
    page[120..124].copy_from_slice(&crc.to_le_bytes());
    disk.write_sectors(log.first, &page).unwrap();
    disk.write_sectors(log.first + 8, &page).unwrap();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed_production();
    let mut catalog = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    {
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut volume = PersistentVolume::initialize(
            Ext2::mount(journal).unwrap(),
            &mut state,
            &mut catalog,
            system(),
        )
        .unwrap();
        volume.commit().unwrap();
        let (file, _) = volume
            .open(
                system(),
                Path::Native("/tmp/large"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        // No caller flush: exercise the service's pre-request budget instead.
        for index in 0..1024u64 {
            volume.checkpoint_budget(24).unwrap();
            volume.write(&file, index * 512, &[b'x'; 512]).unwrap();
        }
        volume.close(file).unwrap();
        volume.commit().unwrap();
        let (file, _) = volume
            .open(
                system(),
                Path::Native("/tmp/healthy"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        volume.write(&file, 0, b"still writable").unwrap();
        volume.close(file).unwrap();
        volume.commit().unwrap();
        let fs = volume.into_inner().unwrap();
        let _journal = fs.into_inner().unwrap();
    }
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut volume =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    let (file, _) = volume
        .open(
            system(),
            Path::Native("/tmp/large"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    assert_eq!(volume.info(&file).unwrap().bytes, 512 * 1024);
    let mut tail = [0; 512];
    assert_eq!(volume.read(&file, 512 * 1023, &mut tail).unwrap(), 512);
    assert_eq!(tail, [b'x'; 512]);
    // The reserve policy must not turn a genuine staging failure into a safe
    // allocator rejection: a single oversized lower-level transaction poisons.
    assert_eq!(
        volume.write(&file, 0, &std::vec![b'z';300*1024]),
        Err(Error::Storage(crate::storage::Error::NoSpace))
    );
    assert_eq!(volume.info(&file), Err(Error::Poisoned));
}
#[test]
fn journal_full_disk_and_failed_commit_preserve_last_durable_catalog() {
    use crate::storage::{Journal, WorkingSet};
    let mut disk = journal_disk();
    let failure = disk.fail_writes();
    let (home, log) = partitions();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
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
        v.commit().unwrap();
        for i in 0..9 {
            let path = std::format!("/users/1001/full{i}");
            let (h, _) = v
                .open(
                    user(1001),
                    Path::Native(&path),
                    Context::root(),
                    options(Disposition::CreateNew),
                )
                .unwrap();
            v.write(&h, 0, &[i as u8; 8192]).unwrap();
            v.close(h).unwrap();
            v.commit().unwrap();
        }
        // Our original unit fixture lacks the guest's seed file: consume the
        // one remaining data block and qualify actual allocator exhaustion.
        let (h, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/tail"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        v.write(&h, 0, &[0xaa; 4096]).unwrap();
        v.close(h).unwrap();
        v.commit().unwrap();
        let (h, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/failed"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        assert_eq!(
            v.write(&h, 0, &[1; 4096]),
            Err(Error::Storage(crate::storage::Error::NoSpace))
        );
        assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(0));
        v.close(h).unwrap();
        let (tail, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/tail"),
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        let mut byte = [0];
        assert_eq!(v.read(&tail, 0, &mut byte), Ok(1));
        assert_eq!(byte, [0xaa]);
        v.close(tail).unwrap();
        v.remove_path(
            user(1001),
            Path::Native("/users/1001/tail"),
            Context::root(),
        )
        .unwrap();
        let (h, _) = v
            .open(
                user(1001),
                Path::Native("/users/1001/failed"),
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        v.write(&h, 0, &[1; 4096]).unwrap();
        v.close(h).unwrap();
        v.remove_path(
            user(1001),
            Path::Native("/users/1001/failed"),
            Context::root(),
        )
        .unwrap();
        v.commit().unwrap();
    }
    {
        let (mut journal, recovery) =
            Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        assert!(!recovery.replayed);
        journal.begin().unwrap();
        let mut v =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        assert!(matches!(
            v.open(
                user(1001),
                Path::Native("/users/1001/failed"),
                Context::root(),
                options(Disposition::OpenExisting)
            ),
            Err(Error::NotFound)
        ));
        v.setting_set(user(1001), 1001, Setting::Theme, 2).unwrap();
        failure.set(true);
        assert!(v.commit().is_err());
        assert_eq!(v.revision(), Err(Error::Poisoned));
        failure.set(false);
    }
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let v =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert_eq!(v.setting_get(user(1001), 1001, Setting::Theme), Ok(0));
    let mut fs = v.into_inner().unwrap();
    let id = fs.open("door.catalog").unwrap();
    assert_eq!(fs.metadata(id).unwrap().bytes, 16384);
}

#[test]
fn stale_file_key_and_directory_context_are_rejected_after_remount() {
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    v.mkdir(
        system(),
        Path::Native("/users/1001"),
        Context::root(),
        Some(Acl::owner_full(1001)),
    )
    .unwrap();
    let (old, _) = v
        .open(
            user(1001),
            Path::Native("/users/1001/a"),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&old, 0, b"persistent").unwrap();
    let cwd = v
        .change_directory(
            user(1001),
            Path::Windows(&wide("C:\\Users\\1001")),
            Context::root(),
        )
        .unwrap();
    let key = v
        .registry_open(
            user(1001),
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &wide("A"),
            Access::READ,
            true,
        )
        .unwrap();
    let disk = v.into_inner().unwrap().into_inner().unwrap();
    let mut v =
        PersistentVolume::mount(Ext2::mount(disk).unwrap(), &mut state, &mut buffer).unwrap();
    assert_eq!(v.info(&old), Err(Error::InvalidHandle));
    assert_eq!(v.registry_version(&key), Err(Error::InvalidHandle));
    assert!(matches!(
        v.open(
            user(1001),
            Path::Windows(&wide("a")),
            cwd,
            options(Disposition::OpenExisting)
        ),
        Err(Error::InvalidHandle)
    ));
    let (h, _) = v
        .open(
            user(1001),
            Path::Native("/users/1001/a"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    v.delete(&h).unwrap();
    let backing = v.info(&h).unwrap().backing;
    let disk = v.into_inner().unwrap().into_inner().unwrap();
    let v = PersistentVolume::mount(Ext2::mount(disk).unwrap(), &mut state, &mut buffer).unwrap();
    let mut fs = v.into_inner().unwrap();
    assert_eq!(fs.metadata(backing), Err(crate::storage::Error::NotFound));
    let disk = fs.into_inner().unwrap();
    Ext2::mount(disk).unwrap();
}

#[test]
fn windows_unc_separator_variants_never_open_or_truncate_local_objects() {
    with_volume(|volume| {
        let (file, _) = volume
            .open(
                user(1001),
                Path::Native("/users/1001/test"),
                Context::root(),
                options(Disposition::CreateNew),
            )
            .unwrap();
        volume.write(&file, 0, b"preserved local bytes").unwrap();
        let identity = volume.info(&file).unwrap();
        let revision = volume.revision().unwrap();
        for path in [
            "//Users/1001/test",
            "/\\Users\\1001\\test",
            "\\/Users\\1001\\test",
            "\\\\Users\\1001\\test",
        ] {
            for disposition in [Disposition::OpenExisting, Disposition::CreateAlways] {
                assert!(
                    matches!(
                        volume.open(
                            user(1001),
                            Path::Windows(&wide(path)),
                            Context::root(),
                            options(disposition)
                        ),
                        Err(Error::Unsupported)
                    ),
                    "path {path:?}"
                );
                assert_eq!(volume.revision().unwrap(), revision);
                assert_eq!(volume.info(&file).unwrap(), identity);
            }
        }
        for path in [
            "/Users/1001/test",
            "\\Users\\1001\\test",
            "C:/Users/1001/test",
            "C://Users//1001//test",
            "\\\\?\\C:\\Users\\1001\\test",
        ] {
            let (alias, created) = volume
                .open(
                    user(1001),
                    Path::Windows(&wide(path)),
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            assert!(!created);
            assert_eq!(volume.info(&alias).unwrap().object, identity.object);
            volume.close(alias).unwrap();
        }
        for path in ["\\\\?\\C:/Users/1001/test", "\\\\?\\C:\\Users/1001/test"] {
            assert!(matches!(
                volume.open(
                    user(1001),
                    Path::Windows(&wide(path)),
                    Context::root(),
                    options(Disposition::OpenExisting)
                ),
                Err(Error::InvalidName)
            ));
        }
        let (native, _) = volume
            .open(
                user(1001),
                Path::Native("//users//1001/test"),
                Context::root(),
                options(Disposition::OpenExisting),
            )
            .unwrap();
        assert_eq!(volume.info(&native).unwrap().object, identity.object);
        volume.close(native).unwrap();
        let mut output = [0; 21];
        assert_eq!(volume.read(&file, 0, &mut output).unwrap(), output.len());
        assert_eq!(&output, b"preserved local bytes");
        volume.close(file).unwrap();
    });
}

#[test]
fn deleted_registry_ancestry_commits_in_every_close_order_and_survives_recovery() {
    use crate::storage::{Journal, WorkingSet};
    // Every close ordering for a three-level deleted subtree must retain all
    // still-needed ancestors and eventually collect every old identity.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut disk = journal_disk();
        let (home, log) = partitions();
        let mut work = Box::new(WorkingSet::new());
        let mut state = State::boxed();
        let mut catalog = Box::new([0; CATALOG_BYTES]);
        {
            let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
            journal.begin().unwrap();
            let mut volume = PersistentVolume::initialize(
                Ext2::mount(journal).unwrap(),
                &mut state,
                &mut catalog,
                system(),
            )
            .unwrap();
            let access = Access::READ.union(Access::WRITE).union(Access::DELETE);
            let mut handles: [Option<KeyHandle>; 3] = [None, None, None];
            for (slot, path) in ["Parent", "Parent\\Child", "Parent\\Child\\Leaf"]
                .into_iter()
                .enumerate()
            {
                handles[slot] = Some(
                    volume
                        .registry_open(
                            user(1001),
                            RegistryRoot::CurrentUser,
                            RegistryView::Bits64,
                            &wide(path),
                            access,
                            true,
                        )
                        .unwrap(),
                );
            }
            volume.commit().unwrap();
            let mut old_ids = [0u64; 3];
            for (slot, spelling) in ["Parent", "Child", "Leaf"].into_iter().enumerate() {
                old_ids[slot] = volume
                    .state
                    .config
                    .keys
                    .iter()
                    .find(|k| k.name == Name::ascii(spelling).unwrap())
                    .unwrap()
                    .id;
            }
            for slot in (0..3).rev() {
                volume
                    .registry_delete_key(handles[slot].as_ref().unwrap())
                    .unwrap();
                volume.commit().unwrap();
            }
            // A new binding with the same spelling cannot alias a tombstone.
            let replacement = volume
                .registry_open(
                    user(1001),
                    RegistryRoot::CurrentUser,
                    RegistryView::Bits64,
                    &wide("Parent"),
                    access,
                    true,
                )
                .unwrap();
            volume
                .registry_set(&replacement, &[], Value::dword(0x1234))
                .unwrap();
            assert!(volume.state.config.keys.iter().any(|k| !k.deleted
                && k.name == Name::ascii("Parent").unwrap()
                && !old_ids.contains(&k.id)));
            for slot in order {
                for handle in handles.iter().flatten() {
                    assert_eq!(volume.registry_get(handle, &[]), Err(Error::KeyDeleted));
                }
                volume
                    .registry_close(handles[slot].take().unwrap())
                    .unwrap();
                volume
                    .setting_set(user(1001), 1001, Setting::Theme, 2)
                    .unwrap();
                volume.commit().unwrap();
                assert_eq!(
                    volume.registry_get(&replacement, &[]),
                    Ok(Value::dword(0x1234))
                );
            }
            assert!(
                volume
                    .state
                    .config
                    .keys
                    .iter()
                    .all(|k| !old_ids.contains(&k.id))
            );
            volume.registry_close(replacement).unwrap();
        }
        let (mut journal, recovery) =
            Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        assert!(!recovery.replayed);
        journal.begin().unwrap();
        let mut volume =
            PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog)
                .unwrap();
        assert_eq!(volume.setting_get(user(1001), 1001, Setting::Theme), Ok(2));
        let live = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Parent"),
                Access::READ,
                false,
            )
            .unwrap();
        assert_eq!(volume.registry_get(&live, &[]), Ok(Value::dword(0x1234)));
        assert!(matches!(
            volume.registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Parent\\Child"),
                Access::READ,
                false
            ),
            Err(Error::NotFound)
        ));
        volume.registry_close(live).unwrap();
    }
}

#[test]
fn retained_deleted_keys_reopen_and_live_descendants_of_deleted_keys_are_corrupt() {
    use crate::storage::{Journal, WorkingSet};
    let mut disk = journal_disk();
    let (home, log) = partitions();
    let mut work = Box::new(WorkingSet::new());
    let mut state = State::boxed();
    let mut catalog = Box::new([0; CATALOG_BYTES]);
    {
        let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
        journal.begin().unwrap();
        let mut volume = PersistentVolume::initialize(
            Ext2::mount(journal).unwrap(),
            &mut state,
            &mut catalog,
            system(),
        )
        .unwrap();
        let access = Access::READ.union(Access::DELETE);
        let parent = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Parent"),
                access,
                true,
            )
            .unwrap();
        let child = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Parent\\Child"),
                access,
                true,
            )
            .unwrap();
        volume.registry_delete_key(&child).unwrap();
        volume.registry_delete_key(&parent).unwrap();
        volume.registry_close(parent).unwrap();
        volume
            .setting_set(user(1001), 1001, Setting::ScalePercent, 150)
            .unwrap();
        volume.commit().unwrap();
        assert_eq!(volume.registry_get(&child, &[]), Err(Error::KeyDeleted));
        let mut wire = Box::new([0; CATALOG_BYTES]);
        codec::encode(volume.state, &mut wire[..]).unwrap();
        let child_slot = volume
            .state
            .config
            .keys
            .iter()
            .position(|k| k.name == Name::ascii("Child").unwrap())
            .unwrap();
        wire[7296 + child_slot * 128 + 22] = 0;
        let checksum = codec::checksum(&wire[..]);
        wire[40..44].copy_from_slice(&checksum.to_le_bytes());
        let mut candidate = State::boxed();
        assert_eq!(
            codec::decode(&wire[..], &mut candidate),
            Err(Error::Corrupt)
        );
        // Simulated reboot drops the still-open child; its committed ancestor
        // chain must mount successfully before process-local opens are cleared.
    }
    let (mut journal, _) = Journal::open(&mut disk, home, log, [7; 16], &mut work).unwrap();
    journal.begin().unwrap();
    let mut volume =
        PersistentVolume::mount(Ext2::mount(journal).unwrap(), &mut state, &mut catalog).unwrap();
    assert!(volume.state.config.keys.iter().all(|k| !k.deleted));
    assert_eq!(
        volume.setting_get(user(1001), 1001, Setting::ScalePercent),
        Ok(150)
    );
    assert!(matches!(
        volume.registry_open(
            user(1001),
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &wide("Parent"),
            Access::READ,
            false
        ),
        Err(Error::NotFound)
    ));
    volume
        .setting_set(user(1001), 1001, Setting::Theme, 1)
        .unwrap();
    volume.commit().unwrap();
}
#[test]
fn registry_records_past_the_catalog_live_in_the_hive_across_remounts() {
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    let rw = Access::READ.union(Access::WRITE).union(Access::DELETE);
    let open = |v: &mut PersistentVolume<'_, _>, i: usize, create| {
        v.registry_open(
            user(1001),
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &wide(&std::format!("Software\\Doors\\T{i}")),
            rw,
            create,
        )
    };
    // 30 keys of 2 values: past the catalog's 24 + 24 slots (the test disk
    // holds a few blocks of hive).
    for i in 0..30 {
        let k = open(&mut volume, i, true).unwrap();
        for j in 0..2u32 {
            volume
                .registry_set(
                    &k,
                    &wide(&std::format!("V{j}")),
                    Value::dword(i as u32 * 100 + j),
                )
                .unwrap();
        }
        volume.registry_close(k).unwrap();
    }
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    for i in [0, 11, 12, 23, 24, 29] {
        let k = open(&mut volume, i, false).unwrap();
        assert_eq!(
            volume.registry_get(&k, &wide("V1")).unwrap(),
            Value::dword(i as u32 * 100 + 1)
        );
        volume.registry_close(k).unwrap();
    }
    // Deleting the later keys shrinks the hive; nothing comes back.
    for i in 20..30 {
        let k = open(&mut volume, i, false).unwrap();
        volume.registry_delete_key(&k).unwrap();
        volume.registry_close(k).unwrap();
    }
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    assert!(matches!(open(&mut volume, 25, false), Err(Error::NotFound)));
    let k = open(&mut volume, 19, false).unwrap();
    assert_eq!(
        volume.registry_get(&k, &wide("V1")).unwrap(),
        Value::dword(1901)
    );
    volume.registry_close(k).unwrap();
    let mut fs = volume.into_inner().unwrap();
    // A damaged hive refuses the mount.
    let hive = fs.open(HIVE_NAME).unwrap();
    let mut byte = [0];
    fs.read(hive, 200, &mut byte).unwrap();
    fs.write(hive, 200, &[byte[0] ^ 0x40]).unwrap();
    let mut reopened = State::boxed();
    assert!(matches!(
        PersistentVolume::mount(fs, &mut reopened, &mut buffer),
        Err(Error::Corrupt)
    ));
}
/// Registry names follow Windows' rules, not file names': a key may hold
/// '/', ':', '*' or '?' (MIME types such as "application/x-doors-reg"), a
/// value name anything but NUL (a program path); both survive a remount, in
/// the catalog and in the hive. A backslash still separates keys.
#[test]
fn registry_names_take_windows_characters_across_remounts() {
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    let rw = Access::READ.union(Access::WRITE).union(Access::DELETE);
    let keys = [
        "Software\\Classes\\MIME\\Database\\Content Type\\application/x-doors-reg",
        "Software\\Doors\\a:b*c?d\"e<f>g|h",
    ];
    // Past the catalog's slots too: the records go to the hive.
    for i in 0..26 {
        let k = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide(&std::format!("Software\\Filler\\K{i}")),
                rw,
                true,
            )
            .unwrap();
        volume.registry_close(k).unwrap();
    }
    for path in keys {
        let k = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide(path),
                rw,
                true,
            )
            .unwrap();
        volume
            .registry_set(&k, &wide("C:\\Apps\\tool.exe"), Value::dword(7))
            .unwrap();
        volume.registry_close(k).unwrap();
    }
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    for path in keys {
        let k = volume
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide(path),
                Access::READ,
                false,
            )
            .unwrap();
        assert_eq!(
            volume
                .registry_get(&k, &wide("C:\\Apps\\tool.exe"))
                .unwrap(),
            Value::dword(7),
            "{path}"
        );
        volume.registry_close(k).unwrap();
    }
    // NUL never; an empty component is skipped as before.
    assert!(matches!(
        volume.registry_open(
            user(1001),
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &[u16::from(b'a'), 0],
            Access::READ,
            false,
        ),
        Err(Error::InvalidName)
    ));
}
#[test]
fn registry_value_table_fills_at_its_capacity_without_partial_records() {
    with_volume(|v| {
        let k = v
            .registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &wide("Software"),
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        let mut stored = 0;
        let full = loop {
            match v.registry_set(&k, &wide(&std::format!("F{stored}")), Value::dword(1)) {
                Ok(()) => stored += 1,
                Err(e) => break e,
            }
        };
        assert_eq!(full, Error::NoSpace);
        assert_eq!(stored, config::VALUES);
        assert!(matches!(
            v.registry_get(&k, &wide(&std::format!("F{stored}"))),
            Err(Error::NotFound)
        ));
        v.registry_close(k).unwrap();
    });
}
#[test]
fn long_registry_values_chain_records_across_remounts() {
    use super::config::{MAX_VALUE_BYTES, RECORD_DATA};
    let mut state = State::boxed();
    let mut buffer = Box::new([0; CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    let rw = Access::READ.union(Access::WRITE).union(Access::DELETE);
    let open = |v: &mut PersistentVolume<'_, _>, create| {
        v.registry_open(
            user(1001),
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &wide(r"Software\Doors\Long"),
            rw,
            create,
        )
    };
    // A REG_EXPAND_SZ path of 300 units: 602 bytes, eight records.
    let text: std::vec::Vec<u8> = r"%SystemRoot%\System32\"
        .encode_utf16()
        .chain(core::iter::repeat_n(u16::from(b'x'), 278))
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(text.len(), 602);
    let k = open(&mut volume, true).unwrap();
    volume
        .registry_set(&k, &wide("Small"), Value::dword(7))
        .unwrap();
    volume
        .registry_set_data(&k, &wide("Path"), RegistryType::ExpandString, &text)
        .unwrap();
    // A malformed long string (no terminator) is refused whole.
    assert_eq!(
        volume.registry_set_data(&k, &wide("Bad"), RegistryType::Binary, &text[..400]),
        Ok(())
    );
    assert_eq!(
        volume.registry_set_data(&k, &wide("Bad"), RegistryType::String, &text[..401]),
        Err(Error::TypeMismatch)
    );
    assert_eq!(
        volume.registry_set_data(
            &k,
            &wide("Huge"),
            RegistryType::Binary,
            &[1; MAX_VALUE_BYTES + 1]
        ),
        Err(Error::BufferTooSmall)
    );
    volume.registry_close(k).unwrap();
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    let k = open(&mut volume, false).unwrap();
    let head = volume.registry_get(&k, &wide("Path")).unwrap();
    assert_eq!((head.total(), head.data().len()), (602, RECORD_DATA));
    // Read whole, and from an offset inside the fourth record.
    let mut out = [0u8; 700];
    assert_eq!(
        volume.registry_read(&k, &wide("Path"), 0, &mut out),
        Ok((RegistryType::ExpandString, 602, 602))
    );
    assert_eq!(&out[..602], &text[..]);
    let mut part = [0u8; 100];
    assert_eq!(
        volume.registry_read(&k, &wide("Path"), 250, &mut part),
        Ok((RegistryType::ExpandString, 602, 100))
    );
    assert_eq!(&part[..], &text[250..350]);
    // Enumeration lists each value once, in order.
    let observed = volume.registry_version(&k).unwrap();
    let names: std::vec::Vec<std::string::String> = (0..)
        .map_while(|i| volume.registry_enumerate_value(&k, i, observed).ok())
        .map(|(n, _)| std::string::String::from_utf16(n.units()).unwrap())
        .collect();
    assert_eq!(names, ["Small", "Path", "Bad"]);
    // A shorter value replaces every record of the longer one.
    volume
        .registry_set_data(&k, &wide("Path"), RegistryType::String, &text[..100])
        .unwrap_err();
    volume
        .registry_set(
            &k,
            &wide("Path"),
            Value::new(RegistryType::Binary, &[9; 10]).unwrap(),
        )
        .unwrap();
    volume.registry_delete_value(&k, &wide("Bad")).unwrap();
    volume.registry_close(k).unwrap();
    let fs = volume.into_inner().unwrap();
    let mut reopened = State::boxed();
    let mut volume = PersistentVolume::mount(fs, &mut reopened, &mut buffer).unwrap();
    let k = open(&mut volume, false).unwrap();
    assert_eq!(volume.registry_get(&k, &wide("Path")).unwrap().total(), 10);
    assert_eq!(volume.registry_get(&k, &wide("Bad")), Err(Error::NotFound));
    let used = reopened_values(&volume);
    assert_eq!(used, 2, "the old chain's records were all freed");
    volume.registry_close(k).unwrap();
}
fn reopened_values<F: crate::storage::Vfs>(volume: &PersistentVolume<'_, F>) -> usize {
    volume
        .state
        .config
        .values
        .iter()
        .filter(|e| e.key != 0)
        .count()
}

#[test]
fn administrators_write_hklm_and_hkcr_follows_the_token() {
    let admin = unsafe { Subject::authenticated(1001, 1 << ADMINISTRATORS, 1).unwrap() };
    with_volume(|v| {
        let p = wide(r"Software\Doors Sample");
        // A standard token may not create or write HKLM keys.
        assert!(matches!(
            v.registry_open(
                user(1001),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &p,
                Access::WRITE,
                true
            ),
            Err(Error::AccessDenied)
        ));
        // An elevated one may.
        let k = v
            .registry_open(
                admin,
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &p,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_set(
            &k,
            &wide("Runs"),
            Value::new(RegistryType::Dword, &42u32.to_le_bytes()).unwrap(),
        )
        .unwrap();
        v.registry_close(k).unwrap();
        let k = v
            .registry_open(
                user(1001),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &p,
                Access::READ,
                false,
            )
            .unwrap();
        assert_eq!(
            v.registry_get(&k, &wide("Runs")).unwrap().data(),
            &42u32.to_le_bytes()
        );
        v.registry_close(k).unwrap();
        // Writing HKCR: the user's own classes, or the machine's when elevated.
        let c = wide(".doorsample");
        let k = v
            .registry_open(
                user(1001),
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &c,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_close(k).unwrap();
        let classes = wide(r"Software\Classes\.doorsample");
        assert!(
            v.registry_open(
                user(1001),
                RegistryRoot::CurrentUser,
                RegistryView::Bits64,
                &classes,
                Access::READ,
                false
            )
            .is_ok()
        );
        assert!(matches!(
            v.registry_open(
                user(1001),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &classes,
                Access::READ,
                false
            ),
            Err(Error::NotFound)
        ));
        let m = wide(".doormachine");
        let k = v
            .registry_open(
                admin,
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &m,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_close(k).unwrap();
        let classes = wide(r"Software\Classes\.doormachine");
        assert!(
            v.registry_open(
                user(1002),
                RegistryRoot::LocalMachine,
                RegistryView::Bits64,
                &classes,
                Access::READ,
                false
            )
            .is_ok()
        );
        // Every user reads machine classes through HKCR.
        assert!(
            v.registry_open(
                user(1002),
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &m,
                Access::READ,
                false
            )
            .is_ok()
        );
        // A class a user registered for itself (HKCU) is seen through HKCR
        // by its unelevated programs, never by its elevated ones (COM
        // lookups: CLSID, AppID, TypeLib, Interface).
        let clsid = wide(r"CLSID\{0D0D0D0D-0000-4000-8000-000000000001}\LocalServer32");
        let k = v
            .registry_open(
                user(1001),
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &clsid,
                Access::READ.union(Access::WRITE),
                true,
            )
            .unwrap();
        v.registry_close(k).unwrap();
        let read = |v: &mut PersistentVolume<'_, _>, s: Subject| {
            v.registry_open(
                s,
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &clsid,
                Access::READ,
                false,
            )
            .map(|k| v.registry_close(k).unwrap())
        };
        assert!(read(v, user(1001)).is_ok());
        assert_eq!(read(v, admin), Err(Error::NotFound));
        // File associations still merge for both.
        assert!(
            v.registry_open(
                admin,
                RegistryRoot::ClassesRoot,
                RegistryView::Bits64,
                &c,
                Access::READ,
                false
            )
            .is_ok()
        );
    });
}

#[test]
fn windows_layout_lets_administrators_install_and_keeps_profiles_private() {
    let admin = unsafe { Subject::authenticated(1001, 1 << ADMINISTRATORS, 1).unwrap() };
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(crate::storage::tests::ext2_disk()).unwrap();
    let mut volume = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    for uid in [1001, 1002] {
        let home = std::format!("/users/{uid}");
        volume
            .mkdir(
                system(),
                Path::Native(&home),
                Context::root(),
                Some(Acl::owner_full(uid)),
            )
            .unwrap();
    }
    {
        let v = &mut volume;
        // An older volume: Program Files without the Administrators entry.
        let mut old = Acl::owner_full(1);
        old.integrity = 1;
        old.push(Ace {
            principal: Principal::Everyone,
            deny: false,
            access: Access::READ.union(Access::TRAVERSE),
            inherit: true,
        })
        .unwrap();
        v.mkdir(
            system(),
            Path::Native("/compat/Program Files"),
            Context::root(),
            Some(old),
        )
        .unwrap();
        assert!(v.ensure_windows_layout(system()).unwrap() > 0);
        // Idempotent.
        assert_eq!(v.ensure_windows_layout(system()), Ok(0));
        for p in [
            "/compat/Program Files",
            "/compat/Program Files (x86)",
            "/compat/Program Files/Common Files",
            "/compat/Program Files (x86)/Common Files",
            "/compat/ProgramData",
            "/compat/Windows/System32/CatRoot/{F750E6C3-38EE-11D1-85E5-00C04FC295EE}",
            "/compat/Windows/SysWOW64",
            "/compat/Windows/Fonts",
            "/users/Public/Documents",
        ] {
            assert!(
                v.metadata(user(1001), Path::Native(p), Context::root())
                    .unwrap()
                    .directory,
                "{p}"
            );
        }
        // A standard token cannot install; an elevated one can, and what it
        // creates stays readable by everyone and writable by administrators.
        assert_eq!(
            v.mkdir(
                user(1001),
                Path::Native("/compat/Program Files/Sample"),
                Context::root(),
                None
            ),
            Err(Error::AccessDenied)
        );
        v.mkdir(
            admin,
            Path::Native("/compat/Program Files/Sample"),
            Context::root(),
            None,
        )
        .unwrap();
        v.mkdir(
            admin,
            Path::Native("/compat/Program Files/Sample/data"),
            Context::root(),
            None,
        )
        .unwrap();
        assert!(
            v.metadata(
                user(1002),
                Path::Native("/compat/Program Files/Sample/data"),
                Context::root()
            )
            .is_ok()
        );
        assert_eq!(
            v.mkdir(
                user(1001),
                Path::Native("/compat/Program Files/Sample/more"),
                Context::root(),
                None
            ),
            Err(Error::AccessDenied)
        );
        // What the elevated installer made is owned by Administrators, not
        // by the user's own ID: the same user's unelevated programs get no
        // owner's rights over it (no new ACL, no taking it over).
        let sample = Path::Native("/compat/Program Files/Sample");
        let (acl, _) = v.security(admin, sample, Context::root()).unwrap();
        assert_eq!(acl.owner, ADMINISTRATORS_OWNER);
        let mut open = acl;
        open.entries[0] = Ace {
            principal: Principal::Everyone,
            deny: false,
            access: Access::ALL,
            inherit: true,
        };
        assert_eq!(
            v.set_security(user(1001), sample, Context::root(), open),
            Err(Error::AccessDenied)
        );
        let mut taken = acl;
        taken.owner = 1001;
        assert_eq!(
            v.set_security(user(1001), sample, Context::root(), taken),
            Err(Error::AccessDenied)
        );
        // Elevated, the same user may.
        v.set_security(admin, sample, Context::root(), taken)
            .unwrap();
        v.set_security(admin, sample, Context::root(), acl).unwrap();
        // Public folders are shared.
        v.mkdir(
            user(1002),
            Path::Native("/users/Public/Documents/shared"),
            Context::root(),
            None,
        )
        .unwrap();
        // The profile's known folders belong to its user; AppData is hidden.
        let appdata = v
            .metadata(
                user(1001),
                Path::Native("/users/1001/AppData"),
                Context::root(),
            )
            .unwrap();
        assert_eq!(appdata.attributes & 0x2, 0x2);
        for p in [
            "/users/1001/AppData/Local/Temp",
            "/users/1001/AppData/LocalLow",
            "/users/1001/AppData/Roaming",
            "/users/1001/Documents",
            "/users/1001/Downloads",
            "/users/1001/Desktop",
        ] {
            assert!(
                v.metadata(user(1001), Path::Native(p), Context::root())
                    .is_ok(),
                "{p}"
            );
            assert_eq!(
                v.metadata(user(1002), Path::Native(p), Context::root())
                    .err(),
                Some(Error::AccessDenied),
                "{p}"
            );
        }
        v.mkdir(
            user(1001),
            Path::Native("/users/1001/Documents/notes"),
            Context::root(),
            None,
        )
        .unwrap();
    }
}
/// The desktop appearance stores any of the 18 pictures (Captured Motion,
/// Sunrise, Flow ...), and the catalog keeps validating after encoding.
#[test]
fn desktop_appearance_survives_every_picture_through_the_catalog() {
    with_volume(|volume| {
        for value in [15, door_service_protocol::window::APPEARANCE_VALUES - 1] {
            volume
                .setting_set(user(1001), 1001, Setting::DesktopAppearance, value)
                .unwrap();
            let mut wire = Box::new([0; CATALOG_BYTES]);
            codec::encode(volume.state, &mut wire[..]).unwrap();
            let mut decoded = State::boxed();
            codec::decode(&wire[..], &mut decoded).unwrap();
            assert_eq!(
                volume.setting_get(user(1001), 1001, Setting::DesktopAppearance),
                Ok(value)
            );
        }
        assert!(
            volume
                .setting_set(
                    user(1001),
                    1001,
                    Setting::DesktopAppearance,
                    door_service_protocol::window::APPEARANCE_VALUES
                )
                .is_err()
        );
    });
}
/// Every preference word Settings stores (including the newest, Recent
/// images and Recent colors) is a writable per-user registry value.
#[test]
fn every_preference_word_is_writable_per_user() {
    with_volume(|volume| {
        for p in door_service_protocol::preferences::ALL {
            let setting = Setting::from_id(p.id).unwrap();
            assert_eq!(
                volume.setting_set(user(1001), 1001, setting, p.default),
                Ok(()),
                "{} ({})",
                p.name,
                p.id
            );
            assert_eq!(volume.setting_get(user(1001), 1001, setting), Ok(p.default));
        }
        let recent = door_service_protocol::preferences::RecentBackgrounds::DEFAULT.used(2);
        let setting =
            Setting::from_id(door_service_protocol::preferences::BACKGROUND_RECENT).unwrap();
        assert_eq!(
            volume.setting_set(user(1001), 1001, setting, recent.bits()),
            Ok(())
        );
    });
}
/// The desktop appearance is the HKCU dword `DesktopAppearance` (the typed
/// settings registry persists every key as one): a value from before (the
/// settings record) reads through until the first write, a Theme change
/// moves the dword's dark bit, and a damaged dword reads as invalid.
#[test]
fn desktop_appearance_is_a_per_user_dword() {
    with_volume(|volume| {
        let get = |v: &PersistentVolume<'_, _>| {
            v.setting_get(user(1001), 1001, Setting::DesktopAppearance)
        };
        volume
            .setting_set(user(1001), 1001, Setting::Theme, 1)
            .unwrap();
        assert_eq!(
            volume.registry_dword_get(1001, "DesktopAppearance"),
            Ok(None)
        );
        assert_eq!(get(volume), Ok(1));
        volume
            .setting_set(user(1001), 1001, Setting::DesktopAppearance, 6)
            .unwrap();
        assert_eq!(
            volume.registry_dword_get(1001, "DesktopAppearance"),
            Ok(Some(6))
        );
        assert_eq!(volume.setting_get(user(1001), 1001, Setting::Theme), Ok(0));
        volume
            .setting_set(user(1001), 1001, Setting::Theme, 1)
            .unwrap();
        assert_eq!(get(volume), Ok(7));
        volume
            .registry_dword_set(1001, "DesktopAppearance", u32::MAX)
            .unwrap();
        assert_eq!(get(volume), Err(Error::InvalidParameter));
    });
}
