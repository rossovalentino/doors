//! The pages of one Windows section (P18): CreateFileMapping memory that
//! any number of views, in any processes, map at once. Sparse like a
//! reservation: CoreCLR reserves a 2 TB SEC_RESERVE section for its
//! double-mapped code heap and commits a few pages of it, so a section
//! costs nothing until pages are committed and touched.
//!
//! A four-level radix of 512-entry nodes; every node is a page from the
//! same page supply as the data pages (the page pool in the kernel), so no
//! fixed table bounds a section. A leaf entry is 0 (reserved), COMMITTED
//! (committed, no page yet: SEC_RESERVE after MEM_COMMIT) or a page's
//! physical address | COMMITTED.
use super::{Error, PAGE_BYTES, process_space::PageSource};

/// The largest section: 2^36 pages (256 TiB), four levels of 512.
pub const MAX_PAGES: u64 = 1 << 36;
const COMMITTED: u64 = 1;
const ADDRESS: u64 = !(PAGE_BYTES - 1);

/// A page supply whose pages the kernel can also read and write (radix
/// nodes, filling file data).
pub trait NodeMemory: PageSource {
    /// Where the kernel sees the page at `physical` (a page from `take`).
    fn words(&mut self, physical: u64) -> Result<*mut u64, Error>;
}

/// One section page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    /// Not committed (a SEC_RESERVE section before MEM_COMMIT).
    Reserved,
    /// Committed; no page is allocated yet (it reads as zeroes).
    Committed,
    /// Committed, backed by this physical page.
    Present(u64),
}

pub struct SectionPages {
    root: u64,
    pages: u64,
    /// SEC_COMMIT: every page is committed from the start.
    all_committed: bool,
    /// Data pages allocated.
    present: u64,
    /// Radix node pages allocated.
    nodes: u64,
}
impl SectionPages {
    pub fn new(pages: u64, all_committed: bool) -> Result<Self, Error> {
        if pages == 0 || pages > MAX_PAGES {
            return Err(Error::Overflow);
        }
        Ok(Self {
            root: 0,
            pages,
            all_committed,
            present: 0,
            nodes: 0,
        })
    }
    pub const fn pages(&self) -> u64 {
        self.pages
    }
    pub const fn all_committed(&self) -> bool {
        self.all_committed
    }
    /// Pages this section holds from the supply (data and nodes).
    pub const fn held_pages(&self) -> u64 {
        self.present + self.nodes
    }
    /// The leaf entry of page `index`, creating nodes when `create`
    /// (None when a node is missing and not created).
    ///
    /// # Safety
    /// `memory` is the supply every node of this section came from.
    unsafe fn leaf(
        &mut self,
        memory: &mut dyn NodeMemory,
        index: u64,
        create: bool,
    ) -> Result<Option<*mut u64>, Error> {
        if index >= self.pages {
            return Err(Error::Address);
        }
        if self.root == 0 {
            if !create {
                return Ok(None);
            }
            self.root = unsafe { memory.take() }?;
            self.nodes += 1;
        }
        let mut node = self.root;
        for shift in [27u64, 18, 9] {
            let words = memory.words(node)?;
            // SAFETY: a node page of this section, 512 entries.
            let slot = unsafe { words.add(((index >> shift) & 511) as usize) };
            let mut next = unsafe { slot.read() };
            if next == 0 {
                if !create {
                    return Ok(None);
                }
                next = unsafe { memory.take() }?;
                self.nodes += 1;
                unsafe { slot.write(next) };
            }
            node = next;
        }
        let words = memory.words(node)?;
        Ok(Some(unsafe { words.add((index & 511) as usize) }))
    }
    /// The state of page `index`.
    ///
    /// # Safety
    /// As `leaf`.
    pub unsafe fn state(
        &mut self,
        memory: &mut dyn NodeMemory,
        index: u64,
    ) -> Result<PageState, Error> {
        let entry = match unsafe { self.leaf(memory, index, false) }? {
            // SAFETY: a leaf entry of this section.
            Some(slot) => unsafe { slot.read() },
            None => 0,
        };
        Ok(if entry & ADDRESS != 0 {
            PageState::Present(entry & ADDRESS)
        } else if entry & COMMITTED != 0 || self.all_committed {
            PageState::Committed
        } else {
            PageState::Reserved
        })
    }
    /// MEM_COMMIT of page `index` (SEC_RESERVE); a committed page stays.
    ///
    /// # Safety
    /// As `leaf`.
    pub unsafe fn commit(&mut self, memory: &mut dyn NodeMemory, index: u64) -> Result<(), Error> {
        if self.all_committed {
            return if index < self.pages {
                Ok(())
            } else {
                Err(Error::Address)
            };
        }
        let slot = unsafe { self.leaf(memory, index, true) }?.ok_or(Error::InvalidTable)?;
        // SAFETY: a leaf entry of this section.
        unsafe {
            if slot.read() == 0 {
                slot.write(COMMITTED);
            }
        }
        Ok(())
    }
    /// The page backing committed page `index`, allocated (zeroed) on its
    /// first use. A reserved page is `NotCommitted`.
    ///
    /// # Safety
    /// As `leaf`.
    pub unsafe fn page(&mut self, memory: &mut dyn NodeMemory, index: u64) -> Result<u64, Error> {
        match unsafe { self.state(memory, index) }? {
            PageState::Present(physical) => Ok(physical),
            PageState::Reserved => Err(Error::NotCommitted),
            PageState::Committed => {
                let slot = unsafe { self.leaf(memory, index, true) }?.ok_or(Error::InvalidTable)?;
                let physical = unsafe { memory.take() }?;
                // SAFETY: a leaf entry of this section.
                unsafe { slot.write(physical | COMMITTED) };
                self.present += 1;
                Ok(physical)
            }
        }
    }
    /// Give every page and node back to `memory`; the section is empty
    /// afterwards. The pages freed (data and nodes).
    ///
    /// # Safety
    /// As `leaf`; no view maps any page of the section any more and their
    /// translations are retired on every CPU.
    pub unsafe fn release(&mut self, memory: &mut dyn NodeMemory) -> Result<u64, Error> {
        if self.root == 0 {
            return Ok(0);
        }
        let freed = unsafe { Self::release_node(memory, self.root, 3) }?;
        self.root = 0;
        self.present = 0;
        self.nodes = 0;
        Ok(freed)
    }
    unsafe fn release_node(
        memory: &mut dyn NodeMemory,
        node: u64,
        depth: u32,
    ) -> Result<u64, Error> {
        let mut freed = 0;
        for i in 0..512 {
            let words = memory.words(node)?;
            // SAFETY: a node page, 512 entries.
            let entry = unsafe { words.add(i).read() };
            if depth == 0 {
                if entry & ADDRESS != 0 {
                    unsafe { memory.give(entry & ADDRESS) }?;
                    freed += 1;
                }
            } else if entry != 0 {
                freed += unsafe { Self::release_node(memory, entry, depth - 1) }?;
            }
        }
        unsafe { memory.give(node) }?;
        Ok(freed + 1)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    extern crate std;
    use super::*;
    use std::{boxed::Box, collections::BTreeMap};
    /// Pages with real memory behind them; fails after `limit`.
    pub(crate) struct Memory {
        pub pages: BTreeMap<u64, Box<[u64; 512]>>,
        next: u64,
        limit: usize,
    }
    impl Memory {
        pub(crate) fn new(limit: usize) -> Self {
            Self {
                pages: BTreeMap::new(),
                next: 0x1000_0000,
                limit,
            }
        }
    }
    impl PageSource for Memory {
        unsafe fn take(&mut self) -> Result<u64, Error> {
            if self.pages.len() >= self.limit {
                return Err(Error::Exhausted);
            }
            self.next += PAGE_BYTES;
            self.pages.insert(self.next, Box::new([0; 512]));
            Ok(self.next)
        }
        unsafe fn give(&mut self, physical: u64) -> Result<(), Error> {
            self.pages
                .remove(&physical)
                .map(|_| ())
                .ok_or(Error::InvalidFree)
        }
    }
    impl NodeMemory for Memory {
        fn words(&mut self, physical: u64) -> Result<*mut u64, Error> {
            self.pages
                .get_mut(&physical)
                .map(|p| p.as_mut_ptr())
                .ok_or(Error::InvalidState)
        }
    }
    #[test]
    fn a_huge_reserved_section_costs_nothing_until_committed() {
        let mut m = Memory::new(64);
        // CoreCLR's double-mapped code heap: 2 TB SEC_RESERVE.
        let mut s = SectionPages::new((2u64 << 40) / PAGE_BYTES, false).unwrap();
        assert_eq!(m.pages.len(), 0);
        let far = s.pages() - 1;
        unsafe {
            assert_eq!(s.state(&mut m, far), Ok(PageState::Reserved));
            assert_eq!(s.page(&mut m, far), Err(Error::NotCommitted));
            assert_eq!(m.pages.len(), 0);
            s.commit(&mut m, far).unwrap();
            assert_eq!(s.state(&mut m, far), Ok(PageState::Committed));
            // Four nodes, no data page yet.
            assert_eq!((m.pages.len(), s.held_pages()), (4, 4));
            let p = s.page(&mut m, far).unwrap();
            assert_eq!(s.page(&mut m, far), Ok(p));
            assert_eq!(s.state(&mut m, far), Ok(PageState::Present(p)));
            assert_eq!(s.held_pages(), 5);
            assert_eq!(s.page(&mut m, s.pages()), Err(Error::Address));
            assert_eq!(s.release(&mut m), Ok(5));
        }
        assert_eq!(m.pages.len(), 0);
    }
    #[test]
    fn a_committed_section_allocates_pages_on_first_use() {
        let mut m = Memory::new(64);
        let mut s = SectionPages::new(1024, true).unwrap();
        unsafe {
            assert_eq!(s.state(&mut m, 7), Ok(PageState::Committed));
            let a = s.page(&mut m, 7).unwrap();
            let b = s.page(&mut m, 600).unwrap();
            assert_ne!(a, b);
            assert_eq!(s.state(&mut m, 8), Ok(PageState::Committed));
            // Two data pages, one leaf node each under shared upper nodes.
            let held = s.held_pages();
            assert_eq!(s.release(&mut m), Ok(held));
        }
        assert_eq!(m.pages.len(), 0);
    }
    #[test]
    fn exhaustion_leaves_the_section_consistent() {
        let mut m = Memory::new(4);
        let mut s = SectionPages::new(16, true).unwrap();
        unsafe {
            // Four nodes fill the supply; the data page does not fit.
            assert_eq!(s.page(&mut m, 3), Err(Error::Exhausted));
            assert_eq!(s.state(&mut m, 3), Ok(PageState::Committed));
            assert_eq!(s.release(&mut m), Ok(4));
        }
        assert_eq!(m.pages.len(), 0);
    }
}
