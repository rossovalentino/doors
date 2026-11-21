//! dbghelp exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}
fn fail<T: Default>(code: u32) -> T {
    unsafe { SetLastError(code) };
    T::default()
}
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

static INITIALIZED: AtomicBool = AtomicBool::new(false);
/// SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS, dbghelp's default.
static OPTIONS: AtomicU32 = AtomicU32::new(0x2 | 0x4);
/// The search path (at most 1024 units) SymSetSearchPathW kept.
static mut SEARCH: [u16; 1024] = [0; 1024];
static SEARCH_LENGTH: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub extern "system" fn SymInitialize(_process: usize, path: usize, _invade: i32) -> i32 {
    INITIALIZED.store(true, Ordering::Release);
    if path != 0 {
        SymSetSearchPathW(_process, path);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SymInitializeW(process: usize, path: usize, invade: i32) -> i32 {
    SymInitialize(process, path, invade)
}
#[unsafe(no_mangle)]
pub extern "system" fn SymCleanup(_process: usize) -> i32 {
    INITIALIZED.store(false, Ordering::Release);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SymSetOptions(options: u32) -> u32 {
    OPTIONS.store(options, Ordering::Relaxed);
    options
}
#[unsafe(no_mangle)]
pub extern "system" fn SymGetOptions() -> u32 {
    OPTIONS.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "system" fn SymSetSearchPathW(_process: usize, path: usize) -> i32 {
    let mut n = 0usize;
    // SAFETY: the caller's string, copied up to the buffer's size; dbghelp
    // is single-threaded by contract (callers serialize its calls).
    unsafe {
        let search = &mut *core::ptr::addr_of_mut!(SEARCH);
        if path != 0 {
            while n < search.len() - 1 {
                let u = (path as *const u16).add(n).read_unaligned();
                if u == 0 {
                    break;
                }
                search[n] = u;
                n += 1;
            }
        }
        search[n] = 0;
    }
    SEARCH_LENGTH.store(n as u32, Ordering::Release);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SymGetSearchPathW(_process: usize, out: usize, length: u32) -> i32 {
    let n = SEARCH_LENGTH.load(Ordering::Acquire) as usize;
    if out == 0 || (length as usize) <= n {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    // SAFETY: the caller's buffer of `length` units.
    unsafe {
        let search = &*core::ptr::addr_of!(SEARCH);
        core::ptr::copy_nonoverlapping(search.as_ptr(), out as *mut u16, n + 1);
    }
    1
}
/// No symbol files: every lookup misses.
macro_rules! no_symbols {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub extern "system" fn $name($($arg: $ty),*) -> i32 {
            fail(ERROR_MOD_NOT_FOUND)
        }
    )*};
}
no_symbols! {
    SymFromAddr(_process: usize, _address: u64, _displacement: usize, _symbol: usize);
    SymFromAddrW(_process: usize, _address: u64, _displacement: usize, _symbol: usize);
    SymGetLineFromAddr64(_process: usize, _address: u64, _displacement: usize, _line: usize);
    SymGetLineFromAddrW64(_process: usize, _address: u64, _displacement: usize, _line: usize);
    SymGetModuleInfo64(_process: usize, _address: u64, _info: usize);
    SymGetModuleInfoW64(_process: usize, _address: u64, _info: usize);
    SymSrvGetFileIndexInfo(_file: usize, _info: usize, _flags: u32);
    SymSrvGetFileIndexInfoW(_file: usize, _info: usize, _flags: u32);
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn VirtualQuery(address: usize, info: usize, length: usize) -> usize;
}
#[cfg(target_pointer_width = "64")]
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlLookupFunctionEntry(pc: u64, base: usize, history: usize) -> usize;
}
/// SymGetModuleBase64: the base of the image holding `address` (its
/// allocation, when the region is MEM_IMAGE), 0 elsewhere.
#[unsafe(no_mangle)]
pub extern "system" fn SymGetModuleBase64(_process: usize, address: u64) -> u64 {
    const POINTER: usize = core::mem::size_of::<usize>();
    let mut info = [0usize; 8];
    let n = unsafe {
        VirtualQuery(
            address as usize,
            info.as_mut_ptr() as usize,
            core::mem::size_of_val(&info),
        )
    };
    // MEMORY_BASIC_INFORMATION.Type: offset 40 on x64, 24 on x86.
    let kind_at = if POINTER == 8 { 40 } else { 24 };
    let bytes = info.as_ptr() as *const u8;
    let kind = unsafe { (bytes.add(kind_at) as *const u32).read_unaligned() };
    if n == 0 || kind != 0x0100_0000 {
        return 0;
    }
    info[1] as u64
}
/// SymFunctionTableAccess64: the unwind entry (x64) for `address`.
#[unsafe(no_mangle)]
pub extern "system" fn SymFunctionTableAccess64(_process: usize, address: u64) -> usize {
    #[cfg(target_pointer_width = "64")]
    {
        let mut base = 0u64;
        unsafe { RtlLookupFunctionEntry(address, &mut base as *mut u64 as usize, 0) }
    }
    #[cfg(not(target_pointer_width = "64"))]
    {
        let _ = address;
        0
    }
}
/// StackWalk64: Doors' unwinder is RtlVirtualUnwind and
/// RtlCaptureStackBackTrace; this walker reports the end of the stack.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn StackWalk64(
    _machine: u32,
    _process: usize,
    _thread: usize,
    _frame: usize,
    _context: usize,
    _read: usize,
    _table: usize,
    _base: usize,
    _translate: usize,
) -> i32 {
    0
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn MiniDumpWriteDump(
    _process: usize,
    _id: u32,
    _file: usize,
    _kind: u32,
    _exception: usize,
    _stream: usize,
    _callback: usize,
) -> i32 {
    fail(ERROR_NOT_SUPPORTED)
}
/// UnDecorateSymbolName: the name unchanged (Doors' modules export
/// undecorated names), truncated to the buffer.
#[unsafe(no_mangle)]
pub extern "system" fn UnDecorateSymbolName(
    name: usize,
    out: usize,
    length: u32,
    _flags: u32,
) -> u32 {
    if name == 0 || out == 0 || length == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut n = 0usize;
    // SAFETY: the caller's ANSI name and buffer of `length` bytes.
    unsafe {
        while n + 1 < length as usize {
            let b = (name as *const u8).add(n).read();
            if b == 0 {
                break;
            }
            (out as *mut u8).add(n).write(b);
            n += 1;
        }
        (out as *mut u8).add(n).write(0);
    }
    n as u32
}
