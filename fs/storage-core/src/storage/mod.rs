//! Bounded native storage mechanisms. All transfers are exact or fail explicitly.
mod block;
pub mod btree;
mod cache;
mod ext2;
mod fat32;
mod gpt;
mod journal;
mod vfs;
pub use block::{BlockDevice, Geometry, PartitionDevice, check_transfer};
pub use ext2::{Ext2, Format, format as format_ext2};
pub use fat32::Fat32;
pub use gpt::{Gpt, Partition, crc32};
pub use journal::{Checkpoint, CommitReport, JOURNAL_PAGES, Journal, RecoveryReport, WorkingSet};
pub use vfs::{FileId, InitFs, Metadata, Mount, OpenFile, Vfs};
pub const SECTOR_BYTES: usize = 512;
pub const BLOCK_BYTES: usize = 4096;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Bounds,
    Alignment,
    Overflow,
    ReadOnly,
    Unsupported,
    Corrupt,
    Io,
    ShortIo,
    DeviceLost,
    Timeout,
    NoSpace,
    NotFound,
    AlreadyExists,
    InvalidName,
    IsDirectory,
    NotDirectory,
    Busy,
    Poisoned,
}
pub type Result<T> = core::result::Result<T, Error>;
fn u16le(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(b[p..p + 2].try_into().unwrap())
}
fn u32le(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn u64le(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 63
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(Error::InvalidName)
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod scale_tests;
#[cfg(test)]
pub(crate) mod tests;
