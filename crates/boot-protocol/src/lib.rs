//! Checked BootInfo v1 encoding and parsing. See specs/contracts/boot-info-v1.md.
//!
//! These Rust values are not wire layouts. All fields are explicitly encoded.
//! The caller supplies a stable, readable byte slice; this crate never maps or
//! dereferences physical addresses and cannot authenticate firmware ownership.
#![no_std]

mod boot;
mod memory;
pub mod platform;
mod startup;
mod wire;

pub use boot::{
    COMMAND_LINE_KIND, FRAMEBUFFER_BYTES, FRAMEBUFFER_KIND, Framebuffer, MAPPING_BYTES,
    MAPPINGS_KIND, MODULE_BYTES, MODULES_KIND, Mapping, Module,
};
pub use memory::{MemoryRegion, Ownership, PAGE_BYTES, PhysicalRange};
pub use platform::{IoApic, PLATFORM_BYTES, PLATFORM_KIND, Platform, Processor};
pub use startup::{
    STARTUP_BYTES, STARTUP_CODE_ROLE, STARTUP_DATA_ROLE, STARTUP_KIND, STARTUP_PAGE_BYTES, Startup,
};
pub use wire::{Header, TableDescriptor};

pub const MAGIC: [u8; 8] = *b"DOORBOOT";
pub const HEADER_BYTES: usize = 64;
pub const TABLE_BYTES: usize = 32;
pub const MEMORY_REGION_BYTES: usize = 40;
pub const MAX_BLOB_BYTES: usize = 1024 * 1024;
pub const MAX_TABLES: u32 = 64;
/// Reserved supervisor paging pool, shared by loader and kernel.
// Thirty-two protected processes (`MAX_PROCESSES`, 13 November 2026) plus
// their private heaps and section aliases: a full 13-process desktop used
// 646 pages, so 2048 (8 MiB) keeps room for 32. The loader reserves this
// bounded pool; individual map rollback stays bounded by the requested
// extent rather than by this aggregate capacity.
pub const KERNEL_TABLE_POOL_PAGES: usize = 4096;
pub const MAX_MEMORY_REGIONS: usize = 16_384;
pub const MEMORY_REGIONS_KIND: u32 = 1;
pub const REQUIRED: u16 = 1;

/// Failures are explicit and contain no borrowed pointers or host dependencies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    ShortRecord,
    BadMagic,
    UnsupportedVersion,
    InvalidRecordSize,
    ReservedBits,
    LimitExceeded,
    EmptyRange,
    Overflow,
    Misaligned,
    OutOfBounds,
    Overlap,
    DuplicateTable,
    UnsupportedRequiredTable,
    MissingMemoryMap,
    InvalidOwnership,
    RuntimeMemoryOwnership,
    UnsortedMemoryMap,
    LiveBootDataNotReserved,
    MissingBootTable,
    BadChecksum,
}

/// Validated immutable view. All tables are checked before any view is returned.
#[derive(Debug)]
pub struct BootInfo<'a> {
    bytes: &'a [u8],
    header: Header,
    memory: TableDescriptor,
}

impl<'a> BootInfo<'a> {
    pub fn parse(input: &'a [u8]) -> Result<Self, Error> {
        let header = Header::decode(input)?;
        let bytes = input
            .get(..header.total_bytes as usize)
            .ok_or(Error::ShortRecord)?;
        let descriptor_range = wire::bounded_range(
            header.tables_offset,
            u64::from(header.table_count) * TABLE_BYTES as u64,
            header.total_bytes,
        )?;
        let mut memory = None;
        for index in 0..header.table_count {
            let table = Self::descriptor(bytes, header, index)?;
            table.validate(header.total_bytes)?;
            let range = wire::bounded_range(table.offset, table.length, header.total_bytes)?;
            if range.start < HEADER_BYTES || wire::overlaps(&range, &descriptor_range) {
                return Err(Error::Overlap);
            }
            for previous in 0..index {
                let other = Self::descriptor(bytes, header, previous)?;
                if other.kind == table.kind {
                    return Err(Error::DuplicateTable);
                }
                let other_range =
                    wire::bounded_range(other.offset, other.length, header.total_bytes)?;
                if wire::overlaps(&range, &other_range) {
                    return Err(Error::Overlap);
                }
            }
            if table.kind == MEMORY_REGIONS_KIND && table.revision == 1 {
                if table.flags != REQUIRED || table.entry_bytes != MEMORY_REGION_BYTES as u32 {
                    return Err(Error::InvalidRecordSize);
                }
                memory = Some(table);
            } else if (table.revision == 1 && matches!(table.kind, 2..=6 | STARTUP_KIND))
                || (table.kind == 5 && table.revision == 2)
            {
                let expected = match table.kind {
                    2 => MODULE_BYTES,
                    3 => MAPPING_BYTES,
                    4 => FRAMEBUFFER_BYTES,
                    5 => PLATFORM_BYTES,
                    STARTUP_KIND => STARTUP_BYTES,
                    _ => 1,
                };
                if table.entry_bytes != expected as u32
                    || table.length / expected as u64 > if table.kind == 6 { 256 } else { 128 }
                {
                    return Err(Error::InvalidRecordSize);
                }
                let payload = &bytes[range];
                match table.kind {
                    2 => {
                        for b in payload.chunks_exact(MODULE_BYTES) {
                            Module::decode(b)?;
                        }
                    }
                    3 => {
                        for b in payload.chunks_exact(MAPPING_BYTES) {
                            Mapping::decode(b)?;
                        }
                    }
                    4 => {
                        if payload.len() != FRAMEBUFFER_BYTES {
                            return Err(Error::InvalidRecordSize);
                        }
                        Framebuffer::decode(payload)?;
                    }
                    5 => {
                        if payload.len() != PLATFORM_BYTES {
                            return Err(Error::InvalidRecordSize);
                        }
                        if table.revision == 1 && payload[67] != 0 {
                            return Err(Error::ReservedBits);
                        }
                        Platform::decode(payload)?;
                    }
                    STARTUP_KIND => {
                        if payload.len() != STARTUP_BYTES {
                            return Err(Error::InvalidRecordSize);
                        }
                        Startup::decode(payload)?;
                    }
                    _ => {
                        if payload.iter().any(|b| !b.is_ascii_graphic() && *b != b' ') {
                            return Err(Error::ReservedBits);
                        }
                    }
                }
            } else if table.flags & REQUIRED != 0 {
                return Err(Error::UnsupportedRequiredTable);
            }
        }
        let memory = memory.ok_or(Error::MissingMemoryMap)?;
        let info = Self {
            bytes,
            header,
            memory,
        };
        info.validate_memory()?;
        Ok(info)
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn memory_region_count(&self) -> usize {
        self.memory.length as usize / MEMORY_REGION_BYTES
    }

    /// Decodes a previously validated entry. Out-of-range indexes never panic.
    pub fn memory_region(&self, index: usize) -> Result<MemoryRegion, Error> {
        if index >= self.memory_region_count() {
            return Err(Error::OutOfBounds);
        }
        let start = self.memory.offset as usize + index * MEMORY_REGION_BYTES;
        MemoryRegion::decode(&self.bytes[start..start + MEMORY_REGION_BYTES])
    }

    /// Only usable-after-handoff regions may enter an initial allocator.
    /// Reclaim-after-copy data needs a separate future lifetime transition.
    pub fn is_allocatable(&self, range: PhysicalRange) -> bool {
        (0..self.memory_region_count()).any(|index| {
            self.memory_region(index).is_ok_and(|region| {
                region.ownership == Ownership::UsableAfterHandoff && region.range.contains(range)
            })
        })
    }

    fn descriptor(bytes: &[u8], header: Header, index: u32) -> Result<TableDescriptor, Error> {
        let start = header.tables_offset as usize + index as usize * TABLE_BYTES;
        TableDescriptor::decode(
            bytes
                .get(start..start + TABLE_BYTES)
                .ok_or(Error::ShortRecord)?,
        )
    }

    fn validate_memory(&self) -> Result<(), Error> {
        let count = self.memory_region_count();
        if count > MAX_MEMORY_REGIONS {
            return Err(Error::LimitExceeded);
        }
        let live = PhysicalRange::new(self.header.self_phys, self.header.total_bytes, 1)?;
        let mut previous: Option<PhysicalRange> = None;
        let mut live_reserved = false;
        for index in 0..count {
            let region = self.memory_region(index)?;
            if let Some(prior) = previous {
                if region.range.start() < prior.start() {
                    return Err(Error::UnsortedMemoryMap);
                }
                if prior.overlaps(region.range) {
                    return Err(Error::Overlap);
                }
            }
            if region.range.contains(live) && region.ownership.protects_boot_data() {
                live_reserved = true;
            }
            previous = Some(region.range);
        }
        if !live_reserved {
            return Err(Error::LiveBootDataNotReserved);
        }
        Ok(())
    }
}
