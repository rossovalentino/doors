//! Names and address text: getaddrinfo, getnameinfo, gethostbyname,
//! gethostname, the services/protocols databases and the inet_* functions.
//! IPv4 and IPv6 (AAAA answers, SOCKADDR_IN6); reverse lookups report no
//! data.
use crate::error::*;
use crate::guest::*;
use crate::text;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};
use door_windows_abi::layout::{self, Field, Field::*};
/// ADDRINFOA/ADDRINFOW: ai_flags, ai_family, ai_socktype, ai_protocol,
/// ai_addrlen (size_t), ai_canonname, ai_addr, ai_next (48 bytes on x64,
/// 32 on x86).
const ADDRINFO: [Field; 8] = [U32, U32, U32, U32, Pointer, Pointer, Pointer, Pointer];
/// ADDRINFOEXW: ADDRINFO's fields, then ai_blob, ai_bufferlength,
/// ai_provider and ai_next (72 bytes on x64, 44 on x86).
const ADDRINFOEX: [Field; 11] = [
    U32, U32, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
];
/// A result record shape and the index of its next pointer.
type Shape = (&'static [Field], usize);
const PLAIN: Shape = (&ADDRINFO, 7);
const EXTENDED: Shape = (&ADDRINFOEX, 10);

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn DoorGetProcessHeap() -> u64;
}
const HEAP_ZERO_MEMORY: u32 = 8;
const AI_PASSIVE: i32 = 0x01;
const AI_CANONNAME: i32 = 0x02;
const AI_NUMERICHOST: i32 = 0x04;
const AI_NUMERICSERV: i32 = 0x08;
const AI_V4MAPPED: i32 = 0x0800;
const NI_NUMERICHOST: i32 = 0x02;
const NI_NAMEREQD: i32 = 0x04;
const NI_NUMERICSERV: i32 = 0x08;
const NI_DGRAM: i32 = 0x10;
const RESOLVE_TIMEOUT_MS: u32 = 10_000;

/// A NUL-terminated ANSI string (at most `max` bytes).
fn ansi(pointer: u64, max: usize) -> Result<Vec<u8>, u32> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 64];
    while out.len() < max {
        let n = chunk.len().min(max - out.len());
        // A string may end near the end of a page: read byte by byte there.
        if read(pointer + out.len() as u64, &mut chunk[..n]).is_err() {
            let mut b = [0u8; 1];
            read(pointer + out.len() as u64, &mut b)?;
            if b[0] == 0 {
                return Ok(out);
            }
            out.push(b[0]);
            continue;
        }
        if let Some(end) = chunk[..n].iter().position(|b| *b == 0) {
            out.extend_from_slice(&chunk[..end]);
            return Ok(out);
        }
        out.extend_from_slice(&chunk[..n]);
    }
    Err(WSAEFAULT)
}
/// A NUL-terminated UTF-16 string as UTF-8 (host names are ASCII).
fn wide(pointer: u64, max: usize) -> Result<Vec<u8>, u32> {
    let mut units = Vec::new();
    loop {
        if units.len() >= max {
            return Err(WSAEFAULT);
        }
        let mut b = [0u8; 2];
        read(pointer + 2 * units.len() as u64, &mut b)?;
        let unit = u16::from_le_bytes(b);
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    let text: alloc::string::String = char::decode_utf16(units)
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect();
    Ok(text.into_bytes())
}
fn heap() -> u64 {
    unsafe { DoorGetProcessHeap() }
}
/// Addresses for `node` of `family` (0: both), as (family, address). None:
/// the passive or loopback default. Without IPv6 connectivity beyond
/// site-local, Windows orders IPv4 first (RFC 6724 precedence).
fn addresses(node: Option<&[u8]>, flags: i32, family: i32) -> Result<Vec<(i32, Ip)>, u32> {
    let want4 = family != AF_INET6;
    let want6 = family != AF_INET;
    let mut out = Vec::new();
    let Some(node) = node else {
        let passive = flags & AI_PASSIVE != 0;
        if want6 {
            let mut any6 = [0u8; 16];
            if !passive {
                any6[15] = 1;
            }
            out.push((AF_INET6, any6));
        }
        if want4 {
            out.push((
                AF_INET,
                mapped(if passive { [0; 4] } else { [127, 0, 0, 1] }),
            ));
        }
        return Ok(out);
    };
    let host = node.split(|b| *b == b'%').next().unwrap_or(node);
    if let Some(a) = text::pton6(host) {
        return if want6 {
            Ok(alloc::vec![(AF_INET6, a)])
        } else {
            Err(WSAHOST_NOT_FOUND)
        };
    }
    if let Some(a) = text::pton4(node).or_else(|| text::inet_addr(node)) {
        return match (want4, flags & AI_V4MAPPED != 0) {
            (true, _) => Ok(alloc::vec![(AF_INET, mapped(a))]),
            (false, true) => Ok(alloc::vec![(AF_INET6, mapped(a))]),
            (false, false) => Err(WSAHOST_NOT_FOUND),
        };
    }
    if flags & AI_NUMERICHOST != 0 || node.contains(&b':') {
        return Err(WSAHOST_NOT_FOUND);
    }
    if node.eq_ignore_ascii_case(b"localhost") {
        if want6 {
            let mut one = [0u8; 16];
            one[15] = 1;
            out.push((AF_INET6, one));
        }
        if want4 {
            out.push((AF_INET, mapped([127, 0, 0, 1])));
        }
        return Ok(out);
    }
    if is_own_name(node) {
        if want4 {
            out.push((AF_INET, mapped(local_address().unwrap_or([127, 0, 0, 1]))));
        }
        if want6 && let Ok((link, global)) = local_addresses6() {
            for a in [global, link] {
                if a != [0; 16] {
                    out.push((AF_INET6, a));
                }
            }
        }
        return Ok(out);
    }
    let mut failure = None;
    if want4 {
        match resolve(node) {
            Ok(list) => out.extend(list.into_iter().map(|a| (AF_INET, mapped(a)))),
            Err(e) => failure = Some(e),
        }
    }
    if want6 {
        match resolve6(node) {
            Ok(list) => out.extend(list.into_iter().map(|a| (AF_INET6, a))),
            Err(e) => failure = failure.or(Some(e)),
        }
        // AI_V4MAPPED: IPv4 answers stand in when there are no AAAA ones.
        if !want4
            && out.is_empty()
            && flags & AI_V4MAPPED != 0
            && let Ok(list) = resolve(node)
        {
            out.extend(list.into_iter().map(|a| (AF_INET6, mapped(a))));
        }
    }
    match (out.is_empty(), failure) {
        (true, Some(e)) => Err(e),
        _ => Ok(out),
    }
}
fn is_own_name(node: &[u8]) -> bool {
    let mut name = [0u8; 64];
    let mut length = [0u8; 4];
    let found =
        unsafe { DoorGetComputerName(name.as_mut_ptr() as u64, 64, length.as_mut_ptr() as u64) }
            == 0;
    found && name[..(u32::from_le_bytes(length) as usize).min(64)].eq_ignore_ascii_case(node)
}
fn resolve(name: &[u8]) -> Result<Vec<[u8; 4]>, u32> {
    let mut out = [[0u8; 4]; 8];
    let mut count = [0u8; 4];
    match unsafe {
        DoorNetResolve(
            name.as_ptr() as u64,
            name.len() as u32,
            out.as_mut_ptr() as u64,
            8,
            count.as_mut_ptr() as u64,
            RESOLVE_TIMEOUT_MS,
        )
    } {
        0 => Ok(out[..u32::from_le_bytes(count) as usize].to_vec()),
        WSAENETDOWN => Err(WSATRY_AGAIN),
        e => Err(e),
    }
}
/// AAAA addresses of `name`.
fn resolve6(name: &[u8]) -> Result<Vec<Ip>, u32> {
    let mut out = [[0u8; 16]; 8];
    let mut count = [0u8; 4];
    match unsafe {
        DoorNetResolve6(
            name.as_ptr() as u64,
            name.len() as u32,
            out.as_mut_ptr() as u64,
            8,
            count.as_mut_ptr() as u64,
            RESOLVE_TIMEOUT_MS,
        )
    } {
        0 => Ok(out[..u32::from_le_bytes(count) as usize].to_vec()),
        WSAENETDOWN => Err(WSATRY_AGAIN),
        e => Err(e),
    }
}
/// The port of `service` for the hints' socket type (None: no service).
fn port(service: Option<&[u8]>, flags: i32, socktype: i32, protocol: i32) -> Result<u16, u32> {
    let Some(service) = service else { return Ok(0) };
    let numeric = !service.is_empty() && service.iter().all(u8::is_ascii_digit);
    if flags & AI_NUMERICSERV != 0 && !numeric {
        return Err(WSAHOST_NOT_FOUND);
    }
    let proto = match (socktype, protocol) {
        (SOCK_DGRAM, _) | (_, IPPROTO_UDP) => 2,
        (SOCK_STREAM, _) | (_, IPPROTO_TCP) => 1,
        _ => 0,
    };
    text::service_port(service, proto).ok_or(WSATYPE_NOT_FOUND)
}
/// Build the result list: one ADDRINFO block per address, each holding its
/// SOCKADDR_IN or SOCKADDR_IN6 and (first only) the canonical name.
fn build(
    shape: Shape,
    addresses: &[(i32, Ip)],
    port: u16,
    hints: [i32; 4],
    canonical: Option<&[u8]>,
    wide_name: bool,
) -> Result<u64, u32> {
    let [flags, _, socktype, protocol] = hints;
    let mut head = 0u64;
    let mut previous = 0u64;
    for (i, &(family, address)) in addresses.iter().enumerate() {
        let sockaddr_bytes: usize = if family == AF_INET6 { 28 } else { 16 };
        let name = if i == 0 && flags & AI_CANONNAME != 0 {
            canonical
        } else {
            None
        };
        let name_bytes = name.map_or(0, |n| {
            if wide_name {
                2 * (n.len() + 1)
            } else {
                n.len() + 1
            }
        });
        // ADDRINFO, then its sockaddr (32 bytes) and canonical name.
        let (fields, next) = shape;
        let info_bytes = layout::size(fields);
        let info_size = info_bytes as u64;
        let block = door_windows_abi::widen(unsafe {
            RtlAllocateHeap(
                door_windows_abi::narrow(heap()),
                HEAP_ZERO_MEMORY,
                door_windows_abi::narrow(info_size + 32 + name_bytes as u64),
            )
        });
        if block == 0 {
            free_list(shape, head);
            return Err(WSA_NOT_ENOUGH_MEMORY);
        }
        let mut record = [0u8; 80];
        let info = &mut record[..info_bytes];
        layout::set(info, fields, 1, family as u32 as u64);
        layout::set(info, fields, 2, socktype as u32 as u64);
        layout::set(info, fields, 3, protocol as u32 as u64);
        layout::set(info, fields, 4, sockaddr_bytes as u64);
        if let Some(n) = name {
            layout::set(info, fields, 5, block + info_size + 32);
            if wide_name {
                let units: Vec<u8> = n
                    .iter()
                    .flat_map(|b| u16::from(*b).to_le_bytes())
                    .chain([0, 0])
                    .collect();
                write(block + info_size + 32, &units)?;
            } else {
                let mut z = n.to_vec();
                z.push(0);
                write(block + info_size + 32, &z)?;
            }
        }
        layout::set(info, fields, 6, block + info_size);
        write(block, info)?;
        let mut sa = [0u8; 28];
        sa[..2].copy_from_slice(&(family as u16).to_le_bytes());
        sa[2..4].copy_from_slice(&port.to_be_bytes());
        if family == AF_INET6 {
            sa[8..24].copy_from_slice(&address);
            if address[0] == 0xfe && address[1] & 0xc0 == 0x80 {
                sa[24..28].copy_from_slice(&LINK_SCOPE.to_le_bytes());
            }
        } else {
            sa[4..8].copy_from_slice(&unmapped(&address).unwrap_or([0; 4]));
        }
        write(block + info_size, &sa[..sockaddr_bytes])?;
        if previous != 0 {
            write_pointer(previous + layout::offset(fields, next) as u64, block)?;
        } else {
            head = block;
        }
        previous = block;
    }
    Ok(head)
}
fn free_list((fields, link): Shape, mut node: u64) {
    while node != 0 {
        let next = read_pointer(node + layout::offset(fields, link) as u64).unwrap_or(0);
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(node),
            )
        };
        node = next;
    }
}
fn lookup(
    shape: Shape,
    node: Option<Vec<u8>>,
    service: Option<Vec<u8>>,
    hints: u64,
    result: u64,
    wide_name: bool,
) -> Result<(), u32> {
    started()?;
    write_pointer(result, 0)?;
    let mut h = [0i32; 4];
    if hints != 0 {
        let mut b = [0u8; 16];
        read(hints, &mut b)?;
        for (slot, chunk) in h.iter_mut().zip(b.chunks_exact(4)) {
            *slot = i32::from_le_bytes(chunk.try_into().unwrap());
        }
    }
    let [flags, family, socktype, protocol] = h;
    if node.is_none() && service.is_none() {
        return Err(WSAHOST_NOT_FOUND);
    }
    if !matches!(family, 0 | AF_INET | AF_INET6) {
        return Err(WSAEAFNOSUPPORT);
    }
    if !matches!(socktype, 0 | SOCK_STREAM | SOCK_DGRAM | 3) {
        return Err(WSAESOCKTNOSUPPORT);
    }
    if flags & AI_CANONNAME != 0 && node.is_none() {
        return Err(WSAEINVAL);
    }
    let port = port(service.as_deref(), flags, socktype, protocol)?;
    let list = addresses(node.as_deref(), flags, family)?;
    if list.is_empty() {
        return Err(WSANO_DATA);
    }
    let head = build(shape, &list, port, h, node.as_deref(), wide_name)?;
    write_pointer(result, head)
}
fn status(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            unsafe { RtlSetLastWin32Error(e) };
            e as i32
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn getaddrinfo(
    node: usize,
    service: usize,
    hints: usize,
    result: usize,
) -> i32 {
    let node = door_windows_abi::widen(node);
    let service = door_windows_abi::widen(service);
    let hints = door_windows_abi::widen(hints);
    let result = door_windows_abi::widen(result);
    {
        status((|| {
            let node = if node == 0 {
                None
            } else {
                Some(ansi(node, 255)?)
            };
            let service = if service == 0 {
                None
            } else {
                Some(ansi(service, 64)?)
            };
            lookup(PLAIN, node, service, hints, result, false)
        })())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAddrInfoW(
    node: usize,
    service: usize,
    hints: usize,
    result: usize,
) -> i32 {
    let node = door_windows_abi::widen(node);
    let service = door_windows_abi::widen(service);
    let hints = door_windows_abi::widen(hints);
    let result = door_windows_abi::widen(result);
    {
        status((|| {
            let node = if node == 0 {
                None
            } else {
                Some(wide(node, 255)?)
            };
            let service = if service == 0 {
                None
            } else {
                Some(wide(service, 64)?)
            };
            lookup(PLAIN, node, service, hints, result, true)
        })())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn freeaddrinfo(list: usize) {
    let list = door_windows_abi::widen(list);
    {
        free_list(PLAIN, list);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeAddrInfoW(list: usize) {
    let list = door_windows_abi::widen(list);
    {
        free_list(PLAIN, list);
    }
}
/// GetAddrInfoExW: the DNS namespace (NS_ALL or NS_DNS) answered
/// synchronously into ADDRINFOEXW records. An overlapped request completes
/// at once too: the status is NO_ERROR and ppResult is filled, which the
/// API allows (callers check for WSA_IO_PENDING before waiting), and the
/// overlapped Internal field records the result for
/// GetAddrInfoExOverlappedResult.
#[unsafe(no_mangle)]
pub extern "system" fn GetAddrInfoExW(
    node: usize,
    service: usize,
    namespace: u32,
    _provider: usize,
    hints: usize,
    result: usize,
    _timeout: usize,
    overlapped: usize,
    _routine: usize,
    handle: usize,
) -> i32 {
    let node = door_windows_abi::widen(node);
    let service = door_windows_abi::widen(service);
    let hints = door_windows_abi::widen(hints);
    let result = door_windows_abi::widen(result);
    let overlapped = door_windows_abi::widen(overlapped);
    let handle = door_windows_abi::widen(handle);
    let outcome = (|| {
        if handle != 0 {
            write_pointer(handle, 0)?;
        }
        // NS_ALL (0) and NS_DNS (12) are served; NS_NLA and the others
        // have no provider.
        if !matches!(namespace, 0 | 12) {
            return Err(WSAEINVAL);
        }
        let node = if node == 0 {
            None
        } else {
            Some(wide(node, 255)?)
        };
        let service = if service == 0 {
            None
        } else {
            Some(wide(service, 64)?)
        };
        lookup(EXTENDED, node, service, hints, result, true)
    })();
    if overlapped != 0 {
        let code = outcome.err().unwrap_or(0);
        let _ = write_pointer(overlapped, u64::from(code));
    }
    status(outcome)
}
/// GetAddrInfoExOverlappedResult: requests complete within GetAddrInfoExW,
/// so the recorded status is final.
#[unsafe(no_mangle)]
pub extern "system" fn GetAddrInfoExOverlappedResult(overlapped: usize) -> i32 {
    let overlapped = door_windows_abi::widen(overlapped);
    if overlapped == 0 {
        return WSAEINVAL as i32;
    }
    read_pointer(overlapped).map_or(WSAEFAULT as i32, |code| code as u32 as i32)
}
/// GetAddrInfoExCancel: nothing is ever pending, so no handle names a
/// request.
#[unsafe(no_mangle)]
pub extern "system" fn GetAddrInfoExCancel(_handle: usize) -> i32 {
    WSA_INVALID_HANDLE as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeAddrInfoExW(list: usize) {
    free_list(EXTENDED, door_windows_abi::widen(list));
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeAddrInfoEx(list: usize) {
    free_list(EXTENDED, door_windows_abi::widen(list));
}
/// getnameinfo: the host is always numeric (no reverse DNS); NI_NAMEREQD
/// therefore fails.
fn name_info(address: u64, length: i32, flags: i32) -> Result<(Vec<u8>, Vec<u8>), u32> {
    started()?;
    let (family, ip, port) = sockaddr_any(address, length)?;
    if flags & NI_NAMEREQD != 0 && flags & NI_NUMERICHOST == 0 {
        return Err(WSAHOST_NOT_FOUND);
    }
    let mut host = [0u8; 46];
    let n = if family == AF_INET6 {
        text::ntop6(ip, &mut host)
    } else {
        let mut v4 = [0u8; 16];
        let n = text::ntop4(unmapped(&ip).unwrap_or([0; 4]), &mut v4);
        host[..n].copy_from_slice(&v4[..n]);
        n
    };
    let service = if flags & NI_NUMERICSERV != 0 {
        None
    } else {
        text::service_name(port, if flags & NI_DGRAM != 0 { 2 } else { 1 })
    };
    let service = match service {
        Some(name) => name.as_bytes().to_vec(),
        None => alloc::format!("{port}").into_bytes(),
    };
    Ok((host[..n].to_vec(), service))
}
fn store_text(pointer: u64, capacity: u32, text: &[u8], wide_text: bool) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    if text.len() + 1 > capacity as usize {
        return Err(WSAEFAULT);
    }
    if wide_text {
        let units: Vec<u8> = text
            .iter()
            .flat_map(|b| u16::from(*b).to_le_bytes())
            .chain([0, 0])
            .collect();
        write(pointer, &units)
    } else {
        let mut z = text.to_vec();
        z.push(0);
        write(pointer, &z)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn getnameinfo(
    address: usize,
    length: i32,
    host: usize,
    host_length: u32,
    service: usize,
    service_length: u32,
    flags: i32,
) -> i32 {
    let address = door_windows_abi::widen(address);
    let host = door_windows_abi::widen(host);
    let service = door_windows_abi::widen(service);
    {
        status((|| {
            let (h, s) = name_info(address, length, flags)?;
            store_text(host, host_length, &h, false)?;
            store_text(service, service_length, &s, false)
        })())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNameInfoW(
    address: usize,
    length: i32,
    host: usize,
    host_length: u32,
    service: usize,
    service_length: u32,
    flags: i32,
) -> i32 {
    let address = door_windows_abi::widen(address);
    let host = door_windows_abi::widen(host);
    let service = door_windows_abi::widen(service);
    {
        status((|| {
            let (h, s) = name_info(address, length, flags)?;
            store_text(host, host_length, &h, true)?;
            store_text(service, service_length, &s, true)
        })())
    }
}

// ---- legacy databases (static results, valid until the next call) ------
/// One process-wide buffer per legacy result, as each call overwrites the
/// previous result (Windows keeps them per thread).
struct Static<const N: usize> {
    owner: AtomicU32,
    bytes: UnsafeCell<[u8; N]>,
}
unsafe impl<const N: usize> Sync for Static<N> {}
impl<const N: usize> Static<N> {
    const fn new() -> Self {
        Self {
            owner: AtomicU32::new(0),
            bytes: UnsafeCell::new([0; N]),
        }
    }
    /// Fill the buffer under its lock; returns its address.
    fn fill(&self, f: impl FnOnce(u64, &mut [u8; N])) -> u64 {
        while self
            .owner
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            unsafe { DoorSleep(0) };
        }
        let bytes = unsafe { &mut *self.bytes.get() };
        let base = bytes.as_ptr() as u64;
        f(base, bytes);
        self.owner.store(0, Ordering::Release);
        base
    }
}
/// HOSTENT: h_name, h_aliases, h_addrtype, h_length, h_addr_list (32 bytes
/// on x64, 16 on x86).
const HOSTENT_FIELDS: [Field; 5] = [Pointer, Pointer, U16, U16, Pointer];
/// SERVENT: s_name, s_aliases, then s_proto and s_port on x64, s_port and
/// s_proto on x86 (winsock2.h orders them by pointer width).
#[cfg(target_pointer_width = "64")]
const SERVENT_FIELDS: [Field; 4] = [Pointer, Pointer, Pointer, U16];
#[cfg(target_pointer_width = "64")]
const SERVENT_PROTO: usize = 2;
#[cfg(target_pointer_width = "64")]
const SERVENT_PORT: usize = 3;
#[cfg(not(target_pointer_width = "64"))]
const SERVENT_FIELDS: [Field; 4] = [Pointer, Pointer, U16, Pointer];
#[cfg(not(target_pointer_width = "64"))]
const SERVENT_PROTO: usize = 3;
#[cfg(not(target_pointer_width = "64"))]
const SERVENT_PORT: usize = 2;
/// PROTOENT: p_name, p_aliases, p_proto.
const PROTOENT_FIELDS: [Field; 3] = [Pointer, Pointer, U16];
/// Sizes of the per-process database entries (WSAAsyncGet* copies them).
pub(crate) const HOSTENT_BYTES: usize = 512;
pub(crate) const SERVENT_BYTES: usize = 128;
pub(crate) const PROTOENT_BYTES: usize = 96;
static HOSTENT: Static<HOSTENT_BYTES> = Static::new();
static SERVENT: Static<SERVENT_BYTES> = Static::new();
static PROTOENT: Static<PROTOENT_BYTES> = Static::new();
static NTOA: Static<16> = Static::new();
/// HOSTENT (Win64): h_name, h_aliases, h_addrtype i16, h_length i16,
/// h_addr_list; then the name, an empty alias list, the address list and
/// the addresses.
#[unsafe(no_mangle)]
pub extern "system" fn gethostbyname(name: usize) -> usize {
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| -> Result<u64, u32> {
            started()?;
            let node = if name == 0 {
                None
            } else {
                Some(ansi(name, 255)?)
            };
            let mut name_text = match &node {
                Some(n) => n.clone(),
                None => {
                    let mut b = [0u8; 64];
                    let mut l = [0u8; 4];
                    unsafe {
                        DoorGetComputerName(b.as_mut_ptr() as u64, 64, l.as_mut_ptr() as u64)
                    };
                    b[..u32::from_le_bytes(l) as usize].to_vec()
                }
            };
            // gethostbyname is IPv4 only.
            let list: Vec<[u8; 4]> = addresses(Some(&name_text), 0, AF_INET)?
                .iter()
                .filter_map(|(_, a)| unmapped(a))
                .collect();
            name_text.truncate(200);
            Ok(HOSTENT.fill(|base, b| {
                b.fill(0);
                let list = &list[..list.len().min(8)];
                let name_at = 32usize;
                b[name_at..name_at + name_text.len()].copy_from_slice(&name_text);
                let aliases_at = 240usize;
                let list_at = 256usize;
                let addresses_at = 336usize;
                for (index, value) in [
                    base + name_at as u64,
                    base + aliases_at as u64,
                    AF_INET as u64,
                    4,
                    base + list_at as u64,
                ]
                .into_iter()
                .enumerate()
                {
                    layout::set(&mut b[..], &HOSTENT_FIELDS, index, value);
                }
                for (i, address) in list.iter().enumerate() {
                    let at = addresses_at + 4 * i;
                    b[at..at + 4].copy_from_slice(address);
                    b[list_at + POINTER * i..list_at + POINTER * (i + 1)]
                        .copy_from_slice(&(base + at as u64).to_le_bytes()[..POINTER]);
                }
            }))
        })();
        result.unwrap_or_else(|e| {
            unsafe { RtlSetLastWin32Error(e) };
            0
        })
    })())
}
/// Reverse lookups are not available: no data, as for an unregistered address.
#[unsafe(no_mangle)]
pub extern "system" fn gethostbyaddr(_address: usize, _length: i32, _family: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        unsafe {
            RtlSetLastWin32Error(if started().is_err() {
                WSANOTINITIALISED
            } else {
                WSANO_DATA
            })
        };
        0
    })())
}
fn host_name() -> Vec<u8> {
    let mut b = [0u8; 64];
    let mut l = [0u8; 4];
    unsafe { DoorGetComputerName(b.as_mut_ptr() as u64, 64, l.as_mut_ptr() as u64) };
    b[..u32::from_le_bytes(l) as usize].to_vec()
}
#[unsafe(no_mangle)]
pub extern "system" fn gethostname(name: usize, length: i32) -> i32 {
    let name = door_windows_abi::widen(name);
    {
        let result =
            started().and_then(|()| store_text(name, length.max(0) as u32, &host_name(), false));
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetHostNameW(name: usize, length: i32) -> i32 {
    let name = door_windows_abi::widen(name);
    {
        let result =
            started().and_then(|()| store_text(name, length.max(0) as u32, &host_name(), true));
        result.map_or_else(fail, |()| ok())
    }
}
/// SERVENT (s_port in network order), then its strings and an empty alias
/// list.
fn servent(name: &str, port: u16, proto: &str) -> u64 {
    SERVENT.fill(|base, b| {
        b.fill(0);
        b[32..32 + name.len()].copy_from_slice(name.as_bytes());
        b[96..96 + proto.len()].copy_from_slice(proto.as_bytes());
        layout::set(&mut b[..], &SERVENT_FIELDS, 0, base + 32);
        layout::set(&mut b[..], &SERVENT_FIELDS, 1, base + 80);
        layout::set(&mut b[..], &SERVENT_FIELDS, SERVENT_PROTO, base + 96);
        layout::set(
            &mut b[..],
            &SERVENT_FIELDS,
            SERVENT_PORT,
            u64::from(port.to_be()),
        );
    })
}
fn protocol_filter(proto: u64) -> Result<(u8, &'static str), u32> {
    if proto == 0 {
        return Ok((0, "tcp"));
    }
    let p = ansi(proto, 16)?;
    if p.eq_ignore_ascii_case(b"tcp") {
        Ok((1, "tcp"))
    } else if p.eq_ignore_ascii_case(b"udp") {
        Ok((2, "udp"))
    } else {
        Err(WSANO_DATA)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn getservbyname(name: usize, proto: usize) -> usize {
    let name = door_windows_abi::widen(name);
    let proto = door_windows_abi::widen(proto);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| -> Result<u64, u32> {
            started()?;
            let n = ansi(name, 64)?;
            let (filter, proto_name) = protocol_filter(proto)?;
            let entry = text::SERVICES
                .iter()
                .find(|(s, _, p)| {
                    s.as_bytes().eq_ignore_ascii_case(&n) && (filter == 0 || p & filter != 0)
                })
                .ok_or(WSANO_DATA)?;
            let proto_name = if filter == 0 && entry.2 == 2 {
                "udp"
            } else {
                proto_name
            };
            Ok(servent(entry.0, entry.1, proto_name))
        })();
        result.unwrap_or_else(|e| {
            unsafe { RtlSetLastWin32Error(e) };
            0
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn getservbyport(port: i32, proto: usize) -> usize {
    let proto = door_windows_abi::widen(proto);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| -> Result<u64, u32> {
            started()?;
            let host_port = u16::from_be(port as u16);
            let (filter, proto_name) = protocol_filter(proto)?;
            let name = text::service_name(host_port, filter).ok_or(WSANO_DATA)?;
            Ok(servent(name, host_port, proto_name))
        })();
        result.unwrap_or_else(|e| {
            unsafe { RtlSetLastWin32Error(e) };
            0
        })
    })())
}
const PROTOCOLS: [(&str, i16); 4] = [("ip", 0), ("icmp", 1), ("tcp", 6), ("udp", 17)];
/// PROTOENT, then its name and an empty alias list.
fn protoent(name: &str, number: i16) -> u64 {
    PROTOENT.fill(|base, b| {
        b.fill(0);
        b[32..32 + name.len()].copy_from_slice(name.as_bytes());
        layout::set(&mut b[..], &PROTOENT_FIELDS, 0, base + 32);
        layout::set(&mut b[..], &PROTOENT_FIELDS, 1, base + 48);
        layout::set(&mut b[..], &PROTOENT_FIELDS, 2, number as u16 as u64);
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn getprotobyname(name: usize) -> usize {
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        let found = started().and_then(|()| ansi(name, 16)).ok().and_then(|n| {
            PROTOCOLS
                .iter()
                .find(|(p, _)| p.as_bytes().eq_ignore_ascii_case(&n))
        });
        match found {
            Some((name, number)) => protoent(name, *number),
            None => {
                unsafe { RtlSetLastWin32Error(WSANO_DATA) };
                0
            }
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn getprotobynumber(number: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        match PROTOCOLS
            .iter()
            .find(|(_, n)| i32::from(*n) == number)
            .filter(|_| started().is_ok())
        {
            Some((name, n)) => protoent(name, *n),
            None => {
                unsafe { RtlSetLastWin32Error(WSANO_DATA) };
                0
            }
        }
    })())
}

// ---- address text --------------------------------------------------------
/// inet_addr: the address as stored (network order); INADDR_NONE on failure.
#[unsafe(no_mangle)]
pub extern "system" fn inet_addr(text_pointer: usize) -> u32 {
    let text_pointer = door_windows_abi::widen(text_pointer);
    {
        ansi(text_pointer, 64)
            .ok()
            .and_then(|t| text::inet_addr(&t))
            .map_or(u32::MAX, u32::from_ne_bytes)
    }
}
/// inet_ntoa: dotted text in a shared static buffer.
#[unsafe(no_mangle)]
pub extern "system" fn inet_ntoa(address: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        NTOA.fill(|_, b| {
            let mut out = [0u8; 16];
            let n = text::ntop4(address.to_ne_bytes(), &mut out);
            b.fill(0);
            b[..n].copy_from_slice(&out[..n]);
        })
    })())
}
/// Address text of `family` in its binary form (4 or 16 bytes); None when
/// the text is not one. IPv6 text may carry a `%scope` suffix.
fn parse_address(family: i32, text_bytes: &[u8]) -> Option<Vec<u8>> {
    match family {
        AF_INET => text::pton4(text_bytes).map(|a| a.to_vec()),
        AF_INET6 => {
            let host = text_bytes
                .split(|b| *b == b'%')
                .next()
                .unwrap_or(text_bytes);
            text::pton6(host).map(|a| a.to_vec())
        }
        _ => None,
    }
}
fn pton(family: i32, text_bytes: Result<Vec<u8>, u32>, destination: u64) -> i32 {
    if family != AF_INET && family != AF_INET6 {
        unsafe { RtlSetLastWin32Error(WSAEAFNOSUPPORT) };
        return -1;
    }
    match text_bytes.map(|t| parse_address(family, &t)) {
        Ok(Some(a)) => write(destination, &a).map_or(-1, |()| 1),
        Ok(None) => 0,
        Err(e) => {
            unsafe { RtlSetLastWin32Error(e) };
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn inet_pton(family: i32, source: usize, destination: usize) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    pton(family, ansi(source, 64), destination)
}
#[unsafe(no_mangle)]
pub extern "system" fn InetPtonW(family: i32, source: usize, destination: usize) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    pton(family, wide(source, 64), destination)
}
/// The text of a binary address of `family` read at `source`.
fn address_text(family: i32, source: u64) -> Result<Vec<u8>, u32> {
    match family {
        AF_INET => {
            let mut a = [0u8; 4];
            read(source, &mut a)?;
            let mut out = [0u8; 16];
            let n = text::ntop4(a, &mut out);
            Ok(out[..n].to_vec())
        }
        AF_INET6 => {
            let mut a = [0u8; 16];
            read(source, &mut a)?;
            let mut out = [0u8; 46];
            let n = text::ntop6(a, &mut out);
            Ok(out[..n].to_vec())
        }
        _ => Err(WSAEAFNOSUPPORT),
    }
}
fn ntop(family: i32, source: u64, destination: u64, size: u64, wide_text: bool) -> u64 {
    let result = address_text(family, source).and_then(|t| {
        store_text(destination, size as u32, &t, wide_text).map_err(|_| WSA_INVALID_PARAMETER)
    });
    match result {
        Ok(()) => destination,
        Err(e) => {
            unsafe { RtlSetLastWin32Error(e) };
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn inet_ntop(
    family: i32,
    source: usize,
    destination: usize,
    size: usize,
) -> usize {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    let size = door_windows_abi::widen(size);
    door_windows_abi::narrow((move || -> u64 {
        ntop(family, source, destination, size, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InetNtopW(
    family: i32,
    source: usize,
    destination: usize,
    size: usize,
) -> usize {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    let size = door_windows_abi::widen(size);
    door_windows_abi::narrow((move || -> u64 {
        ntop(family, source, destination, size, true)
    })())
}
/// "a.b.c.d[:port]" into a SOCKADDR_IN, or "addr", "[addr]:port" and
/// "addr%scope" into a SOCKADDR_IN6.
fn string_to_address(
    text_bytes: &[u8],
    family: i32,
    info: u64,
    address: u64,
    length: u64,
) -> Result<(), u32> {
    started()?;
    if info != 0 {
        return Err(WSAEINVAL);
    }
    let numeric_port = |p: &[u8]| -> Result<u16, u32> {
        text::service_port(p, 0)
            .filter(|_| !p.is_empty() && p.iter().all(u8::is_ascii_digit))
            .ok_or(WSAEINVAL)
    };
    match family {
        AF_INET => {
            let (host, port) = match text_bytes.iter().rposition(|b| *b == b':') {
                Some(at) => (&text_bytes[..at], numeric_port(&text_bytes[at + 1..])?),
                None => (text_bytes, 0),
            };
            let ip = text::inet_addr(host).ok_or(WSAEINVAL)?;
            store_sockaddr(address, length, AF_INET, mapped(ip), port)
        }
        AF_INET6 => {
            let (host, port) = match text_bytes.strip_prefix(b"[") {
                Some(rest) => {
                    let end = rest.iter().position(|b| *b == b']').ok_or(WSAEINVAL)?;
                    let port = match rest[end + 1..].strip_prefix(b":") {
                        Some(p) => numeric_port(p)?,
                        None if rest.len() == end + 1 => 0,
                        None => return Err(WSAEINVAL),
                    };
                    (&rest[..end], port)
                }
                None => (text_bytes, 0),
            };
            let ip: Ip = parse_address(AF_INET6, host)
                .ok_or(WSAEINVAL)?
                .try_into()
                .map_err(|_| WSAEINVAL)?;
            store_sockaddr(address, length, AF_INET6, ip, port)
        }
        _ => Err(WSAEINVAL),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAStringToAddressA(
    text_pointer: usize,
    family: i32,
    info: usize,
    address: usize,
    length: usize,
) -> i32 {
    let text_pointer = door_windows_abi::widen(text_pointer);
    let info = door_windows_abi::widen(info);
    let address = door_windows_abi::widen(address);
    let length = door_windows_abi::widen(length);
    {
        let result = ansi(text_pointer, 64)
            .and_then(|t| string_to_address(&t, family, info, address, length));
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAStringToAddressW(
    text_pointer: usize,
    family: i32,
    info: usize,
    address: usize,
    length: usize,
) -> i32 {
    let text_pointer = door_windows_abi::widen(text_pointer);
    let info = door_windows_abi::widen(info);
    let address = door_windows_abi::widen(address);
    let length = door_windows_abi::widen(length);
    {
        let result = wide(text_pointer, 64)
            .and_then(|t| string_to_address(&t, family, info, address, length));
        result.map_or_else(fail, |()| ok())
    }
}
fn address_to_string(
    address: u64,
    length: u32,
    info: u64,
    out: u64,
    out_length: u64,
    wide_text: bool,
) -> Result<(), u32> {
    started()?;
    if info != 0 {
        return Err(WSAEINVAL);
    }
    let (family, ip, port) = sockaddr_any(address, length as i32)?;
    let mut s = if family == AF_INET6 {
        let mut t = [0u8; 46];
        let n = text::ntop6(ip, &mut t);
        t[..n].to_vec()
    } else {
        let mut t = [0u8; 16];
        let n = text::ntop4(unmapped(&ip).unwrap_or([0; 4]), &mut t);
        t[..n].to_vec()
    };
    if port != 0 {
        if family == AF_INET6 {
            s.insert(0, b'[');
            s.extend_from_slice(alloc::format!("]:{port}").as_bytes());
        } else {
            s.extend_from_slice(alloc::format!(":{port}").as_bytes());
        }
    }
    let mut capacity = [0u8; 4];
    read(out_length, &mut capacity)?;
    let needed = (s.len() + 1) as u32;
    write(out_length, &needed.to_le_bytes())?;
    if u32::from_le_bytes(capacity) < needed {
        return Err(WSAEFAULT);
    }
    store_text(out, needed, &s, wide_text)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAddressToStringA(
    address: usize,
    length: u32,
    info: usize,
    out: usize,
    out_length: usize,
) -> i32 {
    let address = door_windows_abi::widen(address);
    let info = door_windows_abi::widen(info);
    let out = door_windows_abi::widen(out);
    let out_length = door_windows_abi::widen(out_length);
    {
        address_to_string(address, length, info, out, out_length, false)
            .map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAddressToStringW(
    address: usize,
    length: u32,
    info: usize,
    out: usize,
    out_length: usize,
) -> i32 {
    let address = door_windows_abi::widen(address);
    let info = door_windows_abi::widen(info);
    let out = door_windows_abi::widen(out);
    let out_length = door_windows_abi::widen(out_length);
    address_to_string(address, length, info, out, out_length, true).map_or_else(fail, |()| ok())
}
