//! Rust `#[global_allocator]` for Door's original compatibility DLLs
//! (user32, gdi32, comdlg32, comctl32, ...): the process heap of ntdll,
//! the same heap `HeapAlloc(GetProcessHeap(), ...)` uses.
//!
//! Enable feature `guest-heap` of `door-windows-abi` in a PE DLL crate and
//! declare:
//!
//! ```ignore
//! #[global_allocator]
//! static HEAP: crate::allocator::ProcessHeap =
//!     crate::allocator::ProcessHeap;
//! ```
//!
//! The DLL then imports `RtlAllocateHeap`, `RtlFreeHeap`,
//! `RtlReAllocateHeap` and `DoorGetProcessHeap` from ntdll. Requests up to
//! 16-byte alignment map directly; larger alignments over-allocate and keep
//! the heap pointer in the eight bytes below the aligned block. The heap is
//! serialized by ntdll and safe to use from every thread; it never calls
//! back into application code, so it may be used inside window procedures.
#![allow(unsafe_code)]
use crate::heap::{ALIGN, HEAP_ZERO_MEMORY};
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU64, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlReAllocateHeap(heap: usize, flags: u32, pointer: usize, bytes: usize) -> usize;
    fn DoorGetProcessHeap() -> u64;
}

static PROCESS_HEAP: AtomicU64 = AtomicU64::new(0);

fn heap() -> u64 {
    let cached = PROCESS_HEAP.load(Ordering::Acquire);
    if cached != 0 {
        return cached;
    }
    // SAFETY: a plain query of the bootstrap-created process heap handle.
    let value = unsafe { DoorGetProcessHeap() };
    PROCESS_HEAP.store(value, Ordering::Release);
    value
}

fn allocate(layout: Layout, flags: u32) -> *mut u8 {
    let size = layout.size().max(1);
    if layout.align() <= ALIGN {
        // SAFETY: ntdll validates the handle and returns null on failure.
        return unsafe { RtlAllocateHeap(crate::narrow(heap()), flags, size) } as *mut u8;
    }
    let Some(total) = size.checked_add(layout.align() + 8) else {
        return core::ptr::null_mut();
    };
    // SAFETY: as above.
    let raw = unsafe { RtlAllocateHeap(crate::narrow(heap()), flags, total) };
    if raw == 0 {
        return core::ptr::null_mut();
    }
    let aligned = (raw + 8).next_multiple_of(crate::narrow(layout.align() as u64));
    // SAFETY: `aligned - 8 >= raw` and the block holds `align + 8` spare bytes.
    unsafe { ((aligned - 8) as *mut u64).write(crate::widen(raw)) };
    aligned as *mut u8
}

fn block(pointer: *mut u8, layout: Layout) -> u64 {
    if layout.align() <= ALIGN {
        pointer as u64
    } else {
        // SAFETY: `allocate` stored the heap block below every over-aligned
        // pointer it returned; GlobalAlloc callers pass those pointers back.
        unsafe { (pointer as *const u64).sub(1).read() }
    }
}

/// The ntdll process heap as a Rust global allocator.
pub struct ProcessHeap;

// SAFETY: every returned block is exclusively owned, at least `layout.size()`
// bytes and aligned to `layout.align()`; failures return null.
unsafe impl GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocate(layout, 0)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocate(layout, HEAP_ZERO_MEMORY)
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer came from this allocator with this layout.
        unsafe {
            RtlFreeHeap(
                crate::narrow(heap()),
                0,
                crate::narrow(block(pointer, layout)),
            )
        };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if layout.align() <= ALIGN {
            // SAFETY: the pointer is a live direct heap block; a failure
            // leaves it untouched, as GlobalAlloc::realloc requires.
            return unsafe {
                RtlReAllocateHeap(
                    crate::narrow(heap()),
                    0,
                    crate::narrow(pointer as u64),
                    size.max(1),
                )
            } as *mut u8;
        }
        let Ok(moved_layout) = Layout::from_size_align(size, layout.align()) else {
            return core::ptr::null_mut();
        };
        let moved = allocate(moved_layout, 0);
        if !moved.is_null() {
            // SAFETY: both blocks are live and distinct.
            unsafe {
                core::ptr::copy_nonoverlapping(pointer, moved, layout.size().min(size));
                self.dealloc(pointer, layout);
            }
        }
        moved
    }
}
