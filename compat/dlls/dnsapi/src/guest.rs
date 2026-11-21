//! dnsapi.dll exports. Queries are sent from this process to the DNS
//! servers door.network reports (or the caller's server list) over ntdll's
//! `DoorNet*` sockets: UDP first, TCP when the reply is truncated. Record
//! lists are process-heap blocks freed with `DnsRecordListFree`/`DnsFree`.
use crate::*;
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use door_windows_abi::layout::{self, Field, Field::*};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorNetStatus(out: u64) -> u32;
    fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32;
    fn DoorNetSocket(kind: u32, id: u64) -> u32;
    fn DoorNetClose(id: u32) -> u32;
    fn DoorNetConnect(id: u32, address: u32, port: u32) -> u32;
    fn DoorNetSend(id: u32, source: u64, bytes: u32, sent: u64) -> u32;
    fn DoorNetReceive(id: u32, destination: u64, bytes: u32, received: u64) -> u32;
    fn DoorNetSendTo(id: u32, address: u32, port: u32, source: u64, bytes: u32, sent: u64) -> u32;
    fn DoorNetReceiveFrom(id: u32, destination: u64, bytes: u32, received: u64, from: u64) -> u32;
    fn DoorNetState(id: u32, out: u64) -> u32;
    fn DoorGetTickCount64() -> u64;
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
    fn DoorSleep(millis: u32);
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, size: usize) -> usize;
}

const WSAEWOULDBLOCK: u32 = 10035;
const NET_TCP: u32 = 1;
const NET_UDP: u32 = 2;
/// DNS_QUERY_* options this implementation honours.
const NO_WIRE_QUERY: u64 = 0x10;
const USE_TCP_ONLY: u64 = 0x2;
const ACCEPT_TRUNCATED: u64 = 0x1;
const ERROR_CANCELLED: u32 = 1223;

// ---- application memory ------------------------------------------------
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if out.is_empty() {
        return Ok(());
    }
    if pointer == 0 || unsafe { DoorUserCopy(out.as_mut_ptr() as u64, pointer, out.len()) } != 0 {
        return Err(status::ERROR_INVALID_PARAMETER);
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 || unsafe { DoorUserCopy(pointer, data.as_ptr() as u64, data.len()) } != 0 {
        return Err(status::ERROR_INVALID_PARAMETER);
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
/// The program's pointer width (P17: 4 for a 32-bit program).
const POINTER: usize = layout::POINTER;
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// DNS_RECORD's header: pNext, pName, wType, wDataLength, Flags, dwTtl,
/// dwReserved; the Data union follows (at 32 on x64, 24 on x86).
const RECORD_HEADER: [Field; 7] = [Pointer, Pointer, U16, U16, U32, U32, U32];
const DATA: usize = layout::size(&RECORD_HEADER);
/// DNS_QUERY_REQUEST: Version, QueryName, QueryType, QueryOptions,
/// pDnsServerList, InterfaceIndex, pQueryCompletionCallback, pQueryContext.
const QUERY_REQUEST: [Field; 8] = [U32, Pointer, U16, U64, Pointer, U32, Pointer, Pointer];
/// DNS_QUERY_RESULT: Version, QueryStatus, QueryOptions, pQueryRecords,
/// Reserved.
const QUERY_RESULT: [Field; 5] = [U32, U32, U64, Pointer, Pointer];
fn read_u64(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b)?;
    Ok(u64::from_le_bytes(b))
}
/// Text encodings of the three entry-point families.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Charset {
    Unicode = 1,
    Utf8 = 2,
    Ansi = 3,
}
fn name_at(pointer: u64, charset: Charset) -> Result<String, u32> {
    if pointer == 0 {
        return Err(status::ERROR_INVALID_PARAMETER);
    }
    let mut bytes = Vec::new();
    let mut units = Vec::new();
    for i in 0..1024u64 {
        if charset == Charset::Unicode {
            let mut b = [0u8; 2];
            read(pointer + i * 2, &mut b)?;
            let u = u16::from_le_bytes(b);
            if u == 0 {
                return String::from_utf16(&units).map_err(|_| status::ERROR_INVALID_NAME);
            }
            units.push(u);
        } else {
            let mut b = [0u8; 1];
            read(pointer + i, &mut b)?;
            if b[0] == 0 {
                return match charset {
                    Charset::Utf8 => {
                        String::from_utf8(bytes).map_err(|_| status::ERROR_INVALID_NAME)
                    }
                    _ => Ok(bytes.iter().map(|b| char::from(*b)).collect()),
                };
            }
            bytes.push(b[0]);
        }
    }
    Err(status::ERROR_INVALID_NAME)
}

// ---- resolver configuration ------------------------------------------------
/// A DNS server: address and port (53 unless a DnsQueryEx list names one).
type Server = ([u8; 4], u16);
fn adapter_servers() -> Vec<[u8; 4]> {
    let mut b = [0u8; door_service_protocol::net::STATUS_BYTES];
    if unsafe { DoorNetStatus(b.as_mut_ptr() as u64) } != 0 {
        return Vec::new();
    }
    let Some(s) = door_service_protocol::net::Status::decode(&b) else {
        return Vec::new();
    };
    if !(s.present && s.link && s.enabled) {
        return Vec::new();
    }
    s.dns.iter().copied().filter(|d| *d != [0; 4]).collect()
}
fn own_address() -> Option<[u8; 4]> {
    let mut b = [0u8; door_service_protocol::net::STATUS_BYTES];
    if unsafe { DoorNetStatus(b.as_mut_ptr() as u64) } != 0 {
        return None;
    }
    door_service_protocol::net::Status::decode(&b)
        .map(|s| s.address)
        .filter(|a| *a != [0; 4])
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
/// IP4_ARRAY { AddrCount, AddrArray[] } (addresses in network order).
fn ip4_array(pointer: u64) -> Result<Vec<Server>, u32> {
    let count = read_u32(pointer)?;
    if count == 0 || count > 64 {
        return Err(status::ERROR_INVALID_PARAMETER);
    }
    (0..u64::from(count))
        .map(|i| read_u32(pointer + 4 + i * 4).map(|a| (a.to_le_bytes(), 53)))
        .collect()
}

// ---- the wire ----------------------------------------------------------------
static NEXT_ID: AtomicU16 = AtomicU16::new(0x4d21);
fn tick() -> u64 {
    unsafe { DoorGetTickCount64() }
}
struct Socket(u32);
impl Drop for Socket {
    fn drop(&mut self) {
        unsafe { DoorNetClose(self.0) };
    }
}
fn socket(kind: u32) -> Result<Socket, u32> {
    let mut id = 0u32;
    match unsafe { DoorNetSocket(kind, &mut id as *mut u32 as u64) } {
        0 => Ok(Socket(id)),
        _ => Err(status::ERROR_OUTOFMEMORY),
    }
}
fn address_word(a: [u8; 4]) -> u32 {
    u32::from_ne_bytes(a)
}
/// One UDP exchange with `server`; None on timeout.
fn udp((server, port): Server, query: &[u8], millis: u64) -> Result<Option<Vec<u8>>, u32> {
    let s = socket(NET_UDP)?;
    let mut sent = 0u32;
    if unsafe {
        DoorNetSendTo(
            s.0,
            address_word(server),
            u32::from(port),
            query.as_ptr() as u64,
            query.len() as u32,
            &mut sent as *mut u32 as u64,
        )
    } != 0
    {
        return Ok(None);
    }
    let deadline = tick() + millis;
    let mut reply = vec![0u8; 4096];
    loop {
        let mut received = 0u32;
        let mut from = [0u8; 8];
        let code = unsafe {
            DoorNetReceiveFrom(
                s.0,
                reply.as_mut_ptr() as u64,
                reply.len() as u32,
                &mut received as *mut u32 as u64,
                from.as_mut_ptr() as u64,
            )
        };
        // Only the server's own reply to this query counts.
        if code == 0
            && from[..4] == server
            && u16::from_le_bytes([from[4], from[5]]) == port
            && received >= 2
            && reply[..2] == query[..2]
        {
            reply.truncate(received as usize);
            return Ok(Some(reply));
        }
        if code != 0 && code != WSAEWOULDBLOCK {
            return Ok(None);
        }
        if tick() >= deadline {
            return Ok(None);
        }
        unsafe { DoorSleep(5) };
    }
}
/// One TCP exchange (two-byte length prefix, RFC 1035 4.2.2).
fn tcp((server, port): Server, query: &[u8], millis: u64) -> Result<Option<Vec<u8>>, u32> {
    let s = socket(NET_TCP)?;
    if unsafe { DoorNetConnect(s.0, address_word(server), u32::from(port)) } != 0 {
        return Ok(None);
    }
    let deadline = tick() + millis;
    // Data goes out only once the connection is established.
    loop {
        let mut state = [0u8; 12];
        if unsafe { DoorNetState(s.0, state.as_mut_ptr() as u64) } != 0 {
            return Ok(None);
        }
        match state[0] {
            1 => (),
            2 | 3 => break,
            _ => return Ok(None),
        }
        if tick() >= deadline {
            return Ok(None);
        }
        unsafe { DoorSleep(2) };
    }
    let mut framed = (query.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(query);
    let mut done = 0usize;
    while done < framed.len() {
        let mut sent = 0u32;
        let code = unsafe {
            DoorNetSend(
                s.0,
                framed[done..].as_ptr() as u64,
                (framed.len() - done) as u32,
                &mut sent as *mut u32 as u64,
            )
        };
        match code {
            0 => done += sent as usize,
            WSAEWOULDBLOCK => (),
            _ => return Ok(None),
        }
        if tick() >= deadline {
            return Ok(None);
        }
        if code != 0 {
            unsafe { DoorSleep(5) };
        }
    }
    let mut stream = Vec::new();
    let mut buffer = [0u8; 192];
    loop {
        if stream.len() >= 2 {
            let length = usize::from(u16::from_be_bytes([stream[0], stream[1]]));
            if stream.len() >= 2 + length {
                return Ok(Some(stream[2..2 + length].to_vec()));
            }
        }
        let mut received = 0u32;
        let code = unsafe {
            DoorNetReceive(
                s.0,
                buffer.as_mut_ptr() as u64,
                buffer.len() as u32,
                &mut received as *mut u32 as u64,
            )
        };
        match code {
            0 if received == 0 => {
                // The server closed before a whole message, unless the
                // connection is still opening.
                let mut state = [0u8; 12];
                if unsafe { DoorNetState(s.0, state.as_mut_ptr() as u64) } != 0 || state[0] != 1 {
                    return Ok(None);
                }
            }
            0 => stream.extend_from_slice(&buffer[..received as usize]),
            WSAEWOULDBLOCK => (),
            _ => return Ok(None),
        }
        if tick() >= deadline {
            return Ok(None);
        }
        unsafe { DoorSleep(5) };
    }
}
/// Windows' resolver schedule: 1 s on the first server, then 1 s, 2 s and
/// 4 s on every server; ERROR_TIMEOUT when none answers.
fn exchange(servers: &[Server], name: &str, kind: u16, options: u64) -> Result<Response, u32> {
    if servers.is_empty() {
        return Err(status::NO_DNS_SERVERS);
    }
    // A transaction ID an off-path host cannot guess (RFC 5452); the
    // sequence only if the kernel's generator is not ready.
    let mut random = [0u8; 2];
    let id = if unsafe { DoorGenRandom(random.as_mut_ptr() as u64, 2) } == 0 {
        u16::from_le_bytes(random)
    } else {
        NEXT_ID.fetch_add(0x3b, Ordering::Relaxed)
    };
    let query = encode_query(id, name, kind)?;
    let tcp_only = options & USE_TCP_ONLY != 0;
    for (round, millis) in [1000u64, 1000, 2000, 4000].into_iter().enumerate() {
        let targets = if round == 0 { &servers[..1] } else { servers };
        for server in targets {
            let reply = if tcp_only {
                tcp(*server, &query, millis.max(4000))?
            } else {
                udp(*server, &query, millis)?
            };
            let Some(reply) = reply else { continue };
            // A reply must answer this question (RFC 5452).
            let Ok(response) = parse_response(&reply) else {
                continue;
            };
            if !answers(&query, &reply) {
                continue;
            }
            if response.truncated
                && !tcp_only
                && options & ACCEPT_TRUNCATED == 0
                && let Some(full) = tcp(*server, &query, 4000)?
                && answers(&query, &full)
                && let Ok(full) = parse_response(&full)
            {
                return Ok(full);
            }
            return Ok(response);
        }
    }
    Err(status::ERROR_TIMEOUT)
}
/// Names Windows answers without asking a server: localhost, the
/// computer's own name and an IPv4 literal (A queries).
fn local_answer(name: &str, kind: u16) -> Option<Vec<Record>> {
    let record = |data| Record {
        name: String::from(name.strip_suffix('.').unwrap_or(name)),
        kind,
        ttl: 0,
        section: Section::Answer,
        data,
    };
    let bare = name.strip_suffix('.').unwrap_or(name);
    if bare.eq_ignore_ascii_case("localhost") {
        return match kind {
            kind::A => Some(vec![record(Data::A([127, 0, 0, 1]))]),
            kind::AAAA => Some(vec![record(Data::Aaaa([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
            ]))]),
            _ => None,
        };
    }
    if kind == kind::A {
        if let Some(address) = ipv4_literal(bare) {
            return Some(vec![record(Data::A(address))]);
        }
        if bare.eq_ignore_ascii_case(&host_name()) {
            return Some(vec![record(Data::A(
                own_address().unwrap_or([127, 0, 0, 1]),
            ))]);
        }
    }
    None
}
fn query(
    name: &str,
    kind: u16,
    options: u64,
    servers: Option<Vec<Server>>,
) -> Result<Vec<Record>, (u32, Vec<Record>)> {
    if name.is_empty() || validate_name(name, format::DOMAIN) == status::ERROR_INVALID_NAME {
        return Err((status::ERROR_INVALID_NAME, Vec::new()));
    }
    if let Some(records) = local_answer(name, kind) {
        return Ok(records);
    }
    // No resolver cache: a cache-only query finds nothing.
    if options & NO_WIRE_QUERY != 0 {
        return Err((status::RECORD_DOES_NOT_EXIST, Vec::new()));
    }
    let servers =
        servers.unwrap_or_else(|| adapter_servers().into_iter().map(|a| (a, 53)).collect());
    let response = exchange(&servers, name, kind, options).map_err(|e| (e, Vec::new()))?;
    match outcome(&response, kind) {
        status::SUCCESS => Ok(response.records),
        // A negative answer still returns its authority records (the SOA).
        code => Err((code, response.records)),
    }
}

// ---- DNS_RECORD lists ----------------------------------------------------------
/// One process-heap block: a 16-byte header holding its size, then the
/// DNS_RECORD and the strings and data it points to.
fn block(size: usize) -> *mut u8 {
    let layout = core::alloc::Layout::from_size_align(size + 16, 16).expect("record layout");
    let raw = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if raw.is_null() {
        return raw;
    }
    unsafe {
        (raw as *mut usize).write(size);
        raw.add(16)
    }
}
fn release(pointer: u64) {
    if pointer == 0 {
        return;
    }
    unsafe {
        let raw = (pointer - 16) as *mut u8;
        let size = (raw as *const usize).read();
        alloc::alloc::dealloc(
            raw,
            core::alloc::Layout::from_size_align_unchecked(size + 16, 16),
        );
    }
}
fn encode_text(text: &str, charset: Charset) -> Vec<u8> {
    match charset {
        Charset::Unicode => text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect(),
        Charset::Utf8 => text.bytes().chain([0]).collect(),
        // Names off the wire are Latin-1 code points; ANSI keeps their bytes.
        Charset::Ansi => text
            .chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .chain([0])
            .collect(),
    }
}
/// Builds one DNS_RECORD block; returns its address.
fn record(r: &Record, charset: Charset) -> u64 {
    let mut strings: Vec<Vec<u8>> = vec![encode_text(&r.name, charset)];
    let names: Vec<&String> = match &r.data {
        Data::Name(n) => vec![n],
        Data::Mx { exchange, .. } => vec![exchange],
        Data::Srv { target, .. } => vec![target],
        Data::Soa { primary, admin, .. } => vec![primary, admin],
        Data::Txt(list) => list.iter().collect(),
        _ => Vec::new(),
    };
    strings.extend(names.iter().map(|n| encode_text(n, charset)));
    // The Data member at the program's width: DNS_PTR_DATA a pointer,
    // DNS_MX_DATA { pNameExchange, wPreference, Pad } (16/8 bytes),
    // DNS_SRV_DATA { pNameTarget, wPriority, wWeight, wPort, Pad }
    // (16/12), DNS_SOA_DATA { two names, five DWORDs } (40/28),
    // DNS_TXT_DATA { dwStringCount, pStringArray[] }.
    const MX: [Field; 3] = [Pointer, U16, U16];
    const SRV: [Field; 5] = [Pointer, U16, U16, U16, U16];
    const SOA: [Field; 7] = [Pointer, Pointer, U32, U32, U32, U32, U32];
    let data_length = match &r.data {
        Data::A(_) => 4,
        Data::Aaaa(_) => 16,
        Data::Name(_) => POINTER,
        Data::Mx { .. } => layout::size(&MX),
        Data::Srv { .. } => layout::size(&SRV),
        Data::Soa { .. } => layout::size(&SOA),
        Data::Txt(list) => POINTER + POINTER * list.len().max(1),
        Data::Other(raw) => raw.len(),
    };
    let record_bytes = (DATA + data_length).max(88).next_multiple_of(8);
    let total = record_bytes
        + strings
            .iter()
            .map(|s| s.len().next_multiple_of(8))
            .sum::<usize>();
    let base = block(total);
    if base.is_null() {
        return 0;
    }
    let b = unsafe { core::slice::from_raw_parts_mut(base, total) };
    let mut offsets = Vec::new();
    let mut at = record_bytes;
    for s in &strings {
        b[at..at + s.len()].copy_from_slice(s);
        offsets.push(base as u64 + at as u64);
        at += s.len().next_multiple_of(8);
    }
    let header = &RECORD_HEADER;
    layout::set(b, header, 1, offsets[0]);
    layout::set(b, header, 2, u64::from(r.kind));
    layout::set(b, header, 3, data_length as u64);
    layout::set(
        b,
        header,
        4,
        u64::from(r.section as u32 | (charset as u32) << 3),
    );
    layout::set(b, header, 5, u64::from(r.ttl));
    let data = &mut b[DATA..];
    match &r.data {
        Data::A(a) => data[..4].copy_from_slice(a),
        Data::Aaaa(a) => data[..16].copy_from_slice(a),
        Data::Name(_) => layout::set(data, &[Pointer], 0, offsets[1]),
        Data::Mx { preference, .. } => {
            layout::set(data, &MX, 0, offsets[1]);
            layout::set(data, &MX, 1, u64::from(*preference));
        }
        Data::Srv {
            priority,
            weight,
            port,
            ..
        } => {
            layout::set(data, &SRV, 0, offsets[1]);
            layout::set(data, &SRV, 1, u64::from(*priority));
            layout::set(data, &SRV, 2, u64::from(*weight));
            layout::set(data, &SRV, 3, u64::from(*port));
        }
        Data::Soa {
            serial,
            refresh,
            retry,
            expire,
            minimum,
            ..
        } => {
            for (index, value) in [
                offsets[1],
                offsets[2],
                u64::from(*serial),
                u64::from(*refresh),
                u64::from(*retry),
                u64::from(*expire),
                u64::from(*minimum),
            ]
            .into_iter()
            .enumerate()
            {
                layout::set(data, &SOA, index, value);
            }
        }
        Data::Txt(list) => {
            data[..4].copy_from_slice(&(list.len() as u32).to_le_bytes());
            for i in 0..list.len() {
                let at = POINTER * (1 + i);
                data[at..at + POINTER].copy_from_slice(&offsets[1 + i].to_le_bytes()[..POINTER]);
            }
        }
        Data::Other(raw) => data[..raw.len()].copy_from_slice(raw),
    }
    base as u64
}
/// Links the records into a list; 0 for none (or out of memory, after
/// freeing what was built).
fn list(records: &[Record], charset: Charset) -> Result<u64, u32> {
    let mut first = 0u64;
    let mut last = 0u64;
    for r in records {
        let block = record(r, charset);
        if block == 0 {
            DnsRecordListFree(door_windows_abi::narrow(first), 1);
            return Err(status::ERROR_OUTOFMEMORY);
        }
        if last == 0 {
            first = block;
        } else {
            // pNext, a pointer of this DLL's (and the program's) width.
            unsafe { (last as *mut usize).write(block as usize) };
        }
        last = block;
    }
    Ok(first)
}
fn deliver(out: u64, records: &[Record], charset: Charset) -> Result<(), u32> {
    if out == 0 {
        return Ok(());
    }
    let first = list(records, charset)?;
    write_pointer(out, first).inspect_err(|_| DnsRecordListFree(door_windows_abi::narrow(first), 1))
}
fn query_entry(
    name: u64,
    kind: u16,
    options: u32,
    extra: u64,
    results: u64,
    reserved: u64,
    charset: Charset,
) -> u32 {
    if reserved != 0 {
        return status::ERROR_INVALID_PARAMETER;
    }
    if results != 0 && write_pointer(results, 0).is_err() {
        return status::ERROR_INVALID_PARAMETER;
    }
    let name = match name_at(name, charset) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let servers = if extra != 0 {
        match ip4_array(extra) {
            Ok(list) => Some(list),
            Err(e) => return e,
        }
    } else {
        None
    };
    match query(&name, kind, u64::from(options), servers) {
        Ok(records) => deliver(results, &records, charset)
            .err()
            .unwrap_or(status::SUCCESS),
        Err((code, records)) => {
            if !records.is_empty() {
                let _ = deliver(results, &records, charset);
            }
            code
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsQuery_W(
    name: usize,
    kind: u16,
    options: u32,
    extra: usize,
    results: usize,
    reserved: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let extra = door_windows_abi::widen(extra);
    let results = door_windows_abi::widen(results);
    let reserved = door_windows_abi::widen(reserved);
    {
        query_entry(
            name,
            kind,
            options,
            extra,
            results,
            reserved,
            Charset::Unicode,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsQuery_A(
    name: usize,
    kind: u16,
    options: u32,
    extra: usize,
    results: usize,
    reserved: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let extra = door_windows_abi::widen(extra);
    let results = door_windows_abi::widen(results);
    let reserved = door_windows_abi::widen(reserved);
    query_entry(name, kind, options, extra, results, reserved, Charset::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsQuery_UTF8(
    name: usize,
    kind: u16,
    options: u32,
    extra: usize,
    results: usize,
    reserved: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let extra = door_windows_abi::widen(extra);
    let results = door_windows_abi::widen(results);
    let reserved = door_windows_abi::widen(reserved);
    query_entry(name, kind, options, extra, results, reserved, Charset::Utf8)
}
/// DNS_FREE_TYPE: DnsFreeFlat 0 (one block), DnsFreeRecordList 1.
#[unsafe(no_mangle)]
pub extern "system" fn DnsRecordListFree(list: usize, _free_type: u32) {
    let list = door_windows_abi::widen(list);
    {
        let mut next = list;
        while next != 0 {
            let following = unsafe { (next as *const usize).read() } as u64;
            release(next);
            next = following;
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsFree(data: usize, free_type: u32) {
    let data = door_windows_abi::widen(data);
    {
        match free_type {
            1 => DnsRecordListFree(door_windows_abi::narrow(data), 1),
            _ => release(data),
        }
    }
}

// ---- DnsQueryEx ----------------------------------------------------------------
struct Pending {
    name: String,
    kind: u16,
    options: u64,
    servers: Option<Vec<Server>>,
    result: u64,
    callback: u64,
    context: u64,
    cancelled: AtomicBool,
}
/// PDNS_QUERY_COMPLETION_ROUTINE(pQueryContext, pQueryResults).
type Completion = extern "system" fn(usize, usize);
fn complete(p: &Pending) {
    let (code, records) = match query(&p.name, p.kind, p.options, p.servers.clone()) {
        Ok(records) => (status::SUCCESS, records),
        Err(e) => e,
    };
    let code = if p.cancelled.load(Ordering::Acquire) {
        ERROR_CANCELLED
    } else {
        code
    };
    let first = if code == ERROR_CANCELLED {
        0
    } else {
        list(&records, Charset::Unicode).unwrap_or(0)
    };
    // DNS_QUERY_RESULT { Version, QueryStatus, QueryOptions, pQueryRecords }.
    let _ = write(p.result + 4, &code.to_le_bytes());
    let _ = write_pointer(p.result + layout::offset(&QUERY_RESULT, 3) as u64, first);
}
extern "system" fn worker(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let p = unsafe { Box::from_raw(argument as *mut Pending) };
        complete(&p);
        let call: Completion =
            unsafe { core::mem::transmute::<usize, Completion>(p.callback as usize) };
        call(p.context as usize, p.result as usize);
        0
    }
}
/// DNS_ADDR_ARRAY (IPv4 entries only) of a DnsQueryEx request; a port of
/// 0 means 53.
fn addr_array(pointer: u64) -> Result<Vec<Server>, u32> {
    let count = read_u32(pointer + 4)?;
    if count == 0 || count > 64 {
        return Err(status::ERROR_INVALID_PARAMETER);
    }
    // DNS_ADDR_ARRAY header is 32 bytes; each DNS_ADDR is 64 (a 32-byte
    // sockaddr buffer, then data).
    let mut list = Vec::new();
    for i in 0..u64::from(count) {
        let mut sa = [0u8; 8];
        read(pointer + 32 + i * 64, &mut sa)?;
        if u16::from_le_bytes([sa[0], sa[1]]) == 2 {
            let port = u16::from_be_bytes([sa[2], sa[3]]);
            list.push((
                [sa[4], sa[5], sa[6], sa[7]],
                if port == 0 { 53 } else { port },
            ));
        }
    }
    Ok(list)
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsQueryEx(request: usize, result: usize, cancel: usize) -> u32 {
    let request = door_windows_abi::widen(request);
    let result = door_windows_abi::widen(result);
    let cancel = door_windows_abi::widen(cancel);
    {
        let parsed = (|| -> Result<Pending, u32> {
            let version = read_u32(request)?;
            if !(1..=3).contains(&version) || result == 0 {
                return Err(status::ERROR_INVALID_PARAMETER);
            }
            let mut q = [0u8; layout::size(&QUERY_REQUEST)];
            read(request, &mut q)?;
            let field = |index| layout::get(&q, &QUERY_REQUEST, index);
            let name = name_at(field(1), Charset::Unicode)?;
            let list = field(4);
            Ok(Pending {
                name,
                kind: field(2) as u16,
                options: field(3),
                servers: if list != 0 {
                    Some(addr_array(list)?)
                } else {
                    None
                },
                result,
                callback: field(6),
                context: field(7),
                cancelled: AtomicBool::new(false),
            })
        })();
        let pending = match parsed {
            Ok(p) => p,
            Err(e) => return e,
        };
        if pending.callback == 0 {
            complete(&pending);
            return read_u32(result + 4).unwrap_or(status::ERROR_INVALID_PARAMETER);
        }
        let boxed = Box::into_raw(Box::new(pending));
        // DNS_QUERY_CANCEL: the request's address, for DnsCancelQuery.
        if cancel != 0 && write(cancel, &(boxed as u64).to_le_bytes()).is_err() {
            drop(unsafe { Box::from_raw(boxed) });
            return status::ERROR_INVALID_PARAMETER;
        }
        let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, boxed as u64, 0) };
        if thread == 0 {
            drop(unsafe { Box::from_raw(boxed) });
            return status::ERROR_OUTOFMEMORY;
        }
        unsafe { DoorCloseHandle(thread) };
        status::REQUEST_PENDING
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsCancelQuery(cancel: usize) -> u32 {
    let cancel = door_windows_abi::widen(cancel);
    {
        match read_u64(cancel) {
            Ok(pending) if pending != 0 => {
                // The worker reports ERROR_CANCELLED through the callback.
                unsafe {
                    (*(pending as *const Pending))
                        .cancelled
                        .store(true, Ordering::Release)
                };
                status::SUCCESS
            }
            _ => status::ERROR_INVALID_PARAMETER,
        }
    }
}

// ---- names and configuration -----------------------------------------------------
fn validate(name: u64, format: u32, charset: Charset) -> u32 {
    match name_at(name, charset) {
        Ok(n) => validate_name(&n, format),
        Err(_) => status::ERROR_INVALID_NAME,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsValidateName_W(name: usize, format: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    validate(name, format, Charset::Unicode)
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsValidateName_A(name: usize, format: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    validate(name, format, Charset::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsValidateName_UTF8(name: usize, format: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    validate(name, format, Charset::Utf8)
}
fn compare(a: u64, b: u64, charset: Charset) -> i32 {
    match (name_at(a, charset), name_at(b, charset)) {
        (Ok(a), Ok(b)) => i32::from(names_equal(&a, &b)),
        _ => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsNameCompare_W(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    compare(a, b, Charset::Unicode)
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsNameCompare_A(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    compare(a, b, Charset::Ansi)
}
/// Nothing is cached in this process: flushing always succeeds.
#[unsafe(no_mangle)]
pub extern "system" fn DnsFlushResolverCache() -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsFlushResolverCacheEntry_W(_name: usize) -> i32 {
    1
}
/// DNS_CONFIG_TYPE values answered: the DNS server list (IP4_ARRAY) and
/// the host names.
const CONFIG_PRIMARY_DOMAIN_W: u32 = 0;
const CONFIG_PRIMARY_DOMAIN_A: u32 = 1;
const CONFIG_PRIMARY_DOMAIN_UTF8: u32 = 2;
const CONFIG_DNS_SERVER_LIST: u32 = 6;
const CONFIG_HOST_NAME_W: u32 = 13;
const CONFIG_HOST_NAME_A: u32 = 14;
const CONFIG_HOST_NAME_UTF8: u32 = 15;
const CONFIG_FULL_HOST_NAME_W: u32 = 16;
const CONFIG_FULL_HOST_NAME_A: u32 = 17;
const CONFIG_FULL_HOST_NAME_UTF8: u32 = 18;
const CONFIG_FLAG_ALLOC: u32 = 1;
#[unsafe(no_mangle)]
pub extern "system" fn DnsQueryConfig(
    config: u32,
    flags: u32,
    _adapter: usize,
    reserved: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let reserved = door_windows_abi::widen(reserved);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        if reserved != 0 || length == 0 {
            return status::ERROR_INVALID_PARAMETER;
        }
        let data: Vec<u8> = match config {
            CONFIG_DNS_SERVER_LIST => {
                let servers = adapter_servers();
                if servers.is_empty() {
                    return status::NO_DNS_SERVERS;
                }
                let mut b = (servers.len() as u32).to_le_bytes().to_vec();
                for s in servers {
                    b.extend_from_slice(&s);
                }
                b
            }
            // No primary DNS suffix is configured.
            CONFIG_PRIMARY_DOMAIN_W | CONFIG_PRIMARY_DOMAIN_A | CONFIG_PRIMARY_DOMAIN_UTF8 => {
                return 9701;
            }
            CONFIG_HOST_NAME_W | CONFIG_FULL_HOST_NAME_W => {
                encode_text(&host_name(), Charset::Unicode)
            }
            CONFIG_HOST_NAME_A | CONFIG_FULL_HOST_NAME_A => {
                encode_text(&host_name(), Charset::Ansi)
            }
            CONFIG_HOST_NAME_UTF8 | CONFIG_FULL_HOST_NAME_UTF8 => {
                encode_text(&host_name(), Charset::Utf8)
            }
            _ => return status::ERROR_NOT_SUPPORTED,
        };
        if flags & CONFIG_FLAG_ALLOC != 0 {
            // The caller frees this with LocalFree: plain process-heap memory.
            let memory = unsafe {
                RtlAllocateHeap(
                    door_windows_abi::narrow(DoorGetProcessHeap()),
                    0,
                    data.len(),
                )
            };
            if memory == 0 {
                return status::ERROR_OUTOFMEMORY;
            }
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), memory as *mut u8, data.len()) };
            if write_pointer(buffer, door_windows_abi::widen(memory)).is_err() {
                return status::ERROR_INVALID_PARAMETER;
            }
            let _ = write(length, &(data.len() as u32).to_le_bytes());
            return status::SUCCESS;
        }
        let Ok(capacity) = read_u32(length) else {
            return status::ERROR_INVALID_PARAMETER;
        };
        let _ = write(length, &(data.len() as u32).to_le_bytes());
        if buffer == 0 || (capacity as usize) < data.len() {
            return status::ERROR_MORE_DATA;
        }
        write(buffer, &data).err().unwrap_or(status::SUCCESS)
    }
}
/// Dynamic updates (RFC 2136) are not implemented: they fail truthfully.
#[unsafe(no_mangle)]
pub extern "system" fn DnsModifyRecordsInSet_W(
    _add: usize,
    _delete: usize,
    _options: u32,
    _context: usize,
    _servers: usize,
    _reserved: usize,
) -> u32 {
    status::ERROR_NOT_SUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn DnsReplaceRecordSetW(
    _set: usize,
    _options: u32,
    _context: usize,
    _servers: usize,
    _reserved: usize,
) -> u32 {
    status::ERROR_NOT_SUPPORTED
}
