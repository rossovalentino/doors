//! malloc and friends over the process heap (as the UCRT allocates since
//! Windows 10), `_aligned_*` with a header before the aligned block, and
//! the new-handler hooks C++ runtimes call.
use super::{
    DoorGetProcessHeap, RtlAllocateHeap, RtlFreeHeap, RtlReAllocateHeap, RtlSizeHeap, errno,
    set_errno,
};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

const HEAP_ZERO_MEMORY: u32 = 0x8;
fn process_heap() -> usize {
    unsafe { DoorGetProcessHeap() as usize }
}
/// A heap block of `bytes` (zeroed when asked), or 0.
pub(crate) unsafe fn raw_alloc(bytes: usize, zero: bool) -> usize {
    unsafe {
        RtlAllocateHeap(
            process_heap(),
            if zero { HEAP_ZERO_MEMORY } else { 0 },
            bytes.max(1),
        )
    }
}
pub(crate) unsafe fn raw_free(block: usize) {
    unsafe { RtlFreeHeap(process_heap(), 0, block) };
}

static NEW_MODE: AtomicI32 = AtomicI32::new(0);
static NEW_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// `_callnewh`: the program's new handler (set through
/// `_set_new_handler`), asked to free memory: 1 when it returned (retry).
#[unsafe(no_mangle)]
pub extern "C" fn _callnewh(bytes: usize) -> i32 {
    let handler = NEW_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return 0;
    }
    let f: extern "C" fn(usize) -> i32 = unsafe { core::mem::transmute(handler) };
    i32::from(f(bytes) != 0)
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_new_handler(handler: usize) -> usize {
    NEW_HANDLER.swap(handler, Ordering::AcqRel)
}
#[unsafe(no_mangle)]
pub extern "C" fn _query_new_handler() -> usize {
    NEW_HANDLER.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_new_mode(mode: i32) -> i32 {
    NEW_MODE.swap(mode, Ordering::AcqRel)
}
#[unsafe(no_mangle)]
pub extern "C" fn _query_new_mode() -> i32 {
    NEW_MODE.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_heap_handle() -> usize {
    process_heap()
}

fn allocate(bytes: usize, zero: bool) -> *mut u8 {
    if bytes > isize::MAX as usize {
        set_errno(errno::ENOMEM);
        return core::ptr::null_mut();
    }
    loop {
        let block = unsafe { raw_alloc(bytes, zero) };
        if block != 0 {
            return block as *mut u8;
        }
        // malloc calls the new handler only in new mode (_set_new_mode(1)).
        if NEW_MODE.load(Ordering::Acquire) == 0 || _callnewh(bytes) == 0 {
            set_errno(errno::ENOMEM);
            return core::ptr::null_mut();
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn malloc(bytes: usize) -> *mut u8 {
    allocate(bytes, false)
}
#[unsafe(no_mangle)]
pub extern "C" fn _malloc_base(bytes: usize) -> *mut u8 {
    allocate(bytes, false)
}
#[unsafe(no_mangle)]
pub extern "C" fn calloc(count: usize, bytes: usize) -> *mut u8 {
    match count.checked_mul(bytes) {
        Some(total) => allocate(total, true),
        None => {
            set_errno(errno::ENOMEM);
            core::ptr::null_mut()
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _calloc_base(count: usize, bytes: usize) -> *mut u8 {
    calloc(count, bytes)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(block: *mut u8) {
    if !block.is_null() {
        unsafe { raw_free(block as usize) };
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _free_base(block: *mut u8) {
    unsafe { free(block) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(block: *mut u8, bytes: usize) -> *mut u8 {
    if block.is_null() {
        return malloc(bytes);
    }
    if bytes == 0 {
        unsafe { free(block) };
        return core::ptr::null_mut();
    }
    let moved = unsafe { RtlReAllocateHeap(process_heap(), 0, block as usize, bytes) };
    if moved == 0 {
        set_errno(errno::ENOMEM);
    }
    moved as *mut u8
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _realloc_base(block: *mut u8, bytes: usize) -> *mut u8 {
    unsafe { realloc(block, bytes) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _recalloc(block: *mut u8, count: usize, bytes: usize) -> *mut u8 {
    let Some(total) = count.checked_mul(bytes) else {
        set_errno(errno::ENOMEM);
        return core::ptr::null_mut();
    };
    let old = if block.is_null() {
        0
    } else {
        unsafe { _msize(block) }
    };
    let moved = unsafe { realloc(block, total) };
    if !moved.is_null() && total > old {
        unsafe { core::ptr::write_bytes(moved.add(old), 0, total - old) };
    }
    moved
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _msize(block: *mut u8) -> usize {
    if block.is_null() {
        set_errno(errno::EINVAL);
        return usize::MAX;
    }
    unsafe { RtlSizeHeap(process_heap(), 0, block as usize) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _expand(block: *mut u8, bytes: usize) -> *mut u8 {
    // In place only: the block stays where it is when it is big enough.
    if !block.is_null() && unsafe { _msize(block) } >= bytes {
        return block;
    }
    set_errno(errno::ENOMEM);
    core::ptr::null_mut()
}

/// `_aligned_malloc`: the original block's address sits just before the
/// aligned one.
#[unsafe(no_mangle)]
pub extern "C" fn _aligned_malloc(bytes: usize, alignment: usize) -> *mut u8 {
    unsafe { _aligned_offset_malloc(bytes, alignment, 0) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_offset_malloc(
    bytes: usize,
    alignment: usize,
    offset: usize,
) -> *mut u8 {
    if !alignment.is_power_of_two() || (offset != 0 && offset >= bytes) {
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    }
    let word = core::mem::size_of::<usize>();
    let alignment = alignment.max(word);
    let Some(total) = bytes.checked_add(alignment + word) else {
        set_errno(errno::ENOMEM);
        return core::ptr::null_mut();
    };
    let base = malloc(total) as usize;
    if base == 0 {
        return core::ptr::null_mut();
    }
    let aligned = ((base + word + offset + alignment - 1) & !(alignment - 1)) - offset;
    unsafe { *((aligned - word) as *mut usize) = base };
    aligned as *mut u8
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_free(block: *mut u8) {
    if block.is_null() {
        return;
    }
    let word = core::mem::size_of::<usize>();
    let base = unsafe { *((block as usize - word) as *const usize) };
    unsafe { free(base as *mut u8) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_realloc(
    block: *mut u8,
    bytes: usize,
    alignment: usize,
) -> *mut u8 {
    if block.is_null() {
        return _aligned_malloc(bytes, alignment);
    }
    if bytes == 0 {
        unsafe { _aligned_free(block) };
        return core::ptr::null_mut();
    }
    let word = core::mem::size_of::<usize>();
    let base = unsafe { *((block as usize - word) as *const usize) };
    let old = unsafe { _msize(base as *mut u8) } - (block as usize - base);
    let next = _aligned_malloc(bytes, alignment);
    if !next.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(block, next, old.min(bytes));
            _aligned_free(block);
        }
    }
    next
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _aligned_msize(
    block: *mut u8,
    _alignment: usize,
    _offset: usize,
) -> usize {
    let word = core::mem::size_of::<usize>();
    let base = unsafe { *((block as usize - word) as *const usize) };
    unsafe { _msize(base as *mut u8) - (block as usize - base) }
}
/// `_heapchk` and friends: the heap is consistent as far as the CRT knows.
#[unsafe(no_mangle)]
pub extern "C" fn _heapchk() -> i32 {
    -2 // _HEAPOK
}
#[unsafe(no_mangle)]
pub extern "C" fn _heapmin() -> i32 {
    0
}
