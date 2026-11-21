//! Process heap core behind ntdll's `RtlAllocateHeap` family (and so
//! `HeapAlloc`, `GlobalAlloc`, `LocalAlloc` and the compat DLLs' Rust
//! `#[global_allocator]`, see `allocator`).
//!
//! A heap carves blocks from committed segments in fixed size classes; blocks
//! above `MAX_CLASS_BYTES` get a reservation of their own. Every block starts
//! with a 16-byte header, so payloads keep the Windows x64 16-byte alignment.
//! Freed class blocks are reused only by their class (no coalescing); large
//! blocks are returned to the kernel at once. Large blocks are recorded in a
//! table sorted by base address that lives in a provider mapping of its own
//! and doubles on demand, so their number is bounded only by memory and the
//! kernel's region budget (lookups are binary searches over heap-owned
//! memory, never reads of the caller's pointer). Memory is reached only through
//! the `Pages` provider, so this core is host-testable and free of `unsafe`.
//! A caller pointer is trusted only after it has been located inside one of
//! the heap's own segments (or large blocks) and its header checked: foreign
//! pointers, interior pointers to free blocks and double frees are refused.

/// Payload alignment and header size.
pub const ALIGN: usize = 16;
pub const HEADER: usize = 16;
/// Reservation granularity of the Windows personality.
pub const GRANULE: usize = 64 * 1024;
/// Largest class block (header included); larger blocks are mapped alone.
pub const MAX_CLASS_BYTES: usize = 64 * 1024;
/// Segment growth: 64 KiB, doubling up to 8 MiB per segment (each segment is
/// one kernel region, and a process has a bounded number of regions).
pub const MAX_SEGMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 96;
/// Bytes per large-block table entry (base, mapping size) and the initial
/// table mapping (4096 entries); the table doubles when full.
const LARGE_ENTRY: usize = 16;
const LARGE_TABLE_BYTES: usize = GRANULE;
/// Largest request (`HeapAlloc` fails with no memory above it).
pub const MAX_REQUEST: usize = 256 * 1024 * 1024;

pub const HEAP_NO_SERIALIZE: u32 = 0x1;
pub const HEAP_GENERATE_EXCEPTIONS: u32 = 0x4;
pub const HEAP_ZERO_MEMORY: u32 = 0x8;
pub const HEAP_REALLOC_IN_PLACE_ONLY: u32 = 0x10;

const MAGIC: u64 = 0x4448_5021;
const ALLOCATED: u64 = 1;
const LARGE: u64 = 0xffff;
const CLASSES: [usize; 43] = [
    32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024,
    1280, 1536, 1792, 2048, 2560, 3072, 3584, 4096, 5120, 6144, 7168, 8192, 10240, 12288, 14336,
    16384, 20480, 24576, 28672, 32768, 40960, 49152, 57344, 65536,
];
const _: () = assert!(CLASSES[CLASSES.len() - 1] == MAX_CLASS_BYTES);

/// Committed private memory and access to it. The guest provider maps whole
/// Windows reservations; host tests use owned buffers.
pub trait Pages {
    /// Reserve and commit `bytes` (a multiple of `GRANULE`) of zeroed,
    /// read-write memory aligned to `ALIGN`, or `None`.
    fn map(&mut self, bytes: usize) -> Option<usize>;
    /// Release a whole mapping previously returned by `map`.
    fn unmap(&mut self, base: usize, bytes: usize);
    fn read(&self, address: usize) -> u64;
    fn write(&mut self, address: usize, value: u64);
    fn fill(&mut self, address: usize, bytes: usize, value: u8);
    /// Non-overlapping copy between two blocks of this heap.
    fn copy(&mut self, destination: usize, source: usize, bytes: usize);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// STATUS_NO_MEMORY / ERROR_NOT_ENOUGH_MEMORY.
    NoMemory,
    /// Not a live block of this heap (ERROR_INVALID_PARAMETER).
    InvalidPointer,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Segment {
    base: usize,
    bytes: usize,
    used: usize,
}

#[derive(Clone, Copy)]
struct Block {
    header: usize,
    class: u64,
    capacity: usize,
    requested: usize,
}

/// Large-block records `(base, mapping bytes)`, sorted by base, stored in a
/// provider mapping of `bytes` (0 before the first large block).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LargeTable {
    base: usize,
    bytes: usize,
    count: usize,
}

impl LargeTable {
    const EMPTY: Self = Self {
        base: 0,
        bytes: 0,
        count: 0,
    };
    fn capacity(&self) -> usize {
        self.bytes / LARGE_ENTRY
    }
    fn entry(&self, pages: &impl Pages, index: usize) -> (usize, usize) {
        let at = self.base + index * LARGE_ENTRY;
        (pages.read(at) as usize, pages.read(at + 8) as usize)
    }
    fn set(&self, pages: &mut impl Pages, index: usize, (base, bytes): (usize, usize)) {
        let at = self.base + index * LARGE_ENTRY;
        pages.write(at, base as u64);
        pages.write(at + 8, bytes as u64);
    }
    /// `Ok(index)` of the record for `base`, or `Err(insertion index)`.
    fn search(&self, pages: &impl Pages, base: usize) -> Result<usize, usize> {
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let middle = low + (high - low) / 2;
            match self.entry(pages, middle).0.cmp(&base) {
                core::cmp::Ordering::Equal => return Ok(middle),
                core::cmp::Ordering::Less => low = middle + 1,
                core::cmp::Ordering::Greater => high = middle,
            }
        }
        Err(low)
    }
    /// Record a new block, doubling the table mapping when it is full. On
    /// failure the table is unchanged.
    fn insert(&mut self, pages: &mut impl Pages, record: (usize, usize)) -> Result<(), Error> {
        let index = match self.search(pages, record.0) {
            Ok(_) => return Err(Error::NoMemory),
            Err(index) => index,
        };
        if self.count < self.capacity() {
            for slot in (index..self.count).rev() {
                let moved = self.entry(pages, slot);
                self.set(pages, slot + 1, moved);
            }
            self.set(pages, index, record);
            self.count += 1;
            return Ok(());
        }
        let bytes = if self.bytes == 0 {
            LARGE_TABLE_BYTES
        } else {
            self.bytes.checked_mul(2).ok_or(Error::NoMemory)?
        };
        let base = pages.map(bytes).ok_or(Error::NoMemory)?;
        if base == 0 || !base.is_multiple_of(ALIGN) {
            pages.unmap(base, bytes);
            return Err(Error::NoMemory);
        }
        let grown = Self {
            base,
            bytes,
            count: self.count + 1,
        };
        if index > 0 {
            pages.copy(base, self.base, index * LARGE_ENTRY);
        }
        grown.set(pages, index, record);
        if index < self.count {
            pages.copy(
                base + (index + 1) * LARGE_ENTRY,
                self.base + index * LARGE_ENTRY,
                (self.count - index) * LARGE_ENTRY,
            );
        }
        if self.bytes != 0 {
            pages.unmap(self.base, self.bytes);
        }
        *self = grown;
        Ok(())
    }
    fn remove(&mut self, pages: &mut impl Pages, index: usize) {
        for slot in index + 1..self.count {
            let moved = self.entry(pages, slot);
            self.set(pages, slot - 1, moved);
        }
        self.count -= 1;
    }
}

pub struct Heap {
    free: [usize; CLASSES.len()],
    segments: [Segment; MAX_SEGMENTS],
    segment_count: usize,
    large: LargeTable,
    next_segment: usize,
    mapped: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            free: [0; CLASSES.len()],
            segments: [Segment {
                base: 0,
                bytes: 0,
                used: 0,
            }; MAX_SEGMENTS],
            segment_count: 0,
            large: LargeTable::EMPTY,
            next_segment: GRANULE,
            mapped: 0,
        }
    }
    /// Bytes currently mapped from the provider.
    pub fn mapped(&self) -> usize {
        self.mapped
    }
    /// Small-block segments in use (at most `MAX_SEGMENTS`).
    pub fn segments(&self) -> usize {
        self.segment_count
    }
    fn class(total: usize) -> Option<usize> {
        CLASSES.iter().position(|&bytes| bytes >= total)
    }
    pub fn alloc(
        &mut self,
        pages: &mut impl Pages,
        size: usize,
        zero: bool,
    ) -> Result<usize, Error> {
        if size > MAX_REQUEST {
            return Err(Error::NoMemory);
        }
        let total = size + HEADER;
        let Some(class) = Self::class(total) else {
            return self.alloc_large(pages, size, total);
        };
        let header = if self.free[class] != 0 {
            let header = self.free[class];
            self.free[class] = pages.read(header + HEADER) as usize;
            if zero {
                pages.fill(header + HEADER, size, 0);
            } else {
                // Never hand out the free-list link as stale payload.
                pages.write(header + HEADER, 0);
            }
            header
        } else {
            // Fresh segment bytes are zero-filled by the provider.
            self.carve(pages, CLASSES[class])?
        };
        pages.write(header, size as u64);
        pages.write(header + 8, (MAGIC << 32) | (ALLOCATED << 16) | class as u64);
        Ok(header + HEADER)
    }
    fn carve(&mut self, pages: &mut impl Pages, bytes: usize) -> Result<usize, Error> {
        if let Some(segment) = self.segments[..self.segment_count].last_mut()
            && segment.bytes - segment.used >= bytes
        {
            let header = segment.base + segment.used;
            segment.used += bytes;
            return Ok(header);
        }
        if self.segment_count == MAX_SEGMENTS {
            return Err(Error::NoMemory);
        }
        let needed = bytes.next_multiple_of(GRANULE);
        let mut size = self.next_segment.max(needed);
        // Near the budget, fall back to the smallest segment that fits.
        let base = match pages.map(size) {
            Some(base) => base,
            None if size > needed => {
                size = needed;
                pages.map(size).ok_or(Error::NoMemory)?
            }
            None => return Err(Error::NoMemory),
        };
        if !base.is_multiple_of(ALIGN) {
            pages.unmap(base, size);
            return Err(Error::NoMemory);
        }
        self.next_segment = (self.next_segment * 2).min(MAX_SEGMENT_BYTES);
        self.mapped += size;
        self.segments[self.segment_count] = Segment {
            base,
            bytes: size,
            used: bytes,
        };
        self.segment_count += 1;
        Ok(base)
    }
    fn alloc_large(
        &mut self,
        pages: &mut impl Pages,
        size: usize,
        total: usize,
    ) -> Result<usize, Error> {
        let bytes = total.next_multiple_of(GRANULE);
        let base = pages.map(bytes).ok_or(Error::NoMemory)?;
        if base == 0 || !base.is_multiple_of(ALIGN) {
            pages.unmap(base, bytes);
            return Err(Error::NoMemory);
        }
        let table = self.large.bytes;
        if let Err(error) = self.large.insert(pages, (base, bytes)) {
            pages.unmap(base, bytes);
            return Err(error);
        }
        self.mapped = self.mapped + self.large.bytes - table + bytes;
        pages.write(base, size as u64);
        pages.write(base + 8, (MAGIC << 32) | (ALLOCATED << 16) | LARGE);
        Ok(base + HEADER)
    }
    fn locate(&self, pages: &impl Pages, payload: usize) -> Result<Block, Error> {
        if payload < HEADER || !payload.is_multiple_of(ALIGN) {
            return Err(Error::InvalidPointer);
        }
        let header = payload - HEADER;
        let large = self
            .large
            .search(pages, header)
            .ok()
            .map(|index| self.large.entry(pages, index));
        let segment = self.segments[..self.segment_count]
            .iter()
            .find(|segment| header >= segment.base && header < segment.base + segment.used);
        let (limit, large_bytes) = match (large, segment) {
            (Some((base, bytes)), _) => (base + bytes, Some(bytes)),
            (None, Some(segment)) => (segment.base + segment.used, None),
            (None, None) => return Err(Error::InvalidPointer),
        };
        let tag = pages.read(header + 8);
        let class = tag & 0xffff;
        if tag >> 32 != MAGIC || (tag >> 16) & 0xffff != ALLOCATED {
            return Err(Error::InvalidPointer);
        }
        let capacity = match (large_bytes, class) {
            (Some(bytes), LARGE) => bytes - HEADER,
            (None, class) if (class as usize) < CLASSES.len() => {
                let bytes = CLASSES[class as usize];
                if header + bytes > limit {
                    return Err(Error::InvalidPointer);
                }
                bytes - HEADER
            }
            _ => return Err(Error::InvalidPointer),
        };
        let requested = pages.read(header) as usize;
        if requested > capacity {
            return Err(Error::InvalidPointer);
        }
        Ok(Block {
            header,
            class,
            capacity,
            requested,
        })
    }
    pub fn free(&mut self, pages: &mut impl Pages, payload: usize) -> Result<(), Error> {
        let block = self.locate(pages, payload)?;
        if block.class == LARGE {
            let index = self
                .large
                .search(pages, block.header)
                .map_err(|_| Error::InvalidPointer)?;
            let (base, bytes) = self.large.entry(pages, index);
            self.large.remove(pages, index);
            self.mapped -= bytes;
            pages.unmap(base, bytes);
            return Ok(());
        }
        let class = block.class as usize;
        pages.write(block.header + 8, (MAGIC << 32) | class as u64);
        pages.write(block.header + HEADER, self.free[class] as u64);
        self.free[class] = block.header;
        Ok(())
    }
    /// `HeapSize`: the size most recently requested for the block.
    pub fn size(&self, pages: &impl Pages, payload: usize) -> Result<usize, Error> {
        self.locate(pages, payload).map(|block| block.requested)
    }
    pub fn validate(&self, pages: &impl Pages, payload: usize) -> bool {
        self.locate(pages, payload).is_ok()
    }
    /// `HeapReAlloc`: grows or shrinks in place when the block's capacity
    /// allows, otherwise moves (unless `in_place_only`). Newly exposed bytes
    /// are zeroed with `zero`. A failed call leaves the original block live.
    pub fn realloc(
        &mut self,
        pages: &mut impl Pages,
        payload: usize,
        size: usize,
        zero: bool,
        in_place_only: bool,
    ) -> Result<usize, Error> {
        let block = self.locate(pages, payload)?;
        if size > MAX_REQUEST {
            return Err(Error::NoMemory);
        }
        if size <= block.capacity {
            if zero && size > block.requested {
                pages.fill(payload + block.requested, size - block.requested, 0);
            }
            pages.write(block.header, size as u64);
            return Ok(payload);
        }
        if in_place_only {
            return Err(Error::NoMemory);
        }
        let moved = self.alloc(pages, size, false)?;
        pages.copy(moved, payload, block.requested);
        if zero {
            pages.fill(moved + block.requested, size - block.requested, 0);
        }
        self.free(pages, payload)?;
        Ok(moved)
    }
    /// `HeapDestroy`: return every mapping; outstanding pointers die with it.
    pub fn destroy(&mut self, pages: &mut impl Pages) {
        for segment in &self.segments[..self.segment_count] {
            pages.unmap(segment.base, segment.bytes);
        }
        for index in 0..self.large.count {
            let (base, bytes) = self.large.entry(pages, index);
            pages.unmap(base, bytes);
        }
        if self.large.bytes != 0 {
            pages.unmap(self.large.base, self.large.bytes);
        }
        self.reset();
    }
    /// Forget every block in place (no large temporary, so guest callers
    /// keep small stack frames). Mappings must already be returned.
    pub fn reset(&mut self) {
        self.free.fill(0);
        self.segment_count = 0;
        self.large = LargeTable::EMPTY;
        self.next_segment = GRANULE;
        self.mapped = 0;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{collections::BTreeMap, vec, vec::Vec};

    #[derive(Default)]
    struct Fake {
        maps: BTreeMap<usize, Vec<u8>>,
        next: usize,
        budget: usize,
        live: usize,
    }
    impl Fake {
        fn new(budget: usize) -> Self {
            Self {
                next: 0x5000_0000,
                budget,
                ..Self::default()
            }
        }
        fn at(&self, address: usize) -> (&Vec<u8>, usize) {
            let (base, bytes) = self.maps.range(..=address).next_back().unwrap();
            assert!(address < base + bytes.len(), "access outside mappings");
            (bytes, address - base)
        }
        fn at_mut(&mut self, address: usize) -> (&mut Vec<u8>, usize) {
            let (base, bytes) = self.maps.range_mut(..=address).next_back().unwrap();
            assert!(address < base + bytes.len(), "access outside mappings");
            (bytes, address - base)
        }
    }
    impl Pages for Fake {
        fn map(&mut self, bytes: usize) -> Option<usize> {
            assert!(bytes.is_multiple_of(GRANULE));
            if self.live + bytes > self.budget {
                return None;
            }
            let base = self.next;
            self.next += bytes + GRANULE;
            self.live += bytes;
            self.maps.insert(base, vec![0; bytes]);
            Some(base)
        }
        fn unmap(&mut self, base: usize, bytes: usize) {
            assert_eq!(self.maps.remove(&base).unwrap().len(), bytes);
            self.live -= bytes;
        }
        fn read(&self, address: usize) -> u64 {
            let (bytes, offset) = self.at(address);
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
        }
        fn write(&mut self, address: usize, value: u64) {
            let (bytes, offset) = self.at_mut(address);
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn fill(&mut self, address: usize, count: usize, value: u8) {
            if count == 0 {
                return;
            }
            let (bytes, offset) = self.at_mut(address);
            bytes[offset..offset + count].fill(value);
        }
        fn copy(&mut self, destination: usize, source: usize, count: usize) {
            let data: Vec<u8> = (0..count)
                .map(|index| {
                    let (bytes, offset) = self.at(source + index);
                    bytes[offset]
                })
                .collect();
            for (index, value) in data.into_iter().enumerate() {
                let (bytes, offset) = self.at_mut(destination + index);
                bytes[offset] = value;
            }
        }
    }

    #[test]
    fn blocks_are_aligned_sized_reused_and_zeroed_on_request() {
        let mut pages = Fake::new(16 << 20);
        let mut heap = Heap::new();
        let a = heap.alloc(&mut pages, 0, false).unwrap();
        let b = heap.alloc(&mut pages, 100, false).unwrap();
        assert!(a.is_multiple_of(ALIGN) && b.is_multiple_of(ALIGN) && a != b);
        assert_eq!(heap.size(&pages, a), Ok(0));
        assert_eq!(heap.size(&pages, b), Ok(100));
        pages.fill(b, 100, 0xaa);
        heap.free(&mut pages, b).unwrap();
        assert_eq!(heap.free(&mut pages, b), Err(Error::InvalidPointer));
        assert_eq!(heap.size(&pages, b), Err(Error::InvalidPointer));
        let c = heap.alloc(&mut pages, 110, true).unwrap();
        assert_eq!(c, b, "same class reuses the freed block");
        assert!((0..110).all(|i| pages.at(c + i).0[pages.at(c + i).1] == 0));
        assert_eq!(heap.mapped(), GRANULE);
        for bad in [0, 8, c + 16, c + 1, 0x1234_5670] {
            assert_eq!(heap.free(&mut pages, bad), Err(Error::InvalidPointer));
        }
    }

    #[test]
    fn segments_grow_and_large_blocks_map_and_unmap_alone() {
        let mut pages = Fake::new(64 << 20);
        let mut heap = Heap::new();
        let blocks: Vec<usize> = (0..200)
            .map(|i| heap.alloc(&mut pages, 1000 + i, false).unwrap())
            .collect();
        assert!(heap.segment_count > 1);
        assert!(heap.mapped() <= 1 << 20);
        let big = heap.alloc(&mut pages, 300_000, false).unwrap();
        let before = heap.mapped();
        assert_eq!(heap.size(&pages, big), Ok(300_000));
        heap.free(&mut pages, big).unwrap();
        assert_eq!(heap.mapped(), before - 320 * 1024);
        assert_eq!(heap.free(&mut pages, big), Err(Error::InvalidPointer));
        for block in blocks {
            heap.free(&mut pages, block).unwrap();
        }
        heap.destroy(&mut pages);
        assert_eq!(pages.live, 0);
        assert_eq!(heap.mapped(), 0);
    }

    #[test]
    fn realloc_keeps_contents_zeroes_growth_and_honours_in_place_only() {
        let mut pages = Fake::new(16 << 20);
        let mut heap = Heap::new();
        let a = heap.alloc(&mut pages, 20, false).unwrap();
        for i in 0..20 {
            let (bytes, offset) = pages.at_mut(a + i);
            bytes[offset] = i as u8 + 1;
        }
        // 20 + 16 fits the 48-byte class: grows in place, zeroing the tail.
        assert_eq!(heap.realloc(&mut pages, a, 30, true, true), Ok(a));
        assert_eq!(heap.size(&pages, a), Ok(30));
        assert_eq!(pages.at(a + 25).0[pages.at(a + 25).1], 0);
        assert_eq!(
            heap.realloc(&mut pages, a, 5000, false, true),
            Err(Error::NoMemory)
        );
        let moved = heap.realloc(&mut pages, a, 5000, true, false).unwrap();
        assert_ne!(moved, a);
        assert_eq!(heap.size(&pages, a), Err(Error::InvalidPointer));
        for i in 0..20 {
            assert_eq!(pages.at(moved + i).0[pages.at(moved + i).1], i as u8 + 1);
        }
        assert_eq!(pages.at(moved + 4000).0[pages.at(moved + 4000).1], 0);
        let shrunk = heap.realloc(&mut pages, moved, 10, false, false).unwrap();
        assert_eq!((shrunk, heap.size(&pages, shrunk)), (moved, Ok(10)));
    }

    #[test]
    fn exhaustion_fails_cleanly_and_leaves_existing_blocks_live() {
        let mut pages = Fake::new(GRANULE * 3);
        let mut heap = Heap::new();
        // 64 KiB first segment.
        let a = heap.alloc(&mut pages, 40_000, false).unwrap();
        assert_eq!(heap.alloc(&mut pages, 200_000, false), Err(Error::NoMemory));
        assert_eq!(
            heap.alloc(&mut pages, MAX_REQUEST + 1, false),
            Err(Error::NoMemory)
        );
        // Second segment (128 KiB) holds two 64 KiB class blocks.
        let b = heap.alloc(&mut pages, 60_000, false).unwrap();
        let c = heap.alloc(&mut pages, 60_000, false).unwrap();
        assert_eq!(heap.alloc(&mut pages, 60_000, false), Err(Error::NoMemory));
        assert_eq!(
            heap.realloc(&mut pages, a, 100_000, false, false),
            Err(Error::NoMemory)
        );
        assert_eq!(heap.size(&pages, a), Ok(40_000));
        for block in [a, b, c] {
            heap.free(&mut pages, block).unwrap();
        }
        assert_eq!(heap.alloc(&mut pages, 60_000, false), Ok(c));
    }

    /// Sparse provider: mappings are sizes only and written words live in a
    /// map, so thousands of large blocks cost no host memory.
    #[derive(Default)]
    struct Sparse {
        maps: BTreeMap<usize, usize>,
        words: BTreeMap<usize, u64>,
        next: usize,
        budget: usize,
        live: usize,
    }
    impl Sparse {
        fn new(budget: usize) -> Self {
            Self {
                next: 0x1_0000_0000,
                budget,
                ..Self::default()
            }
        }
        fn check(&self, address: usize, count: usize) {
            let (base, bytes) = self.maps.range(..=address).next_back().unwrap();
            assert!(address + count <= base + bytes, "access outside mappings");
            assert!(address.is_multiple_of(8) && count.is_multiple_of(8));
        }
        fn clear(&mut self, address: usize, count: usize) {
            let keys: Vec<usize> = self
                .words
                .range(address..address + count)
                .map(|(key, _)| *key)
                .collect();
            for key in keys {
                self.words.remove(&key);
            }
        }
    }
    impl Pages for Sparse {
        fn map(&mut self, bytes: usize) -> Option<usize> {
            assert!(bytes.is_multiple_of(GRANULE));
            if self.live + bytes > self.budget {
                return None;
            }
            let base = self.next;
            self.next += bytes + GRANULE;
            self.live += bytes;
            self.maps.insert(base, bytes);
            Some(base)
        }
        fn unmap(&mut self, base: usize, bytes: usize) {
            assert_eq!(self.maps.remove(&base), Some(bytes));
            self.clear(base, bytes);
            self.live -= bytes;
        }
        fn read(&self, address: usize) -> u64 {
            self.check(address, 8);
            self.words.get(&address).copied().unwrap_or(0)
        }
        fn write(&mut self, address: usize, value: u64) {
            self.check(address, 8);
            self.words.insert(address, value);
        }
        fn fill(&mut self, address: usize, count: usize, value: u8) {
            assert_eq!(value, 0);
            self.check(address, count);
            self.clear(address, count);
        }
        fn copy(&mut self, destination: usize, source: usize, count: usize) {
            self.check(source, count);
            self.check(destination, count);
            for offset in (0..count).step_by(8) {
                let value = self.read(source + offset);
                self.write(destination + offset, value);
            }
        }
    }

    #[test]
    fn large_blocks_are_unbounded_by_a_fixed_table_and_never_leak() {
        // Smallest large block: one byte over the largest class.
        const SIZE: usize = MAX_CLASS_BYTES - HEADER + 1;
        const BLOCK: usize = 2 * GRANULE;
        const COUNT: usize = 5_000; // > 4096 entries: the table doubles
        let mut pages = Sparse::new(usize::MAX);
        let mut heap = Heap::new();
        let blocks: Vec<usize> = (0..COUNT)
            .map(|index| heap.alloc(&mut pages, SIZE + index % 7, false).unwrap())
            .collect();
        // 5,000 entries of 16 bytes need a 128 KiB table (64 -> 128).
        assert_eq!(heap.large.count, COUNT);
        assert_eq!(heap.large.bytes, 2 * GRANULE);
        assert_eq!(heap.mapped(), COUNT * BLOCK + 2 * GRANULE);
        assert_eq!(pages.live, heap.mapped(), "the old table was returned");
        for (index, &block) in blocks.iter().enumerate() {
            assert_eq!(heap.size(&pages, block), Ok(SIZE + index % 7));
        }
        // Interior pointers and the table itself are not blocks.
        for bad in [blocks[5] + 16, blocks[5] + BLOCK, heap.large.base + HEADER] {
            assert_eq!(heap.free(&mut pages, bad), Err(Error::InvalidPointer));
        }
        // Free a scattered sample (records shift down), then the rest from
        // the end; every remaining record stays findable.
        let mut order: Vec<usize> = (0..COUNT).map(|index| (index * 7919) % COUNT).collect();
        let tail = order.split_off(64);
        let mut rest: Vec<usize> = tail;
        rest.sort_unstable_by(|a, b| b.cmp(a));
        order.extend(rest);
        for (freed, &index) in order.iter().enumerate() {
            heap.free(&mut pages, blocks[index]).unwrap();
            assert_eq!(
                heap.free(&mut pages, blocks[index]),
                Err(Error::InvalidPointer)
            );
            if freed % 97 == 0 && freed + 1 < COUNT {
                let other = order[freed + 1];
                assert_eq!(heap.size(&pages, blocks[other]), Ok(SIZE + other % 7));
            }
        }
        assert_eq!(heap.large.count, 0);
        assert_eq!(heap.mapped(), 2 * GRANULE);
        // Slots are reused after frees, and destroy returns everything.
        let again = heap.alloc(&mut pages, SIZE, true).unwrap();
        assert_eq!(heap.size(&pages, again), Ok(SIZE));
        heap.destroy(&mut pages);
        assert_eq!((pages.live, heap.mapped()), (0, 0));
    }

    #[test]
    fn large_table_growth_inserts_at_the_front_middle_and_end_in_order() {
        let mut pages = Sparse::new(usize::MAX);
        let mut table = LargeTable::EMPTY;
        let full = LARGE_TABLE_BYTES / LARGE_ENTRY;
        // Even bases, appended (no shifting) until the table is full.
        for index in 0..full {
            table
                .insert(&mut pages, ((index + 1) * 0x20, index))
                .unwrap();
        }
        assert_eq!((table.count, table.bytes), (full, LARGE_TABLE_BYTES));
        let first = table.base;
        // A full table grows on a middle insert: both halves are copied.
        table
            .insert(&mut pages, (full / 2 * 0x20 + 0x10, 7))
            .unwrap();
        assert_eq!(
            (table.count, table.bytes),
            (full + 1, 2 * LARGE_TABLE_BYTES)
        );
        assert_ne!(table.base, first);
        assert_eq!(pages.live, 2 * LARGE_TABLE_BYTES, "old table unmapped");
        table.insert(&mut pages, (0x10, 1)).unwrap();
        table.insert(&mut pages, ((full + 5) * 0x20, 2)).unwrap();
        assert_eq!(
            table.insert(&mut pages, (0x10, 3)),
            Err(Error::NoMemory),
            "duplicate base"
        );
        let entries: Vec<(usize, usize)> =
            (0..table.count).map(|i| table.entry(&pages, i)).collect();
        assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(entries[0], (0x10, 1));
        assert_eq!(*entries.last().unwrap(), ((full + 5) * 0x20, 2));
        assert_eq!(
            table
                .search(&pages, full / 2 * 0x20 + 0x10)
                .map(|i| entries[i].1),
            Ok(7)
        );
        assert_eq!(table.search(&pages, 0x18), Err(1));
        table.remove(&mut pages, 0);
        assert_eq!(table.entry(&pages, 0), (0x20, 0));
        assert_eq!(table.count, full + 2);
    }

    #[test]
    fn failed_large_table_growth_unmaps_the_new_block_and_keeps_the_rest() {
        const SIZE: usize = MAX_CLASS_BYTES - HEADER + 1;
        const BLOCK: usize = 2 * GRANULE;
        let full = LARGE_TABLE_BYTES / LARGE_ENTRY;
        // Room for a full first table, its blocks and one more block, but
        // not for the doubled table.
        let budget = LARGE_TABLE_BYTES + (full + 1) * BLOCK;
        let mut pages = Sparse::new(budget);
        let mut heap = Heap::new();
        let blocks: Vec<usize> = (0..full)
            .map(|_| heap.alloc(&mut pages, SIZE, false).unwrap())
            .collect();
        let live = pages.live;
        assert_eq!(heap.alloc(&mut pages, SIZE, false), Err(Error::NoMemory));
        assert_eq!(pages.live, live, "the unrecorded block was unmapped");
        assert_eq!(heap.mapped(), live);
        assert_eq!(heap.large.bytes, LARGE_TABLE_BYTES);
        for &block in &blocks {
            assert_eq!(heap.size(&pages, block), Ok(SIZE));
        }
        // Class allocations still work from the remaining budget.
        let small = heap.alloc(&mut pages, 64, false).unwrap();
        heap.free(&mut pages, small).unwrap();
        for block in blocks.into_iter().rev() {
            heap.free(&mut pages, block).unwrap();
        }
        heap.destroy(&mut pages);
        assert_eq!(pages.live, 0);
    }
}
