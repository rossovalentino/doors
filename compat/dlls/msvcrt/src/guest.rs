use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
#[unsafe(no_mangle)]
static _fltused: i32 = 0;
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorVirtualAlloc(address: u64, bytes: u64, flags: u32, protection: u32) -> u64;
    fn DoorVirtualFree(address: u64, bytes: u64, flags: u32) -> i32;
    fn RtlExitUserProcess(code: u32) -> !;
    fn DoorSleep(millis: u32);
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlReAllocateHeap(heap: usize, flags: u32, pointer: usize, bytes: usize) -> usize;
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "C" fn _errno() -> *mut i32 {
    let base: u64;
    unsafe {
        core::arch::asm!("mov {}, fs:[0]",out(reg) base,options(nostack,readonly,preserves_flags));
    }
    (base + 528) as *mut i32
}
/// The 32-bit build (P17): errno in the thread's TEB32 (fs:[0x18] is its
/// address; the field is in Doors' private range of it).
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "C" fn _errno() -> *mut i32 {
    let teb: u32;
    unsafe {
        core::arch::asm!("mov {}, fs:[0x18]",out(reg) teb,options(nostack,readonly,preserves_flags));
    }
    (teb as usize + 0x3F4) as *mut i32
}
fn error(code: i32) {
    unsafe {
        _errno().write(code);
    }
}
#[derive(Clone, Copy)]
struct Allocation {
    address: u64,
    requested: usize,
}
struct Heap {
    locked: AtomicBool,
    slots: UnsafeCell<[Allocation; 8]>,
}
unsafe impl Sync for Heap {}
static HEAP: Heap = Heap {
    locked: AtomicBool::new(false),
    slots: UnsafeCell::new(
        [Allocation {
            address: 0,
            requested: 0,
        }; 8],
    ),
};
struct Guard;
impl Guard {
    fn acquire() -> Self {
        let mut attempts = 0;
        while HEAP
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            attempts += 1;
            if attempts >= 100 {
                unsafe {
                    RtlExitUserProcess(0xfffffecf);
                }
            }
            unsafe {
                DoorSleep(1);
            }
        }
        Self
    }
    fn slots(&mut self) -> &mut [Allocation; 8] {
        unsafe { &mut *HEAP.slots.get() }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        HEAP.locked.store(false, Ordering::Release);
    }
}
fn allocate(slots: &mut [Allocation; 8], bytes: usize) -> *mut u8 {
    let Some(slot) = slots.iter_mut().find(|slot| slot.address == 0) else {
        error(12);
        return core::ptr::null_mut();
    };
    let Some(rounded) = bytes
        .max(1)
        .checked_add(4095)
        .map(|v| v & !4095)
        .filter(|v| *v <= 1024 * 1024)
    else {
        error(12);
        return core::ptr::null_mut();
    };
    let address = unsafe { DoorVirtualAlloc(0, rounded as u64, 0x3000, 4) };
    if address == 0 {
        error(12);
        return core::ptr::null_mut();
    }
    *slot = Allocation {
        address,
        requested: bytes,
    };
    address as *mut u8
}
/// The process heap (PEB.ProcessHeap), where a Windows application's
/// msvcrt allocates; 0 in a process without one, which keeps the scoped
/// page allocator below.
fn process_heap() -> usize {
    unsafe { DoorGetProcessHeap() as usize }
}
const HEAP_ZERO_MEMORY: u32 = 8;
#[unsafe(no_mangle)]
pub extern "C" fn malloc(bytes: usize) -> *mut u8 {
    let heap = process_heap();
    if heap != 0 {
        let pointer = unsafe { RtlAllocateHeap(heap, 0, bytes.max(1)) };
        if pointer == 0 {
            error(12);
        }
        return pointer as *mut u8;
    }
    allocate(Guard::acquire().slots(), bytes)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(pointer: *mut u8) {
    if pointer.is_null() {
        return;
    }
    let heap = process_heap();
    if heap != 0 {
        if unsafe { RtlFreeHeap(heap, 0, pointer as usize) } == 0 {
            error(22);
        }
        return;
    }
    let mut guard = Guard::acquire();
    let Some(slot) = guard
        .slots()
        .iter_mut()
        .find(|s| s.address == pointer as u64)
    else {
        error(22);
        return;
    };
    if unsafe { DoorVirtualFree(slot.address, 0, 0x8000) } == 0 {
        error(22);
        return;
    }
    *slot = Allocation {
        address: 0,
        requested: 0,
    };
}
#[unsafe(no_mangle)]
pub extern "C" fn calloc(count: usize, bytes: usize) -> *mut u8 {
    let Some(size) = count.checked_mul(bytes) else {
        error(12);
        return core::ptr::null_mut();
    };
    let heap = process_heap();
    if heap != 0 {
        let pointer = unsafe { RtlAllocateHeap(heap, HEAP_ZERO_MEMORY, size.max(1)) };
        if pointer == 0 {
            error(12);
        }
        return pointer as *mut u8;
    }
    malloc(size) // native private commitment is zero-filled
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(pointer: *mut u8, bytes: usize) -> *mut u8 {
    if pointer.is_null() {
        return malloc(bytes);
    }
    if bytes == 0 {
        unsafe {
            free(pointer);
        }
        return core::ptr::null_mut();
    }
    let heap = process_heap();
    if heap != 0 {
        // A failed reallocation keeps the block, as Windows' does.
        let next = unsafe { RtlReAllocateHeap(heap, 0, pointer as usize, bytes) };
        if next == 0 {
            error(12);
        }
        return next as *mut u8;
    }
    let mut guard = Guard::acquire();
    let slots = guard.slots();
    let Some(index) = slots.iter().position(|s| s.address == pointer as u64) else {
        error(22);
        return core::ptr::null_mut();
    };
    let before = slots[index];
    let next = allocate(slots, bytes);
    if next.is_null() {
        return next;
    }
    unsafe {
        memcpy(next, pointer, before.requested.min(bytes));
    }
    if unsafe { DoorVirtualFree(before.address, 0, 0x8000) } == 0 {
        if unsafe { DoorVirtualFree(next as u64, 0, 0x8000) } != 0
            && let Some(slot) = slots.iter_mut().find(|slot| slot.address == next as u64)
        {
            *slot = Allocation {
                address: 0,
                requested: 0,
            };
        }
        error(22);
        return core::ptr::null_mut();
    }
    slots[index] = Allocation {
        address: 0,
        requested: 0,
    };
    next
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    for index in 0..bytes {
        unsafe {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
    destination
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memmove(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    if (destination as usize) > source as usize {
        for index in (0..bytes).rev() {
            unsafe {
                destination
                    .add(index)
                    .write_volatile(source.add(index).read_volatile());
            }
        }
    } else {
        unsafe {
            memcpy(destination, source, bytes);
        }
    }
    destination
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memset(destination: *mut u8, value: i32, bytes: usize) -> *mut u8 {
    for index in 0..bytes {
        unsafe {
            destination.add(index).write_volatile(value as u8);
        }
    }
    destination
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, bytes: usize) -> i32 {
    for index in 0..bytes {
        let (a, b) = unsafe {
            (
                left.add(index).read_volatile(),
                right.add(index).read_volatile(),
            )
        };
        if a != b {
            return i32::from(a) - i32::from(b);
        }
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strlen(value: *const u8) -> usize {
    let mut len = 0;
    while unsafe { value.add(len).read_volatile() } != 0 {
        len += 1;
    }
    len
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(value: *const u16) -> usize {
    let mut len = 0;
    while unsafe { value.add(len).read_volatile() } != 0 {
        len += 1;
    }
    len
}
#[unsafe(no_mangle)]
pub extern "C" fn abs(value: i32) -> i32 {
    value.wrapping_abs()
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { RtlExitUserProcess(0xfffffed0) }
}
/// The personality routine i686 code references for unwinding frames: it
/// never runs (panic = abort) and ends the process like a panic, as the
/// other 32-bit DLLs' does.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
extern "system" fn __CxxFrameHandler3() -> ! {
    unsafe { RtlExitUserProcess(0xfffffed2) }
}
