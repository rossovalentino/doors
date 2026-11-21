//! Catalog v6 (P18 M0a): the namespace past the old 256-node catalog, on a
//! volume at scale: thousands of files and folders, 255-unit names, large
//! folders listed in object-ID order through the node cache, renames across
//! folders, removals, delete-pending reclaim at mount, and the move of an
//! older catalog's nodes into the tree.
use super::*;
use crate::storage::{Ext2, Format, format_ext2, tests::Disk};
use std::string::String;

fn scaled() -> Disk {
    let bytes = 96 * 1024 * 1024;
    let mut disk = Disk::new(bytes / 512);
    format_ext2(&mut disk, Format::sized(bytes, *b"DoorCatalogV6Vol"), true).unwrap();
    disk
}
fn long_name(i: usize, len: usize) -> Vec<u16> {
    let mut units: Vec<u16> = std::format!("{i:06}-").encode_utf16().collect();
    while units.len() < len {
        units.push(u16::from(b'a') + (units.len() % 26) as u16);
    }
    units
}
fn create(v: &mut PersistentVolume<'_, Ext2<Disk>>, path: &str, bytes: &[u8]) {
    let (h, _) = v
        .open(
            system(),
            Path::Native(path),
            Context::root(),
            options(Disposition::CreateNew),
        )
        .unwrap();
    v.write(&h, 0, bytes).unwrap();
    v.close(h).unwrap();
}
fn listing(v: &mut PersistentVolume<'_, Ext2<Disk>>, path: &str) -> Vec<(Vec<u16>, u64)> {
    let mut out = Vec::new();
    let mut after = 0;
    while let Some(e) = v
        .list(system(), Path::Native(path), Context::root(), after)
        .unwrap()
    {
        after = e.info.object;
        out.push((e.name.units().to_vec(), e.info.object));
    }
    out
}

#[test]
fn thousands_of_nodes_long_names_and_big_folders_survive_remount() {
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(scaled()).unwrap();
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    // 40 folders of 60 files (2,400 files, far past the 256-node catalog)
    // and one folder of 1,500.
    for d in 0..40 {
        v.mkdir(
            system(),
            Path::Native(&std::format!("/tmp/d{d}")),
            Context::root(),
            None,
        )
        .unwrap();
        for f in 0..60 {
            create(
                &mut v,
                &std::format!("/tmp/d{d}/f{f}.txt"),
                std::format!("{d}/{f}").as_bytes(),
            );
        }
        v.flush().unwrap();
    }
    v.mkdir(system(), Path::Native("/tmp/big"), Context::root(), None)
        .unwrap();
    let mut names = Vec::new();
    for i in 0..1500 {
        let units = long_name(i, if i % 7 == 0 { 255 } else { 40 + i % 100 });
        let path: Vec<u16> = "/tmp/big/"
            .encode_utf16()
            .chain(units.iter().copied())
            .collect();
        let text = String::from_utf16(&path).unwrap();
        create(&mut v, &text, &[i as u8; 3]);
        names.push(units);
        if i % 100 == 99 {
            v.flush().unwrap();
        }
    }
    // A 256-unit name is refused.
    let too_long: Vec<u16> = "/tmp/big/"
        .encode_utf16()
        .chain(long_name(9, 256))
        .collect();
    assert!(
        v.open(
            system(),
            Path::Native(&String::from_utf16(&too_long).unwrap()),
            Context::root(),
            options(Disposition::CreateNew)
        )
        .is_err()
    );
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    // Every file reads back by path.
    for d in (0..40).step_by(7) {
        for f in (0..60).step_by(11) {
            let (h, _) = v
                .open(
                    system(),
                    Path::Native(&std::format!("/tmp/d{d}/f{f}.txt")),
                    Context::root(),
                    options(Disposition::OpenExisting),
                )
                .unwrap();
            let mut out = [0u8; 8];
            let n = v.read(&h, 0, &mut out).unwrap();
            assert_eq!(&out[..n], std::format!("{d}/{f}").as_bytes());
            v.close(h).unwrap();
        }
    }
    // The big folder lists every entry once, in object-ID order.
    let listed = listing(&mut v, "/tmp/big");
    assert_eq!(listed.len(), 1500);
    assert!(listed.windows(2).all(|w| w[0].1 < w[1].1));
    let mut got: Vec<Vec<u16>> = listed.iter().map(|(n, _)| n.clone()).collect();
    got.sort();
    let mut want = names.clone();
    want.sort();
    assert_eq!(got, want);
    // Move a file across folders, remove a folder's files and the folder.
    let from = String::from_utf16(
        &"/tmp/big/"
            .encode_utf16()
            .chain(names[3].iter().copied())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let (h, _) = v
        .open(
            system(),
            Path::Native(&from),
            Context::root(),
            OpenOptions {
                access: Access::ALL,
                share: Access(7),
                disposition: Disposition::OpenExisting,
            },
        )
        .unwrap();
    v.rename(
        system(),
        &h,
        Path::Native("/tmp/d0/moved.bin"),
        Context::root(),
        false,
    )
    .unwrap();
    v.close(h).unwrap();
    for f in 0..60 {
        v.remove_path(
            system(),
            Path::Native(&std::format!("/tmp/d5/f{f}.txt")),
            Context::root(),
        )
        .unwrap();
    }
    v.remove_path(system(), Path::Native("/tmp/d5"), Context::root())
        .unwrap();
    v.flush().unwrap();
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    assert_eq!(listing(&mut v, "/tmp/big").len(), 1499);
    assert_eq!(listing(&mut v, "/tmp/d0").len(), 61);
    assert!(
        v.metadata(system(), Path::Native("/tmp/d5"), Context::root())
            .is_err()
    );
    let moved = v
        .metadata(system(), Path::Native("/tmp/d0/moved.bin"), Context::root())
        .unwrap();
    assert_eq!(moved.bytes, 3);
    // The tree holds exactly the live nodes: 16 system folders, 39 folders
    // of 60 files and one extra, the big folder, /tmp's 41 children.
    let file = v.nodes_file.unwrap();
    let mut work = Box::new(crate::storage::btree::Work::new());
    let mut tree = nodes::open(&mut v.fs, file, &mut work).unwrap();
    let records = tree.check(&mut work).unwrap();
    let live = 16 + 39 + 39 * 60 + 1 + 1 + 1499;
    // N, C and P records per node (the root has no parent records but is
    // its own: it keeps all three like any node).
    assert_eq!(records, 3 * live as u64);
}

#[test]
fn delete_pending_nodes_are_reclaimed_at_the_next_mount() {
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(scaled()).unwrap();
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    for i in 0..300 {
        create(&mut v, &std::format!("/tmp/p{i}"), b"x");
    }
    v.flush().unwrap();
    // Opened, then deleted while open: delete pending until the last close,
    // which a reboot stands in for.
    let mut held = Vec::new();
    for i in (0..300).step_by(3) {
        let (h, _) = v
            .open(
                system(),
                Path::Native(&std::format!("/tmp/p{i}")),
                Context::root(),
                OpenOptions {
                    access: Access::ALL,
                    share: Access(7),
                    disposition: Disposition::OpenExisting,
                },
            )
            .unwrap();
        v.delete(&h).unwrap();
        held.push(h);
    }
    v.flush().unwrap();
    core::mem::forget(held);
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    assert_eq!(listing(&mut v, "/tmp").len(), 200);
    v.flush().unwrap();
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    assert_eq!(listing(&mut v, "/tmp").len(), 200);
}

#[test]
fn an_older_catalog_moves_its_nodes_into_the_tree() {
    let mut state = State::boxed_production();
    let mut buffer = Box::new([0; PRODUCTION_CATALOG_BYTES]);
    let fs = Ext2::mount(scaled()).unwrap();
    let mut v = PersistentVolume::initialize(fs, &mut state, &mut buffer, system()).unwrap();
    for i in 0..100 {
        create(
            &mut v,
            &std::format!("/tmp/old{i}"),
            std::format!("{i}").as_bytes(),
        );
    }
    // Written as catalog v5 (every node in the catalog), as a P17 volume.
    v.state.tree = false;
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    assert!(v.state.tree);
    v.flush().unwrap();
    let fs = v.into_inner().unwrap();
    let mut state = State::boxed_production();
    let mut v = PersistentVolume::mount(fs, &mut state, &mut buffer).unwrap();
    assert!(v.state.tree);
    assert_eq!(listing(&mut v, "/tmp").len(), 100);
    let (h, _) = v
        .open(
            system(),
            Path::Native("/tmp/old42"),
            Context::root(),
            options(Disposition::OpenExisting),
        )
        .unwrap();
    let mut out = [0u8; 4];
    let n = v.read(&h, 0, &mut out).unwrap();
    assert_eq!(&out[..n], b"42");
    v.close(h).unwrap();
}
