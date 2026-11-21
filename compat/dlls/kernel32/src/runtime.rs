//! Process and machine queries managed runtimes make at start-up (P18 M0c,
//! measured from .NET 10's imports): the performance counter, pointer
//! encoding, debugger and error-mode state, processor, cache, NUMA and
//! affinity layout, memory status, version checks, jobs and fail-fast.
//! Doors has one processor group and one NUMA node; every logical
//! processor GetSystemInfo reports is listed as its own core.
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use door_windows_abi as abi;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorQueryPerformanceCounter() -> u64;
    fn DoorMemoryStatus(out: u64) -> i32;
    fn DoorProcessorCount() -> u32;
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
    fn DoorCreateEventW(security: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorGetCurrentProcessId() -> u32;
    fn RtlSetLastWin32Error(value: u32);
}
fn last_error(value: u32) {
    unsafe { RtlSetLastWin32Error(value) };
}
const INVALID_PARAMETER: u32 = 87;
const INSUFFICIENT_BUFFER: u32 = 122;
const PTR: usize = core::mem::size_of::<usize>();

/// The Windows version Doors reports (`door_windows_abi::os_version`).
pub const MAJOR: u32 = door_windows_abi::os_version::MAJOR;
pub const MINOR: u32 = door_windows_abi::os_version::MINOR;
pub const BUILD: u32 = door_windows_abi::os_version::BUILD;

// ---- performance counter --------------------------------------------------

/// QueryPerformanceFrequency: 10 MHz, as Windows reports it.
const QPC_FREQUENCY: u64 = 10_000_000;
#[unsafe(no_mangle)]
pub extern "system" fn QueryPerformanceCounter(counter: usize) -> i32 {
    unsafe { (counter as *mut u64).write_unaligned(DoorQueryPerformanceCounter()) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryPerformanceFrequency(frequency: usize) -> i32 {
    unsafe { (frequency as *mut u64).write_unaligned(QPC_FREQUENCY) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryThreadCycleTime(_thread: usize, cycles: usize) -> i32 {
    // Cycles are not counted per thread; the clock in 100 ns units keeps
    // the value monotonic, which is what callers difference.
    unsafe { (cycles as *mut u64).write_unaligned(DoorQueryPerformanceCounter()) };
    1
}

// ---- pointer encoding -------------------------------------------------------

static COOKIE: AtomicUsize = AtomicUsize::new(0);
fn cookie() -> usize {
    let c = COOKIE.load(Ordering::Relaxed);
    if c != 0 {
        return c;
    }
    let mut fresh = 0usize;
    unsafe { DoorGenRandom(&mut fresh as *mut usize as u64, PTR as u32) };
    let fresh = fresh | 1;
    match COOKIE.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => fresh,
        Err(other) => other,
    }
}
fn encode(pointer: usize) -> usize {
    (pointer ^ cookie()).rotate_right((cookie() & 0x1f) as u32)
}
fn decode(pointer: usize) -> usize {
    pointer.rotate_left((cookie() & 0x1f) as u32) ^ cookie()
}
#[unsafe(no_mangle)]
pub extern "system" fn EncodePointer(pointer: usize) -> usize {
    encode(pointer)
}
#[unsafe(no_mangle)]
pub extern "system" fn DecodePointer(pointer: usize) -> usize {
    decode(pointer)
}
#[unsafe(no_mangle)]
pub extern "system" fn EncodeSystemPointer(pointer: usize) -> usize {
    encode(pointer)
}
#[unsafe(no_mangle)]
pub extern "system" fn DecodeSystemPointer(pointer: usize) -> usize {
    decode(pointer)
}

// ---- debugger, error modes ---------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn IsDebuggerPresent() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DebugBreak() {
    // EXCEPTION_BREAKPOINT through the ordinary exception path.
    unsafe { core::arch::asm!("int3", options(nomem, nostack)) };
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorDebugOutput(text: u64, units: u64, wide: u32);
}
/// The serial log listens, as a kernel debugger on Windows (`DOOR_DEBUG`
/// lines); no user-mode debugger is ever attached.
#[unsafe(no_mangle)]
pub extern "system" fn OutputDebugStringA(text: usize) {
    if text != 0 {
        unsafe { DoorDebugOutput(door_windows_abi::widen(text), 4096, 0) };
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OutputDebugStringW(text: usize) {
    if text != 0 {
        unsafe { DoorDebugOutput(door_windows_abi::widen(text), 4096, 1) };
    }
}

static ERROR_MODE: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn SetErrorMode(mode: u32) -> u32 {
    ERROR_MODE.swap(mode, Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetErrorMode() -> u32 {
    ERROR_MODE.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadErrorMode(mode: u32, old: usize) -> i32 {
    // SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX.
    if mode & !0x8003 != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let previous = ERROR_MODE.swap(mode, Ordering::Relaxed);
    if old != 0 {
        unsafe { (old as *mut u32).write_unaligned(previous) };
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadErrorMode() -> u32 {
    ERROR_MODE.load(Ordering::Relaxed)
}
/// Thread attach/detach notifications are cheap here; the request is
/// accepted and every module keeps receiving them.
#[unsafe(no_mangle)]
pub extern "system" fn DisableThreadLibraryCalls(_module: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn WerRegisterRuntimeExceptionModule(_dll: usize, _context: usize) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn WerUnregisterRuntimeExceptionModule(_dll: usize, _context: usize) -> i32 {
    0
}
/// RaiseFailFastException: the process ends at once with the record's code
/// (STATUS_FAIL_FAST_EXCEPTION without one); no handler runs.
#[unsafe(no_mangle)]
pub extern "system" fn RaiseFailFastException(record: usize, _context: usize, _flags: u32) -> ! {
    let code = if record != 0 {
        unsafe { (record as *const u32).read_unaligned() }
    } else {
        0xc000_0602
    };
    crate::guest::ExitProcess(code)
}

// ---- processors, caches, NUMA, affinity ---------------------------------------

fn processors() -> u32 {
    unsafe { DoorProcessorCount() }.clamp(1, 64)
}
fn all_mask() -> usize {
    let n = processors() as usize;
    if n >= PTR * 8 {
        usize::MAX
    } else {
        (1usize << n) - 1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetActiveProcessorGroupCount() -> u16 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMaximumProcessorGroupCount() -> u16 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetActiveProcessorCount(group: u16) -> u32 {
    if group == 0 || group == 0xffff {
        processors()
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMaximumProcessorCount(group: u16) -> u32 {
    GetActiveProcessorCount(group)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumaHighestNodeNumber(node: usize) -> i32 {
    unsafe { (node as *mut u32).write_unaligned(0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumaProcessorNodeEx(processor: usize, node: usize) -> i32 {
    // PROCESSOR_NUMBER { WORD Group; BYTE Number; BYTE Reserved }.
    let group = unsafe { (processor as *const u16).read_unaligned() };
    let number = unsafe { ((processor + 2) as *const u8).read() };
    if group != 0 || u32::from(number) >= processors() {
        unsafe { (node as *mut u16).write_unaligned(0xffff) };
        last_error(INVALID_PARAMETER);
        return 0;
    }
    unsafe { (node as *mut u16).write_unaligned(0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumaNodeProcessorMaskEx(node: u16, affinity: usize) -> i32 {
    if node != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    write_group_affinity(affinity, all_mask());
    1
}
fn current_processor() -> u8 {
    // RDTSCP's IA32_TSC_AUX is not set up; Doors runs user threads on the
    // boot processor (P18 M0h raises this).
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcessorNumber() -> u32 {
    u32::from(current_processor())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcessorNumberEx(processor: usize) {
    unsafe {
        (processor as *mut u16).write_unaligned(0);
        ((processor + 2) as *mut u8).write(current_processor());
        ((processor + 3) as *mut u8).write(0);
    }
}
/// GROUP_AFFINITY { KAFFINITY Mask; WORD Group; WORD Reserved[3] }.
fn write_group_affinity(at: usize, mask: usize) {
    unsafe {
        (at as *mut usize).write_unaligned(mask);
        core::ptr::write_bytes((at + PTR) as *mut u8, 0, 8);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessAffinityMask(
    _process: usize,
    process: usize,
    system: usize,
) -> i32 {
    unsafe {
        (process as *mut usize).write_unaligned(all_mask());
        (system as *mut usize).write_unaligned(all_mask());
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessAffinityMask(_process: usize, mask: usize) -> i32 {
    if mask == 0 || mask & !all_mask() != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessGroupAffinity(
    _process: usize,
    count: usize,
    groups: usize,
) -> i32 {
    let available = unsafe { (count as *const u16).read_unaligned() };
    unsafe { (count as *mut u16).write_unaligned(1) };
    if available < 1 || groups == 0 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    unsafe { (groups as *mut u16).write_unaligned(0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadGroupAffinity(_thread: usize, affinity: usize) -> i32 {
    write_group_affinity(affinity, all_mask());
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadGroupAffinity(
    _thread: usize,
    affinity: usize,
    previous: usize,
) -> i32 {
    let mask = unsafe { (affinity as *const usize).read_unaligned() };
    let group = unsafe { ((affinity + PTR) as *const u16).read_unaligned() };
    if group != 0 || mask == 0 || mask & !all_mask() != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    if previous != 0 {
        write_group_affinity(previous, all_mask());
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadAffinityMask(_thread: usize, mask: usize) -> usize {
    if mask == 0 || mask & !all_mask() != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    all_mask()
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadIdealProcessorEx(_thread: usize, processor: usize) -> i32 {
    GetCurrentProcessorNumberEx(processor);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadIdealProcessorEx(
    _thread: usize,
    ideal: usize,
    previous: usize,
) -> i32 {
    let group = unsafe { (ideal as *const u16).read_unaligned() };
    let number = unsafe { ((ideal + 2) as *const u8).read() };
    if group != 0 || u32::from(number) >= processors() {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    if previous != 0 {
        GetCurrentProcessorNumberEx(previous);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadIdealProcessor(_thread: usize, ideal: u32) -> u32 {
    if ideal != 0xffff_ffff && ideal >= processors() {
        last_error(INVALID_PARAMETER);
        return u32::MAX;
    }
    0
}

/// (level, kind, line, size, associativity, shared by every processor)
/// for each cache CPUID describes (deterministic cache parameters, leaf 4
/// on Intel, 0x8000_001D on AMD), or a typical layout without one.
fn caches(out: &mut [(u8, u32, u16, u32, u8, bool); 8]) -> usize {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{__cpuid, __cpuid_count};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    let vendor = __cpuid(0);
    let amd = vendor.ebx == 0x6874_7541; // "Auth"
    let (leaf, available) = if amd {
        (0x8000_001d, __cpuid(0x8000_0000).eax >= 0x8000_001d)
    } else {
        (4, vendor.eax >= 4)
    };
    let mut count = 0;
    if available {
        for index in 0..8 {
            let r = __cpuid_count(leaf, index);
            let kind = r.eax & 0x1f;
            if kind == 0 {
                break;
            }
            let level = ((r.eax >> 5) & 7) as u8;
            let line = (r.ebx & 0xfff) + 1;
            let partitions = ((r.ebx >> 12) & 0x3ff) + 1;
            let ways = ((r.ebx >> 22) & 0x3ff) + 1;
            let sets = r.ecx + 1;
            let size = ways * partitions * line * sets;
            // PROCESSOR_CACHE_TYPE: Unified 0, Instruction 1, Data 2.
            let windows_kind = match kind {
                1 => 2,
                2 => 1,
                _ => 0,
            };
            out[count] = (
                level,
                windows_kind,
                line as u16,
                size,
                ways.min(255) as u8,
                level >= 3,
            );
            count += 1;
        }
    }
    if count == 0 {
        out[0] = (1, 2, 64, 32 << 10, 8, false);
        out[1] = (1, 1, 64, 32 << 10, 8, false);
        out[2] = (2, 0, 64, 1 << 20, 16, false);
        out[3] = (3, 0, 64, 8 << 20, 16, true);
        count = 4;
    }
    count
}

/// SYSTEM_LOGICAL_PROCESSOR_INFORMATION entries: one core per processor,
/// one package, one NUMA node, the caches.
#[unsafe(no_mangle)]
pub extern "system" fn GetLogicalProcessorInformation(buffer: usize, length: usize) -> i32 {
    // { ULONG_PTR ProcessorMask; LOGICAL_PROCESSOR_RELATIONSHIP Relationship;
    //   union (16 bytes, 8-aligned on x64) }.
    const ENTRY: usize = if PTR == 8 { 32 } else { 24 };
    let n = processors() as usize;
    let mut cache = [(0u8, 0u32, 0u16, 0u32, 0u8, false); 8];
    let caches = caches(&mut cache);
    let per_core = cache[..caches].iter().filter(|c| !c.5).count();
    let shared = caches - per_core;
    let entries = n /* cores */ + 1 /* package */ + 1 /* node */ + n * per_core + shared;
    let needed = entries * ENTRY;
    let available = unsafe { (length as *const u32).read_unaligned() } as usize;
    unsafe { (length as *mut u32).write_unaligned(needed as u32) };
    if available < needed || buffer == 0 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    let mut at = buffer;
    let mut put = |mask: usize, relationship: u32, union: [u8; 16]| unsafe {
        core::ptr::write_bytes(at as *mut u8, 0, ENTRY);
        (at as *mut usize).write_unaligned(mask);
        ((at + PTR) as *mut u32).write_unaligned(relationship);
        let union_at = at + ENTRY - 16;
        core::ptr::copy_nonoverlapping(union.as_ptr(), union_at as *mut u8, 16);
        at += ENTRY;
    };
    let cache_union = |c: (u8, u32, u16, u32, u8, bool)| {
        let mut u = [0u8; 16];
        u[0] = c.0;
        u[1] = c.4;
        u[2..4].copy_from_slice(&c.2.to_le_bytes());
        u[4..8].copy_from_slice(&c.3.to_le_bytes());
        u[8..12].copy_from_slice(&c.1.to_le_bytes());
        u
    };
    for cpu in 0..n {
        // RelationProcessorCore, Flags 0 (no SMT sibling listed).
        put(1 << cpu, 0, [0; 16]);
        for c in cache[..caches].iter().filter(|c| !c.5) {
            put(1 << cpu, 2, cache_union(*c));
        }
    }
    for c in cache[..caches].iter().filter(|c| c.5) {
        put(all_mask(), 2, cache_union(*c));
    }
    put(all_mask(), 1, [0; 16]); // RelationNumaNode, node 0
    put(all_mask(), 3, [0; 16]); // RelationProcessorPackage
    1
}

/// SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX records of `relationship`
/// (RelationAll 0xffff for every kind).
#[unsafe(no_mangle)]
pub extern "system" fn GetLogicalProcessorInformationEx(
    relationship: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    // GROUP_AFFINITY: pointer-sized mask, WORD group, 3 reserved WORDs.
    const AFFINITY: usize = if PTR == 8 { 16 } else { 12 };
    let n = processors() as usize;
    let mut cache = [(0u8, 0u32, 0u16, 0u32, 0u8, false); 8];
    let caches = caches(&mut cache);
    let wants = |kind: u32| relationship == 0xffff || relationship == kind;
    // Record sizes (8-byte header + payload, rounded to 8).
    let round = |v: usize| (v + 7) & !7;
    let core = round(8 + 24 + AFFINITY);
    let node = round(8 + 24 + AFFINITY);
    let cache_record = round(8 + 32 + AFFINITY);
    let group = round(8 + 24 + 48);
    let mut needed = 0;
    if wants(0) {
        needed += n * core;
    }
    if wants(1) {
        needed += node;
    }
    if wants(2) {
        needed += cache[..caches]
            .iter()
            .map(|c| if c.5 { cache_record } else { n * cache_record })
            .sum::<usize>();
    }
    if wants(3) {
        needed += core;
    }
    if wants(4) {
        needed += group;
    }
    let available = unsafe { (length as *const u32).read_unaligned() } as usize;
    unsafe { (length as *mut u32).write_unaligned(needed as u32) };
    if available < needed || buffer == 0 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    let mut at = buffer;
    let mut header = |kind: u32, size: usize| -> usize {
        let start = at;
        unsafe {
            core::ptr::write_bytes(start as *mut u8, 0, size);
            (start as *mut u32).write_unaligned(kind);
            ((start + 4) as *mut u32).write_unaligned(size as u32);
        }
        at += size;
        start + 8
    };
    // PROCESSOR_RELATIONSHIP: Flags, EfficiencyClass, Reserved[20],
    // GroupCount, GroupMask[1].
    let processor = |payload: usize, mask: usize| unsafe {
        ((payload + 22) as *mut u16).write_unaligned(1);
        write_group_affinity(payload + 24, mask);
    };
    if wants(0) {
        for cpu in 0..n {
            let p = header(0, core);
            processor(p, 1 << cpu);
        }
    }
    if wants(1) {
        // NUMA_NODE_RELATIONSHIP: NodeNumber, Reserved[18], GroupCount, GroupMask.
        let p = header(1, node);
        unsafe { ((p + 22) as *mut u16).write_unaligned(1) };
        write_group_affinity(p + 24, all_mask());
    }
    if wants(2) {
        // CACHE_RELATIONSHIP: Level, Associativity, LineSize, CacheSize,
        // Type, Reserved[18], GroupCount, GroupMask.
        for c in cache[..caches].iter() {
            let masks = if c.5 { 1 } else { n };
            for i in 0..masks {
                let p = header(2, cache_record);
                unsafe {
                    (p as *mut u8).write(c.0);
                    ((p + 1) as *mut u8).write(c.4);
                    ((p + 2) as *mut u16).write_unaligned(c.2);
                    ((p + 4) as *mut u32).write_unaligned(c.3);
                    ((p + 8) as *mut u32).write_unaligned(c.1);
                    ((p + 30) as *mut u16).write_unaligned(1);
                }
                write_group_affinity(p + 32, if c.5 { all_mask() } else { 1 << i });
            }
        }
    }
    if wants(3) {
        let p = header(3, core);
        processor(p, all_mask());
    }
    if wants(4) {
        // GROUP_RELATIONSHIP: MaximumGroupCount, ActiveGroupCount,
        // Reserved[20], GroupInfo[1] { MaximumProcessorCount,
        // ActiveProcessorCount, Reserved[38], ActiveProcessorMask }.
        let p = header(4, group);
        unsafe {
            (p as *mut u16).write_unaligned(1);
            ((p + 2) as *mut u16).write_unaligned(1);
            ((p + 24) as *mut u8).write(n as u8);
            ((p + 25) as *mut u8).write(n as u8);
            ((p + 64) as *mut usize).write_unaligned(all_mask());
        }
    }
    1
}

// ---- memory status ------------------------------------------------------------

fn memory() -> Option<[u64; 4]> {
    let mut values = [0u64; 4];
    (unsafe { DoorMemoryStatus(values.as_mut_ptr() as u64) } != 0).then_some(values)
}
/// MEMORYSTATUSEX; dwLength must be 64.
#[unsafe(no_mangle)]
pub extern "system" fn GlobalMemoryStatusEx(status: usize) -> i32 {
    if unsafe { (status as *const u32).read_unaligned() } != 64 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let Some([total, free, _, _]) = memory() else {
        return 0;
    };
    let virtual_total: u64 = if PTR == 8 {
        0x7fff_fffe_0000
    } else {
        0x7ffe_0000
    };
    let load = if total == 0 {
        0
    } else {
        (total - free.min(total)) * 100 / total
    };
    let fields = [total, free, total, free, virtual_total, virtual_total, 0];
    unsafe {
        ((status + 4) as *mut u32).write_unaligned(load as u32);
        for (i, value) in fields.iter().enumerate() {
            ((status + 8 + i * 8) as *mut u64).write_unaligned(*value);
        }
    }
    1
}
/// PROCESS_MEMORY_COUNTERS(_EX) of the current process.
#[unsafe(no_mangle)]
pub extern "system" fn K32GetProcessMemoryInfo(process: usize, counters: usize, size: u32) -> i32 {
    let basic = (8 + 8 * PTR) as u32;
    if size < basic {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    if !crate::system::current(abi::widen(process)) {
        last_error(5);
        return 0;
    }
    let Some([_, _, private, _]) = memory() else {
        return 0;
    };
    let private = private as usize;
    unsafe {
        core::ptr::write_bytes(counters as *mut u8, 0, size as usize);
        (counters as *mut u32).write_unaligned(basic.max(size.min(basic + PTR as u32)));
        // Peak and current working set, page-file usage and its peak.
        for slot in [0usize, 1, 6, 7] {
            ((counters + 8 + slot * PTR) as *mut usize).write_unaligned(private);
        }
        if size >= basic + PTR as u32 {
            // PROCESS_MEMORY_COUNTERS_EX.PrivateUsage.
            ((counters + basic as usize) as *mut usize).write_unaligned(private);
        }
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessMemoryInfo(process: usize, counters: usize, size: u32) -> i32 {
    K32GetProcessMemoryInfo(process, counters, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLargePageMinimum() -> usize {
    0
}
/// A low-memory notification that never signals (no memory pressure is
/// reported here), as a manual-reset event handle.
#[unsafe(no_mangle)]
pub extern "system" fn CreateMemoryResourceNotification(kind: u32) -> usize {
    if kind > 1 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    abi::narrow(unsafe { DoorCreateEventW(0, 1, 0, 0) })
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryMemoryResourceNotification(_handle: usize, state: usize) -> i32 {
    unsafe { (state as *mut i32).write_unaligned(0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualUnlock(_address: usize, _bytes: usize) -> i32 {
    // Pages are never paged out; nothing was locked. Windows reports
    // ERROR_NOT_LOCKED for an unlocked range.
    last_error(158);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualLock(_address: usize, _bytes: usize) -> i32 {
    1
}

// ---- thread scheduling hints ----------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn SwitchToThread() -> i32 {
    crate::guest::Sleep(0);
    1
}
static PRIORITY: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadPriority(_thread: usize) -> i32 {
    PRIORITY.load(Ordering::Relaxed) as i32
}
/// SetThreadDescription: S_OK. A thread's description is a debugger's
/// label; no debugger reads it here, so it is not kept.
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDescription(_thread: usize, description: usize) -> i32 {
    if description == 0 {
        // E_INVALIDARG (Windows dereferences the string).
        return 0x8007_0057_u32 as i32;
    }
    0
}
/// GetThreadDescription: S_OK and an empty description in LocalAlloc'd
/// memory (the caller frees it with LocalFree).
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDescription(_thread: usize, out: usize) -> i32 {
    if out == 0 {
        return 0x8007_0057_u32 as i32;
    }
    // LPTR: zeroed, so the string is empty.
    let text = crate::application::LocalAlloc(0x40, 2);
    if text == 0 {
        return 0x8007_000E_u32 as i32;
    }
    unsafe { (out as *mut usize).write_unaligned(text) };
    0
}
/// Priorities are accepted and reported back; the native scheduler keeps
/// one level for application threads (P18 M0h maps them).
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadPriority(_thread: usize, priority: i32) -> i32 {
    if !matches!(priority, -15 | -2..=2 | 15) {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    PRIORITY.store(priority as u32, Ordering::Relaxed);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadStackGuarantee(size: usize) -> i32 {
    unsafe { (size as *mut u32).write_unaligned(0) };
    1
}

// ---- jobs ----------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn IsProcessInJob(_process: usize, _job: usize, result: usize) -> i32 {
    unsafe { (result as *mut i32).write_unaligned(0) };
    1
}
/// A job this process made answers basic accounting (`jobs`); the caller's
/// own job (NULL) or another handle finds none, as no process runs in a
/// job it did not make.
#[unsafe(no_mangle)]
pub extern "system" fn QueryInformationJobObject(
    job: usize,
    class: u32,
    information: usize,
    length: u32,
    returned: usize,
) -> i32 {
    crate::jobs::query(abi::widen(job), class, information, length, returned)
}

// ---- version checks ----------------------------------------------------------------

const VER_MINORVERSION: u32 = 0x1;
const VER_MAJORVERSION: u32 = 0x2;
const VER_BUILDNUMBER: u32 = 0x4;
const VER_PLATFORMID: u32 = 0x8;
const VER_SERVICEPACKMINOR: u32 = 0x10;
const VER_SERVICEPACKMAJOR: u32 = 0x20;
const VER_SUITENAME: u32 = 0x40;
const VER_PRODUCT_TYPE: u32 = 0x80;
#[unsafe(no_mangle)]
pub extern "system" fn VerSetConditionMask(mask: u64, kinds: u32, condition: u8) -> u64 {
    if kinds == 0 || condition == 0 {
        return mask;
    }
    let condition = u64::from(condition & 7);
    let index = if kinds & VER_PRODUCT_TYPE != 0 {
        7
    } else if kinds & VER_SUITENAME != 0 {
        6
    } else if kinds & VER_SERVICEPACKMAJOR != 0 {
        5
    } else if kinds & VER_SERVICEPACKMINOR != 0 {
        4
    } else if kinds & VER_PLATFORMID != 0 {
        3
    } else if kinds & VER_BUILDNUMBER != 0 {
        2
    } else if kinds & VER_MAJORVERSION != 0 {
        1
    } else {
        0
    };
    mask | condition << (index * 3)
}
fn compare(ours: u32, theirs: u32, condition: u64) -> bool {
    match condition {
        1 => ours == theirs,
        2 => ours > theirs,
        3 => ours >= theirs,
        4 => ours < theirs,
        5 => ours <= theirs,
        _ => false,
    }
}
/// `GetSystemTimes`: idle, kernel (which includes idle) and user time,
/// summed over the processors, in 100 ns FILETIMEs. Doors keeps no
/// per-processor busy accounting yet, so every processor counts as idle
/// since boot (the thread pool then sees spare CPU, as on an idle machine).
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemTimes(idle: usize, kernel: usize, user: usize) -> i32 {
    let since_boot = unsafe { DoorQueryPerformanceCounter() };
    let total = since_boot.saturating_mul(u64::from(unsafe { DoorProcessorCount() }.max(1)));
    for (pointer, value) in [(idle, total), (kernel, total), (user, 0)] {
        if pointer != 0 {
            // SAFETY: the caller's FILETIME.
            unsafe { (pointer as *mut u64).write_unaligned(value) };
        }
    }
    1
}
/// `GetVersion`: build, minor and major packed as Windows NT packs them.
#[unsafe(no_mangle)]
pub extern "system" fn GetVersion() -> u32 {
    (BUILD << 16) | (MINOR << 8) | MAJOR
}
/// `GetVersionExW`/`GetVersionExA`: OSVERSIONINFO(EX) with the true
/// version, as Windows answers programs whose manifest declares Windows 10
/// and 11 (the programs Doors runs: .NET, PowerShell, Chromium) — no
/// service pack, VER_SUITE_SINGLEUSERTS, VER_NT_WORKSTATION.
/// `text_bytes`: the CSD version string's bytes (128 units of 2 or 1).
fn version_ex(info: usize, text_bytes: usize) -> i32 {
    if info == 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let size = unsafe { (info as *const u32).read_unaligned() } as usize;
    let plain = 20 + text_bytes;
    if size != plain && size != plain + 8 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    let mut out = [0u8; 284];
    out[..4].copy_from_slice(&(size as u32).to_le_bytes());
    for (at, value) in [
        (4, MAJOR),
        (8, MINOR),
        (12, BUILD),
        (16, door_windows_abi::os_version::PLATFORM),
    ] {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    if size == plain + 8 {
        out[plain + 4..plain + 6].copy_from_slice(&0x100u16.to_le_bytes());
        out[plain + 6] = 1;
    }
    // SAFETY: the caller's structure of `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), info as *mut u8, size) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetVersionExW(info: usize) -> i32 {
    version_ex(info, 256)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetVersionExA(info: usize) -> i32 {
    version_ex(info, 128)
}
/// VerifyVersionInfoW against 10.0.`BUILD`, NT workstation, no service pack.
#[unsafe(no_mangle)]
pub extern "system" fn VerifyVersionInfoW(info: usize, kinds: u32, mask: u64) -> i32 {
    if kinds == 0 || kinds & !0xff != 0 {
        last_error(160);
        return 0;
    }
    let read32 = |offset: usize| unsafe { ((info + offset) as *const u32).read_unaligned() };
    let read16 = |offset: usize| unsafe { ((info + offset) as *const u16).read_unaligned() };
    let condition = |bit: u32| (mask >> (bit.trailing_zeros() * 3)) & 7;
    let mut ok = true;
    // Major, minor and service pack compare as one hierarchical version,
    // under the condition of the most significant one asked for.
    let hierarchy = [
        (VER_MAJORVERSION, MAJOR, read32(4)),
        (VER_MINORVERSION, MINOR, read32(8)),
        (VER_SERVICEPACKMAJOR, 0, u32::from(read16(276))),
        (VER_SERVICEPACKMINOR, 0, u32::from(read16(278))),
    ];
    if let Some(&(first, ..)) = hierarchy.iter().find(|(bit, ..)| kinds & bit != 0) {
        let rule = condition(first);
        let mut ordering = core::cmp::Ordering::Equal;
        for &(bit, ours, theirs) in &hierarchy {
            if kinds & bit != 0 && ordering == core::cmp::Ordering::Equal {
                ordering = ours.cmp(&theirs);
            }
        }
        ok &= match rule {
            1 => ordering.is_eq(),
            2 => ordering.is_gt(),
            3 => ordering.is_ge(),
            4 => ordering.is_lt(),
            5 => ordering.is_le(),
            _ => false,
        };
    }
    if kinds & VER_BUILDNUMBER != 0 {
        ok &= compare(BUILD, read32(12), condition(VER_BUILDNUMBER));
    }
    if kinds & VER_PLATFORMID != 0 {
        ok &= compare(2, read32(16), condition(VER_PLATFORMID));
    }
    if kinds & VER_PRODUCT_TYPE != 0 {
        let theirs = unsafe { ((info + 282) as *const u8).read() };
        ok &= compare(1, u32::from(theirs), condition(VER_PRODUCT_TYPE));
    }
    if kinds & VER_SUITENAME != 0 {
        // VER_SUITE_SINGLEUSERTS; AND (6) needs every bit, OR (7) any.
        let ours = 0x100u32;
        let theirs = u32::from(read16(280));
        ok &= match condition(VER_SUITENAME) {
            6 => ours & theirs == theirs,
            7 => ours & theirs != 0,
            _ => false,
        };
    }
    if ok {
        1
    } else {
        last_error(1150);
        0
    }
}

// ---- processor features -----------------------------------------------------------

/// IsProcessorFeaturePresent from CPUID, honouring the state the kernel
/// enables (AVX only with OSXSAVE and YMM state in XCR0).
#[unsafe(no_mangle)]
pub extern "system" fn IsProcessorFeaturePresent(feature: u32) -> i32 {
    #[cfg(target_arch = "x86")]
    use core::arch::x86::{__cpuid, __cpuid_count};
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::{__cpuid, __cpuid_count};
    let one = __cpuid(1);
    let max = __cpuid(0).eax;
    let seven = if max >= 7 {
        __cpuid_count(7, 0)
    } else {
        __cpuid(0)
    };
    let seven_ebx = if max >= 7 { seven.ebx } else { 0 };
    let extended = __cpuid(0x8000_0000).eax;
    let ext1_edx = if extended >= 0x8000_0001 {
        __cpuid(0x8000_0001).edx
    } else {
        0
    };
    let osxsave = one.ecx & (1 << 27) != 0;
    let xcr0 = if osxsave {
        let (low, high): (u32, u32);
        unsafe {
            core::arch::asm!("xgetbv", in("ecx") 0u32, out("eax") low, out("edx") high,
                options(nomem, nostack));
        }
        (u64::from(high) << 32) | u64::from(low)
    } else {
        0
    };
    let avx = one.ecx & (1 << 28) != 0 && xcr0 & 6 == 6;
    let present = match feature {
        3 => one.edx & (1 << 23) != 0,                                  // MMX
        6 => one.edx & (1 << 25) != 0,                                  // SSE
        8 => one.edx & (1 << 4) != 0,                                   // RDTSC
        9 | 12 => true,                                                 // PAE, NX
        10 => one.edx & (1 << 26) != 0,                                 // SSE2
        13 => one.ecx & 1 != 0,                                         // SSE3
        14 => one.ecx & (1 << 13) != 0,                                 // CMPXCHG16B
        17 => osxsave,                                                  // XSAVE enabled
        28 => one.ecx & (1 << 30) != 0,                                 // RDRAND
        32 => ext1_edx & (1 << 27) != 0,                                // RDTSCP
        36 => one.ecx & (1 << 9) != 0,                                  // SSSE3
        37 => one.ecx & (1 << 19) != 0,                                 // SSE4.1
        38 => one.ecx & (1 << 20) != 0,                                 // SSE4.2
        39 => avx,                                                      // AVX
        40 => avx && seven_ebx & (1 << 5) != 0,                         // AVX2
        41 => avx && xcr0 & 0xe0 == 0xe0 && seven_ebx & (1 << 16) != 0, // AVX-512F
        43 => seven_ebx & (1 << 9) != 0,                                // ERMS
        _ => false,
    };
    i32::from(present)
}

/// The current process id, for modules here that need it.
#[allow(dead_code)]
pub fn process_id() -> u32 {
    unsafe { DoorGetCurrentProcessId() }
}

