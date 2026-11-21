#![no_std]
//! Microsoft Cabinet files (MS-CAB): the archives Windows Installer
//! packages, driver packages and setup programs carry. `Cabinet::parse`
//! reads the header, folders and file records of one cabinet (a set
//! spanning several cabinets is refused: Doors' installers ship single
//! cabinets); `extract` decompresses a file: stored, MSZIP (deflate blocks
//! sharing a 32 KiB window, `inflate`) or LZX (`lzx`). `Writer` builds
//! stored or MSZIP cabinets for tests and Doors' own packages.
extern crate alloc;
pub mod inflate;
pub mod lzx;
mod write;
pub use write::{Compression, Writer};

use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a cabinet, or a structure points outside the file.
    Malformed,
    /// A cabinet set (previous/next cabinet) or Quantum compression.
    Unsupported,
    /// A data block's checksum or compressed data is wrong.
    Corrupt,
    NotFound,
}

const FLAG_PREV: u16 = 0x1;
const FLAG_NEXT: u16 = 0x2;
const FLAG_RESERVE: u16 = 0x4;
/// File attribute: the name is UTF-8.
pub const ATTRIBUTE_UTF8: u16 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Stored,
    MsZip,
    /// LZX with a 2^window-byte window (15..=21).
    Lzx(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Offset of the first CFDATA block.
    pub data: u32,
    pub blocks: u16,
    pub method: Method,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub size: u32,
    /// Offset in the folder's uncompressed stream.
    pub offset: u32,
    pub folder: u16,
    /// MS-DOS date and time.
    pub date: u16,
    pub time: u16,
    pub attributes: u16,
}

pub struct Cabinet<'a> {
    bytes: &'a [u8],
    pub folders: Vec<Folder>,
    pub files: Vec<File>,
    /// Per-CFDATA reserved bytes.
    data_reserve: usize,
    pub set_id: u16,
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
/// A NUL-terminated string at `at` (at most 255 bytes, as MS-CAB allows).
fn string_at(b: &[u8], at: usize) -> Result<(&[u8], usize), Error> {
    let rest = b.get(at..).ok_or(Error::Malformed)?;
    let end = rest
        .iter()
        .take(256)
        .position(|c| *c == 0)
        .ok_or(Error::Malformed)?;
    Ok((&rest[..end], at + end + 1))
}

/// MS-CAB's CFDATA checksum (section 2.4): the XOR of the little-endian
/// 32-bit words, the tail folded in from its high byte, seeded by `seed`.
pub fn checksum(data: &[u8], seed: u32) -> u32 {
    let mut csum = seed;
    let words = data.len() / 4;
    for i in 0..words {
        csum ^= u32::from_le_bytes(data[4 * i..4 * i + 4].try_into().unwrap());
    }
    let tail = &data[4 * words..];
    let mut ul = 0u32;
    for (i, b) in tail.iter().enumerate() {
        ul |= u32::from(*b) << (8 * (tail.len() - 1 - i));
    }
    csum ^ ul
}

impl<'a> Cabinet<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.get(..4) != Some(b"MSCF") {
            return Err(Error::Malformed);
        }
        let size = u32_at(bytes, 8)? as usize;
        if size > bytes.len() {
            return Err(Error::Malformed);
        }
        let bytes = &bytes[..size];
        let files_at = u32_at(bytes, 16)? as usize;
        if bytes.get(24..26) != Some(&[3, 1]) {
            return Err(Error::Unsupported);
        }
        let folder_count = u16_at(bytes, 26)?;
        let file_count = u16_at(bytes, 28)?;
        let flags = u16_at(bytes, 30)?;
        let set_id = u16_at(bytes, 32)?;
        if flags & (FLAG_PREV | FLAG_NEXT) != 0 {
            return Err(Error::Unsupported);
        }
        let mut at = 36;
        let (mut folder_reserve, mut data_reserve) = (0usize, 0usize);
        if flags & FLAG_RESERVE != 0 {
            let header_reserve = usize::from(u16_at(bytes, 36)?);
            folder_reserve = usize::from(*bytes.get(38).ok_or(Error::Malformed)?);
            data_reserve = usize::from(*bytes.get(39).ok_or(Error::Malformed)?);
            at = 40 + header_reserve;
        }
        let mut folders = Vec::with_capacity(usize::from(folder_count));
        for _ in 0..folder_count {
            let data = u32_at(bytes, at)?;
            let blocks = u16_at(bytes, at + 4)?;
            let kind = u16_at(bytes, at + 6)?;
            let method = match kind & 0x0f {
                0 => Method::Stored,
                1 => Method::MsZip,
                3 => {
                    let window = ((kind >> 8) & 0x1f) as u8;
                    if !(15..=21).contains(&window) {
                        return Err(Error::Malformed);
                    }
                    Method::Lzx(window)
                }
                _ => return Err(Error::Unsupported),
            };
            if data as usize > bytes.len() {
                return Err(Error::Malformed);
            }
            folders.push(Folder {
                data,
                blocks,
                method,
            });
            at += 8 + folder_reserve;
        }
        let mut files = Vec::with_capacity(usize::from(file_count));
        let mut at = files_at;
        for _ in 0..file_count {
            let size = u32_at(bytes, at)?;
            let offset = u32_at(bytes, at + 4)?;
            let folder = u16_at(bytes, at + 8)?;
            let date = u16_at(bytes, at + 10)?;
            let time = u16_at(bytes, at + 12)?;
            let attributes = u16_at(bytes, at + 14)?;
            let (raw, next) = string_at(bytes, at + 16)?;
            let name = if attributes & ATTRIBUTE_UTF8 != 0 {
                String::from(core::str::from_utf8(raw).map_err(|_| Error::Malformed)?)
            } else {
                // Without the UTF-8 flag names are in the OEM code page;
                // Latin-1 keeps every byte.
                raw.iter().map(|b| char::from(*b)).collect()
            };
            // 0xFFFD..0xFFFF continue from or into another cabinet.
            if usize::from(folder) >= folders.len() {
                return Err(Error::Unsupported);
            }
            files.push(File {
                name,
                size,
                offset,
                folder,
                date,
                time,
                attributes,
            });
            at = next;
        }
        Ok(Self {
            bytes,
            folders,
            files,
            data_reserve,
            set_id,
        })
    }
    /// Folder `index`'s uncompressed stream, decoded until it holds at least
    /// `limit` bytes (the blocks after that are left alone).
    pub fn folder_data(&self, index: usize, limit: usize) -> Result<Vec<u8>, Error> {
        let folder = self.folders.get(index).ok_or(Error::NotFound)?;
        let mut out = Vec::new();
        let mut at = folder.data as usize;
        let mut inflater = inflate::Inflater::new();
        let mut lzx = match folder.method {
            Method::Lzx(window) => Some(lzx::Decoder::new(window)),
            _ => None,
        };
        for _ in 0..folder.blocks {
            if out.len() >= limit {
                break;
            }
            let csum = u32_at(self.bytes, at)?;
            let compressed = usize::from(u16_at(self.bytes, at + 4)?);
            let uncompressed = usize::from(u16_at(self.bytes, at + 6)?);
            let header = &self.bytes[at + 4..at + 8];
            let start = at + 8 + self.data_reserve;
            let data = self
                .bytes
                .get(start..start + compressed)
                .ok_or(Error::Malformed)?;
            if csum != 0 {
                let seed = checksum(data, 0);
                if checksum(header, seed) != csum {
                    return Err(Error::Corrupt);
                }
            }
            // A block holds at most 32 KiB (MS-CAB allows 6 KiB more for
            // incompressible MSZIP data).
            if uncompressed > 32 * 1024 + 6144 {
                return Err(Error::Corrupt);
            }
            match folder.method {
                Method::Stored => {
                    if compressed != uncompressed {
                        return Err(Error::Corrupt);
                    }
                    out.extend_from_slice(data);
                }
                Method::MsZip => {
                    if data.get(..2) != Some(b"CK") {
                        return Err(Error::Corrupt);
                    }
                    let before = out.len();
                    inflater
                        .inflate(&data[2..], uncompressed, &mut out)
                        .map_err(|_| Error::Corrupt)?;
                    if out.len() - before != uncompressed {
                        return Err(Error::Corrupt);
                    }
                }
                Method::Lzx(_) => {
                    let decoder = lzx.as_mut().unwrap();
                    decoder
                        .block(data, uncompressed, &mut out)
                        .map_err(|_| Error::Corrupt)?;
                }
            }
            at = start + compressed;
        }
        Ok(out)
    }
    /// A file's bytes.
    pub fn extract(&self, file: &File) -> Result<Vec<u8>, Error> {
        let end = file.offset.checked_add(file.size).ok_or(Error::Malformed)? as usize;
        let folder = self.folder_data(usize::from(file.folder), end)?;
        folder
            .get(file.offset as usize..end)
            .map(<[u8]>::to_vec)
            .ok_or(Error::Corrupt)
    }
    /// A file by (case-insensitive) name.
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests;
