//! Single 4 KiB pages for page-granular user memory (P18 M0b: Windows
//! dynamic memory committed and protected page by page, as CoreCLR and V8
//! use it).
//!
//! The frame allocator keeps a bounded number of allocation records and the
//! kernel has no direct physical map, so single pages cannot each be a frame
//! allocation with its own alias. The pool takes 4 MiB chunks instead: one
//! frame allocation and one kernel alias per chunk (zeroed when mapped),
//! sub-allocated by a bitmap. A page is zeroed through the chunk's alias
//! when it is handed out again. A chunk whose pages are all free goes back
//! to the frame allocator while another chunk still has room.
//!
//! Callers own the retirement contract: a page is freed only after every
//! user mapping of it is gone and its TLB entries retired on every CPU.
use super::{
    Error, FrameAllocator, MappedRegion, PAGE_BYTES, PageCount, PageTables, Permissions,
    VirtualAddress,
};

/// Pages per chunk (4 MiB).
pub const CHUNK_PAGES: usize = 1024;
pub const CHUNK_BYTES: u64 = CHUNK_PAGES as u64 * PAGE_BYTES;
const WORDS: usize = CHUNK_PAGES / 64;
/// Chunks the pool holds at most (1 GiB of user pages).
pub const MAX_CHUNKS: usize = 256;
/// Kernel address space the pool's aliases take (one chunk per slot).
pub const ARENA_BYTES: u64 = MAX_CHUNKS as u64 * CHUNK_BYTES;

/// Free-page bookkeeping of one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bitmap {
    /// Set bit = free page.
    free: [u64; WORDS],
    used: u32,
}
impl Bitmap {
    pub const ALL_FREE: Self = Self {
        free: [u64::MAX; WORDS],
        used: 0,
    };
    /// The lowest free page, now taken.
    pub fn take(&mut self) -> Option<usize> {
        let (word, bits) = self.free.iter_mut().enumerate().find(|(_, w)| **w != 0)?;
        let bit = bits.trailing_zeros() as usize;
        *bits &= !(1u64 << bit);
        self.used += 1;
        Some(word * 64 + bit)
    }
    /// Give page `index` back; a page that is already free is an error (a
    /// double free would hand one page to two owners).
    pub fn give(&mut self, index: usize) -> Result<(), Error> {
        let word = self.free.get_mut(index / 64).ok_or(Error::InvalidFree)?;
        let bit = 1u64 << (index % 64);
        if *word & bit != 0 {
            return Err(Error::InvalidFree);
        }
        *word |= bit;
        self.used -= 1;
        Ok(())
    }
    pub const fn used(&self) -> u32 {
        self.used
    }
    pub const fn full(&self) -> bool {
        self.used as usize == CHUNK_PAGES
    }
}

struct Chunk {
    mapping: MappedRegion,
    physical: u64,
    alias: u64,
    map: Bitmap,
}

/// The pool. One instance, owned by the VM and used under its guard.
pub struct PagePool {
    chunks: [Option<Chunk>; MAX_CHUNKS],
    /// Base of the kernel alias arena (`ARENA_BYTES`, chunk `i` at
    /// `arena + i * CHUNK_BYTES`).
    arena: u64,
    in_use: u64,
}
impl PagePool {
    pub const fn new(arena: u64) -> Self {
        Self {
            chunks: [const { None }; MAX_CHUNKS],
            arena,
            in_use: 0,
        }
    }
    /// `new` in place, slot by slot (the pool is too large for the boot
    /// stack).
    ///
    /// # Safety
    /// `at` is valid, aligned, uninitialized storage owned by the caller.
    pub unsafe fn initialize_at(at: *mut Self, arena: u64) {
        unsafe {
            for i in 0..MAX_CHUNKS {
                core::ptr::addr_of_mut!((*at).chunks)
                    .cast::<Option<Chunk>>()
                    .add(i)
                    .write(None);
            }
            core::ptr::addr_of_mut!((*at).arena).write(arena);
            core::ptr::addr_of_mut!((*at).in_use).write(0);
        }
    }
    /// The kernel address of pool page `physical` (through its chunk's
    /// alias), so the kernel can fill a page it committed to a process.
    pub fn alias(&self, physical: u64) -> Option<u64> {
        self.chunks
            .iter()
            .flatten()
            .find(|c| physical >= c.physical && physical < c.physical + CHUNK_BYTES)
            .map(|c| c.alias + (physical - c.physical))
    }
    /// Pages handed out and not freed.
    pub const fn pages_in_use(&self) -> u64 {
        self.in_use
    }
    /// Chunks currently held (each 4 MiB of RAM).
    pub fn chunks(&self) -> usize {
        self.chunks.iter().flatten().count()
    }
    /// One zeroed page: its physical address.
    ///
    /// # Safety
    /// VM guard held; `kernel` is the kernel's page tables and the arena is
    /// reserved for this pool alone.
    pub unsafe fn allocate(
        &mut self,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
    ) -> Result<u64, Error> {
        for chunk in self.chunks.iter_mut().flatten() {
            if let Some(index) = chunk.map.take() {
                let offset = index as u64 * PAGE_BYTES;
                // SAFETY: the page is free (no user mapping) and the chunk's
                // alias is a live, exclusive RW kernel mapping.
                unsafe {
                    core::ptr::write_bytes(
                        (chunk.alias + offset) as *mut u8,
                        0,
                        PAGE_BYTES as usize,
                    )
                };
                self.in_use += 1;
                return Ok(chunk.physical + offset);
            }
        }
        let slot = self
            .chunks
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Exhausted)?;
        let owned = frames.allocate(PageCount::new(CHUNK_PAGES as u64)?, 1)?;
        let alias = VirtualAddress::new(self.arena + slot as u64 * CHUNK_BYTES)?;
        // map_frames zeroes the whole chunk before returning.
        let mapping = match unsafe { kernel.map_frames(owned, alias, Permissions::ReadWrite) } {
            Ok(m) => m,
            Err(failure) => {
                frames.free(failure.frames)?;
                return Err(failure.error);
            }
        };
        let physical = mapping.physical().ok_or(Error::InvalidState)?.get();
        let mut map = Bitmap::ALL_FREE;
        let index = map.take().expect("new chunk");
        self.chunks[slot] = Some(Chunk {
            mapping,
            physical,
            alias: alias.get(),
            map,
        });
        self.in_use += 1;
        Ok(physical + index as u64 * PAGE_BYTES)
    }
    /// Give back the page at `physical` (from `allocate`).
    ///
    /// # Safety
    /// VM guard held; no user mapping of the page remains and its TLB
    /// entries are retired on every CPU.
    pub unsafe fn free(
        &mut self,
        physical: u64,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
    ) -> Result<(), Error> {
        let slot = self
            .chunks
            .iter()
            .position(|c| {
                c.as_ref()
                    .is_some_and(|c| physical >= c.physical && physical < c.physical + CHUNK_BYTES)
            })
            .ok_or(Error::InvalidFree)?;
        if !physical.is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        let chunk = self.chunks[slot].as_mut().expect("located chunk");
        chunk
            .map
            .give(((physical - chunk.physical) / PAGE_BYTES) as usize)?;
        self.in_use -= 1;
        // An empty chunk goes back while another one has room for the next
        // allocation (so a page freed and taken again does not churn).
        let empty = chunk.map.used() == 0;
        let room_elsewhere = self
            .chunks
            .iter()
            .enumerate()
            .any(|(i, c)| i != slot && c.as_ref().is_some_and(|c| !c.map.full()));
        if empty && room_elsewhere {
            let chunk = self.chunks[slot].take().expect("empty chunk");
            let owned = unsafe { kernel.unmap(chunk.mapping) }
                .map_err(|f| f.error)?
                .ok_or(Error::InvalidState)?;
            frames.free(owned)?;
        }
        Ok(())
    }
    /// Every chunk with no page in use back to the frame allocator (the one
    /// `free` keeps for the next allocation too): at shutdown, so every
    /// frame is accounted for. Returns how many chunks went back.
    ///
    /// # Safety
    /// VM guard held; as `free`.
    pub unsafe fn release_empty(
        &mut self,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
    ) -> Result<usize, Error> {
        let mut released = 0;
        for slot in &mut self.chunks {
            if slot.as_ref().is_some_and(|c| c.map.used() == 0) {
                let chunk = slot.take().expect("empty chunk");
                let owned = unsafe { kernel.unmap(chunk.mapping) }
                    .map_err(|f| f.error)?
                    .ok_or(Error::InvalidState)?;
                frames.free(owned)?;
                released += 1;
            }
        }
        Ok(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bitmap_hands_out_each_page_once() {
        let mut b = Bitmap::ALL_FREE;
        let mut taken = [false; CHUNK_PAGES];
        for _ in 0..CHUNK_PAGES {
            let i = b.take().unwrap();
            assert!(!taken[i]);
            taken[i] = true;
        }
        assert!(b.full());
        assert_eq!(b.take(), None);
        b.give(700).unwrap();
        assert_eq!(b.give(700), Err(Error::InvalidFree));
        assert_eq!(b.take(), Some(700));
        assert_eq!(b.give(CHUNK_PAGES), Err(Error::InvalidFree));
        let mut fresh = Bitmap::ALL_FREE;
        assert_eq!(fresh.give(3), Err(Error::InvalidFree));
        assert_eq!(
            (fresh.take(), fresh.take(), fresh.used()),
            (Some(0), Some(1), 2)
        );
    }
    #[test]
    fn the_arena_fits_the_alias_window() {
        assert_eq!(CHUNK_BYTES, 4 << 20);
        assert_eq!(ARENA_BYTES, 1 << 30);
    }
}
