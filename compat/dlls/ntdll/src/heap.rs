//! `RtlAllocateHeap` family over the shared `door_windows_abi::heap` core.
//! Segments are whole private Windows reservations committed read-write in
//! one step; the kernel charges them to the manifest's heap budget. Each heap
//! is serialized by its own lock, never held across a callback.
use super::guest::RtlSetLastWin32Error;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use door_abi::operation as op;
use door_windows_abi::{
    error,
    heap::{self, Heap, Pages},
};

const MAX_HEAPS: usize = 32;
/// HEAP_CREATE_ENABLE_EXECUTE.
const ENABLE_EXECUTE: u32 = 0x4_0000;

struct Slot {
    locked: AtomicBool,
    live: AtomicBool,
    /// HEAP_CREATE_ENABLE_EXECUTE: segments committed EXECUTE_READWRITE.
    executable: AtomicBool,
    heap: UnsafeCell<Heap>,
}
// The per-slot lock owns every access to the heap state.
unsafe impl Sync for Slot {}
static HEAPS: [Slot; MAX_HEAPS] = [const {
    Slot {
        locked: AtomicBool::new(false),
        live: AtomicBool::new(false),
        executable: AtomicBool::new(false),
        heap: UnsafeCell::new(Heap::new()),
    }
}; MAX_HEAPS];
static PROCESS_HEAP: AtomicU64 = AtomicU64::new(0);

/// Segment pages with their PAGE_* protection: READWRITE, or for an
/// executable heap EXECUTE_READWRITE (the kernel keeps each page writable
/// or executable, never both, toggling on the faulting access).
struct Native(u64);
fn pages(handle: u64) -> Native {
    let executable = HEAPS.iter().any(|slot| {
        core::ptr::addr_of!(*slot) as u64 == handle && slot.executable.load(Ordering::Acquire)
    });
    Native(if executable { 0x40 } else { 4 })
}
/// The last failed page request: (1 reserve | 2 commit) << 16 | status.
static LAST_FAILURE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
impl Pages for Native {
    fn map(&mut self, bytes: usize) -> Option<usize> {
        let base = match unsafe { door_sys::raw(op::VM_RESERVE, [0, bytes as u64, 0, 4, 0, 0]) }
            .result()
        {
            Ok(base) => base,
            Err(e) => {
                LAST_FAILURE.store(1 << 16 | e as u64, Ordering::Relaxed);
                return None;
            }
        };
        let access = self.0;
        if let Err(e) =
            unsafe { door_sys::raw(op::VM_COMMIT, [base, bytes as u64, access, 0, 0, 0]) }.result()
        {
            LAST_FAILURE.store(2 << 16 | e as u64, Ordering::Relaxed);
            let _ = unsafe { door_sys::raw(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]) };
            return None;
        }
        Some(base as usize)
    }
    fn unmap(&mut self, base: usize, _bytes: usize) {
        let _ = unsafe { door_sys::raw(op::VM_WINDOWS_RELEASE, [base as u64, 0, 0, 0, 0, 0]) };
    }
    // SAFETY (all accessors): the core only touches addresses inside
    // mappings this provider returned and the heap still owns.
    fn read(&self, address: usize) -> u64 {
        unsafe { (address as *const u64).read() }
    }
    fn write(&mut self, address: usize, value: u64) {
        unsafe { (address as *mut u64).write(value) }
    }
    fn fill(&mut self, address: usize, bytes: usize, value: u8) {
        unsafe { core::ptr::write_bytes(address as *mut u8, value, bytes) }
    }
    fn copy(&mut self, destination: usize, source: usize, bytes: usize) {
        unsafe {
            core::ptr::copy_nonoverlapping(source as *const u8, destination as *mut u8, bytes)
        }
    }
}

struct Lease(&'static Slot);
impl Lease {
    /// Spin with yields; the holder never blocks or calls application code.
    fn acquire(handle: u64) -> Option<Self> {
        let slot = HEAPS
            .iter()
            .find(|slot| core::ptr::addr_of!(**slot) as u64 == handle)?;
        while slot
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let _ = door_sys::yield_now();
        }
        if !slot.live.load(Ordering::Acquire) {
            slot.locked.store(false, Ordering::Release);
            return None;
        }
        Some(Self(slot))
    }
    fn heap(&mut self) -> &mut Heap {
        // SAFETY: the lease exclusively owns the cell until Drop.
        unsafe { &mut *self.0.heap.get() }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.locked.store(false, Ordering::Release);
    }
}

fn create(executable: bool) -> u64 {
    for slot in &HEAPS {
        if slot
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            continue;
        }
        let free = !slot.live.load(Ordering::Acquire);
        if free {
            unsafe { (*slot.heap.get()).reset() };
            slot.executable.store(executable, Ordering::Release);
            slot.live.store(true, Ordering::Release);
        }
        slot.locked.store(false, Ordering::Release);
        if free {
            return core::ptr::addr_of!(*slot) as u64;
        }
    }
    0
}

/// Called once by the bootstrap before any DLL or application code runs.
pub(super) fn initialize() -> Result<u64, u32> {
    let handle = create(false);
    if handle == 0 {
        return Err(door_windows_abi::status::NO_MEMORY);
    }
    PROCESS_HEAP.store(handle, Ordering::Release);
    Ok(handle)
}

fn fail(code: u32) -> u64 {
    RtlSetLastWin32Error(code);
    0
}
fn failure(value: heap::Error) -> u32 {
    match value {
        heap::Error::NoMemory => error::NOT_ENOUGH_MEMORY,
        heap::Error::InvalidPointer => error::INVALID_PARAMETER,
    }
}
const KNOWN: u32 = heap::HEAP_NO_SERIALIZE
    | heap::HEAP_GENERATE_EXCEPTIONS
    | heap::HEAP_ZERO_MEMORY
    | heap::HEAP_REALLOC_IN_PLACE_ONLY;

#[unsafe(no_mangle)]
pub extern "system" fn DoorGetProcessHeap() -> u64 {
    PROCESS_HEAP.load(Ordering::Acquire)
}

/// Growable private heaps only: a caller-supplied base, an explicit commit
/// routine or a fixed maximum size is not part of this profile.
#[unsafe(no_mangle)]
pub extern "system" fn RtlCreateHeap(
    flags: u32,
    base: usize,
    _reserve: usize,
    _commit: usize,
    lock: usize,
    parameters: usize,
) -> usize {
    let base = door_windows_abi::widen(base);
    let lock = door_windows_abi::widen(lock);
    let parameters = door_windows_abi::widen(parameters);
    door_windows_abi::narrow((move || -> u64 {
        // HEAP_GROWABLE (2); HEAP_TAIL_CHECKING_ENABLED, HEAP_FREE_CHECKING_ENABLED
        // and HEAP_CREATE_ALIGN_16 are debugging and alignment hints this
        // heap already meets or ignores; HEAP_CREATE_ENABLE_EXECUTE (0x40000).
        if flags
            & !(heap::HEAP_NO_SERIALIZE
                | heap::HEAP_GENERATE_EXCEPTIONS
                | 0x2
                | 0x20
                | 0x40
                | 0x1_0000
                | ENABLE_EXECUTE)
            != 0
            || base != 0
            || lock != 0
            || parameters != 0
        {
            return fail(error::INVALID_PARAMETER);
        }
        match create(flags & ENABLE_EXECUTE != 0) {
            0 => fail(error::NOT_ENOUGH_MEMORY),
            handle => handle,
        }
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlDestroyHeap(handle: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        if handle == PROCESS_HEAP.load(Ordering::Acquire) {
            RtlSetLastWin32Error(error::INVALID_PARAMETER);
            return handle;
        }
        let Some(mut lease) = Lease::acquire(handle) else {
            RtlSetLastWin32Error(error::INVALID_HANDLE);
            return handle;
        };
        lease.heap().destroy(&mut pages(handle));
        lease.0.live.store(false, Ordering::Release);
        0
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlAllocateHeap(handle: usize, flags: u32, bytes: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        if flags & !KNOWN != 0 {
            return fail(error::INVALID_PARAMETER);
        }
        let Some(mut lease) = Lease::acquire(handle) else {
            return fail(error::INVALID_HANDLE);
        };
        match lease.heap().alloc(
            &mut pages(handle),
            bytes,
            flags & heap::HEAP_ZERO_MEMORY != 0,
        ) {
            Ok(address) => address as u64,
            Err(value) => {
                exhausted(lease.heap(), bytes);
                fail(failure(value))
            }
        }
    })())
}
/// The first failed allocation of the process, on the serial diagnostic
/// channel: how much the heap holds and in how many segments.
fn exhausted(heap: &heap::Heap, request: usize) {
    static REPORTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if REPORTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut text = [0u8; 96];
    let mut n = 0;
    let mut push = |s: &[u8]| {
        for b in s {
            if n < text.len() {
                text[n] = *b;
                n += 1;
            }
        }
    };
    let number = |v: usize, push: &mut dyn FnMut(&[u8])| {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        let mut v = v;
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        push(&digits[i..]);
    };
    push(b"DOOR_HEAP exhausted request=");
    number(request, &mut push);
    push(b" mapped=");
    number(heap.mapped(), &mut push);
    push(b" segments=");
    number(heap.segments(), &mut push);
    let last = LAST_FAILURE.load(Ordering::Relaxed);
    push(if last >> 16 == 1 {
        b" reserve="
    } else {
        b" commit="
    });
    number((last & 0xffff) as usize, &mut push);
    push(b"\n");
    let _ = door_sys::write(&text[..n]);
}

/// Freeing NULL succeeds, as on Windows.
#[unsafe(no_mangle)]
pub extern "system" fn RtlFreeHeap(handle: usize, flags: u32, pointer: usize) -> u8 {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    {
        if flags & !KNOWN != 0 {
            RtlSetLastWin32Error(error::INVALID_PARAMETER);
            return 0;
        }
        let Some(mut lease) = Lease::acquire(handle) else {
            RtlSetLastWin32Error(error::INVALID_HANDLE);
            return 0;
        };
        if pointer == 0 {
            return 1;
        }
        match lease.heap().free(&mut pages(handle), pointer as usize) {
            Ok(()) => 1,
            Err(value) => {
                RtlSetLastWin32Error(failure(value));
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlReAllocateHeap(
    handle: usize,
    flags: u32,
    pointer: usize,
    bytes: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    door_windows_abi::narrow((move || -> u64 {
        if flags & !KNOWN != 0 || pointer == 0 {
            return fail(error::INVALID_PARAMETER);
        }
        let Some(mut lease) = Lease::acquire(handle) else {
            return fail(error::INVALID_HANDLE);
        };
        match lease.heap().realloc(
            &mut pages(handle),
            pointer as usize,
            bytes,
            flags & heap::HEAP_ZERO_MEMORY != 0,
            flags & heap::HEAP_REALLOC_IN_PLACE_ONLY != 0,
        ) {
            Ok(address) => address as u64,
            Err(value) => fail(failure(value)),
        }
    })())
}

/// Returns `(SIZE_T)-1` for a pointer that is not a live block.
#[unsafe(no_mangle)]
pub extern "system" fn RtlSizeHeap(handle: usize, _flags: u32, pointer: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    {
        let Some(mut lease) = Lease::acquire(handle) else {
            RtlSetLastWin32Error(error::INVALID_HANDLE);
            return usize::MAX;
        };
        match lease.heap().size(&Native(4), pointer as usize) {
            Ok(bytes) => bytes,
            Err(value) => {
                RtlSetLastWin32Error(failure(value));
                usize::MAX
            }
        }
    }
}

/// With a NULL block, validates that the handle names a live heap.
#[unsafe(no_mangle)]
pub extern "system" fn RtlValidateHeap(handle: usize, _flags: u32, pointer: usize) -> u8 {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    {
        let Some(mut lease) = Lease::acquire(handle) else {
            return 0;
        };
        u8::from(pointer == 0 || lease.heap().validate(&Native(4), pointer as usize))
    }
}
