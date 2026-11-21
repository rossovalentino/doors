//! Process and thread queries .NET 11 and PowerShell make (from their
//! P/Invoke inventory): handles to this process and its threads,
//! priority classes and boosts, working-set sizes, the session, the image
//! name, memory of this process, power requests, the power status and
//! process-thread attribute lists. Doors schedules by its own priorities
//! and pages on demand: the Windows settings are kept and read back.
//! Other processes are reachable through the handles CreateProcess gave
//! (Get/SetPriorityClass and friends), not by ID yet (M0k).
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use door_windows_abi as abi;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorOpenCurrentProcess() -> u64;
    fn DoorOpenThreadById(id: u32) -> u64;
    fn DoorGetCurrentProcessId() -> u32;
    fn DoorGetProcessId(handle: u64) -> u32;
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
    fn DoorCreateEventW(security: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn RtlSetLastWin32Error(value: u32);
    fn NtWriteVirtualMemory(
        process: usize,
        destination: usize,
        source: usize,
        bytes: usize,
        written: usize,
    ) -> u32;
}

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const NORMAL_PRIORITY_CLASS: u32 = 0x20;

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn write<T: Copy>(pointer: usize, value: T) {
    if pointer != 0 {
        // SAFETY: the caller's out parameter of type T.
        unsafe { (pointer as *mut T).write_unaligned(value) };
    }
}
/// Whether `process` names this process (its pseudo-handle or a handle).
fn own(process: usize) -> bool {
    crate::system::current(abi::widen(process))
}

// ---- handles -------------------------------------------------------------------------

/// `OpenProcess`: a real handle to this process by its ID; another
/// process's ID cannot be opened yet (ERROR_ACCESS_DENIED, as for a
/// protected process).
#[unsafe(no_mangle)]
pub extern "system" fn OpenProcess(_access: u32, _inherit: i32, id: u32) -> usize {
    if id == 0 {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if id != unsafe { DoorGetCurrentProcessId() } {
        fail(ERROR_ACCESS_DENIED);
        return 0;
    }
    abi::narrow(unsafe { DoorOpenCurrentProcess() })
}
/// `OpenThread`: a thread of this process by its ID.
#[unsafe(no_mangle)]
pub extern "system" fn OpenThread(_access: u32, _inherit: i32, id: u32) -> usize {
    abi::narrow(unsafe { DoorOpenThreadById(id) })
}
/// `QueryFullProcessImageNameW`: this process's image (Win32 form; the
/// native form, flag 1, names the same file).
#[unsafe(no_mangle)]
pub extern "system" fn QueryFullProcessImageNameW(
    process: usize,
    _flags: u32,
    buffer: usize,
    size: usize,
) -> i32 {
    if !own(process) {
        return fail(ERROR_ACCESS_DENIED);
    }
    if buffer == 0 || size == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let capacity = unsafe { (size as *const u32).read_unaligned() };
    let n = unsafe { DoorGetModuleFileNameW(0, abi::widen(buffer), capacity) };
    if n == 0 || n >= capacity {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    write(size, n);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn ProcessIdToSessionId(id: u32, session: usize) -> i32 {
    if session == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // Every process of this desktop runs in its session (the PEB's).
    let _ = id;
    #[cfg(target_pointer_width = "64")]
    let at = abi::PEB_ADDRESS + 704;
    #[cfg(not(target_pointer_width = "64"))]
    let at = abi::PEB_ADDRESS + abi::x86::PEB_SESSION_ID as u64;
    let value = unsafe { (at as usize as *const u32).read_unaligned() };
    write(session, value);
    1
}

// ---- priorities and the working set ------------------------------------------------------

static PRIORITY_CLASS: AtomicU32 = AtomicU32::new(NORMAL_PRIORITY_CLASS);
static PROCESS_BOOST_DISABLED: AtomicU32 = AtomicU32::new(0);
static WORKING_SET: [AtomicUsize; 2] = [AtomicUsize::new(204_800), AtomicUsize::new(1_413_120)];

#[unsafe(no_mangle)]
pub extern "system" fn GetPriorityClass(process: usize) -> u32 {
    if own(process) {
        return PRIORITY_CLASS.load(Ordering::Relaxed);
    }
    if unsafe { DoorGetProcessId(abi::widen(process)) } != 0 {
        return NORMAL_PRIORITY_CLASS;
    }
    fail(ERROR_INVALID_HANDLE);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn SetPriorityClass(process: usize, class: u32) -> i32 {
    // IDLE, BELOW_NORMAL, NORMAL, ABOVE_NORMAL, HIGH, REALTIME and the
    // background-mode switches.
    if !matches!(
        class,
        0x40 | 0x4000 | 0x20 | 0x8000 | 0x80 | 0x100 | 0x0010_0000 | 0x0020_0000
    ) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if own(process) {
        if class & 0x0030_0000 == 0 {
            PRIORITY_CLASS.store(class, Ordering::Relaxed);
        }
        return 1;
    }
    if unsafe { DoorGetProcessId(abi::widen(process)) } != 0 {
        return 1;
    }
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessPriorityBoost(_process: usize, disabled: usize) -> i32 {
    if disabled == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    write(
        disabled,
        PROCESS_BOOST_DISABLED.load(Ordering::Relaxed) as i32,
    );
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessPriorityBoost(process: usize, disable: i32) -> i32 {
    if own(process) {
        PROCESS_BOOST_DISABLED.store(u32::from(disable != 0), Ordering::Relaxed);
    }
    1
}
/// Priority boosts: Doors' scheduler raises waking threads on its own; the
/// per-thread switch is accepted and reads back unset.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriorityBoost(thread: usize, _disable: i32) -> i32 {
    if thread == 0 {
        return fail(ERROR_INVALID_HANDLE);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadPriorityBoost(thread: usize, disabled: usize) -> i32 {
    if thread == 0 || disabled == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    write(disabled, 0i32);
    1
}
/// Working-set bounds: Windows' defaults (200 KiB, 1380 KiB), kept as set.
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessWorkingSetSizeEx(
    process: usize,
    minimum: usize,
    maximum: usize,
    flags: usize,
) -> i32 {
    if !own(process) && unsafe { DoorGetProcessId(abi::widen(process)) } == 0 {
        return fail(ERROR_INVALID_HANDLE);
    }
    write(minimum, WORKING_SET[0].load(Ordering::Relaxed));
    write(maximum, WORKING_SET[1].load(Ordering::Relaxed));
    // QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE.
    write(flags, 0x2u32 | 0x8);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessWorkingSetSize(
    process: usize,
    minimum: usize,
    maximum: usize,
) -> i32 {
    GetProcessWorkingSetSizeEx(process, minimum, maximum, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessWorkingSetSizeEx(
    process: usize,
    minimum: usize,
    maximum: usize,
    _flags: u32,
) -> i32 {
    if minimum == usize::MAX && maximum == usize::MAX {
        // Trim the working set: nothing is held beyond what is used.
        return 1;
    }
    if minimum > maximum {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if own(process) {
        WORKING_SET[0].store(minimum, Ordering::Relaxed);
        WORKING_SET[1].store(maximum, Ordering::Relaxed);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessWorkingSetSize(
    process: usize,
    minimum: usize,
    maximum: usize,
) -> i32 {
    SetProcessWorkingSetSizeEx(process, minimum, maximum, 0)
}

// ---- memory of a process -------------------------------------------------------------

/// `ReadProcessMemory` / `WriteProcessMemory` of this process: a checked
/// copy (another process's memory is not reachable yet).
fn copy_own(process: usize, from: usize, to: usize, bytes: usize, done: usize) -> i32 {
    if !own(process) {
        return fail(ERROR_ACCESS_DENIED);
    }
    if bytes != 0 && (from == 0 || to == 0) {
        return fail(998);
    }
    // Checked copies of at most a page at a time.
    let mut moved = 0;
    while moved < bytes {
        let n = (bytes - moved).min(1024);
        let status = unsafe {
            NtWriteVirtualMemory(
                abi::narrow(abi::CURRENT_PROCESS),
                to + moved,
                from + moved,
                n,
                0,
            )
        };
        if status != 0 {
            write(done, moved);
            return fail(299);
        }
        moved += n;
    }
    write(done, bytes);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn ReadProcessMemory(
    process: usize,
    address: usize,
    buffer: usize,
    bytes: usize,
    read: usize,
) -> i32 {
    copy_own(process, address, buffer, bytes, read)
}
#[unsafe(no_mangle)]
pub extern "system" fn WriteProcessMemory(
    process: usize,
    address: usize,
    buffer: usize,
    bytes: usize,
    written: usize,
) -> i32 {
    copy_own(process, buffer, address, bytes, written)
}

// ---- power -------------------------------------------------------------------------------

/// SYSTEM_POWER_STATUS: on AC power, no battery (128: no system battery),
/// as a desktop PC reports.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemPowerStatus(status: usize) -> i32 {
    if status == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut out = [0u8; 12];
    out[0] = 1;
    out[1] = 128;
    out[2] = 255;
    out[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    out[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    write(status, out);
    1
}
/// Power requests (keep the display or system awake): a handle that sets
/// and clears requests; Doors' power policy has no sleep to hold off yet.
#[unsafe(no_mangle)]
pub extern "system" fn PowerCreateRequest(_context: usize) -> usize {
    abi::narrow(unsafe { DoorCreateEventW(0, 1, 0, 0) })
}
#[unsafe(no_mangle)]
pub extern "system" fn PowerSetRequest(request: usize, _kind: u32) -> i32 {
    if request == 0 || request == usize::MAX {
        return fail(ERROR_INVALID_HANDLE);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn PowerClearRequest(request: usize, kind: u32) -> i32 {
    PowerSetRequest(request, kind)
}

// ---- process-thread attribute lists --------------------------------------------------------

/// The list's size for `count` attributes: a header and three pointers
/// (attribute, size, value) per entry.
fn list_bytes(count: u32) -> usize {
    let pointer = core::mem::size_of::<usize>();
    2 * pointer + count as usize * 3 * pointer
}
/// `InitializeProcThreadAttributeList`: the size query (ERROR_INSUFFICIENT_BUFFER
/// with the size), then the list. The attributes CreateProcess honours
/// (the handle list, the parent) are recorded; the others are accepted.
#[unsafe(no_mangle)]
pub extern "system" fn InitializeProcThreadAttributeList(
    list: usize,
    count: u32,
    _flags: u32,
    size: usize,
) -> i32 {
    if size == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let needed = list_bytes(count);
    let given = unsafe { (size as *const usize).read_unaligned() };
    write(size, needed);
    if list == 0 || given < needed {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    unsafe { core::ptr::write_bytes(list as *mut u8, 0, needed) };
    write(list, count as usize);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn UpdateProcThreadAttribute(
    list: usize,
    _flags: u32,
    attribute: usize,
    value: usize,
    bytes: usize,
    _previous: usize,
    _returned: usize,
) -> i32 {
    if list == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let pointer = core::mem::size_of::<usize>();
    let capacity = unsafe { (list as *const usize).read_unaligned() };
    let used = unsafe { ((list + pointer) as *const usize).read_unaligned() };
    if used >= capacity {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    let entry = list + 2 * pointer + used * 3 * pointer;
    write(entry, attribute);
    write(entry + pointer, bytes);
    write(entry + 2 * pointer, value);
    write(list + pointer, used + 1);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteProcThreadAttributeList(_list: usize) {}
