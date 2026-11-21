//! Bounded structural inspectors, shared by host tooling and the future loader.
//! No image is executed, mapped or authenticated by parsing it.
#![no_std]

mod elf;
mod pe;

pub use elf::{ElfImage, Segment};
pub use pe::{ExecutionLevel, Machine, PeImage, Section, execution_level};

pub const MAX_IMAGE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Truncated,
    BadMagic,
    Unsupported,
    Limit,
    Overflow,
    Bounds,
    Alignment,
    Overlap,
    Permissions,
    InvalidEntry,
    DynamicDependency,
    MalformedString,
}

pub(crate) fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(2).ok_or(Error::Overflow)?)
            .ok_or(Error::Truncated)?
            .try_into()
            .map_err(|_| Error::Truncated)?,
    ))
}
pub(crate) fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(4).ok_or(Error::Overflow)?)
            .ok_or(Error::Truncated)?
            .try_into()
            .map_err(|_| Error::Truncated)?,
    ))
}
pub(crate) fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(8).ok_or(Error::Overflow)?)
            .ok_or(Error::Truncated)?
            .try_into()
            .map_err(|_| Error::Truncated)?,
    ))
}
pub(crate) fn extent(
    start: u64,
    length: u64,
    limit: u64,
) -> Result<core::ops::Range<usize>, Error> {
    let end = start.checked_add(length).ok_or(Error::Overflow)?;
    if end > limit || end > usize::MAX as u64 {
        return Err(Error::Bounds);
    }
    Ok(start as usize..end as usize)
}
pub(crate) const fn overlaps(a: u64, alen: u64, b: u64, blen: u64) -> bool {
    alen != 0 && blen != 0 && a < b + blen && b < a + alen
}
pub(crate) fn canonical(address: u64) -> bool {
    address <= 0x0000_7fff_ffff_ffff || address >= 0xffff_8000_0000_0000
}
