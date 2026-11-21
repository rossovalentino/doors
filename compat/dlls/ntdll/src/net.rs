//! Winsock's transport (ws2_32 imports these `DoorNet*` exports): sockets,
//! name resolution and adapter status over door.network. Results are Winsock
//! error codes (0 is success); application memory crosses only through the
//! checked copy, never as a Rust slice. One process-wide connection, opened
//! on first use, is serialized by a short lease.
use super::guest::copy;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use door_service_client::{Error, Network};
use door_service_protocol::{Code, net};

const WSAEWOULDBLOCK: u32 = 10035;
const WSAEINVAL: u32 = 10022;
const WSAENOTSOCK: u32 = 10038;
const WSAEADDRINUSE: u32 = 10048;
const WSAEADDRNOTAVAIL: u32 = 10049;
const WSAENOBUFS: u32 = 10055;
const WSAEOPNOTSUPP: u32 = 10045;
const WSAEACCES: u32 = 10013;
const WSAECONNRESET: u32 = 10054;
const WSAENETDOWN: u32 = 10050;
const WSAEFAULT: u32 = 10014;
const WSAEMSGSIZE: u32 = 10040;
const WSAHOST_NOT_FOUND: u32 = 11001;
const WSATRY_AGAIN: u32 = 11002;

static REGISTRY: AtomicU64 = AtomicU64::new(0);
struct State {
    owner: AtomicU32,
    network: UnsafeCell<Option<Network>>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    owner: AtomicU32::new(0),
    network: UnsafeCell::new(None),
};
/// The process's registry handle (bootstrap); the connection opens lazily.
pub fn initialize(registry: u64) {
    REGISTRY.store(registry, Ordering::Release);
}
struct Lease;
impl Lease {
    fn acquire() -> Result<Self, u32> {
        let id = super::threads::DoorGetCurrentThreadId();
        loop {
            match STATE
                .owner
                .compare_exchange(0, id, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => return Ok(Self),
                Err(owner) if owner == id => return Err(WSAEINVAL),
                Err(_) => door_sys::yield_now().map_err(|_| WSAENETDOWN)?,
            }
        }
    }
    fn network(&mut self) -> Result<&mut Network, u32> {
        // SAFETY: the lease is the only path to the cell.
        let slot = unsafe { &mut *STATE.network.get() };
        if slot.as_ref().is_none_or(|n| n.needs_reconnect()) {
            let registry = REGISTRY.load(Ordering::Acquire);
            *slot = Some(Network::connect(registry).map_err(|_| WSAENETDOWN)?);
        }
        Ok(slot.as_mut().unwrap())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        STATE.owner.store(0, Ordering::Release);
    }
}
fn wsa(error: Error) -> u32 {
    match error {
        Error::Service(Code::WouldBlock) => WSAEWOULDBLOCK,
        Error::Service(Code::InvalidRequest) => WSAEINVAL,
        Error::Service(Code::NotFound) => WSAENOTSOCK,
        Error::Service(Code::Exists) => WSAEADDRINUSE,
        Error::Service(Code::Resources) => WSAENOBUFS,
        Error::Service(Code::Unsupported) => WSAEOPNOTSUPP,
        Error::Service(Code::Denied) => WSAEACCES,
        Error::Service(Code::Closing) => WSAECONNRESET,
        _ => WSAENETDOWN,
    }
}
fn with<R>(f: impl FnOnce(&mut Network) -> Result<R, Error>) -> Result<R, u32> {
    let mut lease = Lease::acquire()?;
    f(lease.network()?).map_err(wsa)
}
fn put(pointer: u64, bytes: &[u8]) -> Result<(), u32> {
    copy(pointer, bytes.as_ptr() as u64, bytes.len()).map_err(|_| WSAEFAULT)
}
fn take(pointer: u64, bytes: &mut [u8]) -> Result<(), u32> {
    copy(bytes.as_mut_ptr() as u64, pointer, bytes.len()).map_err(|_| WSAEFAULT)
}
fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(0)
}
/// `address` is an `in_addr` as stored (network byte order in memory).
fn octets(address: u32) -> [u8; 4] {
    address.to_ne_bytes()
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorNetSocket(kind: u32, id: u64) -> u32 {
    result((|| {
        // Bits 8-15: the Winsock family (AF_INET6 sockets are listed in the
        // IPv6 tables); 0 is AF_INET.
        // Bits 16-23: a raw socket's IP protocol.
        let family = (kind >> 8) & 0xff;
        let protocol = (kind >> 16) as u8;
        if kind >> 24 != 0 || !matches!(family, 0 | 2 | 23) {
            return Err(WSAEINVAL);
        }
        let kind = u8::try_from(kind & 0xff)
            .ok()
            .filter(|k| matches!(*k, net::TCP | net::UDP | net::RAW))
            .filter(|k| (*k == net::RAW) == (protocol != 0))
            .ok_or(WSAEINVAL)?;
        let socket = if kind == net::RAW {
            with(|n| n.socket_raw(family == 23, protocol))?
        } else if family == 23 {
            with(|n| n.socket6(kind))?
        } else {
            with(|n| n.socket(kind))?
        };
        put(id, &socket.to_le_bytes())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetClose(id: u32) -> u32 {
    result(with(|n| n.close(id)))
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetBind(id: u32, port: u32, bound: u64) -> u32 {
    result((|| {
        let port = with(|n| n.bind(id, port as u16))?;
        put(bound, &u32::from(port).to_le_bytes())
    })())
}
/// `DoorNetBind` to the local address at `address` (16 bytes, IPv4
/// mapped): the socket answers on that address alone.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetBindAddress(id: u32, address: u64, port: u32, bound: u64) -> u32 {
    result((|| {
        let mut local = [0u8; 16];
        take(address, &mut local)?;
        let port = with(|n| n.bind_address(id, local, port as u16))?;
        put(bound, &u32::from(port).to_le_bytes())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetConnect(id: u32, address: u32, port: u32) -> u32 {
    result(with(|n| n.connect_to(id, octets(address), port as u16)))
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetListen(id: u32, port: u32, backlog: u32) -> u32 {
    result(with(|n| {
        n.listen_backlog(
            id,
            port as u16,
            backlog.clamp(1, net::MAX_BACKLOG as u32) as u8,
        )
    }))
}
/// `out`: 12 bytes, `[socket id u32][address 4][port u16][0 0]`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetAccept(id: u32, out: u64) -> u32 {
    result((|| {
        let (socket, address, port) = with(|n| n.accept(id))?;
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&socket.to_le_bytes());
        b[4..8].copy_from_slice(&address);
        b[8..10].copy_from_slice(&port.to_le_bytes());
        put(out, &b)
    })())
}
/// Queue as much of `bytes` as the socket takes; nothing taken is
/// WSAEWOULDBLOCK.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetSend(id: u32, source: u64, bytes: u32, sent: u64) -> u32 {
    result((|| {
        let mut done = 0usize;
        let mut chunk = [0u8; net::CHUNK];
        while done < bytes as usize {
            let n = (bytes as usize - done).min(chunk.len());
            take(source + done as u64, &mut chunk[..n])?;
            let taken = with(|net| net.send(id, &chunk[..n]))?;
            done += taken;
            if taken < n {
                break;
            }
        }
        if done == 0 && bytes != 0 {
            return Err(WSAEWOULDBLOCK);
        }
        put(sent, &(done as u32).to_le_bytes())
    })())
}
/// Received bytes; nothing yet is WSAEWOULDBLOCK, the peer's orderly close
/// is success with 0 bytes.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetReceive(id: u32, destination: u64, bytes: u32, received: u64) -> u32 {
    result((|| {
        let mut chunk = [0u8; net::CHUNK];
        let want = (bytes as usize).min(chunk.len());
        let received_bytes = {
            let mut lease = Lease::acquire()?;
            lease.network()?.receive(id, &mut chunk[..want])
        };
        let n = match received_bytes {
            Ok(0) if want != 0 => return Err(WSAEWOULDBLOCK),
            Ok(n) => n,
            // The peer's FIN: the orderly end of the stream.
            Err(Error::Service(Code::Closing)) => 0,
            // Closed without a FIN: reset by the peer.
            Err(Error::Service(Code::Io)) => return Err(WSAECONNRESET),
            Err(e) => return Err(wsa(e)),
        };
        put(destination, &chunk[..n])?;
        put(received, &(n as u32).to_le_bytes())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetSendTo(
    id: u32,
    address: u32,
    port: u32,
    source: u64,
    bytes: u32,
    sent: u64,
) -> u32 {
    result((|| {
        if bytes as usize > net::MAX_DATAGRAM {
            return Err(WSAEMSGSIZE);
        }
        // Parts are read from the caller as they go out.
        let mut fault = false;
        let n = with(|net| {
            net.send_to_with(
                id,
                octets(address),
                port as u16,
                bytes as usize,
                |offset, part| {
                    take(source + offset as u64, part).map_err(|_| {
                        fault = true;
                        Error::Malformed
                    })
                },
            )
        });
        if fault {
            return Err(WSAEFAULT);
        }
        put(sent, &(n? as u32).to_le_bytes())
    })())
}
/// One datagram; `from`: 8 bytes, `[address 4][port u16][0 0]`. A
/// datagram longer than `bytes` fills the buffer and fails with
/// WSAEMSGSIZE, as recvfrom does (the rest is discarded).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetReceiveFrom(
    id: u32,
    destination: u64,
    bytes: u32,
    received: u64,
    from: u64,
) -> u32 {
    result((|| {
        let mut fault = false;
        let reply = with(|net| {
            net.receive_from_with(id, bytes as usize, |offset, part| {
                put(destination + offset as u64, part).map_err(|_| {
                    fault = true;
                    Error::Malformed
                })
            })
        });
        if fault {
            return Err(WSAEFAULT);
        }
        let Some((length, address, port)) = reply? else {
            return Err(WSAEWOULDBLOCK);
        };
        let kept = length.min(bytes as usize);
        put(received, &(kept as u32).to_le_bytes())?;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&address);
        b[4..6].copy_from_slice(&port.to_le_bytes());
        put(from, &b)?;
        if length > bytes as usize {
            return Err(WSAEMSGSIZE);
        }
        Ok(())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetShutdown(id: u32) -> u32 {
    result(with(|n| n.shutdown(id)))
}
/// `out`: 12 bytes, `[state u8][0 0 0][queued in u32][queued out u32]`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetState(id: u32, out: u64) -> u32 {
    result((|| {
        let (state, queued_in, queued_out) = with(|n| n.state(id))?;
        let mut b = [0u8; 12];
        b[0] = state as u8;
        b[4..8].copy_from_slice(&queued_in.to_le_bytes());
        b[8..12].copy_from_slice(&queued_out.to_le_bytes());
        put(out, &b)
    })())
}
/// `out`: 12 bytes, `[local address 4][port u16][peer address 4][port u16]`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetEndpoints(id: u32, out: u64) -> u32 {
    result((|| {
        let ((local, local_port), (peer, peer_port)) = with(|n| n.endpoints(id))?;
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&local);
        b[4..6].copy_from_slice(&local_port.to_le_bytes());
        b[6..10].copy_from_slice(&peer);
        b[10..12].copy_from_slice(&peer_port.to_le_bytes());
        put(out, &b)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetOption(id: u32, option: u32, value: u32) -> u32 {
    result(with(|n| n.option(id, option as u8, value)))
}
/// Join (1) or leave (2) the multicast group at `group` (16 bytes, an IPv4
/// group IPv4-mapped) on `interface` (an IPv4 address or IPv6 index).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetMulticast(id: u32, action: u32, group: u64, interface: u32) -> u32 {
    result((|| {
        let mut address = [0u8; 16];
        take(group, &mut address)?;
        let action = u8::try_from(action).map_err(|_| WSAEINVAL)?;
        let mut lease = Lease::acquire()?;
        match lease.network()?.multicast(id, action, address, interface) {
            Ok(()) => Ok(()),
            // Winsock (measured): a group joined twice is WSAEINVAL, one
            // left without joining WSAEADDRNOTAVAIL.
            Err(Error::Service(Code::Exists)) => Err(WSAEINVAL),
            Err(Error::Service(Code::NotFound)) => Err(WSAEADDRNOTAVAIL),
            Err(e) => Err(wsa(e)),
        }
    })())
}
/// Resolve a UTF-8 host name (blocking, at most `timeout` ms): up to `max`
/// IPv4 addresses into `out` (4 bytes each) and their count.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetResolve(
    name: u64,
    length: u32,
    out: u64,
    max: u32,
    count: u64,
    timeout: u32,
) -> u32 {
    result((|| {
        let mut text = [0u8; 255];
        let length = length as usize;
        if length == 0 || length > text.len() {
            return Err(WSAHOST_NOT_FOUND);
        }
        take(name, &mut text[..length])?;
        let name = core::str::from_utf8(&text[..length]).map_err(|_| WSAHOST_NOT_FOUND)?;
        let (query, immediate) = with(|n| n.resolve(name))
            .map_err(|e| if e == WSAEINVAL { WSAHOST_NOT_FOUND } else { e })?;
        let mut list = [[0u8; 4]; 8];
        let found = if let Some(address) = immediate {
            list[0] = address;
            1
        } else {
            let start = door_sys::time().map_err(|_| WSAENETDOWN)?;
            let limit =
                door_abi::platform::ticks_from_millis(u64::from(timeout)).unwrap_or(u64::MAX);
            loop {
                match with(|n| n.resolved(query)) {
                    Ok(Some(addresses)) => {
                        list = addresses.list;
                        break addresses.count;
                    }
                    Ok(None) => (),
                    Err(WSAENOTSOCK) => return Err(WSAHOST_NOT_FOUND),
                    Err(e) => return Err(e),
                }
                let now = door_sys::time().map_err(|_| WSAENETDOWN)?;
                if now.saturating_sub(start) >= limit {
                    return Err(WSATRY_AGAIN);
                }
                let _ = door_sys::sleep(now + 1);
            }
        };
        let kept = found.min(max as usize);
        for (i, address) in list[..kept].iter().enumerate() {
            put(out + 4 * i as u64, address)?;
        }
        put(count, &(kept as u32).to_le_bytes())
    })())
}
/// The adapter (`net::Status`, `net::STATUS_BYTES` bytes into `out`).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetStatus(out: u64) -> u32 {
    result((|| {
        let status = with(|n| n.status())?;
        put(out, &status.encode())
    })())
}
/// One ICMP echo (`net::EchoRequest`, `length` bytes at `request`): waits
/// for its outcome (`net::Echo`, 18 bytes into `out`) without holding the
/// connection, so the process's sockets keep working meanwhile.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetPing(request: u64, length: u32, out: u64) -> u32 {
    result((|| {
        if length as usize > 12 + net::PATTERN_BYTES {
            return Err(WSAEINVAL);
        }
        let mut b = [0u8; 12 + net::PATTERN_BYTES];
        take(request, &mut b[..length as usize])?;
        let request = net::EchoRequest::decode(&b[..length as usize]).ok_or(WSAEINVAL)?;
        let token = with(|n| n.ping_start(&request))?;
        loop {
            if let Some(echo) = with(|n| n.ping_result(token))? {
                return put(out, &echo.encode());
            }
            let _ = door_sys::time().and_then(|t| door_sys::sleep(t + 1));
        }
    })())
}
/// The stack's TCP and UDP sockets: up to `capacity` `net::Connection`
/// records (20 bytes each) into `out`, the total count into `total`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetConnections(out: u64, capacity: u32, total: u64) -> u32 {
    result((|| {
        let mut page = [net::Connection::default(); net::diag::PAGE];
        let mut written = 0usize;
        let mut b = [0u8; net::CONNECTION_BYTES];
        loop {
            let (all, count) = with(|n| n.connections(written as u16, &mut page))?;
            if written == 0 {
                put(total, &(all as u32).to_le_bytes())?;
            }
            for c in &page[..count] {
                if written < capacity as usize {
                    c.encode(&mut b);
                    put(out + (written * net::CONNECTION_BYTES) as u64, &b)?;
                }
                written += 1;
            }
            if count == 0 || written >= all || written >= capacity as usize {
                return Ok(());
            }
        }
    })())
}

/// The stack's AF_INET6 sockets: up to `capacity` `net::Connection6`
/// records (52 bytes each) into `out`, the total count into `total`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetConnections6(out: u64, capacity: u32, total: u64) -> u32 {
    result((|| {
        let mut page = [net::Connection6::default(); net::diag::PAGE6];
        let mut written = 0usize;
        let mut b = [0u8; net::CONNECTION6_BYTES];
        loop {
            let (all, count) = with(|n| n.connections6(written as u16, &mut page))?;
            if written == 0 {
                put(total, &(all as u32).to_le_bytes())?;
            }
            for c in &page[..count] {
                if written < capacity as usize {
                    c.encode(&mut b);
                    put(out + (written * net::CONNECTION6_BYTES) as u64, &b)?;
                }
                written += 1;
            }
            if count == 0 || written >= all || written >= capacity as usize {
                return Ok(());
            }
        }
    })())
}

/// The IPv4 protocol counters (`net::stats::Counters`, `COUNTERS_BYTES`).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetStatistics(out: u64) -> u32 {
    result((|| {
        let counters = with(|n| n.statistics())?;
        put(out, &counters.encode())
    })())
}
/// The ARP table: up to `capacity` `net::stats::Neighbor` records (12
/// bytes each) into `out`, the total count into `total`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetNeighbors(out: u64, capacity: u32, total: u64) -> u32 {
    use net::stats::{NEIGHBOR_BYTES, Neighbor, PAGE};
    result((|| {
        let mut page = [Neighbor::default(); PAGE];
        let mut written = 0usize;
        let mut b = [0u8; NEIGHBOR_BYTES];
        loop {
            let (all, count) = with(|n| n.neighbors(written as u16, &mut page))?;
            if written == 0 {
                put(total, &(all as u32).to_le_bytes())?;
            }
            for n in &page[..count] {
                if written < capacity as usize {
                    n.encode(&mut b);
                    put(out + (written * NEIGHBOR_BYTES) as u64, &b)?;
                }
                written += 1;
            }
            if count == 0 || written >= all || written >= capacity as usize {
                return Ok(());
            }
        }
    })())
}

// ---- IPv6 and dual-stack (`net::v6`): addresses are 16 bytes in memory
// (an IPv4 peer as ::ffff:a.b.c.d), endpoints 18: `[address 16][port u16]`.
fn address16(pointer: u64) -> Result<[u8; 16], u32> {
    let mut a = [0u8; 16];
    take(pointer, &mut a)?;
    Ok(a)
}
fn put_endpoint(pointer: u64, address: [u8; 16], port: u16) -> Result<(), u32> {
    let mut b = [0u8; 20];
    net::v6::encode_endpoint(address, port, &mut b);
    put(pointer, &b)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetConnect6(id: u32, address: u64, port: u32) -> u32 {
    result((|| {
        let a = address16(address)?;
        with(|n| n.connect_to6(id, a, port as u16))
    })())
}
/// `out`: 24 bytes, `[socket id u32][address 16][port u16][0 0]`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetAccept6(id: u32, out: u64) -> u32 {
    result((|| {
        let (socket, address, port) = with(|n| n.accept6(id))?;
        let mut b = [0u8; 24];
        b[..4].copy_from_slice(&socket.to_le_bytes());
        net::v6::encode_endpoint(address, port, &mut b[4..22]);
        put(out, &b)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetSendTo6(
    id: u32,
    address: u64,
    port: u32,
    source: u64,
    bytes: u32,
    sent: u64,
) -> u32 {
    result((|| {
        if bytes as usize > net::MAX_DATAGRAM {
            return Err(WSAEMSGSIZE);
        }
        let a = address16(address)?;
        let mut fault = false;
        let n = with(|net| {
            net.send_to6_with(id, a, port as u16, bytes as usize, |offset, part| {
                take(source + offset as u64, part).map_err(|_| {
                    fault = true;
                    Error::Malformed
                })
            })
        });
        if fault {
            return Err(WSAEFAULT);
        }
        put(sent, &(n? as u32).to_le_bytes())
    })())
}
/// One datagram; `from`: 20 bytes, `[address 16][port u16][0 0]`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetReceiveFrom6(
    id: u32,
    destination: u64,
    bytes: u32,
    received: u64,
    from: u64,
) -> u32 {
    result((|| {
        let mut fault = false;
        let reply = with(|net| {
            net.receive_from6_with(id, bytes as usize, |offset, part| {
                put(destination + offset as u64, part).map_err(|_| {
                    fault = true;
                    Error::Malformed
                })
            })
        });
        if fault {
            return Err(WSAEFAULT);
        }
        let Some((length, address, port)) = reply? else {
            return Err(WSAEWOULDBLOCK);
        };
        put(received, &(length.min(bytes as usize) as u32).to_le_bytes())?;
        put_endpoint(from, address, port)?;
        if length > bytes as usize {
            return Err(WSAEMSGSIZE);
        }
        Ok(())
    })())
}
/// `out`: 40 bytes, the local then the peer endpoint (20 bytes each).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetEndpoints6(id: u32, out: u64) -> u32 {
    result((|| {
        let ((local, local_port), (peer, peer_port)) = with(|n| n.endpoints6(id))?;
        put_endpoint(out, local, local_port)?;
        put_endpoint(out + 20, peer, peer_port)
    })())
}
/// IPv6 addresses (AAAA) into `out` (16 bytes each) and their count.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetResolve6(
    name: u64,
    length: u32,
    out: u64,
    max: u32,
    count: u64,
    timeout: u32,
) -> u32 {
    result((|| {
        let mut text = [0u8; 255];
        let length = length as usize;
        if length == 0 || length > text.len() {
            return Err(WSAHOST_NOT_FOUND);
        }
        take(name, &mut text[..length])?;
        let name = core::str::from_utf8(&text[..length]).map_err(|_| WSAHOST_NOT_FOUND)?;
        let query = with(|n| n.resolve6(name))
            .map_err(|e| if e == WSAEINVAL { WSAHOST_NOT_FOUND } else { e })?;
        let start = door_sys::time().map_err(|_| WSAENETDOWN)?;
        let limit = door_abi::platform::ticks_from_millis(u64::from(timeout)).unwrap_or(u64::MAX);
        let addresses = loop {
            match with(|n| n.resolved6(query)) {
                Ok(Some(addresses)) => break addresses,
                Ok(None) => (),
                Err(WSAENOTSOCK) => return Err(WSAHOST_NOT_FOUND),
                Err(e) => return Err(e),
            }
            let now = door_sys::time().map_err(|_| WSAENETDOWN)?;
            if now.saturating_sub(start) >= limit {
                return Err(WSATRY_AGAIN);
            }
            let _ = door_sys::sleep(now + 1);
        };
        let kept = addresses.count.min(max as usize);
        for (i, address) in addresses.list[..kept].iter().enumerate() {
            put(out + 16 * i as u64, address)?;
        }
        put(count, &(kept as u32).to_le_bytes())
    })())
}
/// The VPN tunnel's adapter while one runs (`net::vpn::Tunnel`,
/// `TUNNEL_BYTES` into `out`); ERROR_NOT_FOUND (1168) when none does.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetTunnel(out: u64) -> u32 {
    result((|| match with(|n| n.vpn_tunnel())? {
        Some(t) => put(out, &t.encode()),
        None => Err(1168),
    })())
}
/// The adapter's IPv6 side (`net::v6::Status6`, `STATUS_BYTES` into `out`).
#[unsafe(no_mangle)]
pub extern "system" fn DoorNetStatus6(out: u64) -> u32 {
    result((|| {
        let status = with(|n| n.status6())?;
        put(out, &status.encode())
    })())
}
