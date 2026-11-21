extern crate std;
use super::*;
use std::{cell::Cell, collections::BTreeMap, rc::Rc, vec};

#[test]
fn ext2_mount_storage_fits_the_guarded_kernel_stack_budget() {
    // The guest keeps the original 128 KiB boot stack. Mount/Result moves can
    // coexist, so growing this inline cache requires an explicit stack review.
    assert!(core::mem::size_of::<Ext2<&mut Disk>>() <= 6144);
}
#[derive(Clone)]
pub(crate) struct Disk {
    sectors: u64,
    data: BTreeMap<u64, [u8; 512]>,
    read_error: bool,
    write_error: Rc<Cell<bool>>,
    flush_error: Rc<Cell<bool>>,
    fail_after_writes: Rc<Cell<Option<usize>>>,
    flushes: usize,
    /// Power cut: writes left before the cut (see `power_cut_after`).
    cut: Option<usize>,
    /// The cut write lands its first sector.
    torn: bool,
    /// Writes issued (counted while a cut is armed).
    writes: Rc<Cell<usize>>,
}
impl Disk {
    pub(crate) fn add_empty_tail(&mut self, sectors: u64) {
        self.sectors = sectors;
    }
    pub(crate) fn fail_writes(&self) -> Rc<Cell<bool>> {
        self.write_error.clone()
    }
    /// Cut power at write number `cut` from now (None: never; Some(0) before
    /// any write): earlier writes land, the cut write lands only its first
    /// sector when `torn` (else nothing), and every later write or flush fails
    /// without touching the media. Returns this clone's counter of writes
    /// attempted from now on.
    pub(crate) fn power_cut_after(&mut self, cut: Option<usize>, torn: bool) -> Rc<Cell<usize>> {
        self.cut = cut;
        self.torn = torn;
        self.writes = Rc::new(Cell::new(0));
        self.writes.clone()
    }
    pub(crate) fn new(sectors: u64) -> Self {
        Self {
            sectors,
            data: BTreeMap::new(),
            read_error: false,
            write_error: Rc::new(Cell::new(false)),
            flush_error: Rc::new(Cell::new(false)),
            fail_after_writes: Rc::new(Cell::new(None)),
            flushes: 0,
            cut: None,
            torn: false,
            writes: Rc::new(Cell::new(0)),
        }
    }
    fn sector(&mut self, lba: u64) -> &mut [u8; 512] {
        self.data.entry(lba).or_insert([0; 512])
    }
    fn block(&mut self, block: u32, b: &[u8; 4096]) {
        for (i, s) in b.chunks_exact(512).enumerate() {
            self.data
                .insert(u64::from(block) * 8 + i as u64, s.try_into().unwrap());
        }
    }
}
impl BlockDevice for Disk {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: self.sectors,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(&mut self, lba: u64, out: &mut [u8]) -> Result<()> {
        check_transfer(self.geometry(), lba, out.len())?;
        for (i, s) in out.chunks_exact_mut(512).enumerate() {
            s.copy_from_slice(self.data.get(&(lba + i as u64)).unwrap_or(&[0; 512]));
            if self.read_error {
                return Err(Error::ShortIo);
            }
        }
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, b: &[u8]) -> Result<()> {
        check_transfer(self.geometry(), lba, b.len())?;
        self.writes.set(self.writes.get() + 1);
        match self.cut {
            Some(0) => return Err(Error::Io),
            Some(1) => {
                self.cut = Some(0);
                if self.torn {
                    self.data.insert(lba, b[..512].try_into().unwrap());
                }
                return Err(Error::Io);
            }
            Some(n) => self.cut = Some(n - 1),
            None => (),
        }
        let fail = match self.fail_after_writes.get() {
            Some(0) => true,
            Some(n) => {
                self.fail_after_writes.set(Some(n - 1));
                false
            }
            None => false,
        };
        for (i, s) in b.chunks_exact(512).enumerate() {
            self.data.insert(lba + i as u64, s.try_into().unwrap());
            if self.write_error.get() || fail {
                return Err(Error::ShortIo);
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        self.flushes += 1;
        if self.flush_error.get() || self.cut == Some(0) {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }
}
pub(crate) fn ext2_disk() -> Disk {
    let mut disk = Disk::new(32 * 8);
    let mut block = [0; 4096];
    let s = &mut block[1024..2048];
    for (p, v) in [
        (0, 128),
        (4, 32),
        (12, 23),
        (16, 118),
        (20, 0),
        (24, 2),
        (28, 2),
        (32, 8192),
        (36, 8192),
        (40, 128),
        (76, 1),
        (84, 11),
        (96, 2),
    ] {
        put32(s, p, v);
    }
    put16(s, 56, 0xef53);
    put16(s, 58, 1);
    put16(s, 88, 128);
    s[104..120].fill(0x42);
    disk.block(0, &block);
    block.fill(0);
    for (p, v) in [(0, 2), (4, 3), (8, 4)] {
        put32(&mut block, p, v);
    }
    put16(&mut block, 12, 23);
    put16(&mut block, 14, 118);
    put16(&mut block, 16, 1);
    disk.block(1, &block);
    block.fill(0xff);
    block[0] = 0xff;
    block[1] = 1;
    block[2] = 0;
    block[3] = 0;
    disk.block(2, &block);
    block.fill(0xff);
    block[..16].fill(0);
    block[0] = 0xff;
    block[1] = 3;
    disk.block(3, &block);
    block.fill(0);
    let inode = &mut block[128..256];
    put16(inode, 0, 0x41ed);
    put32(inode, 4, 4096);
    put16(inode, 26, 2);
    put32(inode, 28, 8);
    put32(inode, 40, 8);
    disk.block(4, &block);
    block.fill(0);
    put32(&mut block, 0, 2);
    put16(&mut block, 4, 12);
    block[6] = 1;
    block[7] = 2;
    block[8] = b'.';
    put32(&mut block, 12, 2);
    put16(&mut block, 16, 4084);
    block[18] = 2;
    block[19] = 2;
    block[20..22].copy_from_slice(b"..");
    disk.block(8, &block);
    disk
}
#[test]
fn ext2_single_indirect_four_megabytes_reclaim_and_graph_validation() {
    let mut disk = ext2_disk();
    disk.add_empty_tail(2048 * 8);
    let mut b = [0; 4096];
    disk.read_sectors(0, &mut b).unwrap();
    put32(&mut b, 1028, 2048);
    put32(&mut b, 1036, 2039);
    disk.block(0, &b);
    disk.read_sectors(8, &mut b).unwrap();
    put16(&mut b, 12, 2039);
    disk.block(1, &b);
    b.fill(0xff);
    b[..256].fill(0);
    b[0] = 0xff;
    b[1] = 1;
    disk.block(2, &b);
    let mut fs = Ext2::mount(disk).unwrap();
    let id = fs.create("real-app.elf").unwrap();
    let content = vec![0x95; 4 * 1024 * 1024 + 17];
    fs.write(id, 0, &content).unwrap();
    let disk = fs.into_inner().unwrap();
    let mut fs = Ext2::mount(disk.clone()).unwrap();
    let mut actual = vec![0; content.len()];
    assert_eq!(fs.read(id, 0, &mut actual).unwrap(), content.len());
    assert_eq!(actual, content);
    fs.truncate(id, 12 * 4096 - 3).unwrap();
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    fs.write(id, 12 * 4096 + 5, b"tail").unwrap();
    let mut boundary = [1; 12];
    assert_eq!(fs.read(id, 12 * 4096 - 3, &mut boundary).unwrap(), 12);
    assert_eq!(&boundary[..8], &[0; 8]);
    assert_eq!(&boundary[8..], b"tail");
    fs.unlink("real-app.elf", id).unwrap();
    let mut reclaimed = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    let new = reclaimed.create("reused").unwrap();
    reclaimed.write(new, 0, &content).unwrap();
    // A checksum-free standard ext2 pointer table still must reject aliases.
    let mut corrupt = disk;
    let mut inode_block = [0; 4096];
    corrupt.read_sectors(32, &mut inode_block).unwrap();
    let p = (id.inode as usize - 1) * 128;
    let indirect = u32le(&inode_block, p + 88);
    let direct = u32le(&inode_block, p + 40);
    corrupt
        .read_sectors(u64::from(indirect) * 8, &mut b)
        .unwrap();
    put32(&mut b, 0, direct);
    corrupt.block(indirect, &b);
    assert!(matches!(Ext2::mount(corrupt), Err(Error::Corrupt)));
}
#[test]
fn ext2_indirect_allocation_failure_is_atomic_and_does_not_poison() {
    let mut fs = Ext2::mount(ext2_disk()).unwrap();
    let id = fs.create("chosen-name").unwrap();
    fs.write(id, 0, &[0x71; 12 * 4096]).unwrap();
    assert_eq!(
        fs.write(id, 12 * 4096, &[9; 11 * 4096]),
        Err(Error::NoSpace)
    );
    assert!(!fs.is_poisoned());
    assert_eq!(fs.metadata(id).unwrap().bytes, 12 * 4096);
    fs.write(id, 12 * 4096, b"actual indirect bytes").unwrap();
    let mut fs = Ext2::mount(fs.into_inner().unwrap()).unwrap();
    let mut out = [0; 21];
    assert_eq!(fs.read(id, 12 * 4096, &mut out).unwrap(), 21);
    assert_eq!(&out, b"actual indirect bytes");
}
#[test]
fn ext2_unlink_read_failure_before_and_after_mutation_has_explicit_poison() {
    use std::cell::RefCell;
    struct FailRead {
        disk: Rc<RefCell<Disk>>,
        left: Rc<Cell<usize>>,
    }
    impl BlockDevice for FailRead {
        fn geometry(&self) -> Geometry {
            self.disk.borrow().geometry()
        }
        fn read_sectors(&mut self, lba: u64, out: &mut [u8]) -> Result<()> {
            let n = self.left.get();
            if n == 0 {
                return Err(Error::Io);
            }
            self.left.set(n - 1);
            self.disk.borrow_mut().read_sectors(lba, out)
        }
        fn write_sectors(&mut self, lba: u64, data: &[u8]) -> Result<()> {
            self.disk.borrow_mut().write_sectors(lba, data)
        }
        fn flush(&mut self) -> Result<()> {
            self.disk.borrow_mut().flush()
        }
    }
    let mut fs = Ext2::mount(ext2_disk()).unwrap();
    let id = fs.create("remove").unwrap();
    fs.write(id, 0, &[0x44; 6000]).unwrap();
    let initial = fs.into_inner().unwrap();
    let mut after_mutation = 0;
    let mut before_mutation = 0;
    for ordinal in 0..32 {
        let disk = Rc::new(RefCell::new(initial.clone()));
        let left = Rc::new(Cell::new(usize::MAX));
        let mut fs = Ext2::mount(FailRead {
            disk: disk.clone(),
            left: left.clone(),
        })
        .unwrap();
        left.set(ordinal);
        if fs.unlink("remove", id).is_err() {
            let changed = disk.borrow().data != initial.data;
            if changed {
                after_mutation += 1;
                assert!(fs.is_poisoned(), "readordinal{ordinal}");
            } else {
                before_mutation += 1;
            }
        }
    }
    assert!(after_mutation > 0 && before_mutation > 0);
}
#[test]
fn ext2_real_format_write_rename_open_identity_coherence_and_remount() {
    let disk = ext2_disk();
    let fs = Ext2::mount(disk).unwrap();
    let mut mount = Mount::new("/data", fs).unwrap();
    let first = mount.create("/data/pending.bin").unwrap();
    let second = mount.open("/data/pending.bin").unwrap();
    assert_eq!(first, second);
    let mut bytes = vec![0; 6000];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = ((i * 37 + 11) % 251) as u8;
    }
    assert_eq!(mount.write(first, 0, &bytes), Ok(6000));
    let patch = vec![0xa5; 1024];
    mount.write(second, 3584, &patch).unwrap();
    bytes[3584..4608].copy_from_slice(&patch);
    mount
        .rename("/data/pending.bin", "/data/persist.bin")
        .unwrap();
    assert_eq!(mount.open("/data/pending.bin"), Err(Error::NotFound));
    assert_eq!(mount.open("/data/persist.bin").unwrap(), first);
    let mut read = vec![0; 6000];
    assert_eq!(mount.read(second, 0, &mut read), Ok(6000));
    assert_eq!(read, bytes);
    mount.flush().unwrap();
    let disk = mount.into_inner().into_inner().unwrap();
    assert!(disk.flushes >= 3);
    let mut reopened = Ext2::mount(disk).unwrap();
    let id = reopened.open("persist.bin").unwrap();
    assert_eq!(id, first.id);
    reopened.read(id, 0, &mut read).unwrap();
    assert_eq!(read, bytes);
}
#[test]
fn ext2_disk_full_is_preflighted_and_existing_contents_remain_readable() {
    let mut fs = Ext2::mount(ext2_disk()).unwrap();
    let a = fs.create("a").unwrap();
    let b = fs.create("b").unwrap();
    fs.write(a, 0, &vec![7; 12 * 4096]).unwrap();
    fs.write(b, 0, &vec![9; 11 * 4096]).unwrap();
    assert_eq!(fs.write(b, 11 * 4096, &[1]), Err(Error::NoSpace));
    let mut read = [0; 3];
    fs.read(a, 0, &mut read).unwrap();
    assert_eq!(read, [7; 3]);
    fs.flush().unwrap();
    let disk = fs.into_inner().unwrap();
    let mut reopened = Ext2::mount(disk).unwrap();
    assert_eq!(reopened.metadata(b).unwrap().bytes, 11 * 4096);
}
#[test]
fn ext2_short_io_dirty_media_and_unsupported_features_fail_explicitly() {
    let mut disk = ext2_disk();
    disk.read_error = true;
    assert!(matches!(Ext2::mount(disk), Err(Error::ShortIo)));
    let mut disk = ext2_disk();
    put16(disk.sector(2), 58, 0);
    assert!(matches!(Ext2::mount(disk), Err(Error::Corrupt)));
    let mut disk = ext2_disk();
    put32(disk.sector(2), 96, 0x42);
    assert!(matches!(Ext2::mount(disk), Err(Error::Unsupported)));
    let mut disk = ext2_disk();
    put16(disk.sector(64), 4, 0);
    assert!(matches!(Ext2::mount(disk), Err(Error::Corrupt)));
}
#[test]
fn ext2_write_and_flush_errors_poison_instead_of_acknowledging() {
    let disk = ext2_disk();
    let write_fault = disk.write_error.clone();
    let mut fs = Ext2::mount(disk).unwrap();
    write_fault.set(true);
    assert_eq!(fs.create("broken"), Err(Error::ShortIo));
    assert!(fs.is_poisoned());
    assert_eq!(fs.create("later"), Err(Error::Poisoned));
    assert_eq!(fs.flush(), Err(Error::Poisoned));
    let disk = ext2_disk();
    let flush_fault = disk.flush_error.clone();
    let mut fs = Ext2::mount(disk).unwrap();
    let id = fs.create("safe").unwrap();
    fs.write(id, 0, b"abc").unwrap();
    flush_fault.set(true);
    assert_eq!(fs.flush(), Err(Error::Io));
    assert_eq!(fs.read(id, 0, &mut [0; 3]), Err(Error::Poisoned));
}
#[test]
fn ext2_zero_fills_gap_and_rejects_wrong_volume_and_path_aliases() {
    let mut fs = Ext2::mount(ext2_disk()).unwrap();
    let id = fs.create("gap").unwrap();
    fs.write(id, 5000, b"end").unwrap();
    let mut bytes = vec![1; 5003];
    fs.read(id, 0, &mut bytes).unwrap();
    assert!(bytes[..5000].iter().all(|b| *b == 0));
    assert_eq!(&bytes[5000..], b"end");
    let mut wrong = id;
    wrong.volume[0] ^= 1;
    assert_eq!(fs.read(wrong, 0, &mut [0]), Err(Error::NotFound));
    assert_eq!(fs.create("../escape"), Err(Error::InvalidName));
    assert_eq!(fs.rename("gap", "gap"), Ok(()));
}
#[test]
fn ext2_extension_never_exposes_imported_garbage_past_eof() {
    for offset in [200, 5000] {
        let mut fs = Ext2::mount(ext2_disk()).unwrap();
        let id = fs.create("imported").unwrap();
        fs.write(id, 0, b"x").unwrap();
        let mut disk = fs.into_inner().unwrap();
        // A file tail is outside the ext2 logical size and need not be zero on
        // imported media. First free data block is nine in this fixture.
        let mut garbage = [0xa5; 4096];
        garbage[0] = b'x';
        disk.block(9, &garbage);
        let mut reopened = Ext2::mount(disk).unwrap();
        reopened.write(id, offset, b"z").unwrap();
        let mut bytes = vec![0xcc; offset as usize + 1];
        reopened.read(id, 0, &mut bytes).unwrap();
        assert_eq!(bytes[0], b'x');
        assert!(bytes[1..offset as usize].iter().all(|b| *b == 0));
        assert_eq!(bytes[offset as usize], b'z');
    }
}
#[test]
fn ext2_failure_after_dirty_marker_requires_external_repair_before_remount() {
    for writes_before_failure in 1..=4 {
        let mut disk = ext2_disk();
        let budget = disk.fail_after_writes.clone();
        {
            let mut fs = Ext2::mount(&mut disk).unwrap();
            budget.set(Some(writes_before_failure));
            assert_eq!(fs.create("interrupted"), Err(Error::ShortIo));
            assert!(fs.is_poisoned());
            assert_eq!(fs.flush(), Err(Error::Poisoned));
        }
        budget.set(None);
        assert!(matches!(Ext2::mount(&mut disk), Err(Error::Corrupt)));
    }
}
fn gpt_disk() -> Disk {
    let mut d = Disk::new(128);
    let m = d.sector(0);
    m[450] = 0xee;
    put32(m, 454, 1);
    put32(m, 458, 127);
    m[510..].copy_from_slice(&[0x55, 0xaa]);
    let mut table = [0; 16384];
    table[..16].fill(1);
    table[16..32].fill(2);
    table[32..40].copy_from_slice(&34u64.to_le_bytes());
    table[40..48].copy_from_slice(&90u64.to_le_bytes());
    let crc = crc32(&table);
    for i in 0..32 {
        let b: &[u8; 512] = table[i * 512..(i + 1) * 512].try_into().unwrap();
        d.data.insert(2 + i as u64, *b);
        d.data.insert(95 + i as u64, *b);
    }
    for (current, other, table) in [(1, 127, 2), (127, 1, 95)] {
        let h = d.sector(current);
        h[..8].copy_from_slice(b"EFI PART");
        put32(h, 8, 0x10000);
        put32(h, 12, 92);
        for (p, v) in [(24, current), (32, other), (40, 34), (48, 94), (72, table)] {
            h[p..p + 8].copy_from_slice(&v.to_le_bytes());
        }
        h[56..72].fill(3);
        put32(h, 80, 128);
        put32(h, 84, 128);
        put32(h, 88, crc);
        let c = crc32(&h[..92]);
        put32(h, 16, c);
    }
    d
}
#[test]
fn gpt_validates_both_headers_arrays_bounds_and_protective_mbr() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    let mut disk = gpt_disk();
    let g = Gpt::read(&mut disk).unwrap();
    let p = g.partitions().next().unwrap();
    assert_eq!((p.first, p.sectors), (34, 57));
    for (lba, offset) in [(0, 450), (1, 16), (127, 56), (95, 20), (2, 48)] {
        let mut d = gpt_disk();
        d.sector(lba)[offset] ^= 1;
        assert!(matches!(Gpt::read(&mut d), Err(Error::Corrupt)));
    }
    let mut part = PartitionDevice::new(disk, p).unwrap();
    assert_eq!(part.read_sectors(57, &mut [0; 512]), Err(Error::Bounds));
    assert_eq!(part.read_sectors(0, &mut [0; 511]), Err(Error::Alignment));
}
fn fat_disk() -> Disk {
    let mut d = Disk::new(131072);
    let mut boot = [0; 512];
    put16(&mut boot, 11, 512);
    boot[13] = 1;
    put16(&mut boot, 14, 32);
    boot[16] = 2;
    boot[21] = 0xf8;
    put32(&mut boot, 32, 131072);
    put32(&mut boot, 36, 1009);
    put32(&mut boot, 44, 2);
    put16(&mut boot, 48, 1);
    put16(&mut boot, 50, 6);
    boot[66] = 0x29;
    put32(&mut boot, 67, 42);
    boot[510..].copy_from_slice(&[0x55, 0xaa]);
    d.data.insert(0, boot);
    d.data.insert(6, boot);
    let mut info = [0; 512];
    put32(&mut info, 0, 0x4161_5252);
    put32(&mut info, 484, 0x6141_7272);
    put32(&mut info, 488, 129020);
    put32(&mut info, 492, 4);
    put32(&mut info, 508, 0xaa55_0000);
    d.data.insert(1, info);
    d.data.insert(7, info);
    let mut fat = [0; 512];
    for (p, v) in [
        (0, 0x0fff_fff8),
        (4, 0x0fff_ffff),
        (8, 0x0fff_ffff),
        (12, 0x0fff_ffff),
    ] {
        put32(&mut fat, p, v);
    }
    d.data.insert(32, fat);
    d.data.insert(1041, fat);
    let root = d.sector(2050);
    root[..11].copy_from_slice(b"HELLO   TXT");
    root[11] = 0x20;
    put16(root, 26, 3);
    put32(root, 28, 20);
    d.sector(2051)[..20].copy_from_slice(b"Door FAT32 exchange\n");
    d
}
#[test]
fn fat32_reads_standard_exchange_and_rejects_chain_and_backup_corruption() {
    let mut fs = Fat32::mount(fat_disk()).unwrap();
    let id = fs.open("hello.txt").unwrap();
    let mut out = [0; 20];
    assert_eq!(fs.read(id, 0, &mut out), Ok(20));
    assert_eq!(&out, b"Door FAT32 exchange\n");
    assert_eq!(fs.write(id, 0, b"x"), Err(Error::ReadOnly));
    let mut d = fat_disk();
    put32(d.sector(32), 12, 3);
    put32(d.sector(1041), 12, 3);
    assert!(matches!(Fat32::mount(d), Err(Error::Corrupt)));
    let mut d = fat_disk();
    d.sector(6)[13] = 2;
    assert!(matches!(Fat32::mount(d), Err(Error::Corrupt)));
}
#[test]
fn mount_identity_rejects_foreign_open_handles_and_prefix_confusion() {
    let mut a = Mount::new("/data", Ext2::mount(ext2_disk()).unwrap()).unwrap();
    let id = a.create("/data/file").unwrap();
    let mut b = Mount::new("/data", Ext2::mount(ext2_disk()).unwrap()).unwrap();
    assert_eq!(b.read(id, 0, &mut [0]), Err(Error::NotFound));
    assert_eq!(a.open("/database/file"), Err(Error::NotFound));
}
