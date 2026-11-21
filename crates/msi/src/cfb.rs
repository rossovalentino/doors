//! Compound File Binary (MS-CFB) reading: the container Windows Installer
//! databases are. Version 3 (512-byte sectors) and 4 (4096-byte sectors),
//! the FAT chained through the DIFAT, the mini stream for streams under
//! 4096 bytes, and the directory's flat list of entries (the red-black tree
//! links are only followed to find a storage's children).
use alloc::{string::String, vec, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a compound file, or a structure points outside it.
    Malformed,
}

const SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const END_OF_CHAIN: u32 = 0xffff_fffe;
const FREE: u32 = 0xffff_ffff;
const NO_STREAM: u32 = 0xffff_ffff;
const MINI_CUTOFF: u64 = 4096;
const MINI_SECTOR: usize = 64;

/// A directory entry: a storage (2), stream (2) or the root (5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name as UTF-16 code units (MSI names use private-use ranges).
    pub name: Vec<u16>,
    pub kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    pub size: u64,
}
impl Entry {
    pub fn name_string(&self) -> String {
        String::from_utf16_lossy(&self.name)
    }
}

pub struct File<'a> {
    bytes: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    /// The root entry's stream: the mini sectors.
    mini_stream: Vec<u8>,
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or(Error::Malformed)?
            .try_into()
            .unwrap(),
    ))
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or(Error::Malformed)?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(b: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        b.get(at..at + 8)
            .ok_or(Error::Malformed)?
            .try_into()
            .unwrap(),
    ))
}

impl<'a> File<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.get(..8) != Some(&SIGNATURE) {
            return Err(Error::Malformed);
        }
        let major = u16_at(bytes, 26)?;
        let shift = u16_at(bytes, 30)?;
        let sector = match (major, shift) {
            (3, 9) => 512,
            (4, 12) => 4096,
            _ => return Err(Error::Malformed),
        };
        if u16_at(bytes, 32)? != 6 || u16_at(bytes, 28)? != 0xfffe {
            return Err(Error::Malformed);
        }
        let fat_sectors = u32_at(bytes, 44)? as usize;
        let directory_start = u32_at(bytes, 48)?;
        let mini_fat_start = u32_at(bytes, 60)?;
        let mini_fat_sectors = u32_at(bytes, 64)? as usize;
        let difat_start = u32_at(bytes, 68)?;
        let difat_sectors = u32_at(bytes, 72)? as usize;
        let available = bytes.len().saturating_sub(sector) / sector;
        if fat_sectors > available || mini_fat_sectors > available || difat_sectors > available {
            return Err(Error::Malformed);
        }
        let mut file = Self {
            bytes,
            sector,
            fat: Vec::new(),
            mini_fat: Vec::new(),
            mini_stream: Vec::new(),
            entries: Vec::new(),
        };
        // The FAT's sectors: 109 in the header, the rest in DIFAT sectors.
        let mut fat_list = Vec::with_capacity(fat_sectors);
        for i in 0..109 {
            let s = u32_at(bytes, 76 + 4 * i)?;
            if s != FREE && fat_list.len() < fat_sectors {
                fat_list.push(s);
            }
        }
        let mut next = difat_start;
        for _ in 0..difat_sectors {
            if next == END_OF_CHAIN || next == FREE {
                break;
            }
            let data = file.sector_data(next)?;
            let per = sector / 4 - 1;
            for i in 0..per {
                let s = u32_at(data, 4 * i)?;
                if s != FREE && fat_list.len() < fat_sectors {
                    fat_list.push(s);
                }
            }
            next = u32_at(data, 4 * per)?;
        }
        if fat_list.len() != fat_sectors {
            return Err(Error::Malformed);
        }
        for s in fat_list {
            let data = file.sector_data(s)?;
            for chunk in data.chunks_exact(4) {
                file.fat.push(u32::from_le_bytes(chunk.try_into().unwrap()));
            }
        }
        // Directory, mini FAT, mini stream.
        let directory = file.chain(directory_start, None)?;
        for record in directory.chunks_exact(128) {
            let length = usize::from(u16_at(record, 64)?);
            let kind = record[66];
            if kind == 0 {
                file.entries.push(Entry {
                    name: Vec::new(),
                    kind,
                    left: NO_STREAM,
                    right: NO_STREAM,
                    child: NO_STREAM,
                    start: 0,
                    size: 0,
                });
                continue;
            }
            if !(2..=64).contains(&length) || length % 2 != 0 {
                return Err(Error::Malformed);
            }
            let name: Vec<u16> = record[..length - 2]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let size = u64_at(record, 120)?;
            file.entries.push(Entry {
                name,
                kind,
                left: u32_at(record, 68)?,
                right: u32_at(record, 72)?,
                child: u32_at(record, 76)?,
                start: u32_at(record, 116)?,
                // Version 3 files keep only the low 32 bits.
                size: if major == 3 { size & 0xffff_ffff } else { size },
            });
        }
        let root = file.entries.first().ok_or(Error::Malformed)?.clone();
        if root.kind != 5 {
            return Err(Error::Malformed);
        }
        if mini_fat_sectors > 0 {
            let data = file.chain(mini_fat_start, None)?;
            file.mini_fat = data
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                .collect();
        }
        if root.size > 0 {
            file.mini_stream = file.chain(root.start, Some(root.size))?;
        }
        Ok(file)
    }
    fn sector_data(&self, index: u32) -> Result<&'a [u8], Error> {
        let at = (index as usize + 1)
            .checked_mul(self.sector)
            .ok_or(Error::Malformed)?;
        self.bytes.get(at..at + self.sector).ok_or(Error::Malformed)
    }
    /// A FAT chain's bytes, cut to `size` when given.
    fn chain(&self, start: u32, size: Option<u64>) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut s = start;
        // A chain visits each of the file's sectors at most once: a longer
        // one loops (a crafted file must not make it allocate without end).
        let limit = (self.bytes.len() / self.sector).min(self.fat.len()) + 1;
        let mut steps = 0;
        while s != END_OF_CHAIN {
            steps += 1;
            if steps > limit || size.is_some_and(|n| out.len() as u64 >= n) {
                return Err(Error::Malformed);
            }
            out.extend_from_slice(self.sector_data(s)?);
            s = *self.fat.get(s as usize).ok_or(Error::Malformed)?;
        }
        if let Some(n) = size {
            if (out.len() as u64) < n {
                return Err(Error::Malformed);
            }
            out.truncate(n as usize);
        }
        Ok(out)
    }
    fn mini_chain(&self, start: u32, size: u64) -> Result<Vec<u8>, Error> {
        if size > self.mini_stream.len() as u64 {
            return Err(Error::Malformed);
        }
        let mut out = Vec::with_capacity(size as usize);
        let mut s = start;
        let mut steps = 0;
        let limit = (self.mini_stream.len() / MINI_SECTOR).min(self.mini_fat.len()) + 1;
        while (out.len() as u64) < size {
            steps += 1;
            if s == END_OF_CHAIN || steps > limit {
                return Err(Error::Malformed);
            }
            let at = s as usize * MINI_SECTOR;
            out.extend_from_slice(
                self.mini_stream
                    .get(at..at + MINI_SECTOR)
                    .ok_or(Error::Malformed)?,
            );
            s = *self.mini_fat.get(s as usize).ok_or(Error::Malformed)?;
        }
        out.truncate(size as usize);
        Ok(out)
    }
    /// A stream's bytes.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        if entry.kind != 2 {
            return Err(Error::Malformed);
        }
        if entry.size == 0 {
            return Ok(Vec::new());
        }
        if entry.size > self.bytes.len() as u64 {
            return Err(Error::Malformed);
        }
        if entry.size < MINI_CUTOFF {
            self.mini_chain(entry.start, entry.size)
        } else {
            self.chain(entry.start, Some(entry.size))
        }
    }
    /// The children of storage `index` (0: the root), by walking its tree.
    pub fn children(&self, index: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let Some(storage) = self.entries.get(index) else {
            return out;
        };
        let mut stack = vec![storage.child];
        while let Some(i) = stack.pop() {
            if i == NO_STREAM || out.len() > self.entries.len() {
                continue;
            }
            let Some(e) = self.entries.get(i as usize) else {
                continue;
            };
            if out.contains(&(i as usize)) {
                continue;
            }
            out.push(i as usize);
            stack.push(e.left);
            stack.push(e.right);
        }
        out
    }
    /// A stream directly in the root storage, by exact UTF-16 name.
    pub fn stream(&self, name: &[u16]) -> Option<&Entry> {
        self.children(0)
            .into_iter()
            .map(|i| &self.entries[i])
            .find(|e| e.kind == 2 && e.name == name)
    }
}
