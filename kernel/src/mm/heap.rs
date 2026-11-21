use super::Error;
use core::{alloc::Layout, marker::PhantomData, ptr::NonNull};

/// Retained-only bootstrap storage. No implicit global allocator, realloc, or
/// deallocation: every successful block remains owned by this heap's lifetime.
pub struct EarlyHeap<'a> {
    base: NonNull<u8>,
    capacity: usize,
    cursor: usize,
    storage: PhantomData<&'a mut [u8]>,
}

#[derive(Debug)]
pub struct HeapBlock {
    pointer: NonNull<u8>,
    bytes: usize,
}
impl HeapBlock {
    pub const fn as_ptr(&self) -> *mut u8 {
        self.pointer.as_ptr()
    }
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}
impl<'a> EarlyHeap<'a> {
    pub fn new(storage: &'a mut [u8]) -> Self {
        Self {
            // Even an empty Rust slice has a non-null aligned pointer. Derive
            // this once: later allocations never reborrow already issued bytes.
            base: NonNull::new(storage.as_mut_ptr()).expect("slice pointer is non-null"),
            capacity: storage.len(),
            cursor: 0,
            storage: PhantomData,
        }
    }
    pub const fn used_bytes(&self) -> usize {
        self.cursor
    }
    pub fn capacity_bytes(&self) -> usize {
        self.capacity
    }
    pub fn allocate(&mut self, layout: Layout) -> Result<HeapBlock, Error> {
        if layout.size() == 0 {
            return Err(Error::Empty);
        }
        let base = self.base.as_ptr().addr();
        let current = base.checked_add(self.cursor).ok_or(Error::Overflow)?;
        let aligned = current
            .checked_add(layout.align() - 1)
            .ok_or(Error::Overflow)?
            & !(layout.align() - 1);
        let start = aligned.checked_sub(base).ok_or(Error::Overflow)?;
        let end = start.checked_add(layout.size()).ok_or(Error::Overflow)?;
        if end > self.capacity {
            return Err(Error::Exhausted);
        }
        // SAFETY: start..end is within the original exclusive slice, nonempty,
        // and beyond every previous allocation. Pointer addition retains the
        // original backing provenance. No reference spans earlier live blocks.
        let pointer = unsafe {
            let pointer = self.base.as_ptr().add(start);
            core::ptr::write_bytes(pointer, 0, layout.size());
            NonNull::new_unchecked(pointer)
        };
        self.cursor = end;
        Ok(HeapBlock {
            pointer,
            bytes: layout.size(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_zeroing_and_failed_allocation_do_not_overlap() {
        let mut storage = [0xaa; 512];
        let mut heap = EarlyHeap::new(&mut storage[1..]);
        let a = heap
            .allocate(Layout::from_size_align(32, 64).unwrap())
            .unwrap();
        let b = heap
            .allocate(Layout::from_size_align(24, 32).unwrap())
            .unwrap();
        assert_eq!(a.as_ptr() as usize % 64, 0);
        assert_eq!(b.as_ptr() as usize % 32, 0);
        assert!(a.as_ptr() as usize + a.bytes() <= b.as_ptr() as usize);
        assert!(
            unsafe { core::slice::from_raw_parts(a.as_ptr(), a.bytes()) }
                .iter()
                .all(|&b| b == 0)
        );
        let used = heap.used_bytes();
        assert!(matches!(
            heap.allocate(Layout::from_size_align(1024, 1).unwrap()),
            Err(Error::Exhausted)
        ));
        assert_eq!(heap.used_bytes(), used);
        let c = heap
            .allocate(Layout::from_size_align(1, 1).unwrap())
            .unwrap();
        assert!(c.as_ptr() as usize >= b.as_ptr() as usize + b.bytes());
        assert!(matches!(
            heap.allocate(Layout::from_size_align(0, 1).unwrap()),
            Err(Error::Empty)
        ));
    }

    #[test]
    fn issued_block_remains_accessible_after_later_allocations_and_exhaustion() {
        let mut storage = [0xaa; 128];
        let mut heap = EarlyHeap::new(&mut storage);
        let first = heap
            .allocate(Layout::from_size_align(16, 1).unwrap())
            .unwrap();
        // SAFETY: these disjoint retained blocks live in storage for this test;
        // raw reads/writes never create a reference across another allocation.
        unsafe {
            core::ptr::write_bytes(first.as_ptr(), 0x7b, first.bytes());
        }
        let second = heap
            .allocate(Layout::from_size_align(32, 16).unwrap())
            .unwrap();
        unsafe {
            core::ptr::write_bytes(second.as_ptr(), 0x9c, second.bytes());
        }
        assert!(matches!(
            heap.allocate(Layout::from_size_align(128, 1).unwrap()),
            Err(Error::Exhausted)
        ));
        let third = heap
            .allocate(Layout::from_size_align(1, 1).unwrap())
            .unwrap();
        for index in 0..first.bytes() {
            assert_eq!(unsafe { first.as_ptr().add(index).read() }, 0x7b);
        }
        unsafe {
            first.as_ptr().add(first.bytes() - 1).write(0x3e);
        }
        assert_eq!(
            unsafe { first.as_ptr().add(first.bytes() - 1).read() },
            0x3e
        );
        for index in 0..second.bytes() {
            assert_eq!(unsafe { second.as_ptr().add(index).read() }, 0x9c);
        }
        assert_eq!(unsafe { third.as_ptr().read() }, 0);
    }
}
