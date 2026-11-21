//! ws2_32.dll exports: Winsock 2 over door.network (ntdll's `DoorNet*`).
//! Application memory is read and written only through ntdll's checked copy;
//! a blocking call polls its socket without holding the table lock.
use crate::error::*;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    pub(crate) fn RtlGetLastWin32Error() -> u32;
    pub(crate) fn RtlSetLastWin32Error(value: u32);
    pub(crate) fn DoorSleep(millis: u32);
    pub(crate) fn DoorGetTickCount64() -> u64;
    pub(crate) fn DoorGetCurrentThreadId() -> u32;
    pub(crate) fn DoorNetSocket(kind: u32, id: u64) -> u32;
    pub(crate) fn DoorNetClose(id: u32) -> u32;
    pub(crate) fn DoorNetBind(id: u32, port: u32, bound: u64) -> u32;
    pub(crate) fn DoorNetBindAddress(id: u32, address: u64, port: u32, bound: u64) -> u32;
    pub(crate) fn DoorNetConnect(id: u32, address: u32, port: u32) -> u32;
    pub(crate) fn DoorNetListen(id: u32, port: u32, backlog: u32) -> u32;
    pub(crate) fn DoorNetAccept(id: u32, out: u64) -> u32;
    pub(crate) fn DoorNetSend(id: u32, source: u64, bytes: u32, sent: u64) -> u32;
    pub(crate) fn DoorNetReceive(id: u32, destination: u64, bytes: u32, received: u64) -> u32;
    pub(crate) fn DoorNetSendTo(
        id: u32,
        address: u32,
        port: u32,
        source: u64,
        bytes: u32,
        sent: u64,
    ) -> u32;
    pub(crate) fn DoorNetReceiveFrom(
        id: u32,
        destination: u64,
        bytes: u32,
        received: u64,
        from: u64,
    ) -> u32;
    pub(crate) fn DoorNetShutdown(id: u32) -> u32;
    pub(crate) fn DoorNetConnect6(id: u32, address: u64, port: u32) -> u32;
    pub(crate) fn DoorNetAccept6(id: u32, out: u64) -> u32;
    pub(crate) fn DoorNetSendTo6(
        id: u32,
        address: u64,
        port: u32,
        source: u64,
        bytes: u32,
        sent: u64,
    ) -> u32;
    pub(crate) fn DoorNetReceiveFrom6(
        id: u32,
        destination: u64,
        bytes: u32,
        received: u64,
        from: u64,
    ) -> u32;
    pub(crate) fn DoorNetEndpoints6(id: u32, out: u64) -> u32;
    pub(crate) fn DoorNetResolve6(
        name: u64,
        length: u32,
        out: u64,
        max: u32,
        count: u64,
        timeout: u32,
    ) -> u32;
    pub(crate) fn DoorNetStatus6(out: u64) -> u32;
    pub(crate) fn DoorNetState(id: u32, out: u64) -> u32;
    pub(crate) fn DoorNetEndpoints(id: u32, out: u64) -> u32;
    pub(crate) fn DoorNetOption(id: u32, option: u32, value: u32) -> u32;
    pub(crate) fn DoorNetMulticast(id: u32, action: u32, group: u64, interface: u32) -> u32;
    pub(crate) fn DoorNetResolve(
        name: u64,
        length: u32,
        out: u64,
        max: u32,
        count: u64,
        timeout: u32,
    ) -> u32;
    pub(crate) fn DoorNetStatus(out: u64) -> u32;
    pub(crate) fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32;
}

pub(crate) const INVALID_SOCKET: u64 = u64::MAX;
pub(crate) const SOCKET_ERROR: i32 = -1;
pub(crate) const AF_INET: i32 = 2;
pub(crate) const AF_INET6: i32 = 23;
pub(crate) const SOCK_STREAM: i32 = 1;
pub(crate) const SOCK_DGRAM: i32 = 2;
pub(crate) const IPPROTO_TCP: i32 = 6;
pub(crate) const IPPROTO_UDP: i32 = 17;
const NET_TCP: u32 = 1;
const NET_UDP: u32 = 2;
const NET_RAW: u32 = 3;
pub(crate) const SOCK_RAW: i32 = 3;
/// The TTL (and IPv6 hop limit) Windows sends with.
const DEFAULT_TTL: u8 = 128;
/// door.network socket options (`net::option`).
mod net_option {
    pub const TTL: u32 = 4;
    pub const MULTICAST_TTL: u32 = 5;
    pub const MULTICAST_LOOP: u32 = 6;
    pub const HEADER_INCLUDED: u32 = 7;
    pub const MULTICAST_INTERFACE: u32 = 8;
}
/// door.network socket states (`net::SocketState`).
pub(crate) mod state {
    pub const CONNECTING: u8 = 1;
    pub const ESTABLISHED: u8 = 2;
    pub const PEER_CLOSED: u8 = 3;
    pub const CLOSING: u8 = 4;
}
/// Windows' TCP connect gives up after about 21 s (SYN + two retries).
const CONNECT_TIMEOUT_MS: u32 = 21_000;

// ---- application memory ------------------------------------------------
pub(crate) fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if out.is_empty() {
        return Ok(());
    }
    if pointer == 0 {
        return Err(WSAEFAULT);
    }
    // A console program's checked copies move at most 4 KiB each.
    const PIECE: usize = door_windows_abi::APPLICATION_COPY_BYTES;
    for (i, piece) in out.chunks_mut(PIECE).enumerate() {
        let at = pointer + (i * PIECE) as u64;
        if unsafe { DoorUserCopy(piece.as_mut_ptr() as u64, at, piece.len()) } != 0 {
            return Err(WSAEFAULT);
        }
    }
    Ok(())
}
pub(crate) fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if data.is_empty() {
        return Ok(());
    }
    if pointer == 0 {
        return Err(WSAEFAULT);
    }
    const PIECE: usize = door_windows_abi::APPLICATION_COPY_BYTES;
    for (i, piece) in data.chunks(PIECE).enumerate() {
        let at = pointer + (i * PIECE) as u64;
        if unsafe { DoorUserCopy(at, piece.as_ptr() as u64, piece.len()) } != 0 {
            return Err(WSAEFAULT);
        }
    }
    Ok(())
}
/// A pointer-sized program value (P17: 4 bytes on x86; handles such as
/// INVALID_SOCKET sign-extended as Windows widens them).
pub(crate) const POINTER: usize = core::mem::size_of::<usize>();
pub(crate) fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
pub(crate) fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
pub(crate) fn read_i32(pointer: u64) -> Result<i32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(i32::from_le_bytes(b))
}
pub(crate) fn write_i32(pointer: u64, value: i32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// The error of a failed call (WSAGetLastError is the thread's last error).
pub(crate) fn fail(error: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(error) };
    SOCKET_ERROR
}
pub(crate) fn ok() -> i32 {
    0
}
pub(crate) fn tick() -> u64 {
    unsafe { DoorGetTickCount64() }
}

// ---- startup ----------------------------------------------------------
static STARTED: AtomicU32 = AtomicU32::new(0);
pub(crate) fn started() -> Result<(), u32> {
    if STARTED.load(Ordering::Acquire) == 0 {
        Err(WSANOTINITIALISED)
    } else {
        Ok(())
    }
}
/// WSADATA (Win64 layout): versions, max sockets, max datagram, vendor
/// info pointer, description[257], system status[129].
#[unsafe(no_mangle)]
pub extern "system" fn WSAStartup(version: u16, data: usize) -> i32 {
    let data = door_windows_abi::widen(data);
    {
        let (major, minor) = (version & 0xFF, version >> 8);
        if major == 0 || (major == 1 && minor == 0) {
            return WSAVERNOTSUPPORTED as i32;
        }
        // The highest version this DLL supports is 2.2; older requests get theirs.
        let granted: u16 = if major > 2 || (major == 2 && minor >= 2) {
            0x0202
        } else {
            version
        };
        // WSADATA: wVersion, wHighVersion, then (x64) iMaxSockets, iMaxUdpDg,
        // lpVendorInfo, szDescription[257], szSystemStatus[129] (408 bytes) or
        // (x86) szDescription, szSystemStatus, iMaxSockets, iMaxUdpDg,
        // lpVendorInfo (400 bytes); Winsock 2 leaves the last three zero.
        const DESCRIPTION: usize = if POINTER == 8 { 16 } else { 4 };
        let mut b = [0u8; if POINTER == 8 { 408 } else { 400 }];
        b[..2].copy_from_slice(&granted.to_le_bytes());
        b[2..4].copy_from_slice(&0x0202u16.to_le_bytes());
        let description = b"WinSock 2.0";
        b[DESCRIPTION..DESCRIPTION + description.len()].copy_from_slice(description);
        let status = b"Running";
        b[DESCRIPTION + 257..DESCRIPTION + 257 + status.len()].copy_from_slice(status);
        if write(data, &b).is_err() {
            return WSAEFAULT as i32;
        }
        STARTED.fetch_add(1, Ordering::AcqRel);
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSACleanup() -> i32 {
    let mut current = STARTED.load(Ordering::Acquire);
    loop {
        if current == 0 {
            return fail(WSANOTINITIALISED);
        }
        match STARTED.compare_exchange(current, current - 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(now) => current = now,
        }
    }
    if current == 1 {
        // The last cleanup closes every socket the process still holds.
        let ids: alloc::vec::Vec<u32> = Table::lock().map(|mut t| t.drain()).unwrap_or_default();
        for id in ids {
            unsafe { DoorNetClose(id) };
        }
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAGetLastError() -> i32 {
    unsafe { RtlGetLastWin32Error() as i32 }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASetLastError(error: i32) {
    unsafe { RtlSetLastWin32Error(error as u32) }
}

// ---- the socket table ----------------------------------------------------
pub(crate) const SOCKETS: usize = 128;
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Entry {
    pub id: u32,
    pub stream: bool,
    pub nonblocking: bool,
    pub listening: bool,
    pub connected: bool,
    pub connecting: Option<u64>,
    pub bound: bool,
    pub shut_send: bool,
    pub shut_receive: bool,
    pub receive_timeout: u32,
    pub send_timeout: u32,
    pub keep_alive: bool,
    pub no_delay: bool,
    pub reuse: bool,
    pub broadcast: bool,
    pub linger: (u16, u16),
    /// SO_ERROR: a nonblocking connect's failure.
    pub error: u32,
    /// UDP peer from connect().
    pub peer: Option<(Ip, u16)>,
    /// AF_INET or AF_INET6.
    pub family: i32,
    /// IPV6_V6ONLY: an AF_INET6 socket refuses IPv4 (mapped) peers.
    pub v6only: bool,
    /// WSAEventSelect: event handle and network-event mask.
    pub event: u64,
    pub mask: u32,
    /// WSAAsyncSelect: window and message (exclusive with `event`).
    pub window: u64,
    pub message: u32,
    /// Events already signalled and not yet re-enabled.
    pub reported: u32,
    /// Events recorded for WSAEnumNetworkEvents, and FD_CONNECT's error.
    pub pending: u32,
    pub connect_error: u32,
    pub generation: u16,
    /// SOCK_RAW: the IP protocol (0: not raw), and IP_HDRINCL.
    pub raw: u8,
    pub header_included: bool,
    /// IP_TTL (IPV6_UNICAST_HOPS), IP_MULTICAST_TTL (IPV6_MULTICAST_HOPS),
    /// IP_MULTICAST_LOOP and IP_MULTICAST_IF (an IPv4 address as stored,
    /// or an IPv6 interface index).
    pub ttl: u8,
    pub multicast_ttl: u8,
    pub multicast_loop: bool,
    pub multicast_interface: u32,
    /// IP_TOS and IP_DONTFRAGMENT, kept for getsockopt.
    pub tos: u8,
    pub dont_fragment: bool,
}
struct Slots {
    entries: [Option<Entry>; SOCKETS],
    generations: [u16; SOCKETS],
}
struct Shared {
    owner: AtomicU32,
    slots: UnsafeCell<Slots>,
}
unsafe impl Sync for Shared {}
static TABLE: Shared = Shared {
    owner: AtomicU32::new(0),
    slots: UnsafeCell::new(Slots {
        entries: [None; SOCKETS],
        generations: [0; SOCKETS],
    }),
};
pub(crate) struct Table;
impl Table {
    pub(crate) fn lock() -> Result<TableGuard, u32> {
        let me = unsafe { DoorGetCurrentThreadId() };
        loop {
            match TABLE
                .owner
                .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => return Ok(TableGuard),
                Err(owner) if owner == me => return Err(WSAEINVAL),
                Err(_) => unsafe { DoorSleep(0) },
            }
        }
    }
}
pub(crate) struct TableGuard;
impl Drop for TableGuard {
    fn drop(&mut self) {
        TABLE.owner.store(0, Ordering::Release);
    }
}
/// SOCKET values: small multiples of four, never 0 or INVALID_SOCKET; the
/// generation in bits 12.. keeps a stale value from naming a reused slot.
fn encode(index: usize, generation: u16) -> u64 {
    0x100 + (index as u64) * 4 + (u64::from(generation) << 12)
}
fn decode(socket: u64) -> Option<(usize, u16)> {
    let offset = socket.checked_sub(0x100)?;
    let index = (offset & 0xFFF) / 4;
    if offset & 3 != 0 || index as usize >= SOCKETS || offset >> 28 != 0 {
        return None;
    }
    Some((index as usize, (offset >> 12) as u16))
}
impl TableGuard {
    fn slots(&mut self) -> &mut Slots {
        unsafe { &mut *TABLE.slots.get() }
    }
    pub(crate) fn insert(&mut self, mut entry: Entry) -> Result<u64, u32> {
        let slots = self.slots();
        let index = slots
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(WSAEMFILE)?;
        slots.generations[index] = slots.generations[index].wrapping_add(1);
        if slots.generations[index] == 0 {
            slots.generations[index] = 1;
        }
        entry.generation = slots.generations[index];
        slots.entries[index] = Some(entry);
        Ok(encode(index, entry.generation))
    }
    pub(crate) fn get(&mut self, socket: u64) -> Result<&mut Entry, u32> {
        let (index, generation) = decode(socket).ok_or(WSAENOTSOCK)?;
        self.slots().entries[index]
            .as_mut()
            .filter(|e| e.generation == generation)
            .ok_or(WSAENOTSOCK)
    }
    pub(crate) fn remove(&mut self, socket: u64) -> Result<Entry, u32> {
        let (index, generation) = decode(socket).ok_or(WSAENOTSOCK)?;
        let slot = &mut self.slots().entries[index];
        if slot.is_none_or(|e| e.generation != generation) {
            return Err(WSAENOTSOCK);
        }
        Ok(slot.take().unwrap())
    }
    fn drain(&mut self) -> alloc::vec::Vec<u32> {
        self.slots()
            .entries
            .iter_mut()
            .filter_map(Option::take)
            .map(|e| e.id)
            .collect()
    }
    /// Every live socket (WSAEventSelect's watcher).
    pub(crate) fn sockets(&mut self) -> alloc::vec::Vec<(u64, Entry)> {
        let slots = self.slots();
        slots
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|e| (encode(i, e.generation), e)))
            .collect()
    }
}
/// A copy of the entry (`f` may update it under the lock).
pub(crate) fn with_entry<R>(
    socket: u64,
    f: impl FnOnce(&mut Entry) -> Result<R, u32>,
) -> Result<R, u32> {
    started()?;
    let mut table = Table::lock()?;
    f(table.get(socket)?)
}
pub(crate) fn entry(socket: u64) -> Result<Entry, u32> {
    with_entry(socket, |e| Ok(*e))
}

// ---- door.network state ------------------------------------------------
#[derive(Clone, Copy, Debug)]
pub(crate) struct NetState {
    pub state: u8,
    pub queued_in: u32,
    pub queued_out: u32,
}
pub(crate) fn net_state(id: u32) -> Result<NetState, u32> {
    let mut b = [0u8; 12];
    match unsafe { DoorNetState(id, b.as_mut_ptr() as u64) } {
        0 => Ok(NetState {
            state: b[0],
            queued_in: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            queued_out: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        }),
        e => Err(e),
    }
}
/// Retry `attempt` while it would block, unless the socket is nonblocking:
/// sleep a tick between tries, give up after `timeout` ms (0: never).
pub(crate) fn blocking<T>(
    nonblocking: bool,
    timeout: u32,
    mut attempt: impl FnMut() -> Result<T, u32>,
) -> Result<T, u32> {
    let start = tick();
    loop {
        match attempt() {
            Err(WSAEWOULDBLOCK) if !nonblocking => {
                if timeout != 0 && tick().saturating_sub(start) >= u64::from(timeout) {
                    return Err(WSAETIMEDOUT);
                }
                unsafe { DoorSleep(1) };
            }
            other => return other,
        }
    }
}

// ---- SOCKADDR_IN / SOCKADDR_IN6 ------------------------------------------
/// An address in both families' common form: IPv6, an IPv4 address as
/// `::ffff:a.b.c.d` (what door.network's IPv6 requests take).
pub(crate) type Ip = [u8; 16];
pub(crate) const fn mapped(v4: [u8; 4]) -> Ip {
    [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, v4[0], v4[1], v4[2], v4[3],
    ]
}
/// The IPv4 address of a mapped one.
pub(crate) fn unmapped(ip: &Ip) -> Option<[u8; 4]> {
    (ip[..10] == [0; 10] && ip[10..12] == [0xff, 0xff]).then(|| [ip[12], ip[13], ip[14], ip[15]])
}
pub(crate) fn is_unspecified(ip: &Ip) -> bool {
    *ip == [0; 16] || unmapped(ip) == Some([0; 4])
}
pub(crate) const SOCKADDR_IN6_BYTES: i32 = 28;
/// The scope id Windows gives link-local addresses of the Ethernet
/// adapter (door-network's interface index).
pub(crate) const LINK_SCOPE: u32 = 2;
/// (family, address, port) of the SOCKADDR_IN or SOCKADDR_IN6 at
/// `pointer` of `length` bytes.
pub(crate) fn sockaddr_any(pointer: u64, length: i32) -> Result<(i32, Ip, u16), u32> {
    if length < 16 {
        return Err(WSAEFAULT);
    }
    let mut b = [0u8; 28];
    read(pointer, &mut b[..16])?;
    let family = i32::from(u16::from_le_bytes([b[0], b[1]]));
    let port = u16::from_be_bytes([b[2], b[3]]);
    match family {
        AF_INET => Ok((AF_INET, mapped(b[4..8].try_into().unwrap()), port)),
        AF_INET6 => {
            if length < SOCKADDR_IN6_BYTES {
                return Err(WSAEFAULT);
            }
            read(pointer, &mut b)?;
            Ok((AF_INET6, b[8..24].try_into().unwrap(), port))
        }
        _ => Err(WSAEAFNOSUPPORT),
    }
}
/// The address a socket of `family` was given: an AF_INET socket takes
/// SOCKADDR_IN, an AF_INET6 socket SOCKADDR_IN6.
pub(crate) fn sockaddr(pointer: u64, length: i32, family: i32) -> Result<(Ip, u16), u32> {
    if family == AF_INET6 && length < SOCKADDR_IN6_BYTES {
        return Err(WSAEFAULT);
    }
    let (given, ip, port) = sockaddr_any(pointer, length)?;
    if given != family {
        return Err(WSAEAFNOSUPPORT);
    }
    Ok((ip, port))
}
/// Store a SOCKADDR_IN (family AF_INET) or SOCKADDR_IN6 into `pointer`
/// (capacity in `*length`), updating the length; null pointers store
/// nothing. An AF_INET6 socket shows an IPv4 peer as ::ffff:a.b.c.d.
pub(crate) fn store_sockaddr(
    pointer: u64,
    length: u64,
    family: i32,
    address: Ip,
    port: u16,
) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    if length == 0 {
        return Err(WSAEFAULT);
    }
    let capacity = read_i32(length)?;
    let needed = if family == AF_INET6 {
        SOCKADDR_IN6_BYTES
    } else {
        16
    };
    if capacity < needed {
        return Err(WSAEFAULT);
    }
    let mut b = [0u8; 28];
    b[..2].copy_from_slice(&(family as u16).to_le_bytes());
    b[2..4].copy_from_slice(&port.to_be_bytes());
    if family == AF_INET6 {
        b[8..24].copy_from_slice(&address);
        if address[0] == 0xfe && address[1] & 0xc0 == 0x80 {
            b[24..28].copy_from_slice(&LINK_SCOPE.to_le_bytes());
        }
    } else {
        b[4..8].copy_from_slice(&unmapped(&address).unwrap_or([0; 4]));
    }
    write(pointer, &b[..needed as usize])?;
    write_i32(length, needed)
}
fn packed(address: [u8; 4]) -> u32 {
    u32::from_ne_bytes(address)
}

// ---- door.network transport, by the socket's family ------------------------
/// Connect (or set a UDP peer).
pub(crate) fn net_connect(e: &Entry, address: &Ip, port: u16) -> u32 {
    match (e.family, unmapped(address)) {
        (AF_INET, Some(v4)) => unsafe { DoorNetConnect(e.id, packed(v4), u32::from(port)) },
        (AF_INET, None) => WSAEAFNOSUPPORT,
        _ => unsafe { DoorNetConnect6(e.id, address.as_ptr() as u64, u32::from(port)) },
    }
}
/// A waiting connection: (status, socket id, peer, port).
pub(crate) fn net_accept(e: &Entry) -> (u32, u32, Ip, u16) {
    if e.family == AF_INET6 {
        let mut out = [0u8; 24];
        let status = unsafe { DoorNetAccept6(e.id, out.as_mut_ptr() as u64) };
        return (
            status,
            u32::from_le_bytes(out[..4].try_into().unwrap()),
            out[4..20].try_into().unwrap(),
            u16::from_le_bytes([out[20], out[21]]),
        );
    }
    let mut out = [0u8; 12];
    let status = unsafe { DoorNetAccept(e.id, out.as_mut_ptr() as u64) };
    (
        status,
        u32::from_le_bytes(out[..4].try_into().unwrap()),
        mapped(out[4..8].try_into().unwrap()),
        u16::from_le_bytes([out[8], out[9]]),
    )
}
/// One datagram to `address`: (status, bytes sent).
pub(crate) fn net_send_to(
    e: &Entry,
    address: &Ip,
    port: u16,
    source: u64,
    bytes: u32,
) -> (u32, u32) {
    let mut sent = [0u8; 4];
    let status = match (e.family, unmapped(address)) {
        (AF_INET, Some(v4)) => unsafe {
            DoorNetSendTo(
                e.id,
                packed(v4),
                u32::from(port),
                source,
                bytes,
                sent.as_mut_ptr() as u64,
            )
        },
        (AF_INET, None) => WSAEAFNOSUPPORT,
        _ => unsafe {
            DoorNetSendTo6(
                e.id,
                address.as_ptr() as u64,
                u32::from(port),
                source,
                bytes,
                sent.as_mut_ptr() as u64,
            )
        },
    };
    (status, u32::from_le_bytes(sent))
}
/// One datagram into `destination`: (status, bytes kept, sender, port).
pub(crate) fn net_receive_from(e: &Entry, destination: u64, bytes: u32) -> (u32, u32, Ip, u16) {
    let mut got = [0u8; 4];
    if e.family == AF_INET6 {
        let mut from = [0u8; 20];
        let status = unsafe {
            DoorNetReceiveFrom6(
                e.id,
                destination,
                bytes,
                got.as_mut_ptr() as u64,
                from.as_mut_ptr() as u64,
            )
        };
        return (
            status,
            u32::from_le_bytes(got),
            from[..16].try_into().unwrap(),
            u16::from_le_bytes([from[16], from[17]]),
        );
    }
    let mut from = [0u8; 8];
    let status = unsafe {
        DoorNetReceiveFrom(
            e.id,
            destination,
            bytes,
            got.as_mut_ptr() as u64,
            from.as_mut_ptr() as u64,
        )
    };
    (
        status,
        u32::from_le_bytes(got),
        mapped(from[..4].try_into().unwrap()),
        u16::from_le_bytes([from[4], from[5]]),
    )
}
/// Both ends of a door.network socket id of `family`.
pub(crate) fn net_endpoints(family: i32, id: u32) -> Result<(Endpoint, Endpoint), u32> {
    if family == AF_INET6 {
        let mut b = [0u8; 40];
        return match unsafe { DoorNetEndpoints6(id, b.as_mut_ptr() as u64) } {
            0 => Ok((
                (
                    b[..16].try_into().unwrap(),
                    u16::from_le_bytes([b[16], b[17]]),
                ),
                (
                    b[20..36].try_into().unwrap(),
                    u16::from_le_bytes([b[36], b[37]]),
                ),
            )),
            err => Err(err),
        };
    }
    let mut b = [0u8; 12];
    match unsafe { DoorNetEndpoints(id, b.as_mut_ptr() as u64) } {
        0 => Ok((
            (
                mapped(b[..4].try_into().unwrap()),
                u16::from_le_bytes([b[4], b[5]]),
            ),
            (
                mapped(b[6..10].try_into().unwrap()),
                u16::from_le_bytes([b[10], b[11]]),
            ),
        )),
        err => Err(err),
    }
}

// ---- sockets -----------------------------------------------------------
fn create(family: i32, kind: i32, protocol: i32) -> Result<u64, u32> {
    started()?;
    if family != AF_INET && family != AF_INET6 {
        return Err(WSAEAFNOSUPPORT);
    }
    let (stream, raw) = match (kind, protocol) {
        (SOCK_STREAM, 0 | IPPROTO_TCP) => (true, 0),
        (SOCK_DGRAM, 0 | IPPROTO_UDP) => (false, 0),
        (SOCK_STREAM | SOCK_DGRAM, _) => return Err(WSAEPROTONOSUPPORT),
        // Raw sockets need administrator rights: door-network checks the
        // caller's token. IPPROTO_IP (0) and IPPROTO_RAW (255), every
        // protocol at once, go as 255, which it does not support.
        (SOCK_RAW, 1..=254) => (false, protocol as u8),
        (SOCK_RAW, 0 | 255) => (false, 255),
        (SOCK_RAW, _) => return Err(WSAEPROTONOSUPPORT),
        _ => return Err(WSAESOCKTNOSUPPORT),
    };
    let mut id = [0u8; 4];
    // The family rides in bits 8-15: an AF_INET6 socket is listed in the
    // IPv6 tables (GetTcp6Table, netstat's [::] lines); a raw socket's
    // protocol in bits 16-23.
    let family_bits = if family == AF_INET6 {
        (AF_INET6 as u32) << 8
    } else {
        0
    };
    let kind_bits = match (stream, raw) {
        (true, _) => NET_TCP,
        (false, 0) => NET_UDP,
        (false, protocol) => NET_RAW | u32::from(protocol) << 16,
    };
    match unsafe { DoorNetSocket(kind_bits | family_bits, id.as_mut_ptr() as u64) } {
        0 => (),
        WSAENOBUFS => return Err(WSAEMFILE),
        WSAEOPNOTSUPP if raw != 0 => return Err(WSAEPROTONOSUPPORT),
        e => return Err(e),
    }
    let id = u32::from_le_bytes(id);
    let entry = Entry {
        id,
        stream,
        family,
        // IPV6_V6ONLY is on by default on Windows.
        v6only: true,
        raw,
        ttl: DEFAULT_TTL,
        multicast_ttl: 1,
        multicast_loop: true,
        ..Entry::default()
    };
    match Table::lock().and_then(|mut t| t.insert(entry)) {
        Ok(socket) => Ok(socket),
        Err(e) => {
            unsafe { DoorNetClose(id) };
            Err(e)
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn socket(family: i32, kind: i32, protocol: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        create(family, kind, protocol).unwrap_or_else(|e| {
            fail(e);
            INVALID_SOCKET
        })
    })())
}
/// WSASocketW/A: protocol info and group are not supported. Every socket
/// can take overlapped calls, so WSA_FLAG_OVERLAPPED changes nothing.
#[unsafe(no_mangle)]
pub extern "system" fn WSASocketW(
    family: i32,
    kind: i32,
    protocol: i32,
    info: usize,
    group: u32,
    _flags: u32,
) -> usize {
    let info = door_windows_abi::widen(info);
    door_windows_abi::narrow((move || -> u64 {
        if group != 0 {
            fail(WSAEINVAL);
            return INVALID_SOCKET;
        }
        // FROM_PROTOCOL_INFO (-1) takes the value from a WSAEnumProtocols
        // record.
        let (family, kind, protocol) = if info == 0 {
            (family, kind, protocol)
        } else {
            match crate::catalog::socket_triple(info) {
                Ok((f, k, p)) => (
                    if family == -1 { f } else { family },
                    if kind == -1 { k } else { kind },
                    if protocol == -1 { p } else { protocol },
                ),
                Err(e) => {
                    fail(e);
                    return INVALID_SOCKET;
                }
            }
        };
        door_windows_abi::widen(socket(family, kind, protocol))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASocketA(
    family: i32,
    kind: i32,
    protocol: i32,
    info: usize,
    group: u32,
    flags: u32,
) -> usize {
    let info = door_windows_abi::widen(info);
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(WSASocketW(
            family,
            kind,
            protocol,
            door_windows_abi::narrow(info),
            group,
            flags,
        ))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn closesocket(socket: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    {
        let removed = started()
            .and_then(|()| Table::lock())
            .and_then(|mut t| t.remove(socket));
        match removed {
            Ok(e) => {
                crate::overlapped::socket_closed(socket);
                unsafe { DoorNetClose(e.id) };
                ok()
            }
            Err(e) => fail(e),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn bind(socket: usize, name: usize, length: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    {
        let result = (|| {
            let e = entry(socket)?;
            let (address, port) = sockaddr(name, length, e.family)?;
            if e.bound {
                return Err(WSAEINVAL);
            }
            // Only one of this host's addresses (the loopback interface's or
            // the adapter's), or the unspecified one, can be bound.
            if !is_unspecified(&address) && !own_address(&address)? {
                return Err(WSAEADDRNOTAVAIL);
            }
            let mut bound = [0u8; 4];
            // A named address (127.0.0.1, ::1 or the adapter's): the socket
            // answers there alone, as on Windows.
            let status = if is_unspecified(&address) {
                unsafe { DoorNetBind(e.id, u32::from(port), bound.as_mut_ptr() as u64) }
            } else {
                unsafe {
                    DoorNetBindAddress(
                        e.id,
                        address.as_ptr() as u64,
                        u32::from(port),
                        bound.as_mut_ptr() as u64,
                    )
                }
            };
            match status {
                0 => with_entry(socket, |e| {
                    e.bound = true;
                    Ok(())
                }),
                e => Err(e),
            }
        })();
        result.map_or_else(fail, |()| ok())
    }
}
pub(crate) fn local_address() -> Result<[u8; 4], u32> {
    let mut status = [0u8; 88];
    match unsafe { DoorNetStatus(status.as_mut_ptr() as u64) } {
        0 => Ok(status[8..12].try_into().unwrap()),
        e => Err(e),
    }
}
/// The adapter's IPv6 addresses: (link-local, global; zeros when none).
pub(crate) fn local_addresses6() -> Result<(Ip, Ip), u32> {
    let mut status = [0u8; 85];
    match unsafe { DoorNetStatus6(status.as_mut_ptr() as u64) } {
        0 => Ok((
            status[..16].try_into().unwrap(),
            status[16..32].try_into().unwrap(),
        )),
        e => Err(e),
    }
}
/// Is `address` one of the adapter's (IPv4 or IPv6)?
fn own_address(address: &Ip) -> Result<bool, u32> {
    // The loopback interface: 127.0.0.0/8 and ::1.
    let mut loopback6 = [0u8; 16];
    loopback6[15] = 1;
    if *address == loopback6 {
        return Ok(true);
    }
    if let Some(v4) = unmapped(address) {
        return Ok(v4[0] == 127 || v4 == local_address()?);
    }
    let (link, global) = local_addresses6()?;
    Ok(*address == link || (*address == global && global != [0; 16]))
}
#[unsafe(no_mangle)]
pub extern "system" fn connect(socket: usize, name: usize, length: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    connect_impl(socket, name, length).map_or_else(fail, |()| ok())
}
fn connect_impl(socket: u64, name: u64, length: i32) -> Result<(), u32> {
    let e = entry(socket)?;
    let (address, port) = sockaddr(name, length, e.family)?;
    if e.family == AF_INET6 && e.v6only && unmapped(&address).is_some() && !is_unspecified(&address)
    {
        return Err(WSAEAFNOSUPPORT);
    }
    if !e.stream {
        // UDP: a default peer; the unspecified address clears it.
        if is_unspecified(&address) {
            return with_entry(socket, |e| {
                e.peer = None;
                Ok(())
            });
        }
        match net_connect(&e, &address, port) {
            0 => (),
            err => return Err(err),
        }
        return with_entry(socket, |e| {
            e.peer = Some((address, port));
            e.bound = true;
            Ok(())
        });
    }
    if e.listening {
        return Err(WSAEINVAL);
    }
    if e.connected {
        return Err(WSAEISCONN);
    }
    if e.connecting.is_some() {
        return Err(if e.nonblocking {
            WSAEALREADY
        } else {
            WSAEINVAL
        });
    }
    if is_unspecified(&address) || port == 0 {
        return Err(WSAEADDRNOTAVAIL);
    }
    begin_connect(socket, address, port)?;
    if e.nonblocking {
        return Err(WSAEWOULDBLOCK);
    }
    blocking(false, 0, || finish_connect(socket)).map(|_| ())
}
/// Start a stream connect; `finish_connect` follows it (connect, ConnectEx).
pub(crate) fn begin_connect(socket: u64, address: Ip, port: u16) -> Result<(), u32> {
    let e = entry(socket)?;
    unsafe { DoorNetOption(e.id, 3, CONNECT_TIMEOUT_MS) };
    match net_connect(&e, &address, port) {
        0 => (),
        WSAEINVAL => return Err(WSAENETUNREACH),
        err => return Err(err),
    }
    let started_at = tick();
    with_entry(socket, |e| {
        e.connecting = Some(started_at);
        e.bound = true;
        e.reported &= !FD_CONNECT;
        Ok(())
    })
}
/// Advance a pending connect: Ok when established, the failure (also kept
/// in SO_ERROR) when it ended, WSAEWOULDBLOCK while it is still going.
pub(crate) fn finish_connect(socket: u64) -> Result<bool, u32> {
    let e = entry(socket)?;
    let Some(started_at) = e.connecting else {
        return Ok(e.connected);
    };
    let s = net_state(e.id)?;
    let outcome = match s.state {
        state::ESTABLISHED | state::PEER_CLOSED => Ok(true),
        state::CONNECTING => return Err(WSAEWOULDBLOCK),
        // Reset before the handshake finished: refused, unless the
        // stack gave up after the connect timeout.
        _ if tick().saturating_sub(started_at) + 500 >= u64::from(CONNECT_TIMEOUT_MS) => {
            Err(WSAETIMEDOUT)
        }
        _ => Err(WSAECONNREFUSED),
    };
    unsafe { DoorNetOption(e.id, 3, 0) };
    with_entry(socket, |e| {
        e.connecting = None;
        match outcome {
            Ok(_) => e.connected = true,
            Err(err) => e.error = err,
        }
        Ok(())
    })?;
    outcome
}
#[unsafe(no_mangle)]
pub extern "system" fn listen(socket: usize, backlog: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    {
        let result = (|| {
            let e = entry(socket)?;
            if !e.stream {
                return Err(WSAEOPNOTSUPP);
            }
            if e.connected || e.connecting.is_some() {
                return Err(WSAEISCONN);
            }
            if e.listening {
                return Ok(());
            }
            if !e.bound {
                return Err(WSAEINVAL);
            }
            // SOMAXCONN (0x7fffffff) and the SOMAXCONN_HINT forms ask for the
            // most the stack allows.
            let backlog = if backlog <= 0 {
                1
            } else {
                backlog.min(8) as u32
            };
            match unsafe { DoorNetListen(e.id, 0, backlog) } {
                0 => with_entry(socket, |e| {
                    e.listening = true;
                    Ok(())
                }),
                err => Err(err),
            }
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn accept(socket: usize, address: usize, length: usize) -> usize {
    let socket = door_windows_abi::widen(socket);
    let address = door_windows_abi::widen(address);
    let length = door_windows_abi::widen(length);
    door_windows_abi::narrow((move || -> u64 {
        accept_impl(socket, address, length).unwrap_or_else(|e| {
            fail(e);
            INVALID_SOCKET
        })
    })())
}
fn accept_impl(socket: u64, address: u64, length: u64) -> Result<u64, u32> {
    let e = entry(socket)?;
    if !e.listening {
        return Err(WSAEINVAL);
    }
    let needed = if e.family == AF_INET6 {
        SOCKADDR_IN6_BYTES
    } else {
        16
    };
    if address != 0 && (length == 0 || read_i32(length)? < needed) {
        return Err(WSAEFAULT);
    }
    let (id, peer, port) = blocking(e.nonblocking, 0, || match net_accept(&e) {
        (0, id, peer, port) => Ok((id, peer, port)),
        (err, ..) => Err(err),
    })?;
    // The accepted socket inherits the listener's properties.
    let child = Entry {
        id,
        stream: true,
        family: e.family,
        v6only: e.v6only,
        nonblocking: e.nonblocking,
        connected: true,
        bound: true,
        receive_timeout: e.receive_timeout,
        send_timeout: e.send_timeout,
        keep_alive: e.keep_alive,
        no_delay: e.no_delay,
        event: e.event,
        mask: e.mask,
        ttl: e.ttl,
        multicast_ttl: e.multicast_ttl,
        multicast_loop: e.multicast_loop,
        ..Entry::default()
    };
    let accepted = match Table::lock().and_then(|mut t| t.insert(child)) {
        Ok(s) => s,
        Err(err) => {
            unsafe { DoorNetClose(id) };
            return Err(err);
        }
    };
    if e.no_delay {
        unsafe { DoorNetOption(id, 1, 1) };
    }
    // Accepting re-enables FD_ACCEPT for the listener.
    let _ = with_entry(socket, |l| {
        l.reported &= !FD_ACCEPT;
        Ok(())
    });
    store_sockaddr(address, length, e.family, peer, port)?;
    Ok(accepted)
}
pub(crate) const FD_READ: u32 = 0x01;
pub(crate) const FD_WRITE: u32 = 0x02;
pub(crate) const FD_ACCEPT: u32 = 0x08;
pub(crate) const FD_CONNECT: u32 = 0x10;
pub(crate) const FD_CLOSE: u32 = 0x20;

/// Send `bytes` from `buffer`: a blocking socket sends them all.
pub(crate) fn send_bytes(socket: u64, buffer: u64, bytes: u32) -> Result<u32, u32> {
    let e = entry(socket)?;
    if e.shut_send {
        return Err(WSAESHUTDOWN);
    }
    if e.stream && !e.connected {
        return Err(if e.connecting.is_some() {
            WSAEWOULDBLOCK
        } else {
            WSAENOTCONN
        });
    }
    if !e.stream {
        let Some((address, port)) = e.peer else {
            return Err(WSAENOTCONN);
        };
        return send_datagram(socket, address, port, buffer, bytes);
    }
    let mut done = 0u32;
    while done < bytes {
        let mut sent = [0u8; 4];
        let step = blocking(e.nonblocking, e.send_timeout, || {
            match unsafe {
                DoorNetSend(
                    e.id,
                    buffer + u64::from(done),
                    bytes - done,
                    sent.as_mut_ptr() as u64,
                )
            } {
                0 => Ok(u32::from_le_bytes(sent)),
                err => Err(err),
            }
        });
        match step {
            Ok(n) => done += n,
            Err(WSAEWOULDBLOCK) if done > 0 => break,
            Err(err) => return Err(err),
        }
        if e.nonblocking {
            break;
        }
    }
    with_entry(socket, |e| {
        e.reported &= !FD_WRITE;
        Ok(())
    })?;
    Ok(done)
}
pub(crate) fn send_datagram(
    socket: u64,
    address: Ip,
    port: u16,
    buffer: u64,
    bytes: u32,
) -> Result<u32, u32> {
    let e = entry(socket)?;
    if e.stream {
        return Err(WSAEISCONN);
    }
    if unmapped(&address) == Some([255, 255, 255, 255]) && !e.broadcast {
        return Err(WSAEACCES);
    }
    if e.family == AF_INET6 && e.v6only && unmapped(&address).is_some() {
        return Err(WSAEAFNOSUPPORT);
    }
    let sent = blocking(e.nonblocking, e.send_timeout, || {
        match net_send_to(&e, &address, port, buffer, bytes) {
            (0, sent) => Ok(sent),
            (err, _) => Err(err),
        }
    })?;
    with_entry(socket, |e| {
        e.bound = true;
        Ok(())
    })?;
    Ok(sent)
}
/// Receive into `buffer`; 0 is the peer's orderly close.
pub(crate) fn receive_bytes(socket: u64, buffer: u64, bytes: u32, flags: i32) -> Result<u32, u32> {
    let e = entry(socket)?;
    const MSG_PEEK: i32 = 2;
    const MSG_WAITALL: i32 = 8;
    if flags & MSG_PEEK != 0 || flags & !(MSG_PEEK | MSG_WAITALL) != 0 {
        return Err(WSAEOPNOTSUPP);
    }
    if e.shut_receive {
        return Err(WSAESHUTDOWN);
    }
    if !e.stream {
        return receive_datagram(socket, buffer, bytes, 0, 0).map(|(n, _, _)| n);
    }
    if !e.connected {
        return Err(WSAENOTCONN);
    }
    let mut done = 0u32;
    loop {
        let mut got = [0u8; 4];
        let step = blocking(
            e.nonblocking || done > 0 && flags & MSG_WAITALL == 0,
            e.receive_timeout,
            || match unsafe {
                DoorNetReceive(
                    e.id,
                    buffer + u64::from(done),
                    bytes - done,
                    got.as_mut_ptr() as u64,
                )
            } {
                0 => Ok(u32::from_le_bytes(got)),
                err => Err(err),
            },
        );
        match step {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(WSAEWOULDBLOCK) if done > 0 => break,
            Err(err) => return Err(err),
        }
        if done >= bytes || flags & MSG_WAITALL == 0 {
            break;
        }
    }
    with_entry(socket, |e| {
        e.reported &= !FD_READ;
        Ok(())
    })?;
    Ok(done)
}
pub(crate) fn receive_datagram(
    socket: u64,
    buffer: u64,
    bytes: u32,
    from: u64,
    from_length: u64,
) -> Result<(u32, Ip, u16), u32> {
    let e = entry(socket)?;
    if e.stream {
        return receive_bytes(socket, buffer, bytes, 0).map(|n| (n, [0; 16], 0));
    }
    if !e.bound {
        return Err(WSAEINVAL);
    }
    let needed = if e.family == AF_INET6 {
        SOCKADDR_IN6_BYTES
    } else {
        16
    };
    if from != 0 && (from_length == 0 || read_i32(from_length)? < needed) {
        return Err(WSAEFAULT);
    }
    loop {
        // A datagram cut to fit (WSAEMSGSIZE) still reports its sender.
        let mut cut = false;
        let (got, address, port) =
            blocking(
                e.nonblocking,
                e.receive_timeout,
                || match net_receive_from(&e, buffer, bytes) {
                    (0, got, address, port) => Ok((got, address, port)),
                    (WSAEMSGSIZE, got, address, port) => {
                        cut = true;
                        Ok((got, address, port))
                    }
                    (err, ..) => Err(err),
                },
            )?;
        // A connected UDP socket drops datagrams from anyone else, and an
        // IPv6-only socket those from IPv4 senders.
        if e.peer.is_some_and(|p| p != (address, port))
            || (e.family == AF_INET6 && e.v6only && unmapped(&address).is_some())
        {
            continue;
        }
        store_sockaddr(from, from_length, e.family, address, port)?;
        with_entry(socket, |e| {
            e.reported &= !FD_READ;
            Ok(())
        })?;
        if cut {
            return Err(WSAEMSGSIZE);
        }
        return Ok((got, address, port));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn send(socket: usize, buffer: usize, length: i32, flags: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let buffer = door_windows_abi::widen(buffer);
    {
        if length < 0 {
            return fail(WSAEINVAL);
        }
        if flags & !0x4 != 0 {
            // MSG_OOB (1) and MSG_PARTIAL are not supported.
            return fail(WSAEOPNOTSUPP);
        }
        match send_bytes(socket, buffer, length as u32) {
            Ok(n) => n as i32,
            Err(e) => fail(e),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn recv(socket: usize, buffer: usize, length: i32, flags: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let buffer = door_windows_abi::widen(buffer);
    {
        if length < 0 {
            return fail(WSAEINVAL);
        }
        match receive_bytes(socket, buffer, length as u32, flags) {
            Ok(n) => n as i32,
            Err(e) => fail(e),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn sendto(
    socket: usize,
    buffer: usize,
    length: i32,
    flags: i32,
    to: usize,
    to_length: i32,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let buffer = door_windows_abi::widen(buffer);
    let to = door_windows_abi::widen(to);
    {
        let result = (|| {
            if length < 0 {
                return Err(WSAEINVAL);
            }
            if flags & !0x4 != 0 {
                return Err(WSAEOPNOTSUPP);
            }
            let e = entry(socket)?;
            if e.stream || to == 0 {
                return send_bytes(socket, buffer, length as u32);
            }
            let (address, port) = sockaddr(to, to_length, e.family)?;
            send_datagram(socket, address, port, buffer, length as u32)
        })();
        result.map_or_else(fail, |n| n as i32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn recvfrom(
    socket: usize,
    buffer: usize,
    length: i32,
    flags: i32,
    from: usize,
    from_length: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let buffer = door_windows_abi::widen(buffer);
    let from = door_windows_abi::widen(from);
    let from_length = door_windows_abi::widen(from_length);
    {
        let result = (|| {
            if length < 0 {
                return Err(WSAEINVAL);
            }
            if flags != 0 {
                return Err(WSAEOPNOTSUPP);
            }
            receive_datagram(socket, buffer, length as u32, from, from_length).map(|(n, _, _)| n)
        })();
        result.map_or_else(fail, |n| n as i32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn shutdown(socket: usize, how: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    {
        let result = (|| {
            if !(0..=2).contains(&how) {
                return Err(WSAEINVAL);
            }
            let e = entry(socket)?;
            if e.stream && !e.connected {
                return Err(WSAENOTCONN);
            }
            // SD_SEND (1) or SD_BOTH (2) sends FIN; receiving stops locally.
            if how >= 1 && e.stream && !e.shut_send {
                match unsafe { DoorNetShutdown(e.id) } {
                    0 => (),
                    err => return Err(err),
                }
            }
            with_entry(socket, |e| {
                e.shut_send |= how >= 1;
                e.shut_receive |= how != 1;
                Ok(())
            })
        })();
        result.map_or_else(fail, |()| ok())
    }
}
/// An endpoint: address (`Ip` form) and port.
pub(crate) type Endpoint = (Ip, u16);
fn endpoints(socket: u64) -> Result<(Endpoint, Endpoint), u32> {
    let e = entry(socket)?;
    net_endpoints(e.family, e.id)
}
#[unsafe(no_mangle)]
pub extern "system" fn getsockname(socket: usize, name: usize, length: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    let length = door_windows_abi::widen(length);
    {
        let result = (|| {
            let e = entry(socket)?;
            if !e.bound && !e.listening {
                return Err(WSAEINVAL);
            }
            // The address bound (unspecified when bound to any), as door-network
            // answers it.
            let ((address, port), _) = endpoints(socket)?;
            store_sockaddr(name, length, e.family, address, port)
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn getpeername(socket: usize, name: usize, length: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    let length = door_windows_abi::widen(length);
    {
        let result = (|| {
            let e = entry(socket)?;
            if let Some((address, port)) = e.peer {
                return store_sockaddr(name, length, e.family, address, port);
            }
            if !e.connected {
                return Err(WSAENOTCONN);
            }
            let (_, (address, port)) = endpoints(socket)?;
            store_sockaddr(name, length, e.family, address, port)
        })();
        result.map_or_else(fail, |()| ok())
    }
}
const FIONBIO: i32 = 0x8004_667E_u32 as i32;
const FIONREAD: i32 = 0x4004_667F;
const SIOCATMARK: i32 = 0x4004_7307;
#[unsafe(no_mangle)]
pub extern "system" fn ioctlsocket(socket: usize, command: i32, argument: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let argument = door_windows_abi::widen(argument);
    {
        let result = (|| match command {
            FIONBIO => {
                let on = read_i32(argument)? != 0;
                with_entry(socket, |e| {
                    // A socket under WSAEventSelect stays nonblocking.
                    if !on && e.mask != 0 {
                        return Err(WSAEINVAL);
                    }
                    e.nonblocking = on;
                    Ok(())
                })
            }
            FIONREAD => {
                let e = entry(socket)?;
                let n = net_state(e.id)?.queued_in;
                write(argument, &n.to_le_bytes())
            }
            // No urgent data is ever pending.
            SIOCATMARK => write_i32(argument, 1),
            _ => Err(WSAEINVAL),
        })();
        result.map_or_else(fail, |()| ok())
    }
}
const SOL_SOCKET: i32 = 0xFFFF;
const SO_DEBUG: i32 = 0x0001;
const SO_ACCEPTCONN: i32 = 0x0002;
const SO_REUSEADDR: i32 = 0x0004;
const SO_KEEPALIVE: i32 = 0x0008;
const SO_DONTROUTE: i32 = 0x0010;
const SO_BROADCAST: i32 = 0x0020;
const SO_LINGER: i32 = 0x0080;
const SO_OOBINLINE: i32 = 0x0100;
const SO_DONTLINGER: i32 = !SO_LINGER;
const SO_EXCLUSIVEADDRUSE: i32 = !SO_REUSEADDR;
const SO_SNDBUF: i32 = 0x1001;
const SO_RCVBUF: i32 = 0x1002;
const SO_SNDTIMEO: i32 = 0x1005;
const SO_RCVTIMEO: i32 = 0x1006;
const SO_ERROR: i32 = 0x1007;
const SO_TYPE: i32 = 0x1008;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const SO_UPDATE_CONNECT_CONTEXT: i32 = 0x7010;
const TCP_NODELAY: i32 = 0x0001;
const IPPROTO_IPV6: i32 = 41;
const IPV6_V6ONLY: i32 = 27;
const IPPROTO_IP: i32 = 0;
const IP_HDRINCL: i32 = 2;
const IP_TOS: i32 = 3;
const IP_TTL: i32 = 4;
const IP_MULTICAST_IF: i32 = 9;
const IP_MULTICAST_TTL: i32 = 10;
const IP_MULTICAST_LOOP: i32 = 11;
const IP_ADD_MEMBERSHIP: i32 = 12;
const IP_DROP_MEMBERSHIP: i32 = 13;
const IP_DONTFRAGMENT: i32 = 14;
const IPV6_UNICAST_HOPS: i32 = 4;
const IPV6_MULTICAST_IF: i32 = 9;
const IPV6_MULTICAST_HOPS: i32 = 10;
const IPV6_MULTICAST_LOOP: i32 = 11;
/// IPV6_JOIN_GROUP and IPV6_LEAVE_GROUP are the same options.
const IPV6_ADD_MEMBERSHIP: i32 = 12;
const IPV6_DROP_MEMBERSHIP: i32 = 13;
/// Join (`IP_ADD_MEMBERSHIP`) or leave a group: `group` in the 16-byte
/// form door-network takes (IPv4-mapped for IPv4).
fn membership(e: &Entry, join: bool, group: [u8; 16], interface: u32) -> Result<(), u32> {
    // Measured on Windows: a stream socket is WSAEINVAL, an address that is
    // no group WSAEADDRNOTAVAIL.
    if e.stream || e.raw != 0 {
        return Err(WSAEINVAL);
    }
    let multicast = match unmapped(&group) {
        Some(v4) => (224..=239).contains(&v4[0]),
        None => group[0] == 0xff,
    };
    if !multicast {
        return Err(WSAEADDRNOTAVAIL);
    }
    let action = if join { 1 } else { 2 };
    match unsafe { DoorNetMulticast(e.id, action, group.as_ptr() as u64, interface) } {
        0 => Ok(()),
        err => Err(err),
    }
}
/// A door-network option of a datagram (or, for the TTL, any) socket.
fn net_set(e: &Entry, option: u32, value: u32) -> Result<(), u32> {
    match unsafe { DoorNetOption(e.id, option, value) } {
        0 => Ok(()),
        WSAEOPNOTSUPP => Err(WSAEINVAL),
        err => Err(err),
    }
}
/// The buffers door-network gives every socket.
const TCP_BUFFER: i32 = 32 * 1024;
const UDP_BUFFER: i32 = 16 * 1024;
#[unsafe(no_mangle)]
pub extern "system" fn setsockopt(
    socket: usize,
    level: i32,
    name: i32,
    value: usize,
    length: i32,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let value = door_windows_abi::widen(value);
    {
        let result = (|| {
            let int = || -> Result<i32, u32> {
                if length < 1 {
                    return Err(WSAEFAULT);
                }
                // BOOL options may be one byte.
                if length < 4 {
                    let mut b = [0u8; 1];
                    read(value, &mut b)?;
                    return Ok(i32::from(b[0]));
                }
                read_i32(value)
            };
            let e = entry(socket)?;
            match (level, name) {
                (SOL_SOCKET, SO_RCVTIMEO) => {
                    let v = int()?;
                    with_entry(socket, |e| {
                        e.receive_timeout = v.max(0) as u32;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_SNDTIMEO) => {
                    let v = int()?;
                    with_entry(socket, |e| {
                        e.send_timeout = v.max(0) as u32;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_KEEPALIVE) => {
                    let on = int()? != 0;
                    if e.stream {
                        // Windows' default keep-alive time (2 h) and interval.
                        unsafe { DoorNetOption(e.id, 2, if on { 7_200_000 } else { 0 }) };
                    }
                    with_entry(socket, |e| {
                        e.keep_alive = on;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_REUSEADDR | SO_EXCLUSIVEADDRUSE) => {
                    let on = int()? != 0;
                    with_entry(socket, |e| {
                        e.reuse = on && name == SO_REUSEADDR;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_BROADCAST) => {
                    let on = int()? != 0;
                    with_entry(socket, |e| {
                        e.broadcast = on;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_LINGER) => {
                    if length < 4 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 4];
                    read(value, &mut b)?;
                    with_entry(socket, |e| {
                        e.linger = (
                            u16::from_le_bytes([b[0], b[1]]),
                            u16::from_le_bytes([b[2], b[3]]),
                        );
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_DONTLINGER) => {
                    let off = int()? != 0;
                    with_entry(socket, |e| {
                        e.linger.0 = u16::from(!off);
                        Ok(())
                    })
                }
                // Fixed-size stack buffers: accepted, as Windows auto-tunes.
                (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF | SO_DEBUG | SO_DONTROUTE | SO_OOBINLINE) => {
                    int().map(|_| ())
                }
                // AcceptEx's socket takes the listener's options; ConnectEx's
                // is already connected here. The value names the listener.
                (SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT) => {
                    if (length as usize) < POINTER {
                        return Err(WSAEFAULT);
                    }
                    let listener = entry(read_pointer(value)?)?;
                    if !listener.listening || !e.connected {
                        return Err(WSAEINVAL);
                    }
                    with_entry(socket, |e| {
                        e.receive_timeout = listener.receive_timeout;
                        e.send_timeout = listener.send_timeout;
                        e.keep_alive = listener.keep_alive;
                        e.no_delay = listener.no_delay;
                        Ok(())
                    })
                }
                (SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT) if e.connected => Ok(()),
                (SOL_SOCKET, SO_UPDATE_CONNECT_CONTEXT) => Err(WSAENOTCONN),
                (IPPROTO_TCP, TCP_NODELAY) if e.stream => {
                    let on = int()? != 0;
                    unsafe { DoorNetOption(e.id, 1, u32::from(on)) };
                    with_entry(socket, |e| {
                        e.no_delay = on;
                        Ok(())
                    })
                }
                // IPV6_V6ONLY: set before the socket is bound or connected.
                (IPPROTO_IPV6, IPV6_V6ONLY) if e.family == AF_INET6 => {
                    if e.bound || e.connected || e.listening {
                        return Err(WSAEINVAL);
                    }
                    let only = int()? != 0;
                    with_entry(socket, |e| {
                        e.v6only = only;
                        Ok(())
                    })
                }
                // The unicast TTL (hop limit), as measured on Windows: 0-255
                // (0 is kept and read back; datagrams still leave with 1);
                // IPv6's -1 restores the default and reads back as 255.
                (IPPROTO_IP, IP_TTL) | (IPPROTO_IPV6, IPV6_UNICAST_HOPS) => {
                    let v = int()?;
                    let (ttl, sent) = match v {
                        -1 if level == IPPROTO_IPV6 => (255, DEFAULT_TTL),
                        0..=255 => (v as u8, (v as u8).max(1)),
                        _ => return Err(WSAEINVAL),
                    };
                    net_set(&e, net_option::TTL, u32::from(sent))?;
                    with_entry(socket, |e| {
                        e.ttl = ttl;
                        Ok(())
                    })
                }
                (IPPROTO_IP, IP_MULTICAST_TTL) | (IPPROTO_IPV6, IPV6_MULTICAST_HOPS)
                    if !e.stream =>
                {
                    let v = int()?;
                    let ttl = match v {
                        -1 if level == IPPROTO_IPV6 => 1,
                        0..=255 => v as u8,
                        _ => return Err(WSAEINVAL),
                    };
                    net_set(&e, net_option::MULTICAST_TTL, u32::from(ttl))?;
                    with_entry(socket, |e| {
                        e.multicast_ttl = ttl;
                        Ok(())
                    })
                }
                // Winsock's IP_MULTICAST_LOOP applies to the receiving socket.
                (IPPROTO_IP, IP_MULTICAST_LOOP) | (IPPROTO_IPV6, IPV6_MULTICAST_LOOP)
                    if !e.stream && e.raw == 0 =>
                {
                    let on = int()? != 0;
                    net_set(&e, net_option::MULTICAST_LOOP, u32::from(on))?;
                    with_entry(socket, |e| {
                        e.multicast_loop = on;
                        Ok(())
                    })
                }
                // The interface multicast leaves by: the adapter, or the
                // loopback interface (127.0.0.1, IPv6 index 1), which keeps it
                // on this host.
                (IPPROTO_IP, IP_MULTICAST_IF) | (IPPROTO_IPV6, IPV6_MULTICAST_IF) if !e.stream => {
                    if length < 4 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 4];
                    read(value, &mut b)?;
                    if e.raw == 0 {
                        net_set(&e, net_option::MULTICAST_INTERFACE, u32::from_le_bytes(b))?;
                    }
                    with_entry(socket, |e| {
                        e.multicast_interface = u32::from_le_bytes(b);
                        Ok(())
                    })
                }
                // ip_mreq: the group, then the interface's address.
                (IPPROTO_IP, IP_ADD_MEMBERSHIP | IP_DROP_MEMBERSHIP) => {
                    if length < 8 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 8];
                    read(value, &mut b)?;
                    let group = mapped(b[..4].try_into().unwrap());
                    let interface = u32::from_le_bytes(b[4..].try_into().unwrap());
                    membership(&e, name == IP_ADD_MEMBERSHIP, group, interface)
                }
                // ipv6_mreq: the group, then the interface index.
                (IPPROTO_IPV6, IPV6_ADD_MEMBERSHIP | IPV6_DROP_MEMBERSHIP)
                    if e.family == AF_INET6 =>
                {
                    if length < 20 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 20];
                    read(value, &mut b)?;
                    let group: [u8; 16] = b[..16].try_into().unwrap();
                    if unmapped(&group).is_some() {
                        return Err(WSAEINVAL);
                    }
                    let interface = u32::from_le_bytes(b[16..].try_into().unwrap());
                    membership(&e, name == IPV6_ADD_MEMBERSHIP, group, interface)
                }
                (IPPROTO_IP, IP_HDRINCL) if e.raw != 0 && e.family == AF_INET => {
                    let on = int()? != 0;
                    net_set(&e, net_option::HEADER_INCLUDED, u32::from(on))?;
                    with_entry(socket, |e| {
                        e.header_included = on;
                        Ok(())
                    })
                }
                (IPPROTO_IP, IP_HDRINCL) => Err(WSAEINVAL),
                // Accepted and kept: Doors sends no type of service and lets
                // the stack fragment.
                (IPPROTO_IP, IP_TOS) => {
                    let tos = int()?;
                    with_entry(socket, |e| {
                        e.tos = tos as u8;
                        Ok(())
                    })
                }
                (IPPROTO_IP, IP_DONTFRAGMENT) => {
                    let on = int()? != 0;
                    with_entry(socket, |e| {
                        e.dont_fragment = on;
                        Ok(())
                    })
                }
                (SOL_SOCKET | IPPROTO_TCP | IPPROTO_UDP | IPPROTO_IPV6 | 0, _) => {
                    Err(WSAENOPROTOOPT)
                }
                _ => Err(WSAEINVAL),
            }
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn getsockopt(
    socket: usize,
    level: i32,
    name: i32,
    value: usize,
    length: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let value = door_windows_abi::widen(value);
    let length = door_windows_abi::widen(length);
    {
        let result = (|| {
            let e = entry(socket)?;
            let capacity = read_i32(length)?;
            let v: i32 = match (level, name) {
                (SOL_SOCKET, SO_TYPE) => {
                    if e.stream {
                        SOCK_STREAM
                    } else if e.raw != 0 {
                        SOCK_RAW
                    } else {
                        SOCK_DGRAM
                    }
                }
                (IPPROTO_IP, IP_TTL) | (IPPROTO_IPV6, IPV6_UNICAST_HOPS) => i32::from(e.ttl),
                (IPPROTO_IP, IP_MULTICAST_TTL) | (IPPROTO_IPV6, IPV6_MULTICAST_HOPS)
                    if !e.stream =>
                {
                    i32::from(e.multicast_ttl)
                }
                (IPPROTO_IP, IP_MULTICAST_LOOP) | (IPPROTO_IPV6, IPV6_MULTICAST_LOOP)
                    if !e.stream && e.raw == 0 =>
                {
                    i32::from(e.multicast_loop)
                }
                (IPPROTO_IP, IP_MULTICAST_IF) | (IPPROTO_IPV6, IPV6_MULTICAST_IF) if !e.stream => {
                    e.multicast_interface as i32
                }
                (IPPROTO_IP, IP_HDRINCL) if e.raw != 0 && e.family == AF_INET => {
                    i32::from(e.header_included)
                }
                // Measured: not a raw socket's option (WSAEINVAL).
                (IPPROTO_IP, IP_HDRINCL) => return Err(WSAEINVAL),
                (IPPROTO_IP, IP_TOS) => i32::from(e.tos),
                (IPPROTO_IP, IP_DONTFRAGMENT) => i32::from(e.dont_fragment),
                (SOL_SOCKET, SO_ACCEPTCONN) => i32::from(e.listening),
                (SOL_SOCKET, SO_RCVTIMEO) => e.receive_timeout as i32,
                (SOL_SOCKET, SO_SNDTIMEO) => e.send_timeout as i32,
                (SOL_SOCKET, SO_KEEPALIVE) => i32::from(e.keep_alive),
                (SOL_SOCKET, SO_REUSEADDR) => i32::from(e.reuse),
                (SOL_SOCKET, SO_EXCLUSIVEADDRUSE) => 0,
                (SOL_SOCKET, SO_BROADCAST) => i32::from(e.broadcast),
                (SOL_SOCKET, SO_DONTLINGER) => i32::from(e.linger.0 == 0),
                (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => {
                    if e.stream {
                        TCP_BUFFER
                    } else {
                        UDP_BUFFER
                    }
                }
                (SOL_SOCKET, SO_ERROR) => {
                    let error = e.error;
                    with_entry(socket, |e| {
                        e.error = 0;
                        Ok(())
                    })?;
                    error as i32
                }
                (SOL_SOCKET, SO_LINGER) => {
                    if capacity < 4 {
                        return Err(WSAEFAULT);
                    }
                    let mut b = [0u8; 4];
                    b[..2].copy_from_slice(&e.linger.0.to_le_bytes());
                    b[2..].copy_from_slice(&e.linger.1.to_le_bytes());
                    write(value, &b)?;
                    return write_i32(length, 4);
                }
                (IPPROTO_TCP, TCP_NODELAY) if e.stream => i32::from(e.no_delay),
                (IPPROTO_IPV6, IPV6_V6ONLY) if e.family == AF_INET6 => i32::from(e.v6only),
                (SOL_SOCKET | IPPROTO_TCP | IPPROTO_UDP | IPPROTO_IPV6 | 0, _) => {
                    return Err(WSAENOPROTOOPT);
                }
                _ => return Err(WSAEINVAL),
            };
            if capacity < 4 {
                // BOOL options may be read into one byte.
                if capacity >= 1
                    && matches!(
                        name,
                        SO_KEEPALIVE
                            | SO_REUSEADDR
                            | SO_BROADCAST
                            | SO_ACCEPTCONN
                            | TCP_NODELAY
                            | IPV6_V6ONLY
                    )
                {
                    write(value, &[v as u8])?;
                    return write_i32(length, 1);
                }
                return Err(WSAEFAULT);
            }
            write_i32(value, v)?;
            write_i32(length, 4)
        })();
        result.map_or_else(fail, |()| ok())
    }
}
// Byte order helpers.
#[unsafe(no_mangle)]
pub extern "system" fn htons(v: u16) -> u16 {
    v.to_be()
}
#[unsafe(no_mangle)]
pub extern "system" fn ntohs(v: u16) -> u16 {
    u16::from_be(v)
}
#[unsafe(no_mangle)]
pub extern "system" fn htonl(v: u32) -> u32 {
    v.to_be()
}
#[unsafe(no_mangle)]
pub extern "system" fn ntohl(v: u32) -> u32 {
    u32::from_be(v)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAHtons(_socket: usize, v: u16, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    write(out, &v.to_be().to_le_bytes()).map_or_else(fail, |()| ok())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAHtonl(_socket: usize, v: u32, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    write(out, &v.to_be().to_le_bytes()).map_or_else(fail, |()| ok())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSANtohs(_socket: usize, v: u16, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    write(out, &u16::from_be(v).to_le_bytes()).map_or_else(fail, |()| ok())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSANtohl(_socket: usize, v: u32, out: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    write(out, &u32::from_be(v).to_le_bytes()).map_or_else(fail, |()| ok())
}
