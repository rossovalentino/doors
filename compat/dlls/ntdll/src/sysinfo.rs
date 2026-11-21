//! SYSTEM_INFO for GetSystemInfo and GetNativeSystemInfo, laid out at this
//! build's pointer width. x64: the machine as it is. x86 (P17), measured
//! from a WOW64 program on Windows 11 (`x86probe.exe raw`): GetSystemInfo
//! reports an Intel processor (architecture 0, type 586) with a 2 GB user
//! range, GetNativeSystemInfo the x64 machine (architecture 9, type 8664)
//! with the 4 GB range a 32-bit pointer reaches.
use super::guest::{RtlSetLastWin32Error, copy};

const PAGE: u32 = 4096;
const GRANULARITY: u32 = 0x1_0000;
const MINIMUM: u64 = 0x1_0000;

/// (architecture, processor type, maximum application address).
fn machine(native: bool) -> (u16, u32, u64) {
    #[cfg(target_arch = "x86")]
    {
        if native {
            (9, 8664, 0xfffe_ffff)
        } else {
            (0, 586, 0x7ffe_ffff)
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        let _ = native;
        (9, 8664, 0x7fff_fffe_ffff)
    }
}
/// wProcessorLevel and wProcessorRevision as Windows derives them from
/// CPUID leaf 1: the display family, and the display model and stepping.
fn level_and_revision() -> (u16, u16) {
    // CPUID is available to every x86 user program.
    #[cfg(target_arch = "x86_64")]
    let eax = core::arch::x86_64::__cpuid(1).eax;
    #[cfg(target_arch = "x86")]
    let eax = core::arch::x86::__cpuid(1).eax;
    let stepping = eax & 15;
    let model = (eax >> 4) & 15;
    let family = (eax >> 8) & 15;
    let extended_model = (eax >> 16) & 15;
    let extended_family = (eax >> 20) & 255;
    let display_family = if family == 15 {
        family + extended_family
    } else {
        family
    };
    let display_model = if family == 6 || family == 15 {
        model + (extended_model << 4)
    } else {
        model
    };
    (
        display_family as u16,
        ((display_model << 8) | stepping) as u16,
    )
}
/// GetSystemInfo (`native` 0) or GetNativeSystemInfo (1) into `out`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetSystemInfo(native: u32, out: u64) -> u32 {
    let width = core::mem::size_of::<usize>();
    let (architecture, kind, maximum) = machine(native != 0);
    let processors = door_sys::hardware_info().map_or(1, |info| u32::from(info.processors).max(1));
    let mask = if processors >= 64 {
        u64::MAX
    } else {
        (1u64 << processors) - 1
    };
    let (level, revision) = level_and_revision();
    let mut info = [0u8; 48];
    info[..2].copy_from_slice(&architecture.to_le_bytes());
    info[4..8].copy_from_slice(&PAGE.to_le_bytes());
    for (index, value) in [MINIMUM, maximum, mask].into_iter().enumerate() {
        let at = 8 + index * width;
        info[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    let at = 8 + 3 * width;
    info[at..at + 4].copy_from_slice(&processors.to_le_bytes());
    info[at + 4..at + 8].copy_from_slice(&kind.to_le_bytes());
    info[at + 8..at + 12].copy_from_slice(&GRANULARITY.to_le_bytes());
    info[at + 12..at + 14].copy_from_slice(&level.to_le_bytes());
    info[at + 14..at + 16].copy_from_slice(&revision.to_le_bytes());
    let bytes = at + 16;
    match copy(out, info.as_ptr() as u64, bytes) {
        Ok(()) => 1,
        Err(_) => {
            RtlSetLastWin32Error(998);
            0
        }
    }
}

/// QueryPerformanceCounter's value: 100 ns units since boot (frequency
/// 10 MHz, as Windows reports it).
#[unsafe(no_mangle)]
pub extern "system" fn DoorQueryPerformanceCounter() -> u64 {
    door_sys::precise_time().unwrap_or(0)
}
/// GlobalMemoryStatusEx and GetProcessMemoryInfo's figures into `out`
/// (four u64: physical, free physical, this process's private bytes,
/// commit limit). FALSE when the kernel refused.
#[unsafe(no_mangle)]
pub extern "system" fn DoorMemoryStatus(out: u64) -> i32 {
    match door_sys::memory_status() {
        Ok(values) => {
            let mut bytes = [0u8; 32];
            for (chunk, value) in bytes.chunks_exact_mut(8).zip(values) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
            i32::from(copy(out, bytes.as_ptr() as u64, 32).is_ok())
        }
        Err(_) => {
            RtlSetLastWin32Error(1);
            0
        }
    }
}
/// Logical processors, as GetSystemInfo reports them.
#[unsafe(no_mangle)]
pub extern "system" fn DoorProcessorCount() -> u32 {
    door_sys::hardware_info().map_or(1, |info| u32::from(info.processors).max(1))
}

const STATUS_SUCCESS: u32 = 0;
const STATUS_INVALID_INFO_CLASS: u32 = 0xC000_0003;
const STATUS_INFO_LENGTH_MISMATCH: u32 = 0xC000_0004;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;

/// `RtlGetVersion`: RTL_OSVERSIONINFOW (276 bytes) or, sized as one,
/// RTL_OSVERSIONINFOEXW (284: no service pack, VER_SUITE_SINGLEUSERTS,
/// VER_NT_WORKSTATION), with Windows' true version (no compatibility
/// shims). The structure's own first ULONG says which.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetVersion(info: usize) -> u32 {
    use door_windows_abi::os_version as v;
    let info = door_windows_abi::widen(info);
    let mut size = [0u8; 4];
    if info == 0 || copy(size.as_mut_ptr() as u64, info, 4).is_err() {
        return STATUS_INVALID_PARAMETER;
    }
    let size = u32::from_le_bytes(size) as usize;
    if size != 276 && size != 284 {
        return STATUS_INVALID_PARAMETER;
    }
    let mut out = [0u8; 284];
    out[..4].copy_from_slice(&(size as u32).to_le_bytes());
    for (at, value) in [
        (4, v::MAJOR),
        (8, v::MINOR),
        (12, v::BUILD),
        (16, v::PLATFORM),
    ] {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    // wSuiteMask, wProductType.
    out[280..282].copy_from_slice(&0x100u16.to_le_bytes());
    out[282] = 1;
    match copy(info, out.as_ptr() as u64, size) {
        Ok(()) => STATUS_SUCCESS,
        Err(_) => STATUS_ACCESS_VIOLATION,
    }
}
/// `RtlGetNtVersionNumbers`: the build with its free-build marker
/// (0xF000_0000), as Windows reports it.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetNtVersionNumbers(major: usize, minor: usize, build: usize) {
    use door_windows_abi::os_version as v;
    let (major, minor, build) = (
        door_windows_abi::widen(major),
        door_windows_abi::widen(minor),
        door_windows_abi::widen(build),
    );
    for (pointer, value) in [
        (major, v::MAJOR),
        (minor, v::MINOR),
        (build, v::BUILD | 0xF000_0000),
    ] {
        if pointer != 0 {
            let _ = copy(pointer, &value as *const u32 as u64, 4);
        }
    }
}

/// `NtQuerySystemInformation` for the classes programs ask before they
/// run: SystemBasicInformation (0), SystemProcessorInformation (1),
/// SystemProcessorPerformanceInformation (8: per processor, idle since
/// boot; Doors keeps no per-processor busy times yet) and
/// SystemLeapSecondInformation (206: disabled). Other classes are
/// STATUS_INVALID_INFO_CLASS. A buffer of the wrong size is
/// STATUS_INFO_LENGTH_MISMATCH with the size needed.
#[unsafe(no_mangle)]
pub extern "system" fn NtQuerySystemInformation(
    class: u32,
    buffer: usize,
    length: u32,
    returned: usize,
) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let returned = door_windows_abi::widen(returned);
    let width = core::mem::size_of::<usize>();
    let processors = DoorProcessorCount();
    let mut out = [0u8; 48 * 64];
    let (bytes, exact) = match class {
        0 => {
            let physical = door_sys::memory_status().map_or(0, |v| v[0]);
            let (_, _, maximum) = machine(false);
            let mask = if processors >= 64 {
                u64::MAX
            } else {
                (1u64 << processors) - 1
            };
            // TimerResolution (100 ns), PageSize, physical pages (lowest
            // 1, highest the last), AllocationGranularity.
            for (at, value) in [
                (4, 156_250u32),
                (8, PAGE),
                (12, (physical / u64::from(PAGE)) as u32),
                (16, 1),
                (20, (physical / u64::from(PAGE)) as u32),
                (24, GRANULARITY),
            ] {
                out[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            let base = if width == 8 { 32 } else { 28 };
            for (index, value) in [MINIMUM, maximum, mask].into_iter().enumerate() {
                let at = base + index * width;
                out[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
            }
            out[base + 3 * width] = processors.min(127) as u8;
            (if width == 8 { 64 } else { 44 }, true)
        }
        1 => {
            let (architecture, _, _) = machine(false);
            let (level, revision) = level_and_revision();
            out[..2].copy_from_slice(&architecture.to_le_bytes());
            out[2..4].copy_from_slice(&level.to_le_bytes());
            out[4..6].copy_from_slice(&revision.to_le_bytes());
            out[6..8].copy_from_slice(&(processors as u16).to_le_bytes());
            (12, true)
        }
        8 => {
            let since_boot = door_sys::precise_time().unwrap_or(0);
            let count = processors.min(64) as usize;
            for cpu in 0..count {
                let at = cpu * 48;
                // IdleTime and KernelTime (which includes idle time).
                out[at..at + 8].copy_from_slice(&since_boot.to_le_bytes());
                out[at + 8..at + 16].copy_from_slice(&since_boot.to_le_bytes());
            }
            (count * 48, false)
        }
        206 => (8, true),
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if returned != 0 {
        let _ = copy(returned, &(bytes as u32) as *const u32 as u64, 4);
    }
    let fits = if exact {
        length as usize == bytes
    } else {
        length as usize >= bytes
    };
    if !fits {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    match super::guest::copy_bulk(buffer, out.as_ptr() as u64, bytes) {
        Ok(()) => STATUS_SUCCESS,
        Err(_) => STATUS_ACCESS_VIOLATION,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwQuerySystemInformation(
    class: u32,
    buffer: usize,
    length: u32,
    returned: usize,
) -> u32 {
    NtQuerySystemInformation(class, buffer, length, returned)
}

// ---- NtQueryInformationProcess and the Rtl memory helpers (P18) ----------

const STATUS_INFO_LENGTH_MISMATCH_P: u32 = 0xC000_0004;
const STATUS_INVALID_INFO_CLASS_P: u32 = 0xC000_0003;
const STATUS_INVALID_HANDLE_P: u32 = 0xC000_0008;
const STATUS_PORT_NOT_SET: u32 = 0xC000_0353;

/// `NtQueryInformationProcess` for the classes runtimes and PowerShell ask:
/// ProcessBasicInformation (0: still running, the PEB, every processor,
/// normal priority, the process ID and no known parent), ProcessDebugPort
/// (7: none), ProcessWow64Information (26: the PEB32 address for a WOW64
/// process, 0 otherwise), ProcessDebugObjectHandle (30: no debugger) and
/// ProcessDebugFlags (31: not debugged). The current process or a
/// process handle this one holds; other classes are
/// STATUS_INVALID_INFO_CLASS.
#[unsafe(no_mangle)]
pub extern "system" fn NtQueryInformationProcess(
    process: usize,
    class: u32,
    info: usize,
    length: u32,
    returned: usize,
) -> u32 {
    let process = door_windows_abi::widen(process);
    let info = door_windows_abi::widen(info);
    let returned = door_windows_abi::widen(returned);
    let width = core::mem::size_of::<usize>();
    let pid = if process == door_windows_abi::CURRENT_PROCESS {
        u64::from(super::guest::DoorGetCurrentProcessId())
    } else {
        match super::process::DoorGetProcessId(process) {
            0 => return STATUS_INVALID_HANDLE_P,
            id => u64::from(id),
        }
    };
    let mut out = [0u8; 48];
    let bytes = match class {
        0 => {
            let processors = DoorProcessorCount();
            let mask = if processors >= 64 {
                u64::MAX
            } else {
                (1u64 << processors) - 1
            };
            // ExitStatus STATUS_PENDING (still running).
            let fields = [0x103u64, door_windows_abi::PEB_ADDRESS, mask, 8, pid, 0];
            for (i, value) in fields.into_iter().enumerate() {
                out[i * width..(i + 1) * width].copy_from_slice(&value.to_le_bytes()[..width]);
            }
            6 * width
        }
        7 | 26 => width,
        30 => return STATUS_PORT_NOT_SET,
        31 => {
            out[..4].copy_from_slice(&1u32.to_le_bytes());
            4
        }
        _ => return STATUS_INVALID_INFO_CLASS_P,
    };
    if returned != 0 {
        let _ = copy(returned, &(bytes as u32) as *const u32 as u64, 4);
    }
    if (length as usize) < bytes {
        return STATUS_INFO_LENGTH_MISMATCH_P;
    }
    match copy(info, out.as_ptr() as u64, bytes) {
        Ok(()) => 0,
        Err(_) => 0xC000_0005,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwQueryInformationProcess(
    process: usize,
    class: u32,
    info: usize,
    length: u32,
    returned: usize,
) -> u32 {
    NtQueryInformationProcess(process, class, info, length, returned)
}
/// `RtlZeroMemory`, `RtlFillMemory`, `RtlMoveMemory` and `RtlCopyMemory`:
/// exported functions as on Windows (the SDK's macros call them only from
/// languages without the macros, as .NET does through P/Invoke).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlZeroMemory(destination: usize, length: usize) {
    unsafe { core::ptr::write_bytes(destination as *mut u8, 0, length) };
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlFillMemory(destination: usize, length: usize, fill: u8) {
    unsafe { core::ptr::write_bytes(destination as *mut u8, fill, length) };
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlMoveMemory(destination: usize, source: usize, length: usize) {
    unsafe { core::ptr::copy(source as *const u8, destination as *mut u8, length) };
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RtlCopyMemory(destination: usize, source: usize, length: usize) {
    unsafe { core::ptr::copy(source as *const u8, destination as *mut u8, length) };
}
/// The process's placeholder compatibility mode (cloud-file placeholders,
/// RtlSetProcessPlaceholderCompatibilityMode): PHCM_APPLICATION_DEFAULT
/// until set. Doors has no placeholders, so the mode changes nothing.
static PLACEHOLDER_MODE: core::sync::atomic::AtomicI8 = core::sync::atomic::AtomicI8::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn RtlQueryProcessPlaceholderCompatibilityMode() -> i8 {
    PLACEHOLDER_MODE.load(core::sync::atomic::Ordering::Relaxed)
}
/// The previous mode, or PHCM_ERROR_INVALID_PARAMETER (-1) for a mode
/// other than default (0), disguise (1) or expose (2).
#[unsafe(no_mangle)]
pub extern "system" fn RtlSetProcessPlaceholderCompatibilityMode(mode: i8) -> i8 {
    if !(0..=2).contains(&mode) {
        return -1;
    }
    PLACEHOLDER_MODE.swap(mode, core::sync::atomic::Ordering::Relaxed)
}
/// `ApiSetQueryApiSetPresence(namespace, present)`: whether an API set
/// (a UNICODE_STRING such as "api-ms-win-core-file-l1-2-0") resolves to a
/// host DLL in this system.
#[unsafe(no_mangle)]
pub extern "system" fn ApiSetQueryApiSetPresence(namespace: usize, present: usize) -> u32 {
    let (namespace, present) = (
        door_windows_abi::widen(namespace),
        door_windows_abi::widen(present),
    );
    if namespace == 0 || present == 0 {
        return 0xc000_000d;
    }
    // UNICODE_STRING: Length (bytes), MaximumLength, Buffer.
    let length = usize::from(unsafe { (namespace as *const u16).read_unaligned() }) / 2;
    let offset = if cfg!(target_pointer_width = "64") {
        8
    } else {
        4
    };
    let buffer = unsafe { ((namespace + offset) as *const usize).read_unaligned() } as u64;
    let mut name = [0u8; 96];
    let mut ok = buffer != 0 && length <= name.len();
    for (i, out) in name.iter_mut().take(length.min(96)).enumerate() {
        let unit = unsafe { ((buffer + 2 * i as u64) as *const u16).read_unaligned() };
        match u8::try_from(unit) {
            Ok(b) => *out = b.to_ascii_lowercase(),
            Err(_) => ok = false,
        }
    }
    let found = ok
        && core::str::from_utf8(&name[..length])
            .ok()
            .and_then(door_pe_loader::api_set_host)
            .is_some();
    unsafe { (present as *mut u8).write(u8::from(found)) };
    0
}
