//! System information, the Windows folders and the WOW64 queries (P17),
//! for the x64 and the x86 build alike. A 32-bit program is told what
//! WOW64 tells it (measured, `tests/fixtures/windows-x86/x86probe.c`): its
//! process runs under WOW64 on an x64 machine, the system folder is still
//! named System32, the 32-bit DLLs' own folder is SysWOW64.
use door_windows_abi as abi;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetSystemInfo(native: u32, out: u64) -> u32;
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorGetProcessId(handle: u64) -> u32;
    fn DoorGetCurrentProcessId() -> u32;
    fn DoorProcessMachine(handle: u64) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}

/// IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_AMD64.
const MACHINE_I386: u16 = 0x14c;
const MACHINE_AMD64: u16 = 0x8664;
/// This build runs under WOW64 (the 32-bit one).
const WOW64: bool = cfg!(target_arch = "x86");

fn write(destination: u64, bytes: &[u8]) -> bool {
    destination != 0
        && unsafe { DoorUserCopy(destination, bytes.as_ptr() as u64, bytes.len()) } == 0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemInfo(info: usize) {
    unsafe { DoorGetSystemInfo(0, abi::widen(info)) };
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNativeSystemInfo(info: usize) {
    unsafe { DoorGetSystemInfo(1, abi::widen(info)) };
}
/// Whether `process` is the current process (its pseudo-handle or a
/// handle naming it).
pub(crate) fn current(process: u64) -> bool {
    if process == abi::CURRENT_PROCESS {
        return true;
    }
    let id = unsafe { DoorGetProcessId(process) };
    id != 0 && id == unsafe { DoorGetCurrentProcessId() }
}
/// Whether `process` runs under WOW64: this process when it is the 32-bit
/// build, another by the kernel's record of its image (a 64-bit process
/// asking about a 32-bit one is told TRUE too, as on Windows). None when
/// the handle names no process (the last error is set).
fn wow64_process(process: u64) -> Option<bool> {
    if current(process) {
        return Some(WOW64);
    }
    match unsafe { DoorProcessMachine(process) } {
        0 => None,
        machine => Some(machine == u32::from(MACHINE_I386)),
    }
}
/// IsWow64Process: TRUE for a 32-bit process on this 64-bit system.
#[unsafe(no_mangle)]
pub extern "system" fn IsWow64Process(process: usize, result: usize) -> i32 {
    let process = abi::widen(process);
    let Some(wow) = wow64_process(process) else {
        return 0;
    };
    if !write(abi::widen(result), &i32::from(wow).to_le_bytes()) {
        unsafe { RtlSetLastWin32Error(998) };
        return 0;
    }
    1
}
/// IsWow64Process2: the process's machine under WOW64 (I386), else
/// IMAGE_FILE_MACHINE_UNKNOWN, and the native machine (AMD64).
#[unsafe(no_mangle)]
pub extern "system" fn IsWow64Process2(
    process: usize,
    process_machine: usize,
    native_machine: usize,
) -> i32 {
    let process = abi::widen(process);
    let Some(wow) = wow64_process(process) else {
        return 0;
    };
    let machine = if wow { MACHINE_I386 } else { 0 };
    if !write(abi::widen(process_machine), &machine.to_le_bytes())
        || (native_machine != 0 && !write(abi::widen(native_machine), &MACHINE_AMD64.to_le_bytes()))
    {
        unsafe { RtlSetLastWin32Error(998) };
        return 0;
    }
    1
}
// The file system redirector's switches. A 32-bit process's thread turns
// redirection off and back on (ntdll keeps the state in its TEB32; the
// previous state is the opaque PVOID Windows hands back, NULL for "on");
// in a 64-bit process there is no redirector and Windows fails all three
// with ERROR_INVALID_FUNCTION, leaving `old` untouched (measured).
#[cfg(target_arch = "x86")]
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlWow64EnableFsRedirectionEx(disable: usize, old: usize) -> u32;
}
#[cfg(target_arch = "x86")]
fn redirection(disable: bool, old: u64) -> bool {
    match unsafe {
        RtlWow64EnableFsRedirectionEx(
            door_windows_abi::narrow(u64::from(disable)),
            door_windows_abi::narrow(old),
        )
    } {
        0 => true,
        _ => {
            unsafe { RtlSetLastWin32Error(998) };
            false
        }
    }
}
#[cfg(target_arch = "x86_64")]
fn redirection(_disable: bool, _old: u64) -> bool {
    unsafe { RtlSetLastWin32Error(1) };
    false
}
#[unsafe(no_mangle)]
pub extern "system" fn Wow64DisableWow64FsRedirection(old: usize) -> i32 {
    if WOW64 && old == 0 {
        unsafe { RtlSetLastWin32Error(87) };
        return 0;
    }
    i32::from(redirection(true, abi::widen(old)))
}
#[unsafe(no_mangle)]
pub extern "system" fn Wow64RevertWow64FsRedirection(old: usize) -> i32 {
    i32::from(redirection(old != 0, 0))
}
#[unsafe(no_mangle)]
pub extern "system" fn Wow64EnableWow64FsRedirection(enable: u8) -> u8 {
    u8::from(redirection(enable == 0, 0))
}
/// A folder name into a UTF-16 buffer of `size` units: its length on
/// success, else the size it needs (terminator included), as the
/// Get*DirectoryW functions answer.
fn folder(text: &str, buffer: usize, size: u32) -> u32 {
    let units: alloc_free::Units = alloc_free::Units::of(text);
    let length = units.len() as u32;
    if size <= length || buffer == 0 {
        return length + 1;
    }
    let mut bytes = [0u8; 2 * 64];
    for (index, unit) in units.as_slice().iter().chain([&0u16]).enumerate() {
        bytes[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    if !write(abi::widen(buffer), &bytes[..(units.len() + 1) * 2]) {
        unsafe { RtlSetLastWin32Error(998) };
        return 0;
    }
    length
}
/// The ANSI form (the folders are ASCII).
fn folder_a(text: &str, buffer: usize, size: u32) -> u32 {
    let length = text.len() as u32;
    if size <= length || buffer == 0 {
        return length + 1;
    }
    let mut bytes = [0u8; 64];
    bytes[..text.len()].copy_from_slice(text.as_bytes());
    if !write(abi::widen(buffer), &bytes[..text.len() + 1]) {
        unsafe { RtlSetLastWin32Error(998) };
        return 0;
    }
    length
}
const SYSTEM: &str = "C:\\Windows\\System32";
const WOW64_SYSTEM: &str = "C:\\Windows\\SysWOW64";
const WINDOWS: &str = "C:\\Windows";
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDirectoryW(buffer: usize, size: u32) -> u32 {
    folder(SYSTEM, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDirectoryA(buffer: usize, size: u32) -> u32 {
    folder_a(SYSTEM, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemWow64DirectoryW(buffer: usize, size: u32) -> u32 {
    folder(WOW64_SYSTEM, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemWow64DirectoryA(buffer: usize, size: u32) -> u32 {
    folder_a(WOW64_SYSTEM, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowsDirectoryW(buffer: usize, size: u32) -> u32 {
    folder(WINDOWS, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowsDirectoryA(buffer: usize, size: u32) -> u32 {
    folder_a(WINDOWS, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemWindowsDirectoryW(buffer: usize, size: u32) -> u32 {
    folder(WINDOWS, buffer, size)
}

mod alloc_free {
    /// A short ASCII text as UTF-16 units.
    pub struct Units([u16; 63], usize);
    impl Units {
        pub fn of(text: &str) -> Self {
            let mut units = [0u16; 63];
            let mut count = 0;
            for unit in text.encode_utf16().take(63) {
                units[count] = unit;
                count += 1;
            }
            Self(units, count)
        }
        pub fn len(&self) -> usize {
            self.1
        }
        pub fn as_slice(&self) -> &[u16] {
            &self.0[..self.1]
        }
    }
}
