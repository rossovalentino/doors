//! Machine, time and volume queries .NET and PowerShell make (P18, from the
//! runtime's P/Invoke list): local time and the time zone, process and
//! thread times, the unbiased interrupt time, the computer name, drives
//! and the C: volume, long path names, and the file and I/O calls whose
//! Windows behaviour on Doors' storage is "nothing to do".
//!
//! Doors keeps time in UTC and has no time-zone setting for Windows
//! programs yet, so the zone is Windows' "UTC" (Coordinated Universal
//! Time, no daylight saving) and local time equals system time.
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorQueryPerformanceCounter() -> u64;
    fn DoorVolumeUsage(out: u64) -> i32;
    fn RtlSetLastWin32Error(value: u32);
}

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
/// 100 ns ticks per minute (time-zone biases are in minutes).
const TICKS_PER_MINUTE: i64 = 600_000_000;

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
/// A NUL-terminated wide string, bounded, into `out`: its length.
fn read_wide(pointer: usize, out: &mut [u16; 260]) -> usize {
    if pointer == 0 {
        return 0;
    }
    let mut n = 0;
    while n < out.len() {
        let unit = unsafe { (pointer as *const u16).add(n).read_unaligned() };
        if unit == 0 {
            break;
        }
        out[n] = unit;
        n += 1;
    }
    n
}
/// `text` and its NUL into the caller's buffer of `capacity` units.
fn put_wide(buffer: usize, text: &[u16]) {
    for (i, unit) in text.iter().chain([&0]).enumerate() {
        unsafe { (buffer as *mut u16).add(i).write_unaligned(*unit) };
    }
}

// ---- time ---------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetLocalTime(destination: usize) {
    crate::application::GetSystemTime(destination);
}
/// Setting the clock needs SeSystemtimePrivilege, which no Doors program
/// holds.
#[unsafe(no_mangle)]
pub extern "system" fn SetLocalTime(_time: usize) -> i32 {
    fail(ERROR_PRIVILEGE_NOT_HELD)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetSystemTime(_time: usize) -> i32 {
    fail(ERROR_PRIVILEGE_NOT_HELD)
}
/// TIME_ZONE_INFORMATION (172 bytes) of the UTC zone, as Windows fills it.
fn utc_zone() -> [u8; 172] {
    let mut zone = [0u8; 172];
    let name: [u16; 26] = {
        let mut units = [0u16; 26];
        for (slot, unit) in units
            .iter_mut()
            .zip("Coordinated Universal Time".encode_utf16())
        {
            *slot = unit;
        }
        units
    };
    for at in [4, 88] {
        for (i, unit) in name.iter().enumerate() {
            zone[at + 2 * i..at + 2 * i + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    zone
}
/// `GetTimeZoneInformation`: TIME_ZONE_ID_UNKNOWN (0), the zone having no
/// daylight saving.
#[unsafe(no_mangle)]
pub extern "system" fn GetTimeZoneInformation(zone: usize) -> u32 {
    if zone == 0 {
        fail(ERROR_INVALID_PARAMETER);
        return u32::MAX;
    }
    write(zone, utc_zone());
    0
}
/// `GetDynamicTimeZoneInformation`: the zone with its registry key name
/// ("UTC") and dynamic daylight time not disabled.
#[unsafe(no_mangle)]
pub extern "system" fn GetDynamicTimeZoneInformation(zone: usize) -> u32 {
    if zone == 0 {
        fail(ERROR_INVALID_PARAMETER);
        return u32::MAX;
    }
    let mut dynamic = [0u8; 432];
    dynamic[..172].copy_from_slice(&utc_zone());
    for (i, unit) in "UTC".encode_utf16().enumerate() {
        dynamic[172 + 2 * i..174 + 2 * i].copy_from_slice(&unit.to_le_bytes());
    }
    write(zone, dynamic);
    0
}
/// The bias (minutes, UTC = local + bias) a TIME_ZONE_INFORMATION gives:
/// its standard bias (daylight rules are not applied), or none for the
/// current zone (NULL).
fn bias(zone: usize) -> i64 {
    if zone == 0 {
        return 0;
    }
    let read = |at: usize| i64::from(unsafe { ((zone + at) as *const i32).read_unaligned() });
    read(0) + read(84)
}
/// SYSTEMTIME + `minutes` into another SYSTEMTIME, through FILETIME.
fn shift(source: usize, destination: usize, minutes: i64) -> i32 {
    if source == 0 || destination == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut filetime = 0u64;
    if crate::application::SystemTimeToFileTime(source, &mut filetime as *mut u64 as usize) == 0 {
        return 0;
    }
    let moved = (filetime as i64).saturating_add(minutes.saturating_mul(TICKS_PER_MINUTE));
    if moved < 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let moved = moved as u64;
    crate::application::FileTimeToSystemTime(&moved as *const u64 as usize, destination)
}
#[unsafe(no_mangle)]
pub extern "system" fn TzSpecificLocalTimeToSystemTime(
    zone: usize,
    local: usize,
    universal: usize,
) -> i32 {
    shift(local, universal, bias(zone))
}
#[unsafe(no_mangle)]
pub extern "system" fn SystemTimeToTzSpecificLocalTime(
    zone: usize,
    universal: usize,
    local: usize,
) -> i32 {
    shift(universal, local, -bias(zone))
}
#[unsafe(no_mangle)]
pub extern "system" fn FileTimeToLocalFileTime(source: usize, destination: usize) -> i32 {
    if source == 0 || destination == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let value = unsafe { (source as *const u64).read_unaligned() };
    write(destination, value);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalFileTimeToFileTime(source: usize, destination: usize) -> i32 {
    FileTimeToLocalFileTime(source, destination)
}
/// `QueryUnbiasedInterruptTime`: 100 ns since boot (Doors' clock does not
/// count suspended time).
#[unsafe(no_mangle)]
pub extern "system" fn QueryUnbiasedInterruptTime(time: usize) -> i32 {
    if time == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    write(time, unsafe { DoorQueryPerformanceCounter() });
    1
}
/// The current FILETIME.
fn now() -> u64 {
    let mut filetime = 0u64;
    crate::application::GetSystemTimeAsFileTime(&mut filetime as *mut u64 as usize);
    filetime
}
/// `GetProcessTimes` / `GetThreadTimes`: creation is the machine's boot
/// (Doors records no per-process start time for Windows programs yet), no
/// exit, and no kernel or user time accounted.
fn times(creation: usize, exit: usize, kernel: usize, user: usize) -> i32 {
    let boot = now().saturating_sub(unsafe { DoorQueryPerformanceCounter() });
    write(creation, boot);
    write(exit, 0u64);
    write(kernel, 0u64);
    write(user, 0u64);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessTimes(
    _process: usize,
    creation: usize,
    exit: usize,
    kernel: usize,
    user: usize,
) -> i32 {
    times(creation, exit, kernel, user)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadTimes(
    _thread: usize,
    creation: usize,
    exit: usize,
    kernel: usize,
    user: usize,
) -> i32 {
    times(creation, exit, kernel, user)
}

// ---- the computer --------------------------------------------------------------

/// The computer's name (COMPUTERNAME, as the session's environment carries it).
fn computer_name(out: &mut [u16; 260]) -> usize {
    let name: [u16; 13] = {
        let mut units = [0u16; 13];
        for (slot, unit) in units.iter_mut().zip("COMPUTERNAME\0".encode_utf16()) {
            *slot = unit;
        }
        units
    };
    let n = crate::environment::GetEnvironmentVariableW(
        name.as_ptr() as usize,
        out.as_mut_ptr() as usize,
        out.len() as u32,
    ) as usize;
    if n == 0 || n >= out.len() {
        let fallback = "DOORS-PC";
        for (slot, unit) in out.iter_mut().zip(fallback.encode_utf16()) {
            *slot = unit;
        }
        return fallback.len();
    }
    n
}
/// The name into `buffer`, `size` in and out (units, as GetComputerName
/// counts them: without the NUL on success, with it when too small).
fn give_name(text: &[u16], buffer: usize, size: usize) -> i32 {
    if size == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let capacity = unsafe { (size as *const u32).read_unaligned() } as usize;
    if buffer == 0 || capacity <= text.len() {
        write(size, (text.len() + 1) as u32);
        return fail(ERROR_BUFFER_OVERFLOW);
    }
    put_wide(buffer, text);
    write(size, text.len() as u32);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetComputerNameW(buffer: usize, size: usize) -> i32 {
    let mut name = [0u16; 260];
    let n = computer_name(&mut name);
    give_name(&name[..n], buffer, size)
}
/// `GetComputerNameExW`: the host names are the computer name; Doors joins
/// no DNS domain.
#[unsafe(no_mangle)]
pub extern "system" fn GetComputerNameExW(kind: u32, buffer: usize, size: usize) -> i32 {
    let mut name = [0u16; 260];
    let n = match kind {
        0 | 1 | 3 | 4 | 5 | 7 => computer_name(&mut name),
        2 | 6 => 0,
        _ => return fail(ERROR_INVALID_PARAMETER),
    };
    give_name(&name[..n], buffer, size)
}

// ---- drives and the volume ------------------------------------------------------

/// Whether `root` names drive C (`C:`, `C:\`, any case); NULL is the
/// current directory's drive, always C:.
fn is_c(root: usize) -> bool {
    if root == 0 {
        return true;
    }
    let mut units = [0u16; 260];
    let n = read_wide(root, &mut units);
    matches!(&units[..n], [67 | 99, 58] | [67 | 99, 58, 92])
}
/// `GetLogicalDrives`: C: only.
#[unsafe(no_mangle)]
pub extern "system" fn GetLogicalDrives() -> u32 {
    1 << 2
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLogicalDriveStringsW(length: u32, buffer: usize) -> u32 {
    let text = [67u16, 58, 92, 0, 0];
    if (length as usize) < text.len() || buffer == 0 {
        return text.len() as u32;
    }
    for (i, unit) in text.iter().enumerate() {
        unsafe { (buffer as *mut u16).add(i).write_unaligned(*unit) };
    }
    4
}
/// `GetDriveTypeW`: DRIVE_FIXED (3) for C:, DRIVE_NO_ROOT_DIR (1) else.
#[unsafe(no_mangle)]
pub extern "system" fn GetDriveTypeW(root: usize) -> u32 {
    if is_c(root) { 3 } else { 1 }
}
/// `GetDiskFreeSpaceExW`: C:'s volume (any folder on it), from storage's
/// own accounting; the caller's quota is the whole volume.
#[unsafe(no_mangle)]
pub extern "system" fn GetDiskFreeSpaceExW(
    path: usize,
    available: usize,
    total: usize,
    free: usize,
) -> i32 {
    if path != 0 && crate::application::GetFileAttributesW(path) == u32::MAX {
        return 0;
    }
    let mut usage = [0u64; 2];
    if unsafe { DoorVolumeUsage(usage.as_mut_ptr() as u64) } == 0 {
        return 0;
    }
    write(available, usage[1]);
    write(total, usage[0]);
    write(free, usage[1]);
    1
}
/// `GetDiskFreeSpaceW`: the same in 4 KiB clusters of 8 sectors.
#[unsafe(no_mangle)]
pub extern "system" fn GetDiskFreeSpaceW(
    root: usize,
    sectors_per_cluster: usize,
    bytes_per_sector: usize,
    free_clusters: usize,
    total_clusters: usize,
) -> i32 {
    if !is_c(root) {
        return fail(ERROR_PATH_NOT_FOUND);
    }
    let mut usage = [0u64; 2];
    if unsafe { DoorVolumeUsage(usage.as_mut_ptr() as u64) } == 0 {
        return 0;
    }
    write(sectors_per_cluster, 8u32);
    write(bytes_per_sector, 512u32);
    write(
        free_clusters,
        (usage[1] / 4096).min(u64::from(u32::MAX)) as u32,
    );
    write(
        total_clusters,
        (usage[0] / 4096).min(u64::from(u32::MAX)) as u32,
    );
    1
}
/// The C: volume's serial number (fixed: Doors formats one volume).
const VOLUME_SERIAL: u32 = 0xD005_2026;
/// Name-length limit and FILE_CASE_SENSITIVE_SEARCH, CASE_PRESERVED_NAMES,
/// UNICODE_ON_DISK, PERSISTENT_ACLS and NAMED_STREAMS: what Doors' volume
/// keeps.
const VOLUME_FLAGS: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x4_0000;
/// `GetVolumeInformationW` for C:\ (an empty label, as Windows' C: has).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn GetVolumeInformationW(
    root: usize,
    name: usize,
    name_size: u32,
    serial: usize,
    component: usize,
    flags: usize,
    file_system: usize,
    file_system_size: u32,
) -> i32 {
    if !is_c(root) {
        return fail(ERROR_PATH_NOT_FOUND);
    }
    let ntfs = [78u16, 84, 70, 83];
    if (name != 0 && name_size < 1)
        || (file_system != 0 && (file_system_size as usize) <= ntfs.len())
    {
        return fail(122);
    }
    if name != 0 {
        put_wide(name, &[]);
    }
    if file_system != 0 {
        put_wide(file_system, &ntfs);
    }
    write(serial, VOLUME_SERIAL);
    write(component, 255u32);
    write(flags, VOLUME_FLAGS);
    1
}
/// Labels are Administrators' to change.
#[unsafe(no_mangle)]
pub extern "system" fn SetVolumeLabelW(_root: usize, _label: usize) -> i32 {
    fail(5)
}
/// `GetVolumePathNameW`: every path is on C:\.
#[unsafe(no_mangle)]
pub extern "system" fn GetVolumePathNameW(_path: usize, buffer: usize, length: u32) -> i32 {
    if buffer == 0 || length < 4 {
        return fail(206);
    }
    put_wide(buffer, &[67, 58, 92]);
    1
}

// ---- paths and files ---------------------------------------------------------------

/// `GetLongPathNameW` / `GetShortPathNameW`: Doors' names have no 8.3
/// aliases, so an existing path is its own long and short name. The
/// length without the NUL, or the size needed with it.
fn same_path(source: usize, buffer: usize, length: u32) -> u32 {
    if source == 0 {
        fail(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if crate::application::GetFileAttributesW(source) == u32::MAX {
        return 0;
    }
    let mut units = [0u16; 260];
    let n = read_wide(source, &mut units);
    if buffer == 0 || (length as usize) <= n {
        return n as u32 + 1;
    }
    if buffer != source {
        put_wide(buffer, &units[..n]);
    }
    n as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLongPathNameW(source: usize, buffer: usize, length: u32) -> u32 {
    same_path(source, buffer, length)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetShortPathNameW(source: usize, buffer: usize, length: u32) -> u32 {
    same_path(source, buffer, length)
}
/// `SetFileAttributesW`: storage keeps attributes itself (as
/// SetFileInformationByHandle's FileBasicInfo), so an existing file
/// accepts any.
#[unsafe(no_mangle)]
pub extern "system" fn SetFileAttributesW(path: usize, _attributes: u32) -> i32 {
    if crate::application::GetFileAttributesW(path) == u32::MAX {
        return 0;
    }
    1
}
/// `ReplaceFileW`: the replacement takes the replaced file's name (its
/// backup, when named, keeps the old contents).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn ReplaceFileW(
    replaced: usize,
    replacement: usize,
    backup: usize,
    _flags: u32,
    _exclude: usize,
    _reserved: usize,
) -> i32 {
    if crate::application::GetFileAttributesW(replaced) == u32::MAX {
        return 0;
    }
    if backup != 0 && crate::namespace::MoveFileExW(replaced, backup, 1) == 0 {
        return 0;
    }
    // MOVEFILE_REPLACE_EXISTING.
    crate::namespace::MoveFileExW(replacement, replaced, 1)
}
/// Symbolic links need SeCreateSymbolicLinkPrivilege (or Developer Mode),
/// which Doors' programs do not hold; hard links are not kept by Doors'
/// volume.
#[unsafe(no_mangle)]
pub extern "system" fn CreateSymbolicLinkW(_link: usize, _target: usize, _flags: u32) -> u8 {
    fail(ERROR_PRIVILEGE_NOT_HELD);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateHardLinkW(_link: usize, _target: usize, _security: usize) -> i32 {
    fail(50)
}
/// Byte-range locks: a Doors file's sharing mode already protects it, so
/// a range lock always succeeds (single-process view).
#[unsafe(no_mangle)]
pub extern "system" fn LockFile(
    handle: usize,
    _low: u32,
    _high: u32,
    _bytes_low: u32,
    _bytes_high: u32,
) -> i32 {
    if handle == 0 || handle == usize::MAX {
        return fail(6);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn UnlockFile(
    handle: usize,
    _low: u32,
    _high: u32,
    _bytes_low: u32,
    _bytes_high: u32,
) -> i32 {
    LockFile(handle, 0, 0, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn LockFileEx(
    handle: usize,
    _flags: u32,
    _reserved: u32,
    _bytes_low: u32,
    _bytes_high: u32,
    _overlapped: usize,
) -> i32 {
    LockFile(handle, 0, 0, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn UnlockFileEx(
    handle: usize,
    _reserved: u32,
    _bytes_low: u32,
    _bytes_high: u32,
    _overlapped: usize,
) -> i32 {
    LockFile(handle, 0, 0, 0, 0)
}
/// No device answers control codes on Doors' file handles.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DeviceIoControl(
    _device: usize,
    _code: u32,
    _input: usize,
    _input_bytes: u32,
    _output: usize,
    _output_bytes: u32,
    _returned: usize,
    _overlapped: usize,
) -> i32 {
    fail(ERROR_INVALID_FUNCTION)
}
/// Doors' synchronous I/O completes without a cancellable wait.
#[unsafe(no_mangle)]
pub extern "system" fn CancelSynchronousIo(_thread: usize) -> i32 {
    fail(ERROR_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadIOPendingFlag(_thread: usize, pending: usize) -> i32 {
    if pending == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    write(pending, 0i32);
    1
}
/// FILE_SKIP_COMPLETION_PORT_ON_SUCCESS and FILE_SKIP_SET_EVENT_ON_HANDLE:
/// accepted (completions are still posted, which callers handle).
#[unsafe(no_mangle)]
pub extern "system" fn SetFileCompletionNotificationModes(handle: usize, _flags: u8) -> i32 {
    if handle == 0 || handle == usize::MAX {
        return fail(6);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn Beep(_frequency: u32, _duration: u32) -> i32 {
    1
}
/// Not a file on this volume.
#[unsafe(no_mangle)]
pub extern "system" fn DeleteVolumeMountPointW(_mount: usize) -> i32 {
    fail(ERROR_FILE_NOT_FOUND)
}

// ---- packaged applications ----------------------------------------------------------

/// APPMODEL_ERROR_NO_PACKAGE: Doors runs no program as a packaged (MSIX)
/// application yet.
const APPMODEL_ERROR_NO_PACKAGE: i32 = 15700;
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageFamilyName(_length: usize, _name: usize) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageFullName(_length: usize, _name: usize) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentPackageId(_length: usize, _buffer: usize) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentApplicationUserModelId(_length: usize, _id: usize) -> i32 {
    APPMODEL_ERROR_NO_PACKAGE
}

/// `QueryUnbiasedInterruptTimePrecise` and `QueryInterruptTime(Precise)`:
/// the same 100 ns clock (Doors' clock is precise and never suspends).
#[unsafe(no_mangle)]
pub extern "system" fn QueryUnbiasedInterruptTimePrecise(time: usize) {
    if time != 0 {
        write(time, unsafe { DoorQueryPerformanceCounter() });
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryInterruptTimePrecise(time: usize) {
    QueryUnbiasedInterruptTimePrecise(time)
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryInterruptTime(time: usize) {
    QueryUnbiasedInterruptTimePrecise(time)
}

// ---- power (the api-ms-win-power-base contract, powrprof on Windows) ---------------------
/// `PowerDeterminePlatformRoleEx`: PlatformRoleDesktop (Doors reports no
/// battery-backed mobile role).
#[unsafe(no_mangle)]
pub extern "system" fn PowerDeterminePlatformRoleEx(version: u32) -> u32 {
    // POWER_PLATFORM_ROLE_V1 1, V2 2; PlatformRoleUnspecified 0 otherwise.
    if matches!(version, 1 | 2) { 1 } else { 0 }
}
#[unsafe(no_mangle)]
pub extern "system" fn PowerDeterminePlatformRole() -> u32 {
    1
}
/// `CallNtPowerInformation`: ProcessorInformation (11) as one
/// PROCESSOR_POWER_INFORMATION per processor (number, current and maximum
/// MHz unknown to Doors: 0, idle state 0) and SystemExecutionState (16);
/// other levels are not supported. Returns an NTSTATUS.
#[unsafe(no_mangle)]
pub extern "system" fn CallNtPowerInformation(
    level: u32,
    _input: usize,
    _input_length: u32,
    output: usize,
    output_length: u32,
) -> u32 {
    const STATUS_SUCCESS: u32 = 0;
    const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
    const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
    const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
    match level {
        11 => {
            let count = crate::runtime::GetActiveProcessorCount(0xFFFF).max(1);
            let bytes = count as usize * 24;
            if output == 0 {
                return STATUS_INVALID_PARAMETER;
            }
            if (output_length as usize) < bytes {
                return STATUS_BUFFER_TOO_SMALL;
            }
            for i in 0..count {
                let at = output + i as usize * 24;
                let row = [i, 0, 0, 0, 0, 0];
                for (j, v) in row.iter().enumerate() {
                    write(at + 4 * j, *v);
                }
            }
            STATUS_SUCCESS
        }
        16 => {
            if output == 0 || output_length < 4 {
                return STATUS_BUFFER_TOO_SMALL;
            }
            // ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED while the session runs.
            write(output, 0x3u32);
            STATUS_SUCCESS
        }
        _ => STATUS_NOT_IMPLEMENTED,
    }
}

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn NtCompareObjects(first: usize, second: usize) -> u32;
}
/// `CompareObjectHandles`: both handles name the same kernel object.
#[unsafe(no_mangle)]
pub extern "system" fn CompareObjectHandles(first: usize, second: usize) -> i32 {
    i32::from(unsafe { NtCompareObjects(first, second) } == 0)
}
