//! P18 M0a: ext2 volumes at scale and the paged B+tree.
use super::btree::{self, PAGE, Pages, Tree, Work};
use super::tests::Disk;
use super::*;
extern crate std;
use std::boxed::Box;
use std::vec;
use std::vec::Vec;

fn scaled(bytes: u64) -> Disk {
    let mut disk = Disk::new(bytes / 512);
    let shape = Format::sized(bytes, *b"DoorP18ScaledVol");
    format_ext2(&mut disk, shape, true).unwrap();
    disk
}

#[test]
fn a_scaled_volume_mounts_with_many_groups_and_empty_counts() {
    // Two full groups and a short third.
    let disk = scaled((2 * 32_768 + 5_000) * 4096);
    let fs = Ext2::mount(disk).unwrap();
    let (total, free) = fs.usage().unwrap();
    assert_eq!(total, (2 * 32_768 + 5_000) * 4096);
    assert!(free > total * 9 / 10, "{free} of {total}");
}

#[test]
fn thousands_of_objects_grow_the_root_directory_and_survive_remount() {
    let disk = scaled(160 * 1024 * 1024);
    let mut fs = Ext2::mount(disk).unwrap();
    let before = fs.usage().unwrap().1;
    let mut ids = Vec::new();
    for i in 0..3000 {
        let name = std::format!("o{:02x}-object-{i}", i % 256);
        let id = fs.create(&name).unwrap();
        fs.write(id, 0, name.as_bytes()).unwrap();
        ids.push((name, id));
    }
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    for (name, id) in &ids {
        assert_eq!(fs.open(name).unwrap(), *id);
        let mut out = vec![0; name.len()];
        assert_eq!(fs.read(*id, 0, &mut out).unwrap(), name.len());
        assert_eq!(out, name.as_bytes());
    }
    // Rename into a new name (possibly another block), then remove all.
    fs.rename(&ids[0].0, "renamed-first").unwrap();
    assert_eq!(fs.open("renamed-first").unwrap(), ids[0].1);
    assert_eq!(fs.open(&ids[0].0), Err(Error::NotFound));
    fs.unlink("renamed-first", ids[0].1).unwrap();
    for (name, id) in &ids[1..] {
        fs.unlink(name, *id).unwrap();
    }
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    // The root directory keeps its grown blocks; every file block is back.
    let after = fs.usage().unwrap().1;
    assert!(before - after < 64 * 4096, "{before} {after}");
    let id = fs.create("again").unwrap();
    fs.write(id, 0, b"x").unwrap();
}

#[test]
fn double_indirect_files_beyond_four_megabytes_read_truncate_and_free() {
    let disk = scaled(160 * 1024 * 1024);
    let mut fs = Ext2::mount(disk).unwrap();
    let free = fs.usage().unwrap().1;
    let id = fs.create("coreclr.dll").unwrap();
    // 24 MiB and a tail: double-indirect blocks, written in 1 MiB pieces.
    let total = 24 * 1024 * 1024 + 4321;
    let byte = |i: usize| (i.wrapping_mul(2_654_435_761) >> 13) as u8;
    let mut offset = 0;
    while offset < total {
        let n = (1024 * 1024).min(total - offset);
        let piece: Vec<u8> = (offset..offset + n).map(byte).collect();
        assert_eq!(fs.write(id, offset as u64, &piece).unwrap(), n);
        offset += n;
    }
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    assert_eq!(fs.metadata(id).unwrap().bytes, total as u64);
    for at in [
        0usize,
        4095,
        12 * 4096,
        1036 * 4096 - 7,
        1037 * 4096 + 9,
        total - 100,
    ] {
        let mut out = [0u8; 100];
        let n = fs.read(id, at as u64, &mut out).unwrap();
        for (k, b) in out[..n].iter().enumerate() {
            assert_eq!(*b, byte(at + k), "at {}", at + k);
        }
    }
    // Shrink into the single-indirect range, then grow across a gap.
    fs.truncate(id, 2 * 1024 * 1024 + 3).unwrap();
    fs.write(id, 9 * 1024 * 1024, b"grown").unwrap();
    let mut out = [9u8; 16];
    fs.read(id, 9 * 1024 * 1024 - 11, &mut out).unwrap();
    assert_eq!(&out[..11], &[0; 11]);
    assert_eq!(&out[11..], b"grown");
    fs.truncate(id, 0).unwrap();
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    assert_eq!(fs.usage().unwrap().1, free);
    fs.unlink("coreclr.dll", id).unwrap();
}

#[test]
fn a_full_scaled_volume_refuses_before_changing_anything() {
    let disk = scaled(33_000 * 4096);
    let mut fs = Ext2::mount(disk).unwrap();
    let free = fs.usage().unwrap().1;
    let id = fs.create("big").unwrap();
    assert_eq!(
        fs.write(id, 0, &vec![1; (free + 4096) as usize]),
        Err(Error::NoSpace)
    );
    assert!(!fs.is_poisoned());
    assert_eq!(fs.metadata(id).unwrap().bytes, 0);
}

#[test]
fn backups_follow_sparse_super() {
    let groups: Vec<u32> = (0..60).filter(|&g| super::ext2::has_backup(g)).collect();
    assert_eq!(groups, [0, 1, 3, 5, 7, 9, 25, 27, 49]);
}

struct Memory(Vec<[u8; PAGE]>);
impl Pages for Memory {
    fn read(&mut self, page: u32, out: &mut [u8; PAGE]) -> Result<()> {
        out.copy_from_slice(self.0.get(page as usize).ok_or(Error::Bounds)?);
        Ok(())
    }
    fn write(&mut self, page: u32, bytes: &[u8; PAGE]) -> Result<()> {
        let page = page as usize;
        if self.0.len() <= page {
            self.0.resize(page + 1, [0; PAGE]);
        }
        self.0[page] = *bytes;
        Ok(())
    }
}
fn key_of(i: u32) -> Vec<u8> {
    // A parent id then a name of varying length: ordered like a catalog.
    let mut k = (u64::from(i % 97)).to_be_bytes().to_vec();
    let len = 1 + (i as usize * 7919) % 200;
    k.extend((0..len).map(|j| b'a' + ((i as usize + j * 31) % 26) as u8));
    k.extend(i.to_be_bytes());
    k
}
fn value_of(i: u32) -> Vec<u8> {
    let len = (i as usize * 104_729) % 600;
    (0..len).map(|j| (i as usize + j) as u8).collect()
}

#[test]
fn the_btree_inserts_finds_scans_deletes_and_reopens() {
    let mut work = Box::new(Work::new());
    let mut tree = Tree::create(Memory(Vec::new()), &mut work).unwrap();
    let n = 20_000u32;
    for i in 0..n {
        assert!(tree.put(&key_of(i), &value_of(i), &mut work).unwrap());
    }
    assert_eq!(tree.check(&mut work).unwrap(), u64::from(n));
    assert!(tree.header().height >= 3, "{:?}", tree.header());
    // Replace some, delete a third.
    for i in (0..n).step_by(5) {
        assert!(!tree.put(&key_of(i), b"replaced", &mut work).unwrap());
    }
    for i in (0..n).step_by(3) {
        assert!(tree.delete(&key_of(i), &mut work).unwrap());
    }
    assert!(!tree.delete(&key_of(0), &mut work).unwrap());
    let mut tree = Tree::open(tree.into_pages(), &mut work).unwrap();
    let live = tree.check(&mut work).unwrap();
    assert_eq!(live, u64::from(n - n.div_ceil(3)));
    let mut out = vec![0u8; btree::MAX_VALUE];
    for i in 0..n {
        let got = tree.get(&key_of(i), &mut out, &mut work).unwrap();
        if i % 3 == 0 {
            assert_eq!(got, None);
        } else if i % 5 == 0 {
            assert_eq!(&out[..got.unwrap()], b"replaced");
        } else {
            assert_eq!(&out[..got.unwrap()], value_of(i).as_slice());
        }
    }
    // A full scan in key order.
    let mut expected: Vec<Vec<u8>> = (0..n).filter(|i| i % 3 != 0).map(key_of).collect();
    expected.sort();
    let mut k = vec![0u8; btree::MAX_KEY];
    let mut cursor: Vec<u8> = vec![0];
    let mut seen: Vec<Vec<u8>> = Vec::new();
    while let Some((kl, _)) = tree
        .next(&cursor, seen.is_empty(), &mut k, &mut out, &mut work)
        .unwrap()
    {
        cursor = k[..kl].to_vec();
        seen.push(cursor.clone());
    }
    assert_eq!(seen, expected);
    // A prefix scan: one parent's children.
    let parent = 5u64.to_be_bytes();
    let mut first = tree
        .next(&parent, true, &mut k, &mut out, &mut work)
        .unwrap();
    let mut children = 0;
    while let Some((kl, _)) = first {
        if k[..8] != parent {
            break;
        }
        children += 1;
        let at = k[..kl].to_vec();
        first = tree.next(&at, false, &mut k, &mut out, &mut work).unwrap();
    }
    assert_eq!(
        children,
        expected.iter().filter(|e| e[..8] == parent).count()
    );
}

#[test]
fn the_btree_refuses_oversized_records_and_detects_corruption() {
    let mut work = Box::new(Work::new());
    let mut tree = Tree::create(Memory(Vec::new()), &mut work).unwrap();
    assert_eq!(
        tree.put(&[1; btree::MAX_KEY + 1], b"", &mut work),
        Err(Error::Unsupported)
    );
    assert_eq!(
        tree.put(b"k", &[0; btree::MAX_VALUE + 1], &mut work),
        Err(Error::Unsupported)
    );
    for i in 0..3000 {
        tree.put(&key_of(i), &value_of(i), &mut work).unwrap();
    }
    let mut pages = tree.into_pages();
    // Swap the first two slots of a leaf with several cells.
    let leaf = (1..pages.0.len())
        .rev()
        .find(|&p| pages.0[p][0] == 1 && u16::from_le_bytes([pages.0[p][2], pages.0[p][3]]) >= 2)
        .unwrap();
    let page = &mut pages.0[leaf];
    let a = u16::from_le_bytes([page[12], page[13]]);
    let b = u16::from_le_bytes([page[14], page[15]]);
    page[12..14].copy_from_slice(&b.to_le_bytes());
    page[14..16].copy_from_slice(&a.to_le_bytes());
    let mut tree = Tree::open(pages, &mut work).unwrap();
    assert_eq!(tree.check(&mut work), Err(Error::Corrupt));
}
