//! Process, module and thread information Chromium and Electron read at
//! start-up (P18 M7, from their import tables): Toolhelp snapshots, the
//! PSAPI (K32*) module and memory queries, the process-information and
//! mitigation policies, thread stack limits and ids, product information,
//! and the heap and debugger queries whose Windows answer here is fixed.
//!
//! Doors gives a program no view of other processes yet: snapshots and
//! module lists cover the calling process, and a query naming another
//! process fails with ERROR_ACCESS_DENIED, as for a protected process.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorEnumModules(out: u64, capacity: u32) -> u32;
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
    fn DoorGetCurrentProcessId() -> u32;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorGetProcessId(handle: u64) -> u32;
    fn DoorGetThreadId(handle: u64) -> u32;
    fn DoorGetProcessHeap() -> u64;
    fn DoorCreateEventW(security: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn RtlSetLastWin32Error(value: u32);
}

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_BAD_LENGTH: u32 = 24;
const WIDE: bool = cfg!(target_arch = "x86_64");
const POINTER: usize = core::mem::size_of::<usize>();

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn put<T: Copy>(pointer: u64, value: T) {
    if pointer != 0 {
        // SAFETY: the caller's out parameter of type T.
        unsafe { (pointer as *mut T).write_unaligned(value) };
    }
}
fn put_bytes(pointer: u64, bytes: &[u8]) {
    // SAFETY: the caller's structure of at least `bytes.len()` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer as *mut u8, bytes.len()) };
}
fn get<T: Copy>(pointer: u64) -> T {
    // SAFETY: the caller's in parameter of type T.
    unsafe { (pointer as *const T).read_unaligned() }
}
fn put_pointer(out: &mut [u8], at: usize, value: u64) {
    out[at..at + POINTER].copy_from_slice(&value.to_le_bytes()[..POINTER]);
}
/// The calling process, by handle (the pseudo-handle or a handle to it).
fn is_self(process: u64) -> bool {
    process == u64::MAX
        || process == u64::from(u32::MAX)
        || unsafe { DoorGetProcessId(process) == DoorGetCurrentProcessId() }
}
/// The process's modules: (base, size) pairs, the executable first.
fn modules(out: &mut [(u64, u64); 256]) -> usize {
    let n = unsafe { DoorEnumModules(out.as_mut_ptr() as u64, out.len() as u32) } as usize;
    n.min(out.len())
}
/// The module's path (wide, without NUL) into `out`: its length.
fn module_path(base: u64, out: &mut [u16; 1024]) -> usize {
    unsafe { DoorGetModuleFileNameW(base, out.as_mut_ptr() as u64, 1024) as usize }.min(1023)
}
fn file_part(path: &[u16]) -> &[u16] {
    let start = path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i + 1);
    &path[start..]
}
fn copy_name(out: &mut [u8], at: usize, units: usize, name: &[u16]) {
    let n = name.len().min(units - 1);
    for (i, u) in name[..n].iter().enumerate() {
        out[at + 2 * i..at + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    out[at + 2 * n..at + 2 * n + 2].fill(0);
}

// ---- Toolhelp ---------------------------------------------------------------------

const TH32CS_SNAPPROCESS: u32 = 2;
const TH32CS_SNAPTHREAD: u32 = 4;
const TH32CS_SNAPMODULE: u32 = 8;
const TH32CS_SNAPMODULE32: u32 = 0x10;
const SNAPSHOTS: usize = 16;
#[derive(Clone, Copy)]
struct Snapshot {
    handle: u64,
    flags: u32,
    pid: u32,
    thread: u32,
    modules: [(u64, u64); 256],
    module_count: usize,
    /// Next entry to return, per list (process, thread, module).
    cursor: [usize; 3],
}
struct Snapshots {
    held: AtomicBool,
    list: UnsafeCell<[Option<Snapshot>; SNAPSHOTS]>,
}
unsafe impl Sync for Snapshots {}
static SNAPSHOT_TABLE: Snapshots = Snapshots {
    held: AtomicBool::new(false),
    list: UnsafeCell::new([None; SNAPSHOTS]),
};
fn with_snapshots<R>(f: impl FnOnce(&mut [Option<Snapshot>; SNAPSHOTS]) -> R) -> R {
    while SNAPSHOT_TABLE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *SNAPSHOT_TABLE.list.get() });
    SNAPSHOT_TABLE.held.store(false, Ordering::Release);
    r
}
/// CloseHandle of a snapshot: its record goes (the handle is an event).
pub(crate) fn forget(handle: u64) {
    with_snapshots(|list| {
        for slot in list.iter_mut() {
            if slot.is_some_and(|s| s.handle == handle) {
                *slot = None;
            }
        }
    });
}
/// CreateToolhelp32Snapshot: the calling process (and its modules and the
/// calling thread) as Toolhelp lists them.
#[unsafe(no_mangle)]
pub extern "system" fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> usize {
    let me = unsafe { DoorGetCurrentProcessId() };
    if flags & (TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32) != 0 && pid != 0 && pid != me {
        fail(ERROR_ACCESS_DENIED);
        return usize::MAX;
    }
    let handle = unsafe { DoorCreateEventW(0, 1, 1, 0) };
    if handle == 0 {
        return usize::MAX;
    }
    let mut snapshot = Snapshot {
        handle,
        flags,
        pid: me,
        thread: unsafe { DoorGetCurrentThreadId() },
        modules: [(0, 0); 256],
        module_count: 0,
        cursor: [0; 3],
    };
    if flags & (TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32) != 0 {
        snapshot.module_count = modules(&mut snapshot.modules);
    }
    let stored = with_snapshots(|list| {
        list.iter_mut().find(|s| s.is_none()).map(|slot| {
            *slot = Some(snapshot);
        })
    });
    if stored.is_none() {
        crate::guest::CloseHandle(handle as usize);
        fail(8);
        return usize::MAX;
    }
    handle as usize
}
/// The next entry of list `which` (0 processes, 1 threads, 2 modules), or
/// None; `first` rewinds.
fn next(handle: u64, which: usize, first: bool) -> Result<Option<Snapshot>, u32> {
    with_snapshots(|list| {
        let s = list
            .iter_mut()
            .flatten()
            .find(|s| s.handle == handle)
            .ok_or(ERROR_INVALID_HANDLE)?;
        if first {
            s.cursor[which] = 0;
        }
        let count = match which {
            0 => usize::from(s.flags & TH32CS_SNAPPROCESS != 0),
            1 => usize::from(s.flags & TH32CS_SNAPTHREAD != 0),
            _ => s.module_count,
        };
        if s.cursor[which] >= count {
            return Ok(None);
        }
        let current = *s;
        s.cursor[which] += 1;
        Ok(Some(current))
    })
}
fn process_entry(handle: usize, entry: usize, first: bool) -> i32 {
    let entry = door_windows_abi::widen(entry);
    let size = if WIDE { 568 } else { 556 };
    if entry == 0 || get::<u32>(entry) < size {
        return fail(ERROR_BAD_LENGTH);
    }
    let s = match next(door_windows_abi::widen(handle), 0, first) {
        Ok(Some(s)) => s,
        Ok(None) => return fail(ERROR_NO_MORE_FILES),
        Err(code) => return fail(code),
    };
    let mut out = [0u8; 568];
    out[..4].copy_from_slice(&size.to_le_bytes());
    out[8..12].copy_from_slice(&s.pid.to_le_bytes());
    let (threads, parent, priority, name) = if WIDE {
        (28, 32, 36, 44)
    } else {
        (20, 24, 28, 36)
    };
    out[threads..threads + 4].copy_from_slice(&1u32.to_le_bytes());
    out[parent..parent + 4].copy_from_slice(&0u32.to_le_bytes());
    out[priority..priority + 4].copy_from_slice(&8i32.to_le_bytes());
    let mut path = [0u16; 1024];
    let n = module_path(0, &mut path);
    copy_name(&mut out, name, 260, file_part(&path[..n]));
    put_bytes(entry, &out[..size as usize]);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn Process32FirstW(handle: usize, entry: usize) -> i32 {
    process_entry(handle, entry, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn Process32NextW(handle: usize, entry: usize) -> i32 {
    process_entry(handle, entry, false)
}
fn thread_entry(handle: usize, entry: usize, first: bool) -> i32 {
    let entry = door_windows_abi::widen(entry);
    if entry == 0 || get::<u32>(entry) < 28 {
        return fail(ERROR_BAD_LENGTH);
    }
    let s = match next(door_windows_abi::widen(handle), 1, first) {
        Ok(Some(s)) => s,
        Ok(None) => return fail(ERROR_NO_MORE_FILES),
        Err(code) => return fail(code),
    };
    let mut out = [0u8; 28];
    out[..4].copy_from_slice(&28u32.to_le_bytes());
    out[8..12].copy_from_slice(&s.thread.to_le_bytes());
    out[12..16].copy_from_slice(&s.pid.to_le_bytes());
    out[16..20].copy_from_slice(&8i32.to_le_bytes());
    put_bytes(entry, &out);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn Thread32First(handle: usize, entry: usize) -> i32 {
    thread_entry(handle, entry, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn Thread32Next(handle: usize, entry: usize) -> i32 {
    thread_entry(handle, entry, false)
}
fn module_entry(handle: usize, entry: usize, first: bool) -> i32 {
    let entry = door_windows_abi::widen(entry);
    let size: u32 = if WIDE { 1080 } else { 1064 };
    if entry == 0 || get::<u32>(entry) < size {
        return fail(ERROR_BAD_LENGTH);
    }
    let handle = door_windows_abi::widen(handle);
    let index = with_snapshots(|list| {
        list.iter()
            .flatten()
            .find(|s| s.handle == handle)
            .map(|s| if first { 0 } else { s.cursor[2] })
    });
    let s = match next(handle, 2, first) {
        Ok(Some(s)) => s,
        Ok(None) => return fail(ERROR_NO_MORE_FILES),
        Err(code) => return fail(code),
    };
    let (base, bytes) = s.modules[index.unwrap_or(0)];
    let mut out = [0u8; 1080];
    out[..4].copy_from_slice(&size.to_le_bytes());
    out[4..8].copy_from_slice(&1u32.to_le_bytes());
    out[8..12].copy_from_slice(&s.pid.to_le_bytes());
    out[12..16].copy_from_slice(&1u32.to_le_bytes());
    out[16..20].copy_from_slice(&1u32.to_le_bytes());
    let (address, length, module, name, path_at) = if WIDE {
        (24, 32, 40, 48, 560)
    } else {
        (20, 24, 28, 32, 544)
    };
    put_pointer(&mut out, address, base);
    out[length..length + 4].copy_from_slice(&(bytes as u32).to_le_bytes());
    put_pointer(&mut out, module, base);
    let mut path = [0u16; 1024];
    let n = module_path(base, &mut path);
    copy_name(&mut out, name, 256, file_part(&path[..n]));
    copy_name(&mut out, path_at, 260, &path[..n]);
    put_bytes(entry, &out[..size as usize]);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn Module32FirstW(handle: usize, entry: usize) -> i32 {
    module_entry(handle, entry, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn Module32NextW(handle: usize, entry: usize) -> i32 {
    module_entry(handle, entry, false)
}

// ---- PSAPI -------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn K32EnumProcessModules(
    process: usize,
    out: usize,
    bytes: u32,
    needed: usize,
) -> i32 {
    K32EnumProcessModulesEx(process, out, bytes, needed, 3)
}
#[unsafe(no_mangle)]
pub extern "system" fn K32EnumProcessModulesEx(
    process: usize,
    out: usize,
    bytes: u32,
    needed: usize,
    _filter: u32,
) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    let out = door_windows_abi::widen(out);
    let mut list = [(0u64, 0u64); 256];
    let n = modules(&mut list);
    put(door_windows_abi::widen(needed), (n * POINTER) as u32);
    for (i, (base, _)) in list[..n].iter().enumerate() {
        if (i + 1) * POINTER > bytes as usize {
            break;
        }
        let mut entry = [0u8; 8];
        entry[..POINTER].copy_from_slice(&base.to_le_bytes()[..POINTER]);
        put_bytes(out + (i * POINTER) as u64, &entry[..POINTER]);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn K32GetModuleBaseNameW(
    process: usize,
    module: usize,
    buffer: usize,
    size: u32,
) -> u32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED) as u32;
    }
    let mut path = [0u16; 1024];
    let n = module_path(door_windows_abi::widen(module), &mut path);
    if n == 0 {
        return 0;
    }
    let name = file_part(&path[..n]);
    if size == 0 {
        return fail(ERROR_INSUFFICIENT_BUFFER) as u32;
    }
    let stored = name.len().min(size as usize - 1);
    let buffer = door_windows_abi::widen(buffer);
    for (i, u) in name[..stored].iter().chain([&0]).enumerate() {
        put(buffer + 2 * i as u64, *u);
    }
    stored as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn K32GetModuleFileNameExW(
    process: usize,
    module: usize,
    buffer: usize,
    size: u32,
) -> u32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED) as u32;
    }
    unsafe {
        DoorGetModuleFileNameW(
            door_windows_abi::widen(module),
            door_windows_abi::widen(buffer),
            size,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn K32GetModuleInformation(
    process: usize,
    module: usize,
    info: usize,
    bytes: u32,
) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    let size = 3 * POINTER;
    if (bytes as usize) < size {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    let module = door_windows_abi::widen(module);
    let base = if module == 0 {
        crate::guest::GetModuleHandleW(0) as u64
    } else {
        module
    };
    let mut list = [(0u64, 0u64); 256];
    let n = modules(&mut list);
    let Some(&(base, image)) = list[..n].iter().find(|(b, _)| *b == base) else {
        return fail(ERROR_INVALID_HANDLE);
    };
    // AddressOfEntryPoint from the module's own headers.
    let nt = base + u64::from(get::<u32>(base + 0x3c));
    let entry = get::<u32>(nt + 40);
    let mut out = [0u8; 24];
    put_pointer(&mut out, 0, base);
    out[POINTER..POINTER + 4].copy_from_slice(&(image as u32).to_le_bytes());
    put_pointer(
        &mut out,
        2 * POINTER,
        if entry == 0 {
            0
        } else {
            base + u64::from(entry)
        },
    );
    put_bytes(door_windows_abi::widen(info), &out[..size]);
    1
}
/// PERFORMANCE_INFORMATION from the memory status (pages of 4 KiB).
#[unsafe(no_mangle)]
pub extern "system" fn K32GetPerformanceInfo(info: usize, bytes: u32) -> i32 {
    let size = if WIDE { 104 } else { 56 };
    if (bytes as usize) < size {
        return fail(ERROR_BAD_LENGTH);
    }
    let mut status = [0u8; 64];
    status[..4].copy_from_slice(&64u32.to_le_bytes());
    if crate::runtime::GlobalMemoryStatusEx(status.as_mut_ptr() as usize) == 0 {
        return 0;
    }
    let word = |at: usize| u64::from_le_bytes(status[at..at + 8].try_into().unwrap());
    let (total, free, page_total, page_free) = (word(8), word(16), word(24), word(32));
    let pages = |b: u64| b / 4096;
    let values = [
        pages(page_total - page_free),
        pages(page_total),
        pages(page_total - page_free),
        pages(total),
        pages(free),
        0,
        0,
        0,
        0,
        4096,
    ];
    let mut out = [0u8; 104];
    out[..4].copy_from_slice(&(size as u32).to_le_bytes());
    let first = if WIDE { 8 } else { 4 };
    for (i, v) in values.iter().enumerate() {
        put_pointer(&mut out, first + i * POINTER, *v);
    }
    let counts = first + 10 * POINTER;
    out[counts..counts + 4].copy_from_slice(&256u32.to_le_bytes());
    out[counts + 4..counts + 8].copy_from_slice(&1u32.to_le_bytes());
    out[counts + 8..counts + 12].copy_from_slice(&16u32.to_le_bytes());
    put_bytes(door_windows_abi::widen(info), &out[..size]);
    1
}
/// QueryWorkingSetEx: each address's page is valid (resident) when it is
/// committed, with its protection; Doors does not page memory out.
#[unsafe(no_mangle)]
pub extern "system" fn K32QueryWorkingSetEx(process: usize, buffer: usize, bytes: u32) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    let buffer = door_windows_abi::widen(buffer);
    let entry = 2 * POINTER;
    for i in 0..bytes as usize / entry {
        let at = buffer + (i * entry) as u64;
        let address: usize = get(at);
        let mut info = [0u8; 48];
        let attributes =
            if crate::memory::VirtualQuery(address, info.as_mut_ptr() as usize, 48) != 0 {
                let (state, protect) = if WIDE {
                    (
                        u32::from_le_bytes(info[32..36].try_into().unwrap()),
                        u32::from_le_bytes(info[36..40].try_into().unwrap()),
                    )
                } else {
                    (
                        u32::from_le_bytes(info[16..20].try_into().unwrap()),
                        u32::from_le_bytes(info[20..24].try_into().unwrap()),
                    )
                };
                if state == 0x1000 {
                    1u64 | u64::from(protect & 0x7ff) << 4
                } else {
                    0
                }
            } else {
                0
            };
        let mut value = [0u8; 8];
        value[..POINTER].copy_from_slice(&attributes.to_le_bytes()[..POINTER]);
        put_bytes(at + POINTER as u64, &value[..POINTER]);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessHandleCount(process: usize, count: usize) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    // The kernel does not report a count: a typical small process's.
    put(door_windows_abi::widen(count), 64u32);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessHeaps(count: u32, heaps: usize) -> u32 {
    let heaps = door_windows_abi::widen(heaps);
    if count >= 1 && heaps != 0 {
        let heap = unsafe { DoorGetProcessHeap() };
        let mut out = [0u8; 8];
        out[..POINTER].copy_from_slice(&heap.to_le_bytes()[..POINTER]);
        put_bytes(heaps, &out[..POINTER]);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessIoCounters(process: usize, counters: usize) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    put_bytes(door_windows_abi::widen(counters), &[0u8; 48]);
    1
}
/// QueryProcessCycleTime: the process's CPU time at a nominal 3 GHz.
#[unsafe(no_mangle)]
pub extern "system" fn QueryProcessCycleTime(process: usize, cycles: usize) -> i32 {
    let mut times = [0u64; 4];
    let p = times.as_mut_ptr() as usize;
    if crate::machine::GetProcessTimes(process, p, p + 8, p + 16, p + 24) == 0 {
        return 0;
    }
    put(door_windows_abi::widen(cycles), (times[2] + times[3]) * 300);
    1
}

// ---- process and thread policies ----------------------------------------------------

/// GetProcessInformation: memory priority (normal), power throttling
/// (none), the machine (this DLL's) and leap seconds (none).
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessInformation(
    process: usize,
    class: u32,
    info: usize,
    bytes: u32,
) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    let info = door_windows_abi::widen(info);
    let machine: u16 = if WIDE { 0x8664 } else { 0x14c };
    let answer: &[u8] = match class {
        0 => &5u32.to_le_bytes(),
        4 => &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        8 => &[0; 8],
        9 => &[
            machine as u8,
            (machine >> 8) as u8,
            0,
            0,
            // UserEnabled | KernelEnabled.
            3,
            0,
            0,
            0,
        ],
        _ => return fail(ERROR_INVALID_PARAMETER),
    };
    if (bytes as usize) < answer.len() {
        return fail(ERROR_BAD_LENGTH);
    }
    put_bytes(info, answer);
    1
}
/// SetProcessInformation: memory priority and power throttling are hints
/// Doors' scheduler does not take; accepted.
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessInformation(
    process: usize,
    class: u32,
    _info: usize,
    _bytes: u32,
) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    if class > 9 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
/// SetThreadInformation: memory priority, power throttling and the dynamic
/// code opt-out are hints here; accepted.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadInformation(
    _thread: usize,
    class: u32,
    _info: usize,
    _bytes: u32,
) -> i32 {
    if class > 4 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadInformation(
    _thread: usize,
    class: u32,
    info: usize,
    bytes: u32,
) -> i32 {
    let info = door_windows_abi::widen(info);
    match class {
        0 if bytes >= 4 => put(info, 5u32),
        3 if bytes >= 12 => put_bytes(info, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        _ => return fail(ERROR_INVALID_PARAMETER),
    }
    1
}
/// GetProcessMitigationPolicy: no mitigation is enforced on the caller
/// (DEP is always on: its policy says so).
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessMitigationPolicy(
    process: usize,
    policy: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    if !is_self(door_windows_abi::widen(process)) {
        return fail(ERROR_ACCESS_DENIED);
    }
    let buffer = door_windows_abi::widen(buffer);
    if policy > 20 || !matches!(length, 4 | 8 | 16) || buffer == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    put_bytes(buffer, &[0u8; 16][..length]);
    if policy == 0 {
        // ProcessDEPPolicy: Enable | Permanent.
        put(buffer, 1u32);
        if length >= 8 {
            put(buffer + 4, 1u8);
        }
    }
    1
}
/// SetProcessMitigationPolicy: accepted (the policies Chromium sets
/// restrict what the process itself may do later).
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessMitigationPolicy(
    policy: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    if policy > 20 || buffer == 0 || length == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
/// PRODUCT_PROFESSIONAL: Windows 11 Pro, as Doors reports its version.
#[unsafe(no_mangle)]
pub extern "system" fn GetProductInfo(
    _major: u32,
    _minor: u32,
    _sp_major: u32,
    _sp_minor: u32,
    product: usize,
) -> i32 {
    put(door_windows_abi::widen(product), 0x30u32);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessShutdownParameters(level: u32, _flags: u32) -> i32 {
    if level > 0x4ff {
        return fail(ERROR_INVALID_PARAMETER);
    }
    SHUTDOWN_LEVEL.store(level, Ordering::Relaxed);
    1
}
static SHUTDOWN_LEVEL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x280);
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessShutdownParameters(level: usize, flags: usize) -> i32 {
    put(
        door_windows_abi::widen(level),
        SHUTDOWN_LEVEL.load(Ordering::Relaxed),
    );
    put(door_windows_abi::widen(flags), 0u32);
    1
}
/// RegisterApplicationRestart: Doors does not restart programs after an
/// update or a crash yet; registered (S_OK).
#[unsafe(no_mangle)]
pub extern "system" fn RegisterApplicationRestart(_command: usize, _flags: u32) -> u32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterApplicationRestart() -> u32 {
    0
}
/// The console session: the signed-in user's (session 1).
#[unsafe(no_mangle)]
pub extern "system" fn WTSGetActiveConsoleSessionId() -> u32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn CheckRemoteDebuggerPresent(_process: usize, present: usize) -> i32 {
    let present = door_windows_abi::widen(present);
    if present == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    put(present, 0i32);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn AreFileApisANSI() -> i32 {
    1
}
/// NeedCurrentDirectoryForExePathW: the current directory is searched
/// unless NoDefaultCurrentDirectoryInExePath is set.
#[unsafe(no_mangle)]
pub extern "system" fn NeedCurrentDirectoryForExePathW(name: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    if name != 0 {
        let mut i = 0;
        loop {
            let u: u16 = get(name + 2 * i);
            if u == 0 {
                break;
            }
            if u == u16::from(b'\\') {
                return 1;
            }
            i += 1;
        }
    }
    let variable: [u16; 35] = {
        let mut v = [0u16; 35];
        for (slot, u) in v
            .iter_mut()
            .zip("NoDefaultCurrentDirectoryInExePath".encode_utf16())
        {
            *slot = u;
        }
        v
    };
    let set = crate::environment::GetEnvironmentVariableW(variable.as_ptr() as usize, 0, 0) != 0;
    i32::from(!set)
}
/// PrefetchVirtualMemory: a hint (memory is resident).
#[unsafe(no_mangle)]
pub extern "system" fn PrefetchVirtualMemory(
    _process: usize,
    _count: usize,
    _addresses: usize,
    _flags: u32,
) -> i32 {
    1
}
/// IsThreadAFiber: no thread here has been converted to a fiber (Doors has
/// no fibers yet).
#[unsafe(no_mangle)]
pub extern "system" fn IsThreadAFiber() -> i32 {
    0
}
/// The calling thread's TEB (GS:[0x30] on x64, FS:[0x18] on x86).
fn teb() -> u64 {
    let teb: usize;
    // SAFETY: reads the TEB self pointer of the current thread.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("mov {}, gs:[0x30]", out(reg) teb, options(nostack, readonly, preserves_flags));
        #[cfg(target_arch = "x86")]
        core::arch::asm!("mov {}, fs:[0x18]", out(reg) teb, options(nostack, readonly, preserves_flags));
    }
    teb as u64
}
/// GetCurrentThreadStackLimits: the thread's stack reservation (TEB
/// DeallocationStack) up to its base (TEB StackBase).
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThreadStackLimits(low: usize, high: usize) {
    let teb = teb();
    let (base, limit, deallocation) = if WIDE {
        // Doors' 64-bit TEB ends before DeallocationStack (+0x1478): the
        // reservation's allocation base is the stack region's, which
        // VirtualQuery of this frame reports (MEMORY_BASIC_INFORMATION's
        // AllocationBase, after BaseAddress).
        let marker = 0u8;
        let mut info = [0u64; 6];
        let found = crate::memory::VirtualQuery(
            &marker as *const u8 as usize,
            info.as_mut_ptr() as usize,
            48,
        ) != 0;
        (
            get::<u64>(teb + 8),
            get::<u64>(teb + 16),
            if found { info[1] } else { 0 },
        )
    } else {
        (
            u64::from(get::<u32>(teb + 4)),
            u64::from(get::<u32>(teb + 8)),
            u64::from(get::<u32>(teb + 0xe0c)),
        )
    };
    let bottom = if deallocation != 0 && deallocation < limit {
        deallocation
    } else {
        limit
    };
    let mut lo = [0u8; 8];
    let mut hi = [0u8; 8];
    lo[..POINTER].copy_from_slice(&bottom.to_le_bytes()[..POINTER]);
    hi[..POINTER].copy_from_slice(&base.to_le_bytes()[..POINTER]);
    put_bytes(door_windows_abi::widen(low), &lo[..POINTER]);
    put_bytes(door_windows_abi::widen(high), &hi[..POINTER]);
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadId(thread: usize) -> u32 {
    unsafe { DoorGetThreadId(door_windows_abi::widen(thread)) }
}
/// Wow64GetThreadContext: no thread here is a WOW64 thread of this
/// 64-bit process.
#[unsafe(no_mangle)]
pub extern "system" fn Wow64GetThreadContext(_thread: usize, _context: usize) -> i32 {
    fail(ERROR_INVALID_PARAMETER)
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeLibraryAndExitThread(module: usize, code: u32) -> ! {
    crate::guest::FreeLibrary(module);
    crate::guest::ExitThread(code)
}
/// CreateRemoteThread(Ex): a thread in the calling process; another
/// process cannot be entered.
#[unsafe(no_mangle)]
pub extern "system" fn CreateRemoteThread(
    process: usize,
    attributes: usize,
    stack: usize,
    entry: usize,
    argument: usize,
    flags: u32,
    id: usize,
) -> usize {
    if !is_self(door_windows_abi::widen(process)) {
        fail(ERROR_ACCESS_DENIED);
        return 0;
    }
    crate::guest::CreateThread(attributes, stack, entry, argument, flags, id)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateRemoteThreadEx(
    process: usize,
    attributes: usize,
    stack: usize,
    entry: usize,
    argument: usize,
    flags: u32,
    _list: usize,
    id: usize,
) -> usize {
    CreateRemoteThread(process, attributes, stack, entry, argument, flags, id)
}

// ---- heaps ----------------------------------------------------------------------

/// HeapCompact: the largest free block a heap could commit (its growth
/// limit is the process's memory).
#[unsafe(no_mangle)]
pub extern "system" fn HeapCompact(_heap: usize, _flags: u32) -> usize {
    0x10_0000
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapLock(_heap: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapUnlock(_heap: usize) -> i32 {
    1
}
/// HeapSetInformation: the low-fragmentation heap and terminate-on-
/// corruption are how Doors' heap behaves already.
#[unsafe(no_mangle)]
pub extern "system" fn HeapSetInformation(
    _heap: usize,
    class: u32,
    _info: usize,
    _bytes: usize,
) -> i32 {
    if class > 3 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
/// HeapWalk: Doors' heap does not list its blocks; the walk ends at once.
#[unsafe(no_mangle)]
pub extern "system" fn HeapWalk(_heap: usize, _entry: usize) -> i32 {
    fail(ERROR_NO_MORE_ITEMS)
}
