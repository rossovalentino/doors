//! VirtualQuery, the *Ex memory functions for the current process, and the
//! code-buffer flushes JIT runtimes call (P18 M0b). Another process's
//! memory is not reachable: those calls fail with ERROR_ACCESS_DENIED.
use door_windows_abi::{self as abi, memory as vm};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorVirtualAlloc(address: u64, bytes: u64, flags: u32, protection: u32) -> u64;
    fn DoorVirtualFree(address: u64, bytes: u64, flags: u32) -> i32;
    fn DoorVirtualProtect(address: u64, bytes: u64, protection: u32, previous: u64) -> i32;
    fn DoorVirtualQuery(address: u64, buffer: u64, length: u64) -> u64;
    fn DoorFlushProcessWriteBuffers();
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}
const ACCESS_DENIED: u32 = 5;
const BAD_LENGTH: u32 = 24;
const NOACCESS: u32 = 998;

fn foreign(process: u64) -> bool {
    if crate::system::current(process) {
        return false;
    }
    unsafe { RtlSetLastWin32Error(ACCESS_DENIED) };
    true
}
/// The MEMORY_BASIC_INFORMATION of this build's width at `buffer`: the
/// bytes written, or 0 with the last error set.
fn query(address: u64, buffer: u64, length: u64) -> u64 {
    let mut record = [0u8; vm::QUERY_BYTES];
    if unsafe { DoorVirtualQuery(address, record.as_mut_ptr() as u64, record.len() as u64) } == 0 {
        return 0;
    }
    let info = vm::BasicInformation::decode(&record);
    let narrow;
    let bytes: &[u8] = if cfg!(target_arch = "x86") {
        match info.encode32() {
            Some(b) => {
                narrow = b;
                &narrow
            }
            None => {
                unsafe { RtlSetLastWin32Error(87) };
                return 0;
            }
        }
    } else {
        &record
    };
    if length < bytes.len() as u64 {
        unsafe { RtlSetLastWin32Error(BAD_LENGTH) };
        return 0;
    }
    if buffer == 0 || unsafe { DoorUserCopy(buffer, bytes.as_ptr() as u64, bytes.len()) } != 0 {
        unsafe { RtlSetLastWin32Error(NOACCESS) };
        return 0;
    }
    bytes.len() as u64
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualQuery(address: usize, buffer: usize, length: usize) -> usize {
    query(abi::widen(address), abi::widen(buffer), abi::widen(length)) as usize
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualQueryEx(
    process: usize,
    address: usize,
    buffer: usize,
    length: usize,
) -> usize {
    if foreign(abi::widen(process)) {
        return 0;
    }
    VirtualQuery(address, buffer, length)
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualAllocEx(
    process: usize,
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
) -> usize {
    let process = abi::widen(process);
    let address = abi::widen(address);
    let bytes = abi::widen(bytes);
    abi::narrow((move || -> u64 {
        if foreign(process) {
            return 0;
        }
        unsafe { DoorVirtualAlloc(address, bytes, flags, protection) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualFreeEx(
    process: usize,
    address: usize,
    bytes: usize,
    flags: u32,
) -> i32 {
    if foreign(abi::widen(process)) {
        return 0;
    }
    unsafe { DoorVirtualFree(abi::widen(address), abi::widen(bytes), flags) }
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualProtectEx(
    process: usize,
    address: usize,
    bytes: usize,
    protection: u32,
    previous: usize,
) -> i32 {
    if foreign(abi::widen(process)) {
        return 0;
    }
    unsafe {
        DoorVirtualProtect(
            abi::widen(address),
            abi::widen(bytes),
            protection,
            abi::widen(previous),
        )
    }
}
/// x86 keeps instruction fetches coherent with stores; a page a JIT wrote
/// through its writable side is retired by the kernel when it turns
/// executable. Nothing remains to flush.
#[unsafe(no_mangle)]
pub extern "system" fn FlushInstructionCache(
    process: usize,
    _address: usize,
    _bytes: usize,
) -> i32 {
    if foreign(abi::widen(process)) {
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn FlushProcessWriteBuffers() {
    unsafe { DoorFlushProcessWriteBuffers() }
}
/// One NUMA node here: the node is checked and otherwise ignored.
#[unsafe(no_mangle)]
pub extern "system" fn VirtualAllocExNuma(
    process: usize,
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
    node: u32,
) -> usize {
    if node != 0 {
        unsafe { RtlSetLastWin32Error(87) };
        return 0;
    }
    VirtualAllocEx(process, address, bytes, flags, protection)
}
