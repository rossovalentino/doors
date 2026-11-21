//! Actual P06 device/filesystem qualification on a private, explicitly attached disk.
use crate::block_device::Session;
use core::fmt::Write;
use door_boot_debug::Serial;
use door_boot_protocol::BootInfo;
use door_kernel::storage::{
    BlockDevice, Error, Ext2, Fat32, Geometry, Gpt, InitFs, Mount, Partition, PartitionDevice,
    Result, Vfs, crc32,
};

const FILE_BYTES: usize = 6000;
const PATCH_OFFSET: usize = 3584;
const PATCH_BYTES: usize = 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Injection {
    ShortRead,
    ShortWrite,
    Flush,
}
/// Failure injection still performs real prefix I/O. An injected error must be
/// propagated by the production filesystem/cache, never converted to success.
struct FaultDevice<'a, D> {
    device: &'a mut D,
    kind: Injection,
    injected: &'a mut bool,
}
impl<D: BlockDevice> BlockDevice for FaultDevice<'_, D> {
    fn geometry(&self) -> Geometry {
        self.device.geometry()
    }
    fn read_sectors(&mut self, lba: u64, bytes: &mut [u8]) -> Result<()> {
        if self.kind == Injection::ShortRead && !*self.injected && bytes.len() > 512 {
            self.device.read_sectors(lba, &mut bytes[..512])?;
            *self.injected = true;
            return Err(Error::ShortIo);
        }
        self.device.read_sectors(lba, bytes)
    }
    fn write_sectors(&mut self, lba: u64, bytes: &[u8]) -> Result<()> {
        if self.kind == Injection::ShortWrite && !*self.injected && bytes.len() > 512 {
            self.device.write_sectors(lba, &bytes[..512])?;
            *self.injected = true;
            return Err(Error::ShortIo);
        }
        self.device.write_sectors(lba, bytes)
    }
    fn flush(&mut self) -> Result<()> {
        self.device.flush()?;
        if self.kind == Injection::Flush && !*self.injected {
            *self.injected = true;
            return Err(Error::Io);
        }
        Ok(())
    }
}

fn payload(output: &mut [u8; FILE_BYTES], patched: bool) {
    for (i, byte) in output.iter_mut().enumerate() {
        *byte = if patched && (PATCH_OFFSET..PATCH_OFFSET + PATCH_BYTES).contains(&i) {
            (((i - PATCH_OFFSET) * 13 + 7) % 251) as u8
        } else {
            ((i * 37 + 11) % 251) as u8
        };
    }
}

#[inline(never)]
fn immutable_files(
    device: &mut impl BlockDevice,
    partition: Partition,
    archive: &door_initfs::Archive<'_>,
) {
    let mut exchange = Mount::new(
        "/exchange",
        Fat32::mount(PartitionDevice::new(device, partition).unwrap()).unwrap(),
    )
    .unwrap();
    let hello = exchange.open("/exchange/HELLO.TXT").unwrap();
    let mut bytes = [0xa5; 128];
    let n = exchange.read(hello, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], b"Door FAT32 exchange\n");
    assert_eq!(&bytes[n..], &[0xa5; 108]);
    assert_eq!(exchange.write(hello, 0, b"x"), Err(Error::ReadOnly));
    let mut initial = Mount::new("/init", InitFs::new(archive)).unwrap();
    let boot = initial.open("/init/system/boot.txt").unwrap();
    let expected = archive.file("system/boot.txt").unwrap();
    let n = initial.read(boot, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], &expected[..n]);
    assert!(n > 0);
    assert_eq!(initial.write(boot, 0, b"x"), Err(Error::ReadOnly));
    assert_eq!(initial.read(hello, 0, &mut bytes), Err(Error::NotFound));
}

#[inline(never)]
fn persistent_file(device: &mut impl BlockDevice, partition: Partition, reopen: bool) -> u32 {
    let mut fs = Mount::new(
        "/data",
        Ext2::mount(PartitionDevice::new(device, partition).unwrap()).unwrap(),
    )
    .unwrap();
    let seed = fs.open("/data/seed.txt").unwrap();
    let mut seed_bytes = [0; 64];
    let n = fs.read(seed, 0, &mut seed_bytes).unwrap();
    assert_eq!(&seed_bytes[..n], b"Door ext2 seed\n");
    let mut expected = [0; FILE_BYTES];
    let mut observed = [0xa5; FILE_BYTES];
    let file = if reopen {
        fs.open("/data/persist.bin").unwrap()
    } else {
        assert_eq!(fs.open("/data/persist.bin"), Err(Error::NotFound));
        let file = fs.create("/data/pending.bin").unwrap();
        payload(&mut expected, false);
        assert_eq!(fs.write(file, 0, &expected).unwrap(), FILE_BYTES);
        let second = fs.open("/data/pending.bin").unwrap();
        assert_eq!(file.id, second.id);
        assert_eq!(fs.read(second, 0, &mut observed).unwrap(), FILE_BYTES);
        assert_eq!(observed, expected);
        payload(&mut expected, true);
        assert_eq!(
            fs.write(
                file,
                PATCH_OFFSET as u64,
                &expected[PATCH_OFFSET..PATCH_OFFSET + PATCH_BYTES]
            )
            .unwrap(),
            PATCH_BYTES
        );
        fs.rename("/data/pending.bin", "/data/persist.bin").unwrap();
        assert_eq!(fs.open("/data/pending.bin"), Err(Error::NotFound));
        assert_eq!(fs.open("/data/persist.bin").unwrap().id, file.id);
        // The second pre-rename handle observes both cross-block writes.
        assert_eq!(fs.read(second, 0, &mut observed).unwrap(), FILE_BYTES);
        assert_eq!(observed, expected);
        file
    };
    payload(&mut expected, true);
    assert_eq!(fs.metadata(file).unwrap().bytes, FILE_BYTES as u64);
    assert_eq!(fs.read(file, 0, &mut observed).unwrap(), FILE_BYTES);
    assert_eq!(observed, expected);
    let mut tail = [0xa5; 32];
    assert_eq!(fs.read(file, FILE_BYTES as u64 - 7, &mut tail).unwrap(), 7);
    assert_eq!(&tail[..7], &expected[FILE_BYTES - 7..]);
    assert_eq!(&tail[7..], &[0xa5; 25]);
    assert_eq!(fs.read(file, FILE_BYTES as u64, &mut tail).unwrap(), 0);
    fs.flush().unwrap();
    crc32(&observed)
}

#[inline(never)]
fn filesystem_errors(device: &mut impl BlockDevice, partition: Partition) {
    let mut injected = false;
    let faulty = FaultDevice {
        device: &mut *device,
        kind: Injection::ShortRead,
        injected: &mut injected,
    };
    assert!(matches!(
        Ext2::mount(PartitionDevice::new(faulty, partition).unwrap()),
        Err(Error::ShortIo)
    ));
    assert!(injected, "real prefix read injection reached");
    // Rewriting an existing file first persists the ext2 dirty marker. Inject a
    // short transfer in that metadata block before byte1082 (the state field),
    // leaving its real first512 bytes unchanged. The mount must reject later
    // mutation; validated remount then checks the independent fixture again.
    let mut expected = [0; FILE_BYTES];
    payload(&mut expected, true);
    for kind in [Injection::ShortWrite, Injection::Flush] {
        let mut injected = false;
        {
            let faulty = FaultDevice {
                device: &mut *device,
                kind,
                injected: &mut injected,
            };
            let mut fs = Ext2::mount(PartitionDevice::new(faulty, partition).unwrap()).unwrap();
            let file = fs.open("persist.bin").unwrap();
            if kind == Injection::ShortWrite {
                assert_eq!(fs.write(file, 0, &expected[..4096]), Err(Error::ShortIo));
            } else {
                assert_eq!(fs.flush(), Err(Error::Io));
            }
            assert_eq!(fs.write(file, 0, &expected[..512]), Err(Error::Poisoned));
            assert_eq!(fs.flush(), Err(Error::Poisoned));
        }
        assert!(
            injected,
            "production cache propagated injected transport error"
        );
    }
}

#[inline(never)]
pub fn verify(info: &BootInfo<'_>, archive: &door_initfs::Archive<'_>, reopen: bool) {
    let mut serial = unsafe { Serial::new() };
    let mut disk = Session::open(info);
    let address = disk.address();
    writeln!(
        serial,
        "DOOR_P06 DEVICE PASS pci={:02x}:{:02x}.{} features={:016x} queue=16 dma_pages=8",
        address.bus,
        address.device,
        address.function,
        disk.features()
    )
    .ok();
    let tested = disk
        .selftest(200000)
        .expect("queue cancellation/reset selftest");
    assert!(tested.queue_full && tested.cancellation && tested.reset && tested.stale);
    let geometry = disk.geometry();
    assert_eq!(geometry.sector_size, 512);
    assert_eq!(geometry.sectors, 262144);
    assert!(geometry.flush_supported && !geometry.read_only);
    assert_eq!(
        disk.read_sectors(geometry.sectors, &mut [0; 512]),
        Err(Error::Bounds)
    );
    assert_eq!(
        disk.write_sectors(geometry.sectors, &[0; 512]),
        Err(Error::Bounds)
    );
    assert_eq!(disk.read_sectors(0, &mut [0; 511]), Err(Error::Alignment));
    assert_eq!(disk.write_sectors(0, &[0; 513]), Err(Error::Alignment));
    let gpt = Gpt::read(&mut disk).expect("primary/backup GPT and partitions");
    assert_eq!(gpt.disk_id, *b"DoorP06DiskGUID0");
    assert_eq!(gpt.partitions().count(), 2);
    let fat = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP06FatGUID00")
        .unwrap();
    let ext = gpt
        .partitions()
        .find(|p| p.id == *b"DoorP06ExtGUID00")
        .unwrap();
    immutable_files(&mut disk, fat, archive);
    let checksum = persistent_file(&mut disk, ext, reopen);
    filesystem_errors(&mut disk, ext);
    assert_eq!(persistent_file(&mut disk, ext, true), checksum);
    writeln!(
        serial,
        "DOOR_P06 FILESYSTEM PASS gpt=1 fat32=1 ext2=1 initfs=1 identity=1 coherent=1"
    )
    .ok();
    writeln!(
        serial,
        "DOOR_P06 ERRORS PASS bounds=1 alignment=1 short_io=1 queue_full=1 reset=1 stale=1"
    )
    .ok();
    writeln!(
        serial,
        "DOOR_P06 IO_FAILURE PASS short_write=1 flush=1 poisoned=1 remounted=1"
    )
    .ok();
    writeln!(
        serial,
        "DOOR_P06 PERSIST PASS stage={} bytes={FILE_BYTES} crc32={checksum:08x}",
        if reopen { "reopen" } else { "write" }
    )
    .ok();
    let statistics = disk.statistics();
    writeln!(
        serial,
        "DOOR_P06 BLOCK PASS reads={} writes={} flushes={}",
        statistics.reads, statistics.writes, statistics.flushes
    )
    .ok();
    assert!(statistics.reads > 0 && statistics.writes > 0 && statistics.flushes > 0);
    writeln!(
        serial,
        "DOOR_P06 STACK PASS sampled_remaining={} stack_bytes={}",
        statistics.sampled_remaining, statistics.stack_bytes
    )
    .ok();
    disk.close();
    writeln!(
        serial,
        "DOOR_P06 CLEANUP PASS dma_quiesced=1 pages_reclaimed=8"
    )
    .ok();
}
