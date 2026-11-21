//! Private four-level table mechanics. Physical table addresses are translated
//! through the owned supervisor alias; they are never dereferenced as pointers.
use super::paging::{MAX_NEW_TABLES, TableBitmap};
use super::{Error, PAGE_BYTES, Permissions, UserAccess};
use core::{
    ptr::NonNull,
    sync::atomic::{Ordering, compiler_fence},
};

mod pages;
pub use pages::{Fault, Page, Protection};

/// Table pages per chunk: the first chunk comes with the space, more are
/// added as the process maps memory (`add_chunk`), up to
/// `MAX_TABLE_CHUNKS` (P18: Chromium's sparse heaps need thousands).
pub(super) const TABLE_PAGES: usize = 256;
pub(super) const MAX_TABLE_CHUNKS: usize = 25;
const CHUNK_WORDS: usize = TABLE_PAGES * 512;
const ADDRESS: u64 = 0x000f_ffff_ffff_f000;
const PRESENT: u64 = 1;
const WRITE: u64 = 2;
const USER: u64 = 4;
const NX: u64 = 1 << 63;
const ACCESSED_DIRTY: u64 = 0x60;

/// Which table pages are in use, by global index (chunk * TABLE_PAGES +
/// page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Used([TableBitmap<4>; MAX_TABLE_CHUNKS]);
impl Used {
    fn contains(&self, index: usize) -> bool {
        index < MAX_TABLE_CHUNKS * TABLE_PAGES
            && self.0[index / TABLE_PAGES].contains(index % TABLE_PAGES)
    }
    fn insert(&mut self, index: usize) {
        self.0[index / TABLE_PAGES].insert(index % TABLE_PAGES);
    }
    fn remove(&mut self, index: usize) {
        self.0[index / TABLE_PAGES].remove(index % TABLE_PAGES);
    }
    fn count(&self) -> u32 {
        self.0.iter().map(TableBitmap::count).sum()
    }
}

pub(super) struct UserTable {
    /// Each chunk's kernel alias and physical base (chunk 0 holds the root).
    chunks: [Option<(NonNull<u64>, u64)>; MAX_TABLE_CHUNKS],
    physical: u64,
    used: Used,
    retire: fn(u64, u64),
}
impl UserTable {
    /// Caller owns TABLE_PAGES contiguous zeroed table pages for this lifetime.
    pub(super) unsafe fn new(base: NonNull<u64>, physical: u64, retire: fn(u64, u64)) -> Self {
        let mut chunks = [None; MAX_TABLE_CHUNKS];
        chunks[0] = Some((base, physical));
        let mut used = Used([TableBitmap::empty(); MAX_TABLE_CHUNKS]);
        used.0[0] = TableBitmap::root();
        Self {
            chunks,
            physical,
            used,
            retire,
        }
    }
    pub(super) const fn root(&self) -> u64 {
        self.physical
    }
    pub(super) fn used_pages(&self) -> u32 {
        self.used.count()
    }
    /// Table pages not in use across the chunks.
    pub(super) fn free_pages(&self) -> usize {
        self.chunks.iter().flatten().count() * TABLE_PAGES - self.used.count() as usize
    }
    /// Another chunk of TABLE_PAGES table pages (the caller owns them and
    /// the alias for this table's lifetime); its index, or None when every
    /// chunk slot is taken.
    pub(super) fn add_chunk(&mut self, base: NonNull<u64>, physical: u64) -> Option<usize> {
        let slot = self.chunks.iter().position(Option::is_none)?;
        self.chunks[slot] = Some((base, physical));
        Some(slot)
    }
    /// The global index of the table page at `table` (a physical address).
    fn index_of(&self, table: u64) -> Option<usize> {
        self.chunks.iter().enumerate().find_map(|(c, chunk)| {
            let (_, physical) = (*chunk)?;
            let offset = table.checked_sub(physical)?;
            (offset.is_multiple_of(PAGE_BYTES) && offset / PAGE_BYTES < TABLE_PAGES as u64)
                .then(|| c * TABLE_PAGES + (offset / PAGE_BYTES) as usize)
        })
    }
    /// The physical address of table page `index`.
    fn table_physical(&self, index: usize) -> u64 {
        let (_, physical) = self.chunks[index / TABLE_PAGES].expect("allocated chunk");
        physical + (index % TABLE_PAGES) as u64 * PAGE_BYTES
    }
    /// A free table page, marked used and zeroed: its physical address and
    /// global index.
    fn allocate_table(&mut self) -> Result<(u64, usize), Error> {
        let index = (0..MAX_TABLE_CHUNKS * TABLE_PAGES)
            .find(|i| self.chunks[i / TABLE_PAGES].is_some() && !self.used.contains(*i))
            .ok_or(Error::Exhausted)?;
        self.used.insert(index);
        // Reused rollback pages are zeroed before publication.
        // SAFETY: an unused page of an owned chunk.
        unsafe { core::ptr::write_bytes(self.word(index * 512), 0, 512) };
        Ok((self.table_physical(index), index))
    }
    /// The word at global `offset` (table index * 512 + entry).
    fn word(&self, offset: usize) -> *mut u64 {
        let (base, _) = self.chunks[offset / CHUNK_WORDS].expect("allocated chunk");
        // SAFETY: inside the chunk's owned alias.
        unsafe { base.as_ptr().add(offset % CHUNK_WORDS) }
    }
    fn entry_offset(&self, table: u64, index: u64) -> Result<usize, Error> {
        let page = self.index_of(table).ok_or(Error::InvalidTable)?;
        if !self.used.contains(page) || index >= 512 {
            return Err(Error::InvalidTable);
        }
        Ok(page * 512 + index as usize)
    }
    fn read(&self, offset: usize) -> u64 {
        // SAFETY: every offset comes from entry_offset in an owned alias.
        unsafe { self.word(offset).read_volatile() }
    }
    fn write(&mut self, offset: usize, value: u64) {
        // SAFETY: one VM writer owns every chunk; the aliases remain live.
        unsafe { self.word(offset).write_volatile(value) };
        compiler_fence(Ordering::SeqCst);
    }
    fn leaf(&self, address: u64) -> Result<Option<usize>, Error> {
        let mut table = self.physical;
        for shift in [39, 30, 21, 12] {
            let offset = self.entry_offset(table, (address >> shift) & 511)?;
            let entry = self.read(offset);
            if entry & PRESENT == 0 {
                return if entry == 0 {
                    Ok(None)
                } else {
                    Err(Error::InvalidTable)
                };
            }
            if shift == 12 {
                return Ok(Some(offset));
            }
            if entry & (1 << 7) != 0 {
                return Err(Error::InvalidTable);
            }
            table = entry & ADDRESS;
        }
        Err(Error::InvalidTable)
    }
    pub(super) fn translate(&self, address: u64, access: UserAccess) -> Result<u64, Error> {
        let mut table = self.physical;
        for shift in [39, 30, 21, 12] {
            let entry = self.read(self.entry_offset(table, (address >> shift) & 511)?);
            if entry & PRESENT == 0 {
                return Err(Error::NotCommitted);
            }
            if entry & USER == 0
                || (access == UserAccess::Write && entry & WRITE == 0)
                || (access == UserAccess::Execute && entry & NX != 0)
            {
                return Err(Error::AccessDenied);
            }
            // Huge pages, global translations and alternate cache attributes are
            // never part of the bounded native mapping contract.
            if entry & (0x18 | 0x80 | 0x100) != 0 {
                return Err(Error::InvalidTable);
            }
            if shift == 12 {
                return Ok((entry & ADDRESS) | (address & 4095));
            }
            table = entry & ADDRESS;
        }
        Err(Error::InvalidTable)
    }
    pub(super) fn map(
        &mut self,
        start: u64,
        physical: u64,
        pages: u64,
        permissions: Permissions,
    ) -> Result<(), Error> {
        for page in 0..pages {
            if self.leaf(start + page * PAGE_BYTES)?.is_some() {
                return Err(Error::Overlap);
            }
        }
        let mut links = [(0usize, 0u32); MAX_NEW_TABLES];
        let mut links_len = 0;
        let mut mapped = 0;
        let result = (|| {
            for page in 0..pages {
                let address = start + page * PAGE_BYTES;
                let mut table = self.physical;
                for shift in [39, 30, 21] {
                    let offset = self.entry_offset(table, (address >> shift) & 511)?;
                    let mut entry = self.read(offset);
                    if entry == 0 {
                        let (table_physical, index) = self.allocate_table()?;
                        entry = table_physical | PRESENT | WRITE | USER;
                        links[links_len] = (offset, index as u32);
                        links_len += 1;
                        self.write(offset, entry);
                    } else if entry & (PRESENT | WRITE | USER | NX | 0x180)
                        != PRESENT | WRITE | USER
                    {
                        return Err(Error::InvalidTable);
                    }
                    table = entry & ADDRESS;
                }
                let offset = self.entry_offset(table, (address >> 12) & 511)?;
                self.write(offset, (physical + page * PAGE_BYTES) | flags(permissions));
                mapped += 1;
                (self.retire)(self.physical, address);
            }
            Ok(())
        })();
        if result.is_err() {
            for page in 0..mapped {
                let address = start + page * PAGE_BYTES;
                let offset = self
                    .leaf(address)
                    .expect("rollback table")
                    .expect("rollback leaf");
                self.write(offset, 0);
                (self.retire)(self.physical, address);
            }
            for &(offset, index) in links[..links_len].iter().rev() {
                self.write(offset, 0);
                // Callback must cover paging-structure caches, not just a leaf.
                (self.retire)(self.physical, start);
                self.used.remove(index as usize);
            }
        }
        result
    }
    pub(super) fn validate_mapping(
        &self,
        start: u64,
        physical: u64,
        pages: u64,
        permissions: Permissions,
    ) -> Result<(), Error> {
        for page in 0..pages {
            let address = start + page * PAGE_BYTES;
            let offset = self.leaf(address)?.ok_or(Error::NotCommitted)?;
            if self.read(offset) & !ACCESSED_DIRTY
                != (physical + page * PAGE_BYTES) | flags(permissions)
            {
                return Err(Error::InvalidMapping);
            }
            self.translate(
                address,
                match permissions {
                    Permissions::ReadOnly => UserAccess::Read,
                    Permissions::ReadWrite => UserAccess::Write,
                    Permissions::ReadExecute => UserAccess::Execute,
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn remove(&mut self, start: u64, pages: u64) {
        for page in 0..pages {
            let address = start + page * PAGE_BYTES;
            let offset = self
                .leaf(address)
                .expect("prevalidated table")
                .expect("prevalidated leaf");
            self.write(offset, 0);
            (self.retire)(self.physical, address);
        }
    }
    pub(super) fn protect(&mut self, start: u64, pages: u64, permissions: Permissions) {
        for page in 0..pages {
            let address = start + page * PAGE_BYTES;
            let offset = self
                .leaf(address)
                .expect("prevalidated table")
                .expect("prevalidated leaf");
            self.write(offset, (self.read(offset) & ADDRESS) | flags(permissions));
            (self.retire)(self.physical, address);
        }
    }
    pub(super) fn retire_root(&self) {
        (self.retire)(self.physical, 0);
    }
}
fn flags(permissions: Permissions) -> u64 {
    PRESENT
        | USER
        | match permissions {
            Permissions::ReadOnly => NX,
            Permissions::ReadWrite => NX | WRITE,
            Permissions::ReadExecute => 0,
        }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{boxed::Box, vec};
    fn model() -> (Box<[u64]>, UserTable) {
        let mut pool = vec![0u64; TABLE_PAGES * 512].into_boxed_slice();
        let tree = unsafe {
            UserTable::new(
                NonNull::new(pool.as_mut_ptr()).unwrap(),
                0x200000,
                |_, _| {},
            )
        };
        (pool, tree)
    }
    #[test]
    fn private_roots_isolate_and_require_all_level_permissions() {
        let (_a, mut first) = model();
        let (_b, mut second) = model();
        first
            .map(0x400000, 0x800000, 2, Permissions::ReadWrite)
            .unwrap();
        second
            .map(0x400000, 0x900000, 1, Permissions::ReadExecute)
            .unwrap();
        assert_eq!(first.translate(0x400123, UserAccess::Write), Ok(0x800123));
        assert_eq!(
            second.translate(0x400123, UserAccess::Execute),
            Ok(0x900123)
        );
        assert_eq!(
            first.translate(0x400000, UserAccess::Execute),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            second.translate(0x400000, UserAccess::Write),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            first.translate(0x3ff000, UserAccess::Read),
            Err(Error::NotCommitted)
        );
        assert_eq!(
            second.translate(0x401000, UserAccess::Read),
            Err(Error::NotCommitted)
        );
        let entry = first.read(0);
        first.write(0, entry & !USER);
        assert_eq!(
            first.translate(0x400000, UserAccess::Read),
            Err(Error::AccessDenied)
        );
        first.write(0, entry & !WRITE);
        assert_eq!(
            first.translate(0x400000, UserAccess::Write),
            Err(Error::AccessDenied)
        );
        assert_eq!(first.translate(0x400000, UserAccess::Read), Ok(0x800000));
    }
    #[test]
    fn added_chunks_hold_more_tables_than_the_first() {
        let (_pool, mut tree) = model();
        // One page every 2 MiB: one leaf table each, past the first chunk.
        let map = |tree: &mut UserTable, count: u64| -> Result<(), Error> {
            for i in 0..count {
                tree.map(
                    0x4000_0000 + i * (2 << 20),
                    0x800000,
                    1,
                    Permissions::ReadOnly,
                )?;
            }
            Ok(())
        };
        assert_eq!(map(&mut tree, 300), Err(Error::Exhausted));
        let (_pool2, mut tree) = model();
        let mut extra = vec![0u64; TABLE_PAGES * 512].into_boxed_slice();
        assert_eq!(
            tree.add_chunk(NonNull::new(extra.as_mut_ptr()).unwrap(), 0x1000_0000),
            Some(1)
        );
        assert_eq!(tree.free_pages(), 2 * TABLE_PAGES - 1);
        map(&mut tree, 300).unwrap();
        // Tables in the second chunk are found by their physical address.
        assert_eq!(
            tree.translate(0x4000_0000 + 299 * (2 << 20), UserAccess::Read),
            Ok(0x800000)
        );
        assert!(tree.used_pages() as usize > TABLE_PAGES);
    }
    #[test]
    fn structural_exhaustion_rolls_back_only_new_tables_and_leaves() {
        let (_pool, mut tree) = model();
        tree.map(0x400000, 0x800000, 1, Permissions::ReadWrite)
            .unwrap();
        // Retain most pages to force failure after one new intermediate link.
        for index in 5..TABLE_PAGES {
            tree.used.insert(index);
        }
        let before = tree.used;
        assert_eq!(
            tree.map(1 << 39, 0xa00000, 1, Permissions::ReadOnly),
            Err(Error::Exhausted)
        );
        assert_eq!(tree.used, before);
        assert_eq!(tree.leaf(1 << 39), Ok(None));
        assert_eq!(tree.translate(0x400000, UserAccess::Write), Ok(0x800000));
    }
    #[test]
    fn overlap_protection_and_unmap_are_transactional() {
        let (_pool, mut tree) = model();
        tree.map(0x401000, 0x800000, 1, Permissions::ReadWrite)
            .unwrap();
        assert_eq!(
            tree.map(0x400000, 0x900000, 2, Permissions::ReadOnly),
            Err(Error::Overlap)
        );
        assert_eq!(tree.leaf(0x400000), Ok(None));
        tree.validate_mapping(0x401000, 0x800000, 1, Permissions::ReadWrite)
            .unwrap();
        tree.protect(0x401000, 1, Permissions::ReadExecute);
        assert_eq!(
            tree.translate(0x401000, UserAccess::Write),
            Err(Error::AccessDenied)
        );
        assert_eq!(tree.translate(0x401000, UserAccess::Execute), Ok(0x800000));
        tree.remove(0x401000, 1);
        assert_eq!(
            tree.translate(0x401000, UserAccess::Read),
            Err(Error::NotCommitted)
        );
        assert_eq!(tree.used_pages(), 4);
    }
    #[test]
    fn exhaustion_after_a_published_leaf_retires_it_before_returning() {
        let (_pool, mut tree) = model();
        tree.map(0x10000, 0x800000, 1, Permissions::ReadWrite)
            .unwrap();
        for index in 0..TABLE_PAGES {
            tree.used.insert(index);
        }
        assert_eq!(
            tree.map(0x1ff000, 0x900000, 2, Permissions::ReadOnly),
            Err(Error::Exhausted)
        );
        assert_eq!(tree.leaf(0x1ff000), Ok(None));
        assert_eq!(tree.leaf(0x200000), Ok(None));
        assert_eq!(tree.translate(0x10000, UserAccess::Write), Ok(0x800000));
    }
}
