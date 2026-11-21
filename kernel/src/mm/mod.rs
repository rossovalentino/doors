//! P03 single-CPU memory ownership, retained bootstrap heap and page tables.
mod address;
mod device;
mod frame;
mod heap;
mod page_pool;
mod paging;
mod process_space;
mod reservations;
mod section_pages;
mod user_table;

pub use address::{
    ByteCount, DeviceAddress, FileOffset, FrameNumber, PageCount, PhysicalAddress, VirtualAddress,
};
pub use device::validate_device_extent;
pub use frame::{FrameAllocator, FrameStats, MAX_ALLOCATIONS, OwnedFrames};
pub use heap::{EarlyHeap, HeapBlock};
pub use page_pool::{ARENA_BYTES as PAGE_POOL_BYTES, MAX_CHUNKS as PAGE_POOL_CHUNKS, PagePool};
pub use paging::{MAX_MAPPINGS, MapFailure, MappedRegion, PageTables, Permissions, UnmapFailure};
pub use process_space::{PageSource, PoolPages, Query, mem};
pub use process_space::{
    ProcessSpace, RegionId, RegionInfo, RegionTable, USER_END, USER_MIN, UserAccess,
};
pub use reservations::{Direction, GRANULARITY, MAPPED, MAX_RESERVATIONS, Reservation};
pub use section_pages::{MAX_PAGES as SECTION_MAX_PAGES, NodeMemory, PageState, SectionPages};
pub use user_table::{Fault, Protection};

pub const PAGE_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Overflow,
    Alignment,
    Empty,
    Address,
    InvalidMemoryMap,
    Exhausted,
    AllocationLimit,
    InvalidFree,
    Overlap,
    InvalidTable,
    MappingLimit,
    InvalidMapping,
    InterruptsEnabled,
    AccessDenied,
    NotCommitted,
    InvalidState,
}
