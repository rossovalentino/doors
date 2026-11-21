//! File mappings and placeholders (P18): CreateFileMapping (W, A, Numa,
//! FromApp, 2), OpenFileMapping, MapViewOfFile (Ex, ExNuma, FromApp, 3,
//! Numa2), UnmapViewOfFile (Ex, 2), FlushViewOfFile and VirtualAlloc2,
//! over ntdll's sections (`DoorCreateFileMapping`, `DoorMapViewOfFile`).
//! Only the current process's memory is reachable: another process's
//! handle fails with ERROR_ACCESS_DENIED, as for the other *Ex calls.
use door_windows_abi as abi;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateFileMapping(file: u64, protect: u32, maximum: u64, name: u64) -> u64;
    fn DoorOpenFileMapping(access: u32, name: u64) -> u64;
    fn DoorMapViewOfFile(
        section: u64,
        access: u32,
        offset: u64,
        bytes: u64,
        address: u64,
        allocation: u32,
        protection: u32,
    ) -> u64;
    fn DoorUnmapViewOfFile(address: u64, flags: u32) -> i32;
    fn DoorFlushViewOfFile(address: u64, bytes: u64) -> i32;
    fn DoorVirtualAlloc2(
        address: u64,
        bytes: u64,
        flags: u32,
        protection: u32,
        requirements: u64,
    ) -> u64;
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}
const INVALID_PARAMETER: u32 = 87;
const ACCESS_DENIED: u32 = 5;

fn error(value: u32) -> usize {
    unsafe { RtlSetLastWin32Error(value) };
    0
}
fn foreign(process: usize) -> bool {
    if crate::system::current(abi::widen(process)) {
        return false;
    }
    unsafe { RtlSetLastWin32Error(ACCESS_DENIED) };
    true
}
/// An ANSI name widened (0 stays 0), then `f` with the wide pointer.
fn with_wide(name: usize, f: impl FnOnce(usize) -> usize) -> usize {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(abi::widen(name), &mut units) {
        Ok(wide) => f(abi::narrow(wide)),
        Err(code) => error(code),
    }
}
fn join(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateFileMappingW(
    file: usize,
    _attributes: usize,
    protect: u32,
    maximum_high: u32,
    maximum_low: u32,
    name: usize,
) -> usize {
    abi::narrow(unsafe {
        DoorCreateFileMapping(
            abi::widen(file),
            protect,
            join(maximum_high, maximum_low),
            abi::widen(name),
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFileMappingA(
    file: usize,
    attributes: usize,
    protect: u32,
    maximum_high: u32,
    maximum_low: u32,
    name: usize,
) -> usize {
    with_wide(name, |wide| {
        CreateFileMappingW(file, attributes, protect, maximum_high, maximum_low, wide)
    })
}
/// One NUMA node: node 0 or NUMA_NO_PREFERRED_NODE.
fn node_ok(node: u32) -> bool {
    node == 0 || node == u32::MAX
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFileMappingNumaW(
    file: usize,
    attributes: usize,
    protect: u32,
    maximum_high: u32,
    maximum_low: u32,
    name: usize,
    node: u32,
) -> usize {
    if !node_ok(node) {
        return error(INVALID_PARAMETER);
    }
    CreateFileMappingW(file, attributes, protect, maximum_high, maximum_low, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFileMappingNumaA(
    file: usize,
    attributes: usize,
    protect: u32,
    maximum_high: u32,
    maximum_low: u32,
    name: usize,
    node: u32,
) -> usize {
    if !node_ok(node) {
        return error(INVALID_PARAMETER);
    }
    CreateFileMappingA(file, attributes, protect, maximum_high, maximum_low, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateFileMappingFromApp(
    file: usize,
    attributes: usize,
    protect: u32,
    maximum: u64,
    name: usize,
) -> usize {
    CreateFileMappingW(
        file,
        attributes,
        protect,
        (maximum >> 32) as u32,
        maximum as u32,
        name,
    )
}
/// CreateFileMapping2: the protection and SEC_* attributes apart; the
/// extended parameters may name NUMA node 0 only.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateFileMapping2(
    file: usize,
    attributes: usize,
    _access: u32,
    protect: u32,
    allocation: u32,
    maximum: u64,
    name: usize,
    parameters: usize,
    count: u32,
) -> usize {
    match extended(parameters, count) {
        Ok(None) => CreateFileMappingFromApp(file, attributes, protect | allocation, maximum, name),
        _ => error(INVALID_PARAMETER),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenFileMappingW(access: u32, _inherit: i32, name: usize) -> usize {
    abi::narrow(unsafe { DoorOpenFileMapping(access, abi::widen(name)) })
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenFileMappingA(access: u32, inherit: i32, name: usize) -> usize {
    with_wide(name, |wide| OpenFileMappingW(access, inherit, wide))
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenFileMappingFromApp(access: u32, inherit: i32, name: usize) -> usize {
    OpenFileMappingW(access, inherit, name)
}

fn view(section: usize, access: u32, offset: u64, bytes: usize, address: usize) -> usize {
    abi::narrow(unsafe {
        DoorMapViewOfFile(
            abi::widen(section),
            access,
            offset,
            abi::widen(bytes),
            abi::widen(address),
            0,
            0,
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn MapViewOfFile(
    section: usize,
    access: u32,
    offset_high: u32,
    offset_low: u32,
    bytes: usize,
) -> usize {
    view(section, access, join(offset_high, offset_low), bytes, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn MapViewOfFileEx(
    section: usize,
    access: u32,
    offset_high: u32,
    offset_low: u32,
    bytes: usize,
    address: usize,
) -> usize {
    view(
        section,
        access,
        join(offset_high, offset_low),
        bytes,
        address,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn MapViewOfFileExNuma(
    section: usize,
    access: u32,
    offset_high: u32,
    offset_low: u32,
    bytes: usize,
    address: usize,
    node: u32,
) -> usize {
    if !node_ok(node) {
        return error(INVALID_PARAMETER);
    }
    view(
        section,
        access,
        join(offset_high, offset_low),
        bytes,
        address,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn MapViewOfFileFromApp(
    section: usize,
    access: u32,
    offset: u64,
    bytes: usize,
) -> usize {
    view(section, access, offset, bytes, 0)
}

/// MEM_EXTENDED_PARAMETER entries (16 bytes each): MEM_ADDRESS_REQUIREMENTS
/// (type 1) as a 24-byte (lowest, highest, alignment) record in `out`,
/// NUMA node 0 (type 2) and attribute flags 0 (type 5) accepted; anything
/// else is invalid. Some(record) when requirements were given.
fn extended(parameters: usize, count: u32) -> Result<Option<[u64; 3]>, ()> {
    let mut requirements = None;
    for i in 0..count as usize {
        let mut entry = [0u8; 16];
        let at = abi::widen(parameters) + 16 * i as u64;
        if unsafe { DoorUserCopy(entry.as_mut_ptr() as u64, at, 16) } != 0 {
            return Err(());
        }
        let kind = entry[0];
        let value = u64::from_le_bytes(entry[8..16].try_into().unwrap());
        let value = if cfg!(target_arch = "x86") {
            value & 0xFFFF_FFFF
        } else {
            value
        };
        match kind {
            1 if value != 0 => {
                // MEM_ADDRESS_REQUIREMENTS: three pointer-sized fields.
                let width = core::mem::size_of::<usize>();
                let mut raw = [0u8; 24];
                if unsafe { DoorUserCopy(raw.as_mut_ptr() as u64, value, 3 * width) } != 0 {
                    return Err(());
                }
                let field = |j: usize| {
                    let mut b = [0u8; 8];
                    b[..width].copy_from_slice(&raw[j * width..(j + 1) * width]);
                    u64::from_le_bytes(b)
                };
                requirements = Some([field(0), field(1), field(2)]);
            }
            2 if value == 0 || value == u64::from(u32::MAX) => {}
            5 if value == 0 => {}
            _ => return Err(()),
        }
    }
    Ok(requirements)
}

/// MapViewOfFile3: `allocation` MEM_REPLACE_PLACEHOLDER (over a
/// placeholder of exactly the view's range) or MEM_RESERVE; `protection`
/// the view's PAGE_* protection.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn MapViewOfFile3(
    section: usize,
    process: usize,
    address: usize,
    offset: u64,
    bytes: usize,
    allocation: u32,
    protection: u32,
    parameters: usize,
    count: u32,
) -> usize {
    if foreign(process) {
        return 0;
    }
    match extended(parameters, count) {
        Ok(None) => {}
        _ => return error(INVALID_PARAMETER),
    }
    abi::narrow(unsafe {
        DoorMapViewOfFile(
            abi::widen(section),
            0,
            offset,
            abi::widen(bytes),
            abi::widen(address),
            allocation,
            protection,
        )
    })
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn MapViewOfFile3FromApp(
    section: usize,
    process: usize,
    address: usize,
    offset: u64,
    bytes: usize,
    allocation: u32,
    protection: u32,
    parameters: usize,
    count: u32,
) -> usize {
    MapViewOfFile3(
        section, process, address, offset, bytes, allocation, protection, parameters, count,
    )
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn MapViewOfFileNuma2(
    section: usize,
    process: usize,
    offset: u64,
    address: usize,
    bytes: usize,
    allocation: u32,
    protection: u32,
    node: u32,
) -> usize {
    if !node_ok(node) {
        return error(INVALID_PARAMETER);
    }
    MapViewOfFile3(
        section, process, address, offset, bytes, allocation, protection, 0, 0,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn UnmapViewOfFile(address: usize) -> i32 {
    unsafe { DoorUnmapViewOfFile(abi::widen(address), 0) }
}
#[unsafe(no_mangle)]
pub extern "system" fn UnmapViewOfFileEx(address: usize, flags: u32) -> i32 {
    unsafe { DoorUnmapViewOfFile(abi::widen(address), flags) }
}
#[unsafe(no_mangle)]
pub extern "system" fn UnmapViewOfFile2(process: usize, address: usize, flags: u32) -> i32 {
    if foreign(process) {
        return 0;
    }
    unsafe { DoorUnmapViewOfFile(abi::widen(address), flags) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FlushViewOfFile(address: usize, bytes: usize) -> i32 {
    unsafe { DoorFlushViewOfFile(abi::widen(address), abi::widen(bytes)) }
}

/// VirtualAlloc2: placeholders and address requirements (see ntdll's
/// `DoorVirtualAlloc2`).
#[unsafe(no_mangle)]
pub extern "system" fn VirtualAlloc2(
    process: usize,
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
    parameters: usize,
    count: u32,
) -> usize {
    if process != 0 && foreign(process) {
        return 0;
    }
    let requirements = match extended(parameters, count) {
        Ok(r) => r,
        Err(()) => return error(INVALID_PARAMETER),
    };
    let record = requirements.unwrap_or([0; 3]);
    let pointer = if requirements.is_some() {
        record.as_ptr() as u64
    } else {
        0
    };
    abi::narrow(unsafe {
        DoorVirtualAlloc2(
            abi::widen(address),
            abi::widen(bytes),
            flags,
            protection,
            pointer,
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualAlloc2FromApp(
    process: usize,
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
    parameters: usize,
    count: u32,
) -> usize {
    VirtualAlloc2(
        process, address, bytes, flags, protection, parameters, count,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualAllocFromApp(
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
) -> usize {
    VirtualAlloc2(0, address, bytes, flags, protection, 0, 0)
}
/// DiscardVirtualMemory: the contents may be discarded; they are kept
/// here (always allowed), as MEM_RESET keeps them.
#[unsafe(no_mangle)]
pub extern "system" fn DiscardVirtualMemory(_address: usize, _bytes: usize) -> u32 {
    0
}
