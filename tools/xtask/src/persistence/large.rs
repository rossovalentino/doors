//! P18 M0a: a large user disk. The P07 disk's partitions and identities
//! (the FAT exchange volume, the home volume, the redo journal), with the
//! home volume grown to fill the disk and formatted as an ext2 volume at
//! scale (many groups, large files: catalog v6 keeps its nodes in a tree
//! there). Written as a sparse file: only metadata occupies host space.
use super::{HOME_FIRST, LOG_SECTORS};
use door_storage_core::storage::{BlockDevice, Error, Format, Geometry, crc32, format_ext2};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

/// The home volume's identity on a large disk.
pub const VOLUME: [u8; 16] = *b"DoorP18Ext2UUID0";

/// The home partition of a large disk as a block device.
struct Window<'a> {
    file: &'a mut File,
    first: u64,
    sectors: u64,
}
impl BlockDevice for Window<'_> {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: self.sectors,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(
        &mut self,
        lba: u64,
        output: &mut [u8],
    ) -> door_storage_core::storage::Result<()> {
        door_storage_core::storage::check_transfer(self.geometry(), lba, output.len())?;
        self.file
            .seek(SeekFrom::Start((self.first + lba) * 512))
            .and_then(|_| self.file.read_exact(output))
            .map_err(|_| Error::Io)
    }
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> door_storage_core::storage::Result<()> {
        door_storage_core::storage::check_transfer(self.geometry(), lba, input.len())?;
        self.file
            .seek(SeekFrom::Start((self.first + lba) * 512))
            .and_then(|_| self.file.write_all(input))
            .map_err(|_| Error::Io)
    }
    fn flush(&mut self) -> door_storage_core::storage::Result<()> {
        Ok(())
    }
}

fn put(bytes: &mut [u8], at: usize, value: u64, width: usize) {
    bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn at(file: &mut File, offset: u64, bytes: &[u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.write_all(bytes))
        .map_err(|e| e.to_string())
}

/// The large disk's layout: (home sectors, journal first sector).
pub fn layout(bytes: u64) -> Result<(u64, u64), String> {
    let sectors = bytes / 512;
    if !bytes.is_multiple_of(1024 * 1024) || bytes < 512 * 1024 * 1024 {
        return Err("a large disk is whole MiBs, at least 512 MiB".into());
    }
    // The journal ends before the backup GPT (33 sectors and the header).
    let log_first = (sectors - 34 - LOG_SECTORS) & !7;
    Ok((log_first - HOME_FIRST, log_first))
}

/// Writes the large disk at `path` (a new sparse file of `bytes`).
pub fn write(path: &Path, bytes: u64) -> Result<(), String> {
    let (home_sectors, log_first) = layout(bytes)?;
    let sectors = bytes / 512;
    let base = super::format::image(false);
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // Sparse on NTFS: unwritten ranges take no host space.
    let sparse = std::process::Command::new("fsutil")
        .args(["sparse", "setflag"])
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !sparse.status.success() {
        return Err(format!("fsutil sparse setflag {} failed", path.display()));
    }
    file.set_len(bytes).map_err(|e| e.to_string())?;
    // Protective MBR and the primary GPT: the P07 entries with the home and
    // journal partitions moved to their large extents.
    let mut head = base[..34 * 512].to_vec();
    put(&mut head, 458, (sectors - 1).min(u64::from(u32::MAX)), 4);
    for entry in head[1024..1024 + 128 * 128].chunks_exact_mut(128) {
        if &entry[16..32] == b"DoorP07ExtGUID00" {
            put(entry, 32, HOME_FIRST, 8);
            put(entry, 40, HOME_FIRST + home_sectors - 1, 8);
        } else if &entry[..16] == b"DoorRedoJournal1" {
            put(entry, 32, log_first, 8);
            put(entry, 40, log_first + LOG_SECTORS - 1, 8);
        }
    }
    let array = head[1024..1024 + 128 * 128].to_vec();
    let array_crc = u64::from(crc32(&array));
    let mut primary = head[512..1024].to_vec();
    put(&mut primary, 32, sectors - 1, 8);
    put(&mut primary, 48, sectors - 34, 8);
    put(&mut primary, 88, array_crc, 4);
    let mut backup = primary.clone();
    put(&mut backup, 24, sectors - 1, 8);
    put(&mut backup, 32, 1, 8);
    put(&mut backup, 72, sectors - 33, 8);
    for header in [&mut primary, &mut backup] {
        put(header, 16, 0, 4);
        let crc = u64::from(crc32(&header[..92]));
        put(header, 16, crc, 4);
    }
    head[512..1024].copy_from_slice(&primary);
    at(&mut file, 0, &head)?;
    at(&mut file, (sectors - 33) * 512, &array)?;
    at(&mut file, (sectors - 1) * 512, &backup)?;
    // The FAT exchange volume, as on every P07 disk.
    let fat = 2048 * 512..HOME_FIRST as usize * 512;
    at(&mut file, fat.start as u64, &base[fat])?;
    // The home volume at scale.
    let mut window = Window {
        file: &mut file,
        first: HOME_FIRST,
        sectors: home_sectors,
    };
    let shape = Format::sized(home_sectors * 512, VOLUME);
    format_ext2(&mut window, shape, true).map_err(|e| format!("large home volume: {e:?}"))?;
    // The journal's two header pages, naming the large extents.
    let mut journal =
        base[super::LOG_FIRST as usize * 512..super::LOG_FIRST as usize * 512 + 8192].to_vec();
    for page in journal.chunks_exact_mut(4096) {
        for (offset, value) in [
            (80, HOME_FIRST),
            (88, home_sectors),
            (96, log_first),
            (104, LOG_SECTORS),
        ] {
            put(page, offset, value, 8);
        }
        put(page, 120, 0, 4);
        let crc = u64::from(crc32(page));
        put(page, 120, crc, 4);
    }
    at(&mut file, log_first * 512, &journal)?;
    file.sync_all().map_err(|e| e.to_string())
}
