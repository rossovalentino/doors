//! ucrtbase.dll's exports: the C ABI over `door_crt`'s logic, ntdll's heap
//! and kernel32's files, environment, threads and time. Narrow strings are
//! UTF-8, the process code page (CP_ACP is UTF-8 on Doors).
mod convert;
mod env;
mod heap;
mod locale;
mod math;
mod runtime;
mod stdio;
mod string;
mod thread;
mod time;

use core::sync::atomic::{AtomicU32, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    pub(crate) fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    pub(crate) fn RtlReAllocateHeap(heap: usize, flags: u32, pointer: usize, bytes: usize)
    -> usize;
    pub(crate) fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    pub(crate) fn DoorGetProcessHeap() -> u64;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn GetCommandLineW() -> usize;
    pub(crate) fn GetEnvironmentStringsW() -> usize;
    pub(crate) fn FreeEnvironmentStringsW(block: usize) -> i32;
    pub(crate) fn SetEnvironmentVariableW(name: usize, value: usize) -> i32;
    pub(crate) fn GetStdHandle(which: u32) -> usize;
    pub(crate) fn WriteFile(
        file: usize,
        data: usize,
        bytes: u32,
        written: usize,
        overlapped: usize,
    ) -> i32;
    pub(crate) fn ReadFile(
        file: usize,
        data: usize,
        bytes: u32,
        read: usize,
        overlapped: usize,
    ) -> i32;
    pub(crate) fn CreateFileW(
        path: usize,
        access: u32,
        share: u32,
        security: usize,
        disposition: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    pub(crate) fn CloseHandle(handle: usize) -> i32;
    pub(crate) fn SetFilePointerEx(file: usize, distance: i64, out: usize, method: u32) -> i32;
    pub(crate) fn FlushFileBuffers(file: usize) -> i32;
    pub(crate) fn GetFileType(file: usize) -> u32;
    pub(crate) fn DuplicateHandle(
        source_process: usize,
        source: usize,
        target_process: usize,
        target: usize,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    pub(crate) fn DeleteFileW(path: usize) -> i32;
    pub(crate) fn MoveFileExW(from: usize, to: usize, flags: u32) -> i32;
    pub(crate) fn GetFileAttributesW(path: usize) -> u32;
    pub(crate) fn CreateDirectoryW(path: usize, security: usize) -> i32;
    pub(crate) fn GetCurrentDirectoryW(length: u32, buffer: usize) -> u32;
    pub(crate) fn CreateThread(
        security: usize,
        stack: usize,
        start: usize,
        argument: usize,
        flags: u32,
        id: usize,
    ) -> usize;
    pub(crate) fn ExitThread(code: u32) -> !;
    pub(crate) fn ExitProcess(code: u32) -> !;
    pub(crate) fn GetSystemTimeAsFileTime(out: usize);
    pub(crate) fn FlsAlloc(callback: usize) -> u32;
    pub(crate) fn FlsGetValue(index: u32) -> usize;
    pub(crate) fn FlsSetValue(index: u32, value: usize) -> i32;
    pub(crate) fn AcquireSRWLockExclusive(lock: usize);
    pub(crate) fn ReleaseSRWLockExclusive(lock: usize);
    pub(crate) fn GetConsoleMode(console: usize, mode: usize) -> i32;
    pub(crate) fn WriteConsoleW(
        console: usize,
        data: usize,
        units: u32,
        written: usize,
        reserved: usize,
    ) -> i32;
    pub(crate) fn RaiseFailFastException(record: usize, context: usize, flags: u32) -> !;
    pub(crate) fn IsDebuggerPresent() -> i32;
}

pub(crate) const CURRENT_PROCESS: usize = usize::MAX;

/// errno values the CRT sets.
pub(crate) mod errno {
    pub const ENOENT: i32 = 2;
    pub const EBADF: i32 = 9;
    pub const ENOMEM: i32 = 12;
    pub const EACCES: i32 = 13;
    pub const EEXIST: i32 = 17;
    pub const EINVAL: i32 = 22;
    pub const EMFILE: i32 = 24;
    pub const ERANGE: i32 = 34;
    pub const EILSEQ: i32 = 42;
    pub const STRUNCATE: i32 = 80;
}

/// What each thread keeps: errno, _doserrno, strtok's and wcstok's places,
/// the thread's locale choice, the numbers gmtime returns into.
#[repr(C)]
pub(crate) struct PerThread {
    pub errno: i32,
    pub doserrno: u32,
    pub strtok: usize,
    pub wcstok: usize,
    pub own_locale: i32,
    pub tm: door_crt::time::Tm,
    pub rand: u32,
}
static FLS: AtomicU32 = AtomicU32::new(u32::MAX);
unsafe extern "system" fn release_thread(block: usize) {
    if block != 0 {
        unsafe { heap::raw_free(block) };
    }
}
static FALLBACK: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// The calling thread's block, made on first use (a static one if even
/// that fails, so errno always has a home).
pub(crate) fn thread() -> &'static mut PerThread {
    let mut index = FLS.load(Ordering::Acquire);
    if index == u32::MAX {
        let fresh = unsafe { FlsAlloc(release_thread as *const () as usize) };
        index = match FLS.compare_exchange(u32::MAX, fresh, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => fresh,
            Err(existing) => existing,
        };
    }
    let mut block = unsafe { FlsGetValue(index) };
    if block == 0 {
        block = unsafe { heap::raw_alloc(core::mem::size_of::<PerThread>(), true) };
        if block == 0 || unsafe { FlsSetValue(index, block) } == 0 {
            static mut SPARE: PerThread = PerThread {
                errno: 0,
                doserrno: 0,
                strtok: 0,
                wcstok: 0,
                own_locale: 0,
                tm: door_crt::time::Tm {
                    sec: 0,
                    min: 0,
                    hour: 0,
                    mday: 0,
                    mon: 0,
                    year: 0,
                    wday: 0,
                    yday: 0,
                    isdst: 0,
                },
                rand: 1,
            };
            FALLBACK.store(1, Ordering::Relaxed);
            // SAFETY: only reached when the heap or FLS is exhausted.
            return unsafe { &mut *core::ptr::addr_of_mut!(SPARE) };
        }
        unsafe { (*(block as *mut PerThread)).rand = 1 };
    }
    // SAFETY: this thread's own zeroed block.
    unsafe { &mut *(block as *mut PerThread) }
}
pub(crate) fn set_errno(value: i32) {
    thread().errno = value;
}

#[unsafe(no_mangle)]
pub extern "C" fn _errno() -> *mut i32 {
    &mut thread().errno
}
#[unsafe(no_mangle)]
pub extern "C" fn __doserrno() -> *mut u32 {
    &mut thread().doserrno
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_errno(out: *mut i32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = thread().errno };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_errno(value: i32) -> i32 {
    set_errno(value);
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_doserrno(out: *mut u32) -> i32 {
    if out.is_null() {
        return errno::EINVAL;
    }
    unsafe { *out = thread().doserrno };
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _set_doserrno(value: u32) -> i32 {
    thread().doserrno = value;
    0
}
/// errno from a Windows error (the CRT's `__acrt_errno_map_os_error`).
pub(crate) fn map_os_error(code: u32) {
    let t = thread();
    t.doserrno = code;
    t.errno = match code {
        2 | 3 | 15 | 18 | 53 | 67 | 123 | 161 | 206 => errno::ENOENT,
        4 => errno::EMFILE,
        5 | 19..=36 | 65 | 82 | 1224 => errno::EACCES,
        6 => errno::EBADF,
        8 | 14 => errno::ENOMEM,
        80 | 183 => errno::EEXIST,
        _ => errno::EINVAL,
    };
}

/// Units of a NUL-terminated narrow or wide string at `p` (no bound).
pub(crate) unsafe fn length<T: Copy + PartialEq + Default>(p: *const T) -> usize {
    let mut n = 0;
    while unsafe { *p.add(n) } != T::default() {
        n += 1;
    }
    n
}

/// Invalid-parameter handling (`_invalid_parameter_noinfo`): the handler
/// the program set, or fail fast as the CRT's default does.
pub(crate) fn invalid_parameter() {
    runtime::invalid_parameter_handler();
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { ExitProcess(0xfffffed0) }
}
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
extern "system" fn __CxxFrameHandler3() -> ! {
    unsafe { ExitProcess(0xfffffed2) }
}
#[unsafe(no_mangle)]
static _fltused: i32 = 0;
/// Units of a NUL-terminated string of any unit type.
pub(crate) unsafe fn length_of_units<T: Copy + Into<u32>>(p: *const T) -> usize {
    let mut n = 0;
    while unsafe { (*p.add(n)).into() } != 0 {
        n += 1;
    }
    n
}
