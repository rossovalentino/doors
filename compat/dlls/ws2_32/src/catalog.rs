//! The protocol catalog and the service-lookup entry points: WSAEnumProtocols
//! lists the four base providers Doors' sockets implement (TCP and UDP over
//! IPv4 and IPv6, as Windows' MSAFD Tcpip entries). There is no namespace
//! provider for WSALookupService* (the NLA network list included), which
//! callers treat as unknown connectivity rather than offline; sockets are
//! process-local, so WSADuplicateSocket cannot hand one to another process.
use crate::error::*;
use crate::guest::*;

/// sizeof(WSAPROTOCOL_INFOW): five flag words, the provider GUID, the
/// catalog id, WSAPROTOCOLCHAIN (32 bytes), nine ints, two words and
/// szProtocol[256].
pub(crate) const INFO_W: usize = 628;
/// sizeof(WSAPROTOCOL_INFOA): szProtocol is 256 bytes.
const INFO_A: usize = 372;
const NAME: usize = 116;
pub(crate) const FAMILY: usize = 76;
pub(crate) const SOCKET_TYPE: usize = 88;
pub(crate) const PROTOCOL: usize = 92;
const WSASERVICE_NOT_FOUND: u32 = 10108;
const WSA_E_NO_MORE: u32 = 10110;
const AF_INET: i32 = 2;
const AF_INET6: i32 = 23;
/// {E70F1AA0-AB8B-11CF-8CA3-00805F48A192}, MSAFD Tcpip's provider id.
const PROVIDER: [u8; 16] = [
    0xa0, 0x1a, 0x0f, 0xe7, 0x8b, 0xab, 0xcf, 0x11, 0x8c, 0xa3, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92,
];

struct Protocol {
    family: i32,
    socket_type: i32,
    protocol: i32,
    flags: u32,
    message: u32,
    name: &'static str,
}
const PROTOCOLS: [Protocol; 4] = [
    Protocol {
        family: AF_INET,
        socket_type: 1,
        protocol: 6,
        flags: 0x0002_0066,
        message: 0,
        name: "MSAFD Tcpip [TCP/IP]",
    },
    Protocol {
        family: AF_INET,
        socket_type: 2,
        protocol: 17,
        flags: 0x0002_0609,
        message: 65467,
        name: "MSAFD Tcpip [UDP/IP]",
    },
    Protocol {
        family: AF_INET6,
        socket_type: 1,
        protocol: 6,
        flags: 0x0002_0066,
        message: 0,
        name: "MSAFD Tcpip [TCP/IPv6]",
    },
    Protocol {
        family: AF_INET6,
        socket_type: 2,
        protocol: 17,
        flags: 0x0002_0609,
        message: 65467,
        name: "MSAFD Tcpip [UDP/IPv6]",
    },
];

fn put(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
/// One WSAPROTOCOL_INFO record (wide or ANSI name).
fn record(index: usize, p: &Protocol, wide: bool) -> [u8; INFO_W] {
    let mut out = [0u8; INFO_W];
    put(&mut out, 0, p.flags);
    // PFL_MATCHES_PROTOCOL_ZERO: protocol 0 selects this entry.
    put(&mut out, 16, 0x8);
    out[20..36].copy_from_slice(&PROVIDER);
    put(&mut out, 36, 1001 + index as u32);
    // A base protocol: ChainLen 1.
    put(&mut out, 40, 1);
    put(&mut out, 72, 2);
    put(&mut out, FAMILY, p.family as u32);
    let address = if p.family == AF_INET6 { 28 } else { 16 };
    put(&mut out, 80, address);
    put(&mut out, 84, address);
    put(&mut out, SOCKET_TYPE, p.socket_type as u32);
    put(&mut out, PROTOCOL, p.protocol as u32);
    put(&mut out, 108, p.message);
    for (i, b) in p.name.bytes().enumerate() {
        if wide {
            out[NAME + 2 * i] = b;
        } else {
            out[NAME + i] = b;
        }
    }
    out
}
/// The protocols `filter` (a zero-terminated list, or none) selects.
fn selected(filter: u64) -> Result<[bool; 4], u32> {
    if filter == 0 {
        return Ok([true; 4]);
    }
    let mut chosen = [false; 4];
    for i in 0..64u64 {
        let wanted = read_i32(filter + 4 * i)?;
        if wanted == 0 {
            break;
        }
        for (slot, p) in chosen.iter_mut().zip(&PROTOCOLS) {
            *slot |= p.protocol == wanted;
        }
    }
    Ok(chosen)
}
fn enumerate(filter: usize, buffer: usize, length: usize, wide: bool) -> i32 {
    let filter = door_windows_abi::widen(filter);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    let size = if wide { INFO_W } else { INFO_A };
    let outcome = (|| {
        if length == 0 {
            return Err(WSAEFAULT);
        }
        let chosen = selected(filter)?;
        let count = chosen.iter().filter(|c| **c).count();
        let needed = (count * size) as u32;
        let mut have = [0u8; 4];
        read(length, &mut have)?;
        if buffer == 0 || u32::from_le_bytes(have) < needed {
            write(length, &needed.to_le_bytes())?;
            return Err(WSAENOBUFS);
        }
        let mut at = buffer;
        for (i, p) in PROTOCOLS.iter().enumerate() {
            if chosen[i] {
                write(at, &record(i, p, wide)[..size])?;
                at += size as u64;
            }
        }
        write(length, &needed.to_le_bytes())?;
        Ok(count as i32)
    })();
    match outcome {
        Ok(count) => count,
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAEnumProtocolsW(filter: usize, buffer: usize, length: usize) -> i32 {
    enumerate(filter, buffer, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAEnumProtocolsA(filter: usize, buffer: usize, length: usize) -> i32 {
    enumerate(filter, buffer, length, false)
}
/// The (family, type, protocol) a WSAPROTOCOL_INFO record names.
pub(crate) fn socket_triple(info: u64) -> Result<(i32, i32, i32), u32> {
    Ok((
        read_i32(info + FAMILY as u64)?,
        read_i32(info + SOCKET_TYPE as u64)?,
        read_i32(info + PROTOCOL as u64)?,
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn WSADuplicateSocketW(socket: usize, _process: u32, info: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let info = door_windows_abi::widen(info);
    match entry(socket) {
        Err(e) => fail(e),
        Ok(_) if info == 0 => fail(WSAEFAULT),
        // door.network sockets belong to the creating process.
        Ok(_) => fail(WSAEOPNOTSUPP),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSADuplicateSocketA(socket: usize, process: u32, info: usize) -> i32 {
    WSADuplicateSocketW(socket, process, info)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceBeginW(query: usize, _flags: u32, handle: usize) -> i32 {
    let query = door_windows_abi::widen(query);
    let handle = door_windows_abi::widen(handle);
    if let Err(e) = started() {
        return fail(e);
    }
    if query == 0 || handle == 0 {
        return fail(WSAEFAULT);
    }
    fail(WSASERVICE_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceBeginA(query: usize, flags: u32, handle: usize) -> i32 {
    WSALookupServiceBeginW(query, flags, handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceNextW(
    _handle: usize,
    _flags: u32,
    _length: usize,
    _results: usize,
) -> i32 {
    fail(WSA_E_NO_MORE)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceNextA(
    handle: usize,
    flags: u32,
    length: usize,
    results: usize,
) -> i32 {
    WSALookupServiceNextW(handle, flags, length, results)
}
/// No lookup handle is ever issued.
#[unsafe(no_mangle)]
pub extern "system" fn WSALookupServiceEnd(_handle: usize) -> i32 {
    fail(WSA_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASetServiceW(_info: usize, _operation: u32, _flags: u32) -> i32 {
    if let Err(e) = started() {
        return fail(e);
    }
    fail(WSAEOPNOTSUPP)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASetServiceA(info: usize, operation: u32, flags: u32) -> i32 {
    WSASetServiceW(info, operation, flags)
}
