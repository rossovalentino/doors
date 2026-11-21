//! iphlpapi.dll exports over door.network's status (ntdll's `DoorNet*`).
//! Caller buffers are written only through ntdll's checked copy; tables the
//! API allocates (`GetIfTable2`, ...) come from the process heap and are
//! released by `FreeMibTable`. Change notifications poll the adapter status
//! on a worker thread and call back on it, as Windows calls back on a
//! worker thread.
use crate::*;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(value: u32);
    fn DoorNetStatus(out: u64) -> u32;
    fn DoorNetStatus6(out: u64) -> u32;
    fn DoorNetTunnel(out: u64) -> u32;
    fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorCreateEventW(attributes: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorNetPing(request: u64, length: u32, out: u64) -> u32;
    fn DoorNetConnections(out: u64, capacity: u32, total: u64) -> u32;
    fn DoorNetConnections6(out: u64, capacity: u32, total: u64) -> u32;
    fn DoorNetStatistics(out: u64) -> u32;
    fn DoorNetNeighbors(out: u64, capacity: u32, total: u64) -> u32;
}

const ERROR_IO_PENDING: u32 = 997;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;

// ---- application memory ------------------------------------------------
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if out.is_empty() {
        return Ok(());
    }
    if pointer == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    // A console program's checked copies move at most 4 KiB each.
    const PIECE: usize = door_windows_abi::APPLICATION_COPY_BYTES;
    for (i, piece) in out.chunks_mut(PIECE).enumerate() {
        let at = pointer + (i * PIECE) as u64;
        if unsafe { DoorUserCopy(piece.as_mut_ptr() as u64, at, piece.len()) } != 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if data.is_empty() {
        return Ok(());
    }
    if pointer == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    const PIECE: usize = door_windows_abi::APPLICATION_COPY_BYTES;
    for (i, piece) in data.chunks(PIECE).enumerate() {
        let at = pointer + (i * PIECE) as u64;
        if unsafe { DoorUserCopy(at, piece.as_ptr() as u64, piece.len()) } != 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
fn write_u64(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// A pointer-sized program value (P17: 4 bytes on x86, sign-extended).
const POINTER: usize = door_windows_abi::layout::POINTER;
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// OVERLAPPED.hEvent: after Internal, InternalHigh, Offset and OffsetHigh.
const OVERLAPPED_EVENT: u64 = (2 * POINTER + 8) as u64;
/// A NUL-terminated string at `pointer` (UTF-16 when `wide`), at most
/// `max` units.
fn text(pointer: u64, wide: bool, max: usize) -> Result<String, u32> {
    if pointer == 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let unit = if wide { 2 } else { 1 };
    let mut units = Vec::new();
    for i in 0..max {
        let mut b = [0u8; 2];
        read(pointer + (i * unit) as u64, &mut b[..unit])?;
        let u = if wide {
            u16::from_le_bytes(b)
        } else {
            u16::from(b[0])
        };
        if u == 0 {
            return String::from_utf16(&units).map_err(|_| ERROR_INVALID_PARAMETER);
        }
        units.push(u);
    }
    Err(ERROR_INVALID_PARAMETER)
}
/// Writes `value` NUL-terminated into a buffer of `capacity` units.
fn store(pointer: u64, capacity: usize, value: &str, wide: bool) -> Result<(), u32> {
    let bytes: Vec<u8> = if wide {
        value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    } else {
        value.bytes().chain([0]).collect()
    };
    let units = if wide { bytes.len() / 2 } else { bytes.len() };
    if units > capacity {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    write(pointer, &bytes)
}

// ---- the model ----------------------------------------------------------
fn status() -> Option<door_service_protocol::net::Status> {
    let mut b = [0u8; door_service_protocol::net::STATUS_BYTES];
    (unsafe { DoorNetStatus(b.as_mut_ptr() as u64) } == 0)
        .then(|| door_service_protocol::net::Status::decode(&b))
        .flatten()
}
fn status6() -> Option<door_service_protocol::net::v6::Status6> {
    let mut b = [0u8; door_service_protocol::net::v6::STATUS_BYTES];
    (unsafe { DoorNetStatus6(b.as_mut_ptr() as u64) } == 0)
        .then(|| door_service_protocol::net::v6::Status6::decode(&b))
        .flatten()
}
fn tunnel() -> Option<door_service_protocol::net::vpn::Tunnel> {
    let mut b = [0u8; door_service_protocol::net::vpn::TUNNEL_BYTES];
    (unsafe { DoorNetTunnel(b.as_mut_ptr() as u64) } == 0)
        .then(|| door_service_protocol::net::vpn::Tunnel::decode(&b))
        .flatten()
}
fn current() -> Vec<Interface> {
    let mut list = interfaces(status().as_ref(), status6().as_ref());
    if let Some(t) = tunnel() {
        add_tunnel(&mut list, &t);
    }
    list
}
fn host_name() -> String {
    let mut b = [0u8; 64];
    let mut length = 0u32;
    if unsafe { DoorGetComputerName(b.as_mut_ptr() as u64, 64, &mut length as *mut u32 as u64) }
        != 0
    {
        return String::from("localhost");
    }
    String::from_utf8_lossy(&b[..length as usize]).into_owned()
}
/// Copies `data` into a caller buffer of `*size` bytes, or reports the size
/// it needs (`ERROR_BUFFER_OVERFLOW`/`ERROR_INSUFFICIENT_BUFFER`).
fn deliver(buffer: u64, size: u64, data: &[u8], overflow: u32) -> u32 {
    let Ok(capacity) = read_u32(size) else {
        return ERROR_INVALID_PARAMETER;
    };
    if buffer == 0 || (capacity as usize) < data.len() {
        return write_u32(size, data.len() as u32).map_or(ERROR_INVALID_PARAMETER, |()| overflow);
    }
    write(buffer, data).map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS)
}

// ---- tables allocated for the caller -------------------------------------
/// Process-heap memory with its size in a 16-byte header, freed by
/// `FreeMibTable`.
fn table(data: &[u8]) -> u64 {
    let layout = core::alloc::Layout::from_size_align(data.len() + 16, 16).expect("table layout");
    let raw = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if raw.is_null() {
        return 0;
    }
    unsafe {
        (raw as *mut usize).write(data.len());
        core::ptr::copy_nonoverlapping(data.as_ptr(), raw.add(16), data.len());
        raw.add(16) as u64
    }
}
fn give(out: u64, data: &[u8]) -> u32 {
    let pointer = table(data);
    if pointer == 0 {
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    if write_pointer(out, pointer).is_err() {
        FreeMibTable(door_windows_abi::narrow(pointer));
        return ERROR_INVALID_PARAMETER;
    }
    ERROR_SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeMibTable(memory: usize) {
    let memory = door_windows_abi::widen(memory);
    {
        if memory == 0 {
            return;
        }
        unsafe {
            let raw = (memory - 16) as *mut u8;
            let size = (raw as *const usize).read();
            alloc::alloc::dealloc(
                raw,
                core::alloc::Layout::from_size_align_unchecked(size + 16, 16),
            );
        }
    }
}
/// A MIB_*_TABLE2 header (count, padding to 8) followed by `rows`.
fn counted(rows: &[Vec<u8>]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 8];
    b[..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        b.extend_from_slice(row);
    }
    b
}
fn family_ok(family: u32) -> Result<bool, u32> {
    match family as u16 {
        AF_UNSPEC | AF_INET if family >> 16 == 0 => Ok(true),
        AF_INET6 if family >> 16 == 0 => Ok(false),
        _ => Err(ERROR_INVALID_PARAMETER),
    }
}

// ---- adapters -------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn GetAdaptersAddresses(
    family: u32,
    flags: u32,
    reserved: usize,
    buffer: usize,
    size: usize,
) -> u32 {
    let reserved = door_windows_abi::widen(reserved);
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if reserved != 0 || size == 0 || family >> 16 != 0 {
            return ERROR_INVALID_PARAMETER;
        }
        match adapters_addresses(buffer, family as u16, flags, &current()) {
            Ok(data) => deliver(buffer, size, &data, ERROR_BUFFER_OVERFLOW),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAdaptersInfo(buffer: usize, size: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        match adapters_info(buffer, &current()) {
            Ok(data) => deliver(buffer, size, &data, ERROR_BUFFER_OVERFLOW),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNetworkParams(buffer: usize, size: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        let dns: Vec<[u8; 4]> = current().iter().flat_map(|i| i.dns.clone()).collect();
        deliver(
            buffer,
            size,
            &fixed_info(buffer, &host_name(), &dns),
            ERROR_BUFFER_OVERFLOW,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumberOfInterfaces(count: usize) -> u32 {
    let count = door_windows_abi::widen(count);
    write_u32(count, current().len() as u32).map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAdapterIndex(name: usize, index: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    let index = door_windows_abi::widen(index);
    {
        let result = (|| {
            let name = text(name, true, 512)?;
            let list = current();
            let found = list
                .iter()
                .find(|i| {
                    let guid = i.guid_text();
                    name.eq_ignore_ascii_case(&guid)
                        || name
                            .to_ascii_uppercase()
                            .ends_with(&alloc::format!("TCPIP_{guid}"))
                })
                .ok_or(ERROR_FILE_NOT_FOUND)?;
            write_u32(index, found.index)
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFriendlyIfIndex(index: u32) -> u32 {
    index & 0x00ff_ffff
}

// ---- interfaces -------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn GetIfTable(buffer: usize, size: usize, order: i32) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        let list = current();
        let rows: Vec<Interface> = if order != 0 {
            ordered(&list).into_iter().cloned().collect()
        } else {
            list
        };
        deliver(buffer, size, &if_table(&rows), ERROR_INSUFFICIENT_BUFFER)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIfEntry(row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let result = (|| {
            let index = read_u32(row + 512)?;
            let list = current();
            let interface = by_index(&list, index).ok_or(ERROR_INVALID_PARAMETER)?;
            write(row, &if_row(interface))
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIfTable2(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let list = current();
        give(
            out,
            &if_table2(&ordered(&list).into_iter().cloned().collect::<Vec<_>>()),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIfTable2Ex(level: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if level > 1 {
            return ERROR_INVALID_PARAMETER;
        }
        GetIfTable2(door_windows_abi::narrow(out))
    }
}
/// The interface a MIB row names: its LUID when set, else its index.
fn row_interface(list: &[Interface], luid_at: u64, index_at: u64) -> Result<&Interface, u32> {
    let luid = read_u64(luid_at)?;
    if luid != 0 {
        return by_luid(list, luid).ok_or(ERROR_FILE_NOT_FOUND);
    }
    let index = read_u32(index_at)?;
    by_index(list, index).ok_or(ERROR_FILE_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIfEntry2(row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let result = (|| {
            let list = current();
            let interface = row_interface(&list, row, row + 8)?;
            write(row, &if_row2(interface))
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIfEntry2Ex(level: u32, row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        if level > 1 {
            return ERROR_INVALID_PARAMETER;
        }
        GetIfEntry2(door_windows_abi::narrow(row))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpInterfaceEntry(row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let result = (|| {
            let mut family = [0u8; 2];
            read(row, &mut family)?;
            match u16::from_le_bytes(family) {
                AF_INET => (),
                AF_INET6 => return Err(ERROR_NOT_FOUND),
                _ => return Err(ERROR_INVALID_PARAMETER),
            }
            let list = current();
            let interface = row_interface(&list, row + 8, row + 16)?;
            write(row, &ip_interface_row(interface))
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpInterfaceTable(family: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        match family_ok(family) {
            Err(e) => e,
            Ok(ipv4) => {
                let list = current();
                let rows: Vec<Vec<u8>> = if ipv4 {
                    ordered(&list)
                        .iter()
                        .map(|i| ip_interface_row(i).to_vec())
                        .collect()
                } else {
                    Vec::new()
                };
                give(out, &counted(&rows))
            }
        }
    }
}

// ---- addresses and routes -------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn GetIpAddrTable(buffer: usize, size: usize, order: i32) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        deliver(
            buffer,
            size,
            &ip_addr_table(&current(), order != 0),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUnicastIpAddressTable(family: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        match family_ok(family) {
            Err(e) => e,
            Ok(ipv4) => {
                let list = current();
                let rows: Vec<Vec<u8>> = if ipv4 {
                    list.iter()
                        .filter_map(unicast_row)
                        .map(|r| r.to_vec())
                        .collect()
                } else {
                    Vec::new()
                };
                give(out, &counted(&rows))
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUnicastIpAddressEntry(row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let result = (|| {
            let mut address = [0u8; 28];
            read(row, &mut address)?;
            if u16::from_le_bytes([address[0], address[1]]) != AF_INET {
                return Err(ERROR_NOT_FOUND);
            }
            let list = current();
            let interface = row_interface(&list, row + 32, row + 40)?;
            match unicast_row(interface) {
                Some(found) if found[4..8] == address[4..8] => write(row, &found),
                _ => Err(ERROR_NOT_FOUND),
            }
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpForwardTable(buffer: usize, size: usize, order: i32) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        deliver(
            buffer,
            size,
            &ip_forward_table(&current(), order != 0),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpForwardTable2(family: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        match family_ok(family) {
            Err(e) => e,
            Ok(ipv4) => {
                let list = current();
                let rows: Vec<Vec<u8>> = if ipv4 {
                    routes(&list)
                        .iter()
                        .filter_map(|r| {
                            by_index(&list, r.index).map(|i| forward_row2(r, i).to_vec())
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                give(out, &counted(&rows))
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetBestInterface(destination: u32, index: usize) -> u32 {
    let index = door_windows_abi::widen(index);
    {
        match best_interface(&current(), destination.to_le_bytes()) {
            Ok(found) => {
                write_u32(index, found).map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS)
            }
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetBestInterfaceEx(address: usize, index: usize) -> u32 {
    let address = door_windows_abi::widen(address);
    let index = door_windows_abi::widen(index);
    {
        let mut sa = [0u8; 8];
        if read(address, &mut sa).is_err() {
            return ERROR_INVALID_PARAMETER;
        }
        if u16::from_le_bytes([sa[0], sa[1]]) != AF_INET {
            return ERROR_NETWORK_UNREACHABLE;
        }
        GetBestInterface(
            u32::from_le_bytes([sa[4], sa[5], sa[6], sa[7]]),
            door_windows_abi::narrow(index),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetBestRoute(destination: u32, _source: u32, row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let list = current();
        let address = destination.to_le_bytes();
        let Ok(index) = best_interface(&list, address) else {
            return ERROR_NETWORK_UNREACHABLE;
        };
        // The longest matching prefix of the routing table on that interface.
        let table = ip_forward_table(&list, false);
        let count = u32::from_le_bytes(table[..4].try_into().unwrap()) as usize;
        let best = (0..count)
            .map(|i| &table[4 + i * 56..4 + (i + 1) * 56])
            .filter(|r| u32::from_le_bytes(r[16..20].try_into().unwrap()) == index)
            .filter(|r| {
                let (dest, mask) = (
                    u32::from_be_bytes(r[..4].try_into().unwrap()),
                    u32::from_be_bytes(r[4..8].try_into().unwrap()),
                );
                u32::from_be_bytes(address) & mask == dest
            })
            .max_by_key(|r| u32::from_be_bytes(r[4..8].try_into().unwrap()).count_ones());
        match best {
            Some(found) => write(row, found).map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS),
            None => ERROR_NETWORK_UNREACHABLE,
        }
    }
}

// ---- interface names and identifiers ------------------------------------------
fn luid_at(pointer: u64) -> Result<u64, u32> {
    read_u64(pointer)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceLuidToIndex(luid: usize, index: usize) -> u32 {
    let luid = door_windows_abi::widen(luid);
    let index = door_windows_abi::widen(index);
    {
        let result = (|| {
            let list = current();
            let interface = by_luid(&list, luid_at(luid)?).ok_or(ERROR_FILE_NOT_FOUND)?;
            write_u32(index, interface.index)
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceIndexToLuid(index: u32, luid: usize) -> u32 {
    let luid = door_windows_abi::widen(luid);
    {
        let list = current();
        match by_index(&list, index) {
            Some(interface) => write_u64(luid, interface.luid())
                .map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS),
            None => {
                let _ = write_u64(luid, 0);
                ERROR_FILE_NOT_FOUND
            }
        }
    }
}
fn luid_to(
    luid: u64,
    out: u64,
    capacity: usize,
    wide: bool,
    pick: fn(&Interface) -> String,
) -> u32 {
    let result = (|| {
        let list = current();
        let interface = by_luid(&list, luid_at(luid)?).ok_or(ERROR_FILE_NOT_FOUND)?;
        store(out, capacity, &pick(interface), wide)
    })();
    result.err().unwrap_or(ERROR_SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceLuidToNameW(luid: usize, name: usize, length: usize) -> u32 {
    let luid = door_windows_abi::widen(luid);
    let name = door_windows_abi::widen(name);
    luid_to(luid, name, length, true, Interface::name)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceLuidToNameA(luid: usize, name: usize, length: usize) -> u32 {
    let luid = door_windows_abi::widen(luid);
    let name = door_windows_abi::widen(name);
    luid_to(luid, name, length, false, Interface::name)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceLuidToAlias(
    luid: usize,
    alias: usize,
    length: usize,
) -> u32 {
    let luid = door_windows_abi::widen(luid);
    let alias = door_windows_abi::widen(alias);
    luid_to(luid, alias, length, true, |i| i.alias.clone())
}
fn to_luid(
    name: u64,
    wide: bool,
    luid: u64,
    find: for<'a> fn(&'a [Interface], &str) -> Option<&'a Interface>,
) -> u32 {
    let result = (|| {
        let name = text(name, wide, 256)?;
        let list = current();
        let interface = find(&list, &name).ok_or(ERROR_INVALID_PARAMETER)?;
        write_u64(luid, interface.luid())
    })();
    result.err().unwrap_or(ERROR_SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceNameToLuidW(name: usize, luid: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    let luid = door_windows_abi::widen(luid);
    to_luid(name, true, luid, by_name)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceNameToLuidA(name: usize, luid: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    let luid = door_windows_abi::widen(luid);
    to_luid(name, false, luid, by_name)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceAliasToLuid(alias: usize, luid: usize) -> u32 {
    let alias = door_windows_abi::widen(alias);
    let luid = door_windows_abi::widen(luid);
    to_luid(alias, true, luid, by_alias)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceLuidToGuid(luid: usize, guid: usize) -> u32 {
    let luid = door_windows_abi::widen(luid);
    let guid = door_windows_abi::widen(guid);
    {
        let result = (|| {
            let list = current();
            let interface = by_luid(&list, luid_at(luid)?).ok_or(ERROR_FILE_NOT_FOUND)?;
            write(guid, &interface.guid)
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertInterfaceGuidToLuid(guid: usize, luid: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let luid = door_windows_abi::widen(luid);
    {
        let result = (|| {
            let mut g = [0u8; 16];
            read(guid, &mut g)?;
            let list = current();
            let interface = by_guid(&list, &g).ok_or(ERROR_FILE_NOT_FOUND)?;
            write_u64(luid, interface.luid())
        })();
        result.err().unwrap_or(ERROR_SUCCESS)
    }
}
/// `if_nametoindex`: the interface name (`ethernet_32768`); 0 when unknown.
#[unsafe(no_mangle)]
pub extern "system" fn if_nametoindex(name: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    {
        let list = current();
        text(name, false, 256)
            .ok()
            .and_then(|n| by_name(&list, &n).map(|i| i.index))
            .unwrap_or(0)
    }
}
/// `if_indextoname`: writes the name into an IF_NAMESIZE (256) buffer.
#[unsafe(no_mangle)]
pub extern "system" fn if_indextoname(index: u32, name: usize) -> usize {
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        let list = current();
        match by_index(&list, index) {
            Some(interface) if store(name, 256, &interface.name(), false).is_ok() => name,
            _ => 0,
        }
    })())
}

// ---- change notifications ----------------------------------------------------------
/// What a change watcher compares between polls: index, up, address,
/// gateway.
type State = (u32, bool, Option<([u8; 4], u8)>, Option<[u8; 4]>);
fn snapshot() -> Vec<State> {
    current()
        .iter()
        .map(|i| (i.index, i.up, i.ipv4, i.gateway))
        .collect()
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Interface,
    Unicast,
    Route,
    /// NotifyAddrChange/NotifyRouteChange: signal an event once.
    Event,
}
struct Watch {
    kind: Kind,
    callback: u64,
    context: u64,
    /// Overlapped event handle (Kind::Event).
    event: u64,
    overlapped: u64,
    live: AtomicBool,
    /// The worker has left (it frees nothing; Cancel waits on this).
    done: AtomicBool,
}
static WATCHES: AtomicU32 = AtomicU32::new(0);
/// PIPINTERFACE_CHANGE_CALLBACK and its kin: (context, row, type) at
/// pointer width (P17).
type Callback = extern "system" fn(usize, usize, u32);
const MIB_PARAMETER_NOTIFICATION: u32 = 0;
const MIB_ADD_INSTANCE: u32 = 1;
const MIB_DELETE_INSTANCE: u32 = 2;
const MIB_INITIAL_NOTIFICATION: u32 = 3;
fn notify(watch: &Watch, interface: Option<&Interface>, kind: u32) {
    let call: Callback =
        unsafe { core::mem::transmute::<usize, Callback>(watch.callback as usize) };
    let row: Option<Vec<u8>> = match (watch.kind, interface) {
        (Kind::Interface, Some(i)) => Some(ip_interface_row(i).to_vec()),
        (Kind::Unicast, Some(i)) => unicast_row(i).map(|r| r.to_vec()),
        (Kind::Route, Some(i)) => routes(core::slice::from_ref(i))
            .first()
            .map(|r| forward_row2(r, i).to_vec()),
        _ => None,
    };
    let pointer = row.as_ref().map_or(0, |r| r.as_ptr() as u64);
    call(watch.context as usize, pointer as usize, kind);
}
extern "system" fn watcher(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let watch = unsafe { &*(argument as *const Watch) };
        let mut before = snapshot();
        while watch.live.load(Ordering::Acquire) {
            unsafe { DoorSleep(500) };
            let now = snapshot();
            if now == before || !watch.live.load(Ordering::Acquire) {
                continue;
            }
            if watch.kind == Kind::Event {
                // Completed: STATUS_SUCCESS in the OVERLAPPED, then its event.
                if watch.overlapped != 0 {
                    let _ = write_pointer(watch.overlapped, 0);
                }
                unsafe { DoorSetEvent(watch.event) };
                break;
            }
            let list = current();
            for interface in &list {
                let old = before.iter().find(|s| s.0 == interface.index);
                let kind = match (watch.kind, old) {
                    (_, None) => MIB_ADD_INSTANCE,
                    (Kind::Unicast | Kind::Route, Some(o))
                        if o.2.is_none() && interface.ipv4.is_some() =>
                    {
                        MIB_ADD_INSTANCE
                    }
                    (Kind::Unicast | Kind::Route, Some(o))
                        if o.2.is_some() && interface.ipv4.is_none() =>
                    {
                        MIB_DELETE_INSTANCE
                    }
                    (_, Some(o))
                        if (o.1, o.2, o.3) != (interface.up, interface.ipv4, interface.gateway) =>
                    {
                        MIB_PARAMETER_NOTIFICATION
                    }
                    _ => continue,
                };
                // A deleted address is reported with the row it had.
                let mut reported = interface.clone();
                if kind == MIB_DELETE_INSTANCE {
                    reported.ipv4 = old.and_then(|o| o.2);
                }
                notify(watch, Some(&reported), kind);
            }
            before = now;
        }
        watch.done.store(true, Ordering::Release);
        0
    }
}
fn register(kind: Kind, family: u32, callback: u64, context: u64, initial: u8, handle: u64) -> u32 {
    if callback == 0 || handle == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    if let Err(e) = family_ok(family) {
        return e;
    }
    let watch = Box::leak(Box::new(Watch {
        kind,
        callback,
        context,
        event: 0,
        overlapped: 0,
        live: AtomicBool::new(true),
        done: AtomicBool::new(false),
    }));
    let pointer = watch as *const Watch as u64;
    if write_pointer(handle, pointer).is_err() {
        return ERROR_INVALID_PARAMETER;
    }
    if initial != 0 {
        notify(watch, None, MIB_INITIAL_NOTIFICATION);
    }
    let thread = unsafe { DoorCreateThreadEx(watcher as *const () as u64, pointer, 0) };
    if thread == 0 {
        watch.live.store(false, Ordering::Release);
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    unsafe { DoorCloseHandle(thread) };
    WATCHES.fetch_add(1, Ordering::Relaxed);
    ERROR_SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyIpInterfaceChange(
    family: u32,
    callback: usize,
    context: usize,
    initial: u8,
    handle: usize,
) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    let handle = door_windows_abi::widen(handle);
    register(Kind::Interface, family, callback, context, initial, handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyUnicastIpAddressChange(
    family: u32,
    callback: usize,
    context: usize,
    initial: u8,
    handle: usize,
) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    let handle = door_windows_abi::widen(handle);
    register(Kind::Unicast, family, callback, context, initial, handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyRouteChange2(
    family: u32,
    callback: usize,
    context: usize,
    initial: u8,
    handle: usize,
) -> u32 {
    let callback = door_windows_abi::widen(callback);
    let context = door_windows_abi::widen(context);
    let handle = door_windows_abi::widen(handle);
    register(Kind::Route, family, callback, context, initial, handle)
}
/// Stops a watcher and waits until its callback is no longer running (a
/// watch is never freed: a late worker may still read it).
#[unsafe(no_mangle)]
pub extern "system" fn CancelMibChangeNotify2(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        if handle == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        let watch = unsafe { &*(handle as *const Watch) };
        watch.live.store(false, Ordering::Release);
        while !watch.done.load(Ordering::Acquire) {
            unsafe { DoorSleep(10) };
        }
        WATCHES.fetch_sub(1, Ordering::Relaxed);
        ERROR_SUCCESS
    }
}
/// NotifyAddrChange/NotifyRouteChange: without OVERLAPPED, waits for the
/// next change; with one, returns ERROR_IO_PENDING and signals its event.
fn change(handle: u64, overlapped: u64) -> u32 {
    if overlapped == 0 {
        let before = snapshot();
        while snapshot() == before {
            unsafe { DoorSleep(500) };
        }
        return ERROR_SUCCESS;
    }
    let Ok(event) = read_pointer(overlapped + OVERLAPPED_EVENT) else {
        return ERROR_INVALID_PARAMETER;
    };
    let event = if event == 0 {
        unsafe { DoorCreateEventW(0, 1, 0, 0) }
    } else {
        event
    };
    let watch = Box::leak(Box::new(Watch {
        kind: Kind::Event,
        callback: 0,
        context: 0,
        event,
        overlapped,
        live: AtomicBool::new(true),
        done: AtomicBool::new(false),
    }));
    let pointer = watch as *const Watch as u64;
    // STATUS_PENDING in OVERLAPPED.Internal until the change.
    let _ = write_pointer(overlapped, 0x103);
    if handle != 0 && write_pointer(handle, event).is_err() {
        return ERROR_INVALID_PARAMETER;
    }
    let thread = unsafe { DoorCreateThreadEx(watcher as *const () as u64, pointer, 0) };
    if thread == 0 {
        return ERROR_NOT_ENOUGH_MEMORY;
    }
    unsafe { DoorCloseHandle(thread) };
    remember_overlapped(overlapped, pointer);
    ERROR_IO_PENDING
}
/// OVERLAPPED -> watch, for CancelIPChangeNotify.
static PENDING: spin::Table = spin::Table::new();
fn remember_overlapped(overlapped: u64, watch: u64) {
    PENDING.insert(overlapped, watch);
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyAddrChange(handle: usize, overlapped: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let overlapped = door_windows_abi::widen(overlapped);
    change(handle, overlapped)
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyRouteChange(handle: usize, overlapped: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let overlapped = door_windows_abi::widen(overlapped);
    change(handle, overlapped)
}
#[unsafe(no_mangle)]
pub extern "system" fn CancelIPChangeNotify(overlapped: usize) -> i32 {
    let overlapped = door_windows_abi::widen(overlapped);
    {
        match PENDING.remove(overlapped) {
            Some(watch) => {
                let watch = unsafe { &*(watch as *const Watch) };
                watch.live.store(false, Ordering::Release);
                i32::from(true)
            }
            None => {
                unsafe { RtlSetLastWin32Error(ERROR_NOT_FOUND) };
                0
            }
        }
    }
}
/// A tiny fixed table guarded by a spin lock (no allocation under the lock).
mod spin {
    use core::sync::atomic::{AtomicBool, Ordering};
    pub struct Table {
        lock: AtomicBool,
        slots: core::cell::UnsafeCell<[(u64, u64); 32]>,
    }
    unsafe impl Sync for Table {}
    impl Table {
        pub const fn new() -> Self {
            Self {
                lock: AtomicBool::new(false),
                slots: core::cell::UnsafeCell::new([(0, 0); 32]),
            }
        }
        fn with<T>(&self, f: impl FnOnce(&mut [(u64, u64); 32]) -> T) -> T {
            while self.lock.swap(true, Ordering::Acquire) {
                core::hint::spin_loop();
            }
            let result = f(unsafe { &mut *self.slots.get() });
            self.lock.store(false, Ordering::Release);
            result
        }
        pub fn insert(&self, key: u64, value: u64) {
            self.with(|slots| {
                if let Some(slot) = slots.iter_mut().find(|s| s.0 == 0 || s.0 == key) {
                    *slot = (key, value);
                }
            });
        }
        pub fn remove(&self, key: u64) -> Option<u64> {
            self.with(|slots| {
                let slot = slots.iter_mut().find(|s| s.0 == key && key != 0)?;
                let value = slot.1;
                *slot = (0, 0);
                Some(value)
            })
        }
    }
}

// ---- protocol statistics and the neighbour table -----------------------------
fn statistics() -> Result<door_service_protocol::net::stats::Counters, u32> {
    use door_service_protocol::net::stats::{COUNTERS_BYTES, Counters};
    let mut raw = [0u8; COUNTERS_BYTES];
    if unsafe { DoorNetStatistics(raw.as_mut_ptr() as u64) } != 0 {
        return Err(ERROR_NOT_SUPPORTED);
    }
    Counters::decode(&raw).ok_or(ERROR_GEN_FAILURE)
}
/// The family a *StatisticsEx call names: IPv4 or IPv6, anything else
/// ERROR_NOT_SUPPORTED as Windows answers.
fn stats_family(family: u32) -> Result<bool, u32> {
    match family {
        2 => Ok(false),
        23 => Ok(true),
        _ => Err(ERROR_NOT_SUPPORTED),
    }
}
fn stats(
    out: u64,
    family: u32,
    build: impl FnOnce(&door_service_protocol::net::stats::Counters, bool) -> Vec<u8>,
) -> u32 {
    if out == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let v6 = match stats_family(family) {
        Ok(v6) => v6,
        Err(e) => return e,
    };
    match statistics() {
        Ok(c) => write(out, &build(&c, v6)).map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS),
        Err(e) => e,
    }
}
fn pick(
    c: &door_service_protocol::net::stats::Counters,
    v6: bool,
) -> &door_service_protocol::net::stats::Family {
    if v6 { &c.v6 } else { &c.v4 }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpStatisticsEx(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    stats(out, family, |c, v6| ip_stats(pick(c, v6), &current(), v6))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIpStatistics(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    GetIpStatisticsEx(door_windows_abi::narrow(out), 2)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTcpStatisticsEx(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    stats(out, family, |c, v6| tcp_stats(pick(c, v6)))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTcpStatistics(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    GetTcpStatisticsEx(door_windows_abi::narrow(out), 2)
}
/// GetTcpStatisticsEx2: MIB_TCPSTATS2, the segment counts widened to 64 bits.
#[unsafe(no_mangle)]
pub extern "system" fn GetTcpStatisticsEx2(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        stats(out, family, |c, v6| {
            let f = pick(c, v6);
            let mut b = Vec::with_capacity(72);
            for v in [
                4,
                5,
                u32::MAX,
                u32::MAX,
                f.tcp_active_opens,
                f.tcp_passive_opens,
                f.tcp_attempt_fails,
                f.tcp_estab_resets,
                f.tcp_curr_estab,
            ] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&u64::from(f.tcp_in_segs).to_le_bytes());
            b.extend_from_slice(&u64::from(f.tcp_out_segs).to_le_bytes());
            for v in [
                f.tcp_retrans_segs,
                f.tcp_in_errs,
                f.tcp_out_rsts,
                f.tcp_num_conns,
            ] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUdpStatisticsEx(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    stats(out, family, |c, v6| udp_stats(pick(c, v6)))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUdpStatistics(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    GetUdpStatisticsEx(door_windows_abi::narrow(out), 2)
}
/// GetUdpStatisticsEx2: MIB_UDPSTATS2, the datagram counts widened.
#[unsafe(no_mangle)]
pub extern "system" fn GetUdpStatisticsEx2(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        stats(out, family, |c, v6| {
            let f = pick(c, v6);
            let mut b = Vec::with_capacity(32);
            b.extend_from_slice(&u64::from(f.udp_in_datagrams).to_le_bytes());
            b.extend_from_slice(&f.udp_no_ports.to_le_bytes());
            b.extend_from_slice(&f.udp_in_errors.to_le_bytes());
            b.extend_from_slice(&u64::from(f.udp_out_datagrams).to_le_bytes());
            b.extend_from_slice(&f.udp_num_addrs.to_le_bytes());
            b.extend_from_slice(&[0; 4]);
            b
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIcmpStatistics(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    stats(out, 2, |c, _| icmp_stats(&c.v4))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetIcmpStatisticsEx(out: usize, family: u32) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if !matches!(family, 2 | 23) {
            return ERROR_INVALID_PARAMETER;
        }
        stats(out, family, |c, v6| icmp_stats_ex(pick(c, v6), v6))
    }
}
/// door.network's neighbour table (the fixed entries included).
fn neighbors() -> Result<Vec<door_service_protocol::net::stats::Neighbor>, u32> {
    use door_service_protocol::net::stats::{NEIGHBOR_BYTES, Neighbor};
    let mut total = 0u32;
    if unsafe { DoorNetNeighbors(0, 0, &mut total as *mut u32 as u64) } != 0 {
        return Err(ERROR_NOT_SUPPORTED);
    }
    let mut raw = alloc::vec![0u8; total as usize * NEIGHBOR_BYTES];
    let mut again = 0u32;
    if unsafe {
        DoorNetNeighbors(
            raw.as_mut_ptr() as u64,
            total,
            &mut again as *mut u32 as u64,
        )
    } != 0
    {
        return Err(ERROR_NOT_SUPPORTED);
    }
    Ok(raw
        .chunks_exact(NEIGHBOR_BYTES)
        .filter_map(Neighbor::decode)
        .collect())
}
/// GetIpNetTable: the ARP table; ERROR_NO_DATA when it is empty.
#[unsafe(no_mangle)]
pub extern "system" fn GetIpNetTable(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        if size == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        let list = match neighbors() {
            Ok(list) => list,
            Err(e) => return e,
        };
        if !list.iter().any(|n| n.ipv4().is_some()) {
            return ERROR_NO_DATA;
        }
        deliver(
            table,
            size,
            &ip_net_table(&current(), &list, order != 0),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
/// GetIpNetTable2: MIB_IPNET_TABLE2 of IPv4, IPv6 or both families.
#[unsafe(no_mangle)]
pub extern "system" fn GetIpNetTable2(family: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return ERROR_INVALID_PARAMETER;
        }
        let (v4, v6) = match family {
            0 => (true, true),
            2 => (true, false),
            23 => (false, true),
            _ => return ERROR_INVALID_PARAMETER,
        };
        let list = match neighbors() {
            Ok(list) => list,
            Err(e) => return e,
        };
        let interfaces = current();
        let Some(ethernet) = interfaces.iter().find(|i| !i.loopback()) else {
            return give(out, &counted(&[]));
        };
        let rows: Vec<Vec<u8>> = list
            .iter()
            .filter(|n| if n.ipv4().is_some() { v4 } else { v6 })
            .map(|n| ip_net_row2(n, ethernet).to_vec())
            .collect();
        give(out, &counted(&rows))
    }
}
/// GetIpNetEntry2: one neighbour by address (and interface).
#[unsafe(no_mangle)]
pub extern "system" fn GetIpNetEntry2(row: usize) -> u32 {
    let row = door_windows_abi::widen(row);
    {
        let mut raw = [0u8; 88];
        if row == 0 || read(row, &mut raw).is_err() {
            return ERROR_INVALID_PARAMETER;
        }
        let family = u16::from_le_bytes([raw[0], raw[1]]);
        let wanted: [u8; 16] = match family {
            AF_INET => door_service_protocol::net::v6::mapped(raw[4..8].try_into().unwrap()),
            AF_INET6 => raw[8..24].try_into().unwrap(),
            _ => return ERROR_INVALID_PARAMETER,
        };
        let interfaces = current();
        let Some(ethernet) = interfaces.iter().find(|i| !i.loopback()) else {
            return ERROR_NOT_FOUND;
        };
        let list = match neighbors() {
            Ok(list) => list,
            Err(e) => return e,
        };
        match list.iter().find(|n| n.address == wanted) {
            Some(n) => write(row, &ip_net_row2(n, ethernet))
                .map_or(ERROR_INVALID_PARAMETER, |()| ERROR_SUCCESS),
            None => ERROR_NOT_FOUND,
        }
    }
}
// ---- TCP and UDP tables (netstat) -------------------------------------------
/// door.network's sockets.
fn connections() -> Vec<door_service_protocol::net::Connection> {
    use door_service_protocol::net::{CONNECTION_BYTES, Connection};
    let mut total = 0u32;
    if unsafe { DoorNetConnections(0, 0, &mut total as *mut u32 as u64) } != 0 {
        return Vec::new();
    }
    let mut raw = alloc::vec![0u8; total as usize * CONNECTION_BYTES];
    let mut again = 0u32;
    if unsafe {
        DoorNetConnections(
            raw.as_mut_ptr() as u64,
            total,
            &mut again as *mut u32 as u64,
        )
    } != 0
    {
        return Vec::new();
    }
    raw.chunks_exact(CONNECTION_BYTES)
        .filter_map(Connection::decode)
        .collect()
}
/// A port as the MIB rows store it: network byte order in the low word.
fn mib_port(port: u16) -> u32 {
    u32::from(port.to_be())
}
fn mib_address(octets: [u8; 4]) -> u32 {
    u32::from_ne_bytes(octets)
}
/// TCP rows of a TCP_TABLE_CLASS, in the stack's order or sorted.
fn tcp_rows(class: u32, order: bool) -> Result<Vec<Vec<u8>>, u32> {
    use door_service_protocol::net::tcp_state;
    let (listeners, established, pid, module) = match class {
        0 => (true, false, false, false),
        1 => (false, true, false, false),
        2 => (true, true, false, false),
        3 => (true, false, true, false),
        4 => (false, true, true, false),
        5 => (true, true, true, false),
        6 => (true, false, true, true),
        7 => (false, true, true, true),
        8 => (true, true, true, true),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let mut list: Vec<_> = connections()
        .into_iter()
        .filter(|c| c.protocol == 6)
        .filter(|c| {
            if c.state == tcp_state::LISTEN {
                listeners
            } else {
                established
            }
        })
        .collect();
    if order {
        list.sort_by_key(|c| (c.local, c.local_port, c.remote, c.remote_port));
    }
    Ok(list
        .iter()
        .map(|c| {
            let mut row = Vec::with_capacity(if module { 160 } else { 24 });
            for v in [
                u32::from(c.state),
                mib_address(c.local),
                mib_port(c.local_port),
                mib_address(c.remote),
                mib_port(c.remote_port),
            ] {
                row.extend_from_slice(&v.to_le_bytes());
            }
            if pid {
                row.extend_from_slice(&c.pid.to_le_bytes());
            }
            if module {
                // liCreateTimestamp and OwningModuleInfo: not tracked.
                row.resize(160, 0);
            }
            row
        })
        .collect())
}
/// A MIB_*TABLE: the entry count, then the rows (8-aligned for 160-byte
/// module rows).
fn mib_table(rows: &[Vec<u8>]) -> Vec<u8> {
    let aligned = rows
        .first()
        .is_some_and(|r| r.len() % 8 == 0 && r.len() > 24);
    let mut b = alloc::vec![0u8; if aligned { 8 } else { 4 }];
    b[..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        b.extend_from_slice(row);
    }
    b
}
/// door.network's AF_INET6 sockets.
fn connections6() -> Vec<door_service_protocol::net::Connection6> {
    use door_service_protocol::net::{CONNECTION6_BYTES, Connection6};
    let mut total = 0u32;
    if unsafe { DoorNetConnections6(0, 0, &mut total as *mut u32 as u64) } != 0 {
        return Vec::new();
    }
    let mut raw = alloc::vec![0u8; total as usize * CONNECTION6_BYTES];
    let mut again = 0u32;
    if unsafe {
        DoorNetConnections6(
            raw.as_mut_ptr() as u64,
            total,
            &mut again as *mut u32 as u64,
        )
    } != 0
    {
        return Vec::new();
    }
    raw.chunks_exact(CONNECTION6_BYTES)
        .filter_map(Connection6::decode)
        .collect()
}
/// A MIB table whose rows start at `header` (4, or 8 for rows holding a
/// LARGE_INTEGER).
fn mib_table_at(rows: &[Vec<u8>], header: usize) -> Vec<u8> {
    let mut b = alloc::vec![0u8; header];
    b[..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        b.extend_from_slice(row);
    }
    b
}
/// The IPv6 TCP sockets of `class`, sorted when asked. Windows has no
/// basic classes for AF_INET6 (ERROR_INVALID_PARAMETER, as measured on the
/// reference PC).
fn tcp6_list(class: u32, order: bool) -> Result<Vec<door_service_protocol::net::Connection6>, u32> {
    use door_service_protocol::net::tcp_state;
    let (listeners, established) = match class {
        3 | 6 => (true, false),
        4 | 7 => (false, true),
        5 | 8 => (true, true),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let mut list: Vec<_> = connections6()
        .into_iter()
        .filter(|c| c.protocol == 6)
        .filter(|c| {
            if c.state == tcp_state::LISTEN {
                listeners
            } else {
                established
            }
        })
        .collect();
    if order {
        list.sort_by_key(|c| (c.local, c.local_port, c.remote, c.remote_port));
    }
    Ok(list)
}
/// MIB_TCP6ROW_OWNER_PID (56 bytes) or, with `module`,
/// MIB_TCP6ROW_OWNER_MODULE (192: creation time and module info zero).
fn tcp6_owner_row(c: &door_service_protocol::net::Connection6, module: bool) -> Vec<u8> {
    let mut row = Vec::with_capacity(if module { 192 } else { 56 });
    row.extend_from_slice(&c.local);
    row.extend_from_slice(&c.local_scope.to_le_bytes());
    row.extend_from_slice(&mib_port(c.local_port).to_le_bytes());
    row.extend_from_slice(&c.remote);
    row.extend_from_slice(&c.remote_scope.to_le_bytes());
    row.extend_from_slice(&mib_port(c.remote_port).to_le_bytes());
    row.extend_from_slice(&u32::from(c.state).to_le_bytes());
    row.extend_from_slice(&c.pid.to_le_bytes());
    if module {
        row.resize(192, 0);
    }
    row
}
#[unsafe(no_mangle)]
pub extern "system" fn GetExtendedTcpTable(
    table: usize,
    size: usize,
    order: i32,
    family: u32,
    class: u32,
    _reserved: u32,
) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let bytes = match family as u16 {
            AF_INET => match tcp_rows(class, order != 0) {
                Ok(r) => mib_table(&r),
                Err(e) => return e,
            },
            AF_INET6 => match tcp6_list(class, order != 0) {
                Ok(list) => {
                    let module = class >= 6;
                    let rows: Vec<Vec<u8>> =
                        list.iter().map(|c| tcp6_owner_row(c, module)).collect();
                    mib_table_at(&rows, if module { 8 } else { 4 })
                }
                Err(e) => return e,
            },
            _ => return ERROR_INVALID_PARAMETER,
        };
        deliver(table, size, &bytes, ERROR_INSUFFICIENT_BUFFER)
    }
}
/// GetTcp6Table: MIB_TCP6ROW (state, then the addresses with their scope
/// ids and ports).
#[unsafe(no_mangle)]
pub extern "system" fn GetTcp6Table(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows: Vec<Vec<u8>> = tcp6_list(5, order != 0)
            .unwrap_or_default()
            .iter()
            .map(|c| {
                let mut row = Vec::with_capacity(52);
                row.extend_from_slice(&u32::from(c.state).to_le_bytes());
                row.extend_from_slice(&c.local);
                row.extend_from_slice(&c.local_scope.to_le_bytes());
                row.extend_from_slice(&mib_port(c.local_port).to_le_bytes());
                row.extend_from_slice(&c.remote);
                row.extend_from_slice(&c.remote_scope.to_le_bytes());
                row.extend_from_slice(&mib_port(c.remote_port).to_le_bytes());
                row
            })
            .collect();
        deliver(
            table,
            size,
            &mib_table_at(&rows, 4),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
/// GetTcp6Table2: MIB_TCP6ROW2 (the owner row plus the offload state).
#[unsafe(no_mangle)]
pub extern "system" fn GetTcp6Table2(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows: Vec<Vec<u8>> = tcp6_list(5, order != 0)
            .unwrap_or_default()
            .iter()
            .map(|c| {
                let mut row = tcp6_owner_row(c, false);
                row.extend_from_slice(&0u32.to_le_bytes());
                row
            })
            .collect();
        deliver(
            table,
            size,
            &mib_table_at(&rows, 4),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
/// GetTcpTable: MIB_TCPROW (state, addresses, ports).
#[unsafe(no_mangle)]
pub extern "system" fn GetTcpTable(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows = tcp_rows(2, order != 0).unwrap_or_default();
        deliver(table, size, &mib_table(&rows), ERROR_INSUFFICIENT_BUFFER)
    }
}
/// GetTcpTable2: MIB_TCPROW2 (plus the owning process and offload state).
#[unsafe(no_mangle)]
pub extern "system" fn GetTcpTable2(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows: Vec<Vec<u8>> = tcp_rows(5, order != 0)
            .unwrap_or_default()
            .into_iter()
            .map(|mut r| {
                r.extend_from_slice(&0u32.to_le_bytes());
                r
            })
            .collect();
        deliver(table, size, &mib_table(&rows), ERROR_INSUFFICIENT_BUFFER)
    }
}
fn udp_rows(class: u32, order: bool) -> Result<Vec<Vec<u8>>, u32> {
    let (pid, module) = match class {
        0 => (false, false),
        1 => (true, false),
        2 => (true, true),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let mut list: Vec<_> = connections()
        .into_iter()
        .filter(|c| c.protocol == 17)
        .collect();
    if order {
        list.sort_by_key(|c| (c.local, c.local_port));
    }
    Ok(list
        .iter()
        .map(|c| {
            let mut row = Vec::new();
            row.extend_from_slice(&mib_address(c.local).to_le_bytes());
            row.extend_from_slice(&mib_port(c.local_port).to_le_bytes());
            if pid {
                row.extend_from_slice(&c.pid.to_le_bytes());
            }
            if module {
                row.resize(160, 0);
            }
            row
        })
        .collect())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetExtendedUdpTable(
    table: usize,
    size: usize,
    order: i32,
    family: u32,
    class: u32,
    _reserved: u32,
) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let bytes = match family as u16 {
            AF_INET => match udp_rows(class, order != 0) {
                Ok(r) => mib_table(&r),
                Err(e) => return e,
            },
            AF_INET6 => match udp6_rows(class, order != 0) {
                Ok(r) => mib_table_at(&r, if class == 2 { 8 } else { 4 }),
                Err(e) => return e,
            },
            _ => return ERROR_INVALID_PARAMETER,
        };
        deliver(table, size, &bytes, ERROR_INSUFFICIENT_BUFFER)
    }
}
/// IPv6 UDP rows: MIB_UDP6ROW (24 bytes), _OWNER_PID (28) or
/// _OWNER_MODULE (176: pid, then the creation time at 32, the flags at 40
/// and the module info at 48, all zero).
fn udp6_rows(class: u32, order: bool) -> Result<Vec<Vec<u8>>, u32> {
    if class > 2 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut list: Vec<_> = connections6()
        .into_iter()
        .filter(|c| c.protocol == 17)
        .collect();
    if order {
        list.sort_by_key(|c| (c.local, c.local_port));
    }
    Ok(list
        .iter()
        .map(|c| {
            let mut row = Vec::with_capacity(176);
            row.extend_from_slice(&c.local);
            row.extend_from_slice(&c.local_scope.to_le_bytes());
            row.extend_from_slice(&mib_port(c.local_port).to_le_bytes());
            if class >= 1 {
                row.extend_from_slice(&c.pid.to_le_bytes());
            }
            if class == 2 {
                row.resize(176, 0);
            }
            row
        })
        .collect())
}
/// GetUdp6Table: MIB_UDP6ROW.
#[unsafe(no_mangle)]
pub extern "system" fn GetUdp6Table(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows = udp6_rows(0, order != 0).unwrap_or_default();
        deliver(
            table,
            size,
            &mib_table_at(&rows, 4),
            ERROR_INSUFFICIENT_BUFFER,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUdpTable(table: usize, size: usize, order: i32) -> u32 {
    let table = door_windows_abi::widen(table);
    let size = door_windows_abi::widen(size);
    {
        let rows = udp_rows(0, order != 0).unwrap_or_default();
        deliver(table, size, &mib_table(&rows), ERROR_INSUFFICIENT_BUFFER)
    }
}

// ---- ICMP echo (IcmpSendEcho) ----------------------------------------------
const IP_FLAG_DF: u8 = 0x2;
/// ICMP handles: a nonzero marker per open handle.
static ICMP_HANDLES: AtomicU32 = AtomicU32::new(0);
const ICMP_BASE: u64 = 0x1c3e_0000;
#[unsafe(no_mangle)]
pub extern "system" fn IcmpCreateFile() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        ICMP_BASE + u64::from(ICMP_HANDLES.fetch_add(1, Ordering::Relaxed) % 0xffff + 1)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn Icmp6CreateFile() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        // No IPv6 stack.
        set_last(ERROR_NOT_SUPPORTED);
        INVALID_HANDLE_VALUE
    })())
}
fn icmp_handle(handle: u64) -> bool {
    handle > ICMP_BASE && handle <= ICMP_BASE + 0xffff
}
#[unsafe(no_mangle)]
pub extern "system" fn IcmpCloseHandle(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if !icmp_handle(handle) {
            set_last(ERROR_INVALID_HANDLE);
            return 0;
        }
        1
    }
}
/// One echo through door.network: its outcome.
fn echo(
    destination: [u8; 4],
    data: &[u8],
    ttl: u8,
    flags: u8,
    timeout: u32,
) -> Result<door_service_protocol::net::Echo, u32> {
    use door_service_protocol::net::{EchoRequest, PATTERN_BYTES};
    let mut pattern = [0u8; PATTERN_BYTES];
    // The caller's data repeats from its first PATTERN_BYTES bytes, as
    // Windows sends it (exact for data that size or smaller).
    let n = data.len().min(PATTERN_BYTES);
    pattern[..n].copy_from_slice(&data[..n]);
    let request = EchoRequest {
        address: destination,
        ttl: if ttl == 0 { 128 } else { ttl },
        dont_fragment: flags & IP_FLAG_DF != 0,
        size: data.len() as u16,
        timeout: timeout.clamp(1, 600_000),
        pattern,
        // Empty data stays empty (pattern_len 0 would mean Windows' own).
        pattern_len: n.max(usize::from(data.is_empty())) as u8,
    };
    let mut bytes = [0u8; 12 + PATTERN_BYTES];
    let len = request.encode(&mut bytes);
    let mut out = [0u8; 18];
    match unsafe { DoorNetPing(bytes.as_ptr() as u64, len as u32, out.as_mut_ptr() as u64) } {
        0 => door_service_protocol::net::Echo::decode(&out).ok_or(ERROR_GEN_FAILURE),
        _ => Err(ERROR_NETWORK_UNREACHABLE),
    }
}
/// ICMP_ECHO_REPLY: Address, Status, RoundTripTime, DataSize, Reserved,
/// Data, then IP_OPTION_INFORMATION (Ttl, Tos, Flags, OptionsSize,
/// OptionsData): 40 bytes on x64, 28 on x86.
const ECHO_REPLY: [door_windows_abi::layout::Field; 11] = {
    use door_windows_abi::layout::Field::*;
    [U32, U32, U32, U16, U16, Pointer, U8, U8, U8, U8, Pointer]
};
const ECHO_REPLY_BYTES: usize = door_windows_abi::layout::size(&ECHO_REPLY);
/// IcmpSendEcho / IcmpSendEcho2(Ex) without an event or APC: one
/// ICMP_ECHO_REPLY followed by the echoed data.
#[allow(clippy::too_many_arguments)]
fn send_echo(
    handle: u64,
    destination: u32,
    data: u64,
    size: u16,
    options: u64,
    reply: u64,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    if !icmp_handle(handle) {
        set_last(ERROR_INVALID_HANDLE);
        return 0;
    }
    let need = ECHO_REPLY_BYTES as u32 + u32::from(size) + 8;
    if reply == 0 || reply_size < need {
        set_last(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }
    let mut payload = alloc::vec![0u8; usize::from(size)];
    if size > 0 && read(data, &mut payload).is_err() {
        set_last(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let (mut ttl, mut flags) = (128u8, 0u8);
    if options != 0 {
        let mut o = [0u8; 4];
        if read(options, &mut o).is_err() {
            set_last(ERROR_INVALID_PARAMETER);
            return 0;
        }
        ttl = o[0];
        flags = o[2];
    }
    let outcome = match echo(destination.to_ne_bytes(), &payload, ttl, flags, timeout) {
        Ok(e) => e,
        Err(e) => {
            set_last(e);
            return 0;
        }
    };
    let mut r = alloc::vec![0u8; ECHO_REPLY_BYTES];
    r[..4].copy_from_slice(&u32::from_ne_bytes(outcome.from).to_le_bytes());
    r[4..8].copy_from_slice(&outcome.status.to_le_bytes());
    r[8..12].copy_from_slice(&outcome.rtt.to_le_bytes());
    let echoed = if outcome.status == 0 {
        payload.len()
    } else {
        0
    };
    r[12..14].copy_from_slice(&(echoed as u16).to_le_bytes());
    door_windows_abi::layout::set(&mut r, &ECHO_REPLY, 5, reply + ECHO_REPLY_BYTES as u64);
    r[door_windows_abi::layout::offset(&ECHO_REPLY, 6)] = outcome.ttl;
    r.extend_from_slice(&payload[..echoed]);
    if write(reply, &r).is_err() {
        set_last(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // Windows returns the reply even for an error status, with the last
    // error set to it.
    if outcome.status != 0 {
        set_last(outcome.status);
        return if outcome.status == door_service_protocol::net::ip_status::REQ_TIMED_OUT {
            0
        } else {
            1
        };
    }
    1
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn IcmpSendEcho(
    handle: usize,
    destination: u32,
    data: usize,
    size: u16,
    options: usize,
    reply: usize,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    let options = door_windows_abi::widen(options);
    let reply = door_windows_abi::widen(reply);
    {
        send_echo(
            handle,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        )
    }
}
/// IcmpSendEcho2: with an event or APC routine the call is asynchronous
/// on Windows; here the echo completes before the call returns and the
/// event is signalled (ERROR_IO_PENDING is still what the caller sees).
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn IcmpSendEcho2(
    handle: usize,
    event: usize,
    apc: usize,
    _context: usize,
    destination: u32,
    data: usize,
    size: u16,
    options: usize,
    reply: usize,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let event = door_windows_abi::widen(event);
    let apc = door_windows_abi::widen(apc);
    let data = door_windows_abi::widen(data);
    let options = door_windows_abi::widen(options);
    let reply = door_windows_abi::widen(reply);
    {
        if apc != 0 {
            set_last(ERROR_NOT_SUPPORTED);
            return 0;
        }
        let n = send_echo(
            handle,
            destination,
            data,
            size,
            options,
            reply,
            reply_size,
            timeout,
        );
        if event != 0 {
            unsafe { DoorSetEvent(event) };
            set_last(ERROR_IO_PENDING);
            return 0;
        }
        n
    }
}
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn IcmpSendEcho2Ex(
    handle: usize,
    event: usize,
    apc: usize,
    context: usize,
    _source: u32,
    destination: u32,
    data: usize,
    size: u16,
    options: usize,
    reply: usize,
    reply_size: u32,
    timeout: u32,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let event = door_windows_abi::widen(event);
    let apc = door_windows_abi::widen(apc);
    let context = door_windows_abi::widen(context);
    let data = door_windows_abi::widen(data);
    let options = door_windows_abi::widen(options);
    let reply = door_windows_abi::widen(reply);
    {
        IcmpSendEcho2(
            door_windows_abi::narrow(handle),
            door_windows_abi::narrow(event),
            door_windows_abi::narrow(apc),
            door_windows_abi::narrow(context),
            destination,
            door_windows_abi::narrow(data),
            size,
            door_windows_abi::narrow(options),
            door_windows_abi::narrow(reply),
            reply_size,
            timeout,
        )
    }
}
/// IcmpParseReplies: the replies IcmpSendEcho2 left in the buffer (one).
#[unsafe(no_mangle)]
pub extern "system" fn IcmpParseReplies(reply: usize, _size: u32) -> u32 {
    let reply = door_windows_abi::widen(reply);
    {
        let Ok(status) = read_u32(reply + 4) else {
            return 0;
        };
        if status == door_service_protocol::net::ip_status::REQ_TIMED_OUT {
            set_last(status);
            return 0;
        }
        1
    }
}
fn set_last(code: u32) {
    unsafe { RtlSetLastWin32Error(code) };
}

// ---- P18: the rest of Chromium's imports --------------------------------------------------
/// GetBestRoute2: the longest-prefix IPv4 route to the destination (on the
/// interface given by LUID or index, if any) as MIB_IPFORWARD_ROW2, and
/// that interface's address as the best source. IPv6 destinations are
/// unreachable (Doors routes IPv4).
#[unsafe(no_mangle)]
pub extern "system" fn GetBestRoute2(
    luid: usize,
    index: u32,
    _source: usize,
    destination: usize,
    _options: u32,
    row: usize,
    best_source: usize,
) -> u32 {
    let (luid, destination) = (
        door_windows_abi::widen(luid),
        door_windows_abi::widen(destination),
    );
    let (row, best_source) = (
        door_windows_abi::widen(row),
        door_windows_abi::widen(best_source),
    );
    if destination == 0 || row == 0 || best_source == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let mut sa = [0u8; 8];
    if read(destination, &mut sa).is_err() {
        return ERROR_INVALID_PARAMETER;
    }
    if u16::from_le_bytes([sa[0], sa[1]]) != AF_INET {
        return ERROR_NETWORK_UNREACHABLE;
    }
    let address = [sa[4], sa[5], sa[6], sa[7]];
    let list = current();
    let wanted = if luid != 0 {
        let mut l = [0u8; 8];
        if read(luid, &mut l).is_err() {
            return ERROR_INVALID_PARAMETER;
        }
        let l = u64::from_le_bytes(l);
        match list.iter().find(|i| i.luid() == l) {
            Some(i) => Some(i.index),
            None => return ERROR_NOT_FOUND,
        }
    } else if index != 0 {
        Some(index)
    } else {
        None
    };
    let index = match wanted {
        Some(i) => i,
        None => match best_interface(&list, address) {
            Ok(i) => i,
            Err(_) => return ERROR_NETWORK_UNREACHABLE,
        },
    };
    let prefix_mask = |p: u8| u32::from_be_bytes(mask(p));
    let best = routes(&list)
        .into_iter()
        .filter(|r| r.index == index)
        .filter(|r| {
            u32::from_be_bytes(address) & prefix_mask(r.prefix) == u32::from_be_bytes(r.destination)
        })
        .max_by_key(|r| r.prefix);
    let (Some(route), Some(interface)) = (best, by_index(&list, index)) else {
        return ERROR_NETWORK_UNREACHABLE;
    };
    let source = interface.ipv4.map_or([0; 4], |(a, _)| a);
    match write(row, &forward_row2(&route, interface))
        .and_then(|()| write(best_source, &sockaddr_inet(source)))
    {
        Ok(()) => ERROR_SUCCESS,
        Err(_) => ERROR_INVALID_PARAMETER,
    }
}
/// GetInterfaceInfo: IP_INTERFACE_INFO (NumAdapters, then each adapter's
/// index and `\DEVICE\TCPIP_{GUID}` name in 128 units) for the IPv4
/// adapters other than loopback.
#[unsafe(no_mangle)]
pub extern "system" fn GetInterfaceInfo(table: usize, size: usize) -> u32 {
    let (table, size) = (
        door_windows_abi::widen(table),
        door_windows_abi::widen(size),
    );
    if size == 0 {
        return ERROR_INVALID_PARAMETER;
    }
    let list = current();
    let adapters: Vec<&Interface> = list
        .iter()
        .filter(|i| !i.loopback() && i.ipv4.is_some())
        .collect();
    if adapters.is_empty() {
        return ERROR_NO_DATA;
    }
    let mut b = Vec::with_capacity(4 + adapters.len() * 260);
    b.extend_from_slice(&(adapters.len() as u32).to_le_bytes());
    for a in &adapters {
        b.extend_from_slice(&a.index.to_le_bytes());
        let name: Vec<u16> = alloc::format!(r"\DEVICE\TCPIP_{}", a.guid_text())
            .encode_utf16()
            .collect();
        let mut units = [0u16; 128];
        for (u, n) in units.iter_mut().zip(name.iter().take(127)) {
            *u = *n;
        }
        b.extend(units.iter().flat_map(|u| u.to_le_bytes()));
    }
    let mut have = [0u8; 4];
    if read(size, &mut have).is_err() {
        return ERROR_INVALID_PARAMETER;
    }
    if table == 0 || (u32::from_le_bytes(have) as usize) < b.len() {
        let _ = write(size, &(b.len() as u32).to_le_bytes());
        return ERROR_INSUFFICIENT_BUFFER;
    }
    match write(table, &b) {
        Ok(()) => ERROR_SUCCESS,
        Err(_) => ERROR_INVALID_PARAMETER,
    }
}
/// door.network owns DHCP: programs cannot release or renew leases.
#[unsafe(no_mangle)]
pub extern "system" fn IpReleaseAddress(_adapter: usize) -> u32 {
    ERROR_NOT_SUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn IpRenewAddress(_adapter: usize) -> u32 {
    ERROR_NOT_SUPPORTED
}
