//! Overlapped Winsock: WSASend/WSARecv(From)/WSASendTo with an OVERLAPPED,
//! the Microsoft extension functions WSAIoctl hands out (AcceptEx,
//! GetAcceptExSockaddrs, ConnectEx, DisconnectEx, TransmitFile), and
//! cancellation. An operation is queued and the call returns
//! WSA_IO_PENDING; one worker thread per process advances every queued
//! operation without blocking, then completes it the way the I/O manager
//! does (`DoorCompleteIo`: OVERLAPPED status and byte count, its event,
//! the socket's completion port) and queues the completion routine, if
//! any, as an APC to the thread that started it.
use crate::error::*;
use crate::guest::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCompleteIo(handle: u64, overlapped: u64, status: u32, bytes: u32) -> i32;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorQueueUserAPCById(function: u64, thread: u32, argument: u64) -> u32;
    fn DoorRegisterIoCancel(hook: u64) -> i32;
    fn DoorForgetIoHandle(handle: u64);
    fn DoorWaitForSingleObjectEx(handle: u64, millis: u32, alertable: i32) -> u32;
    fn DoorReadFile(handle: u64, buffer: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
}

const STATUS_PENDING: u32 = 0x103;
const STATUS_CANCELLED: u32 = 0xc000_0120;
/// Queued operations a process may have.
const MAX_OPERATIONS: usize = 1024;

/// A Winsock error as the NTSTATUS an OVERLAPPED carries.
pub(crate) fn status_of(error: u32) -> u32 {
    match error {
        0 => 0,
        WSAECONNRESET => 0xc000_020d,
        WSAECONNABORTED => 0xc000_0241,
        WSAECONNREFUSED => 0xc000_0236,
        WSAETIMEDOUT => 0xc000_00b5,
        WSAENETUNREACH => 0xc000_023c,
        WSAEHOSTUNREACH => 0xc000_023d,
        WSAEMSGSIZE => 0x8000_0005,
        WSAENOTCONN | WSAESHUTDOWN => 0xc000_0140,
        WSA_OPERATION_ABORTED => STATUS_CANCELLED,
        _ => 0xc000_0001,
    }
}
/// And back, for WSAGetOverlappedResult.
pub(crate) fn error_of(status: u32) -> u32 {
    match status {
        0 => 0,
        0xc000_020d => WSAECONNRESET,
        0xc000_0241 => WSAECONNABORTED,
        0xc000_0236 => WSAECONNREFUSED,
        0xc000_00b5 => WSAETIMEDOUT,
        0xc000_023c => WSAENETUNREACH,
        0xc000_023d => WSAEHOSTUNREACH,
        0x8000_0005 => WSAEMSGSIZE,
        0xc000_0140 => WSAENOTCONN,
        STATUS_CANCELLED => WSA_OPERATION_ABORTED,
        _ => WSAENETDOWN,
    }
}

enum Kind {
    Receive {
        buffers: Vec<(u64, u32)>,
        flags: u64,
    },
    Send {
        buffers: Vec<(u64, u32)>,
        done: u64,
    },
    ReceiveFrom {
        buffer: (u64, u32),
        from: u64,
        from_length: u64,
        flags: u64,
    },
    SendTo {
        data: Vec<u8>,
        address: Ip,
        port: u16,
    },
    Accept {
        /// The socket that takes the connection (op.socket listens).
        accept: u64,
        buffer: u64,
        receive: u32,
        local: u32,
        remote: u32,
        received: u32,
        /// The connection, once accepted: (service id, peer, port).
        taken: Option<(u32, Ip, u16)>,
    },
    Connect {
        data: (u64, u32),
        done: u32,
    },
    /// TransmitFile's head, file bytes and tail, read up front.
    Transmit {
        data: Vec<u8>,
        done: usize,
        disconnect: bool,
    },
    Disconnect,
}
struct Operation {
    socket: u64,
    overlapped: u64,
    routine: u64,
    thread: u32,
    kind: Kind,
    cancelled: bool,
}
struct Shared {
    held: AtomicBool,
    value: UnsafeCell<Vec<Operation>>,
}
// SAFETY: `held` serializes every access.
unsafe impl Sync for Shared {}
static QUEUE: Shared = Shared {
    held: AtomicBool::new(false),
    value: UnsafeCell::new(Vec::new()),
};
static WORKER: AtomicBool = AtomicBool::new(false);
fn queue<R>(f: impl FnOnce(&mut Vec<Operation>) -> R) -> R {
    while QUEUE.held.swap(true, Ordering::Acquire) {
        unsafe { DoorSleep(0) };
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *QUEUE.value.get() });
    QUEUE.held.store(false, Ordering::Release);
    r
}
/// Queue an operation; the caller returns WSA_IO_PENDING.
fn start(socket: u64, overlapped: u64, routine: u64, kind: Kind) -> Result<(), u32> {
    if overlapped == 0 {
        return Err(WSAEFAULT);
    }
    // OVERLAPPED.Internal says the operation runs.
    write_pointer(overlapped, u64::from(STATUS_PENDING))?;
    let thread = unsafe { DoorGetCurrentThreadId() };
    let queued = queue(|q| {
        if q.len() >= MAX_OPERATIONS {
            return false;
        }
        q.push(Operation {
            socket,
            overlapped,
            routine,
            thread,
            kind,
            cancelled: false,
        });
        true
    });
    if !queued {
        return Err(WSAENOBUFS);
    }
    if !WORKER.swap(true, Ordering::AcqRel) {
        unsafe { DoorRegisterIoCancel(cancel_hook as *const () as u64) };
        let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, 0, 0) };
        if thread == 0 {
            WORKER.store(false, Ordering::Release);
            return Err(WSAENOBUFS);
        }
        unsafe { DoorCloseHandle(thread) };
    }
    Ok(())
}
/// The completion routine's call, run as an APC on the starting thread.
struct Routine {
    routine: u64,
    error: u32,
    bytes: u32,
    overlapped: u64,
}
extern "system" fn routine_apc(argument: usize) {
    let argument = door_windows_abi::widen(argument);
    {
        // SAFETY: boxed by `complete` for exactly this call.
        let r = unsafe { Box::from_raw(argument as *mut Routine) };
        // SAFETY: the application's LPWSAOVERLAPPED_COMPLETION_ROUTINE.
        let f: extern "system" fn(u32, u32, usize, u32) =
            unsafe { core::mem::transmute(r.routine as usize) };
        f(r.error, r.bytes, r.overlapped as usize, 0);
    }
}
fn complete(op: &Operation, error: u32, bytes: u32) {
    let status = status_of(error);
    unsafe { DoorCompleteIo(op.socket, op.overlapped, status, bytes) };
    if op.routine != 0 {
        let record = Box::into_raw(Box::new(Routine {
            routine: op.routine,
            error,
            bytes,
            overlapped: op.overlapped,
        }));
        if unsafe {
            DoorQueueUserAPCById(routine_apc as *const () as u64, op.thread, record as u64)
        } == 0
        {
            // SAFETY: not queued, so still ours.
            drop(unsafe { Box::from_raw(record) });
        }
    }
}
/// An overlapped call that finished at once (WSAIoctl): its OVERLAPPED,
/// event, port and routine hear of it as of any other.
pub(crate) fn finished(socket: u64, overlapped: u64, routine: u64, bytes: u32) {
    let op = Operation {
        socket,
        overlapped,
        routine,
        thread: unsafe { DoorGetCurrentThreadId() },
        kind: Kind::Disconnect,
        cancelled: false,
    };
    complete(&op, 0, bytes);
}
/// One nonblocking step: Some((error, bytes)) when the operation ended.
fn advance(op: &mut Operation) -> Option<(u32, u32)> {
    if op.cancelled {
        return Some((WSA_OPERATION_ABORTED, 0));
    }
    let Ok(socket) = entry(op.socket) else {
        return Some((WSA_OPERATION_ABORTED, 0));
    };
    let id = socket.id;
    match &mut op.kind {
        Kind::Receive { buffers, flags } => {
            let mut total = 0u32;
            for &(buffer, length) in buffers.iter() {
                let mut got = [0u8; 4];
                match unsafe { DoorNetReceive(id, buffer, length, got.as_mut_ptr() as u64) } {
                    0 => {
                        let n = u32::from_le_bytes(got);
                        total += n;
                        if n < length {
                            break;
                        }
                    }
                    WSAEWOULDBLOCK if total == 0 => return None,
                    WSAEWOULDBLOCK => break,
                    err => return Some((err, total)),
                }
            }
            if *flags != 0 {
                let _ = write(*flags, &0u32.to_le_bytes());
            }
            Some((0, total))
        }
        Kind::Send { buffers, done } => {
            let total: u64 = buffers.iter().map(|b| u64::from(b.1)).sum();
            while *done < total {
                // The buffer and offset `done` falls in.
                let mut skip = *done;
                let Some(&(buffer, length)) = buffers.iter().find(|b| {
                    if skip < u64::from(b.1) {
                        true
                    } else {
                        skip -= u64::from(b.1);
                        false
                    }
                }) else {
                    break;
                };
                let mut sent = [0u8; 4];
                match unsafe {
                    DoorNetSend(
                        id,
                        buffer + skip,
                        length - skip as u32,
                        sent.as_mut_ptr() as u64,
                    )
                } {
                    0 => *done += u64::from(u32::from_le_bytes(sent)),
                    WSAEWOULDBLOCK => return None,
                    err => return Some((err, *done as u32)),
                }
            }
            Some((0, *done as u32))
        }
        Kind::ReceiveFrom {
            buffer,
            from,
            from_length,
            flags,
        } => {
            let (status, got, address, port) = net_receive_from(&socket, buffer.0, buffer.1);
            match status {
                0 | WSAEMSGSIZE => {
                    let n = got.min(buffer.1);
                    let _ = store_sockaddr(*from, *from_length, socket.family, address, port);
                    if *flags != 0 {
                        let _ = write(*flags, &0u32.to_le_bytes());
                    }
                    Some((status, n))
                }
                WSAEWOULDBLOCK => None,
                err => Some((err, 0)),
            }
        }
        Kind::SendTo {
            data,
            address,
            port,
        } => {
            match net_send_to(
                &socket,
                address,
                *port,
                data.as_ptr() as u64,
                data.len() as u32,
            ) {
                (0, sent) => Some((0, sent)),
                (WSAEWOULDBLOCK, _) => None,
                (err, _) => Some((err, 0)),
            }
        }
        Kind::Accept {
            accept,
            buffer,
            receive,
            local,
            remote,
            received,
            taken,
        } => {
            if taken.is_none() {
                match net_accept(&socket) {
                    (0, child, peer, port) => {
                        // The accept socket takes the connection.
                        let adopted = with_entry(*accept, |e| {
                            let placeholder = e.id;
                            e.id = child;
                            e.connected = true;
                            e.bound = true;
                            Ok(placeholder)
                        });
                        match adopted {
                            Ok(placeholder) => unsafe {
                                DoorNetClose(placeholder);
                            },
                            Err(_) => {
                                unsafe { DoorNetClose(child) };
                                return Some((WSA_OPERATION_ABORTED, 0));
                            }
                        }
                        *taken = Some((child, peer, port));
                    }
                    (WSAEWOULDBLOCK, ..) => return None,
                    (err, ..) => return Some((err, 0)),
                }
            }
            let (child, peer, port) = taken.unwrap();
            // With a receive length, AcceptEx waits for the first data.
            if *receive > 0 && *received == 0 {
                let mut got = [0u8; 4];
                match unsafe { DoorNetReceive(child, *buffer, *receive, got.as_mut_ptr() as u64) } {
                    0 => *received = u32::from_le_bytes(got),
                    WSAEWOULDBLOCK => return None,
                    err => return Some((err, 0)),
                }
            }
            let ((mut own, local_port), _) =
                net_endpoints(socket.family, child).unwrap_or((([0; 16], 0), ([0; 16], 0)));
            if is_unspecified(&own) {
                own = mapped(local_address().unwrap_or([0; 4]));
            }
            let at = *buffer + u64::from(*receive);
            let _ = write(at, &address_block(socket.family, own, local_port, *local));
            let _ = write(
                at + u64::from(*local),
                &address_block(socket.family, peer, port, *remote),
            );
            Some((0, *received))
        }
        Kind::Connect { data, done } => {
            match finish_connect(op.socket) {
                Ok(true) => (),
                Ok(false) | Err(WSAEWOULDBLOCK) => return None,
                Err(err) => return Some((err, 0)),
            }
            while *done < data.1 {
                let mut sent = [0u8; 4];
                match unsafe {
                    DoorNetSend(
                        id,
                        data.0 + u64::from(*done),
                        data.1 - *done,
                        sent.as_mut_ptr() as u64,
                    )
                } {
                    0 => *done += u32::from_le_bytes(sent),
                    WSAEWOULDBLOCK => return None,
                    err => return Some((err, *done)),
                }
            }
            Some((0, *done))
        }
        Kind::Transmit {
            data,
            done,
            disconnect,
        } => {
            while *done < data.len() {
                let mut sent = [0u8; 4];
                match unsafe {
                    DoorNetSend(
                        id,
                        data[*done..].as_ptr() as u64,
                        (data.len() - *done) as u32,
                        sent.as_mut_ptr() as u64,
                    )
                } {
                    0 => *done += u32::from_le_bytes(sent) as usize,
                    WSAEWOULDBLOCK => return None,
                    err => return Some((err, *done as u32)),
                }
            }
            if *disconnect {
                unsafe { DoorNetShutdown(id) };
            }
            Some((0, *done as u32))
        }
        Kind::Disconnect => {
            unsafe { DoorNetShutdown(id) };
            let _ = with_entry(op.socket, |e| {
                e.shut_send = true;
                Ok(())
            });
            Some((0, 0))
        }
    }
}
/// An AcceptEx address block: [length u32][SOCKADDR_IN], zero-padded.
/// An AcceptEx address block: `[length u32]` then the SOCKADDR_IN or
/// SOCKADDR_IN6 of the socket's family, zero-padded to `size`.
fn address_block(family: i32, address: Ip, port: u16, size: u32) -> Vec<u8> {
    let mut b = alloc::vec![0u8; (size as usize).max(4 + SOCKADDR_IN6_BYTES as usize)];
    let length: u32 = if family == AF_INET6 { 28 } else { 16 };
    b[..4].copy_from_slice(&length.to_le_bytes());
    b[4..6].copy_from_slice(&(family as u16).to_le_bytes());
    b[6..8].copy_from_slice(&port.to_be_bytes());
    if family == AF_INET6 {
        b[12..28].copy_from_slice(&address);
    } else {
        b[8..12].copy_from_slice(&unmapped(&address).unwrap_or([0; 4]));
    }
    b.truncate(size as usize);
    b
}
extern "system" fn worker(_argument: usize) -> u32 {
    {
        loop {
            let mut ops = queue(core::mem::take);
            let mut progressed = false;
            let mut keep = Vec::new();
            for mut op in ops.drain(..) {
                match advance(&mut op) {
                    Some((error, bytes)) => {
                        complete(&op, error, bytes);
                        progressed = true;
                    }
                    None => keep.push(op),
                }
            }
            let idle = keep.is_empty();
            // Operations queued meanwhile follow those still waiting.
            queue(|q| {
                keep.append(q);
                *q = keep;
            });
            if !progressed {
                unsafe { DoorSleep(if idle { 20 } else { 1 }) };
            }
        }
    }
}
/// CancelIoEx on a socket (ntdll forwards what is not a file): the
/// matching operations end with ERROR_OPERATION_ABORTED. 1 if any.
extern "system" fn cancel_hook(handle: usize, overlapped: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        queue(|q| {
            let mut any = 0;
            for op in q.iter_mut() {
                if op.socket == handle && (overlapped == 0 || op.overlapped == overlapped) {
                    op.cancelled = true;
                    any = 1;
                }
            }
            any
        })
    }
}
/// closesocket: its operations are aborted and it leaves its port.
pub(crate) fn socket_closed(socket: u64) {
    cancel_hook(door_windows_abi::narrow(socket), 0);
    unsafe { DoorForgetIoHandle(socket) };
}

// ---- the overlapped forms of the WSA calls ---------------------------------
pub(crate) fn receive(
    socket: u64,
    buffers: Vec<(u64, u32)>,
    flags: u64,
    overlapped: u64,
    routine: u64,
) -> Result<(), u32> {
    let e = entry(socket)?;
    if e.stream && !e.connected {
        return Err(WSAENOTCONN);
    }
    if !e.stream {
        let first = *buffers.first().ok_or(WSAEINVAL)?;
        return start(
            socket,
            overlapped,
            routine,
            Kind::ReceiveFrom {
                buffer: first,
                from: 0,
                from_length: 0,
                flags,
            },
        );
    }
    start(
        socket,
        overlapped,
        routine,
        Kind::Receive { buffers, flags },
    )
}
pub(crate) fn receive_from(
    socket: u64,
    buffer: (u64, u32),
    from: u64,
    from_length: u64,
    flags: u64,
    overlapped: u64,
    routine: u64,
) -> Result<(), u32> {
    let e = entry(socket)?;
    if !e.stream && !e.bound {
        return Err(WSAEINVAL);
    }
    start(
        socket,
        overlapped,
        routine,
        Kind::ReceiveFrom {
            buffer,
            from,
            from_length,
            flags,
        },
    )
}
pub(crate) fn send(
    socket: u64,
    buffers: Vec<(u64, u32)>,
    overlapped: u64,
    routine: u64,
) -> Result<(), u32> {
    let e = entry(socket)?;
    if e.shut_send {
        return Err(WSAESHUTDOWN);
    }
    if !e.stream {
        let (address, port) = e.peer.ok_or(WSAENOTCONN)?;
        return send_to(socket, buffers, address, port, overlapped, routine);
    }
    if !e.connected {
        return Err(WSAENOTCONN);
    }
    start(socket, overlapped, routine, Kind::Send { buffers, done: 0 })
}
pub(crate) fn send_to(
    socket: u64,
    buffers: Vec<(u64, u32)>,
    address: Ip,
    port: u16,
    overlapped: u64,
    routine: u64,
) -> Result<(), u32> {
    let mut data = Vec::new();
    for (buffer, length) in buffers {
        let at = data.len();
        data.resize(at + length as usize, 0);
        read(buffer, &mut data[at..])?;
    }
    let _ = with_entry(socket, |e| {
        e.bound = true;
        Ok(())
    });
    start(
        socket,
        overlapped,
        routine,
        Kind::SendTo {
            data,
            address,
            port,
        },
    )
}
/// WSAGetOverlappedResult: the bytes and error of a finished operation;
/// with `wait`, waits for one still running.
pub(crate) fn result(overlapped: u64, bytes: u64, wait: bool, flags: u64) -> Result<(), u32> {
    loop {
        // OVERLAPPED.Internal (the status) and InternalHigh (the bytes),
        // a pointer each.
        let status = read_pointer(overlapped)? as u32;
        if status != STATUS_PENDING {
            let transferred = read_pointer(overlapped + POINTER as u64)? as u32;
            write(bytes, &transferred.to_le_bytes())?;
            if flags != 0 {
                write(flags, &0u32.to_le_bytes())?;
            }
            return match error_of(status) {
                0 => Ok(()),
                e => Err(e),
            };
        }
        if !wait {
            return Err(WSA_IO_INCOMPLETE);
        }
        // OVERLAPPED.hEvent: after Offset and OffsetHigh.
        let event = read_pointer(overlapped + (2 * POINTER + 8) as u64)? & !1;
        if event != 0 {
            unsafe { DoorWaitForSingleObjectEx(event, 50, 0) };
        } else {
            unsafe { DoorSleep(1) };
        }
    }
}

// ---- extension functions (WSAIoctl SIO_GET_EXTENSION_FUNCTION_POINTER) -------
const WSAID_ACCEPTEX: [u8; 16] = guid(
    0xb536_7df1,
    0xcbac,
    0x11cf,
    [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
);
const WSAID_GETACCEPTEXSOCKADDRS: [u8; 16] = guid(
    0xb536_7df2,
    0xcbac,
    0x11cf,
    [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
);
const WSAID_CONNECTEX: [u8; 16] = guid(
    0x25a2_07b9,
    0xddf3,
    0x4660,
    [0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e],
);
const WSAID_DISCONNECTEX: [u8; 16] = guid(
    0x7fda_2e11,
    0x8630,
    0x436f,
    [0xa0, 0x31, 0xf5, 0x36, 0xa6, 0xee, 0xc1, 0x57],
);
const WSAID_TRANSMITFILE: [u8; 16] = guid(
    0xb536_7df0,
    0xcbac,
    0x11cf,
    [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
);
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}
/// The function a GUID names, for WSAIoctl.
pub(crate) fn extension(id: &[u8; 16]) -> Option<u64> {
    Some(match *id {
        WSAID_ACCEPTEX => AcceptEx as *const () as u64,
        WSAID_GETACCEPTEXSOCKADDRS => GetAcceptExSockaddrs as *const () as u64,
        WSAID_CONNECTEX => connect_ex as *const () as u64,
        WSAID_DISCONNECTEX => disconnect_ex as *const () as u64,
        WSAID_TRANSMITFILE => TransmitFile as *const () as u64,
        _ => return None,
    })
}
/// AcceptEx: `accept` (a new, unbound socket) takes the next connection
/// of `listener`; the first data (`receive` bytes, if any) and both
/// addresses land in `buffer`.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn AcceptEx(
    listener: usize,
    accept: usize,
    buffer: usize,
    receive: u32,
    local: u32,
    remote: u32,
    _received: usize,
    overlapped: usize,
) -> i32 {
    let listener = door_windows_abi::widen(listener);
    let accept = door_windows_abi::widen(accept);
    let buffer = door_windows_abi::widen(buffer);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        let result = (|| {
            let l = entry(listener)?;
            let a = entry(accept)?;
            if !l.listening {
                return Err(WSAEINVAL);
            }
            if a.bound || a.connected || !a.stream {
                return Err(WSAEINVAL);
            }
            if local < 32 || remote < 32 || buffer == 0 {
                return Err(WSAEFAULT);
            }
            // The operation is the listener's: its port gets the packet.
            start(
                listener,
                overlapped,
                0,
                Kind::Accept {
                    accept,
                    buffer,
                    receive,
                    local,
                    remote,
                    received: 0,
                    taken: None,
                },
            )
        })();
        pending(result)
    }
}
/// GetAcceptExSockaddrs: pointers to the two addresses AcceptEx stored.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn GetAcceptExSockaddrs(
    buffer: usize,
    receive: u32,
    local: u32,
    _remote: u32,
    local_out: usize,
    local_length: usize,
    remote_out: usize,
    remote_length: usize,
) {
    let buffer = door_windows_abi::widen(buffer);
    let local_out = door_windows_abi::widen(local_out);
    let local_length = door_windows_abi::widen(local_length);
    let remote_out = door_windows_abi::widen(remote_out);
    let remote_length = door_windows_abi::widen(remote_length);
    {
        let local_at = buffer + u64::from(receive);
        let remote_at = local_at + u64::from(local);
        // Each block starts with its address's length (16 or 28).
        let length_at = |at: u64| read_i32(at).unwrap_or(16);
        let _ = write_pointer(local_out, local_at + 4);
        let _ = write(local_length, &length_at(local_at).to_le_bytes());
        let _ = write_pointer(remote_out, remote_at + 4);
        let _ = write(remote_length, &length_at(remote_at).to_le_bytes());
    }
}
/// ConnectEx: connect a bound stream socket, then send `data`.
extern "system" fn connect_ex(
    socket: usize,
    name: usize,
    length: i32,
    data: usize,
    data_length: u32,
    _sent: usize,
    overlapped: usize,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        let result = (|| {
            let e = entry(socket)?;
            let (address, port) = sockaddr(name, length, e.family)?;
            if !e.stream || !e.bound {
                return Err(WSAEINVAL);
            }
            if e.connected {
                return Err(WSAEISCONN);
            }
            if is_unspecified(&address) || port == 0 {
                return Err(WSAEADDRNOTAVAIL);
            }
            begin_connect(socket, address, port)?;
            start(
                socket,
                overlapped,
                0,
                Kind::Connect {
                    data: (data, data_length),
                    done: 0,
                },
            )
        })();
        pending(result)
    }
}
extern "system" fn disconnect_ex(
    socket: usize,
    overlapped: usize,
    _flags: u32,
    _reserved: u32,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        let result = (|| {
            let e = entry(socket)?;
            if !e.connected {
                return Err(WSAENOTCONN);
            }
            if overlapped == 0 {
                unsafe { DoorNetShutdown(e.id) };
                return Ok(());
            }
            start(socket, overlapped, 0, Kind::Disconnect)
        })();
        match result {
            Ok(()) if overlapped == 0 => 1,
            other => pending(other),
        }
    }
}
/// TransmitFile: the head buffer, `bytes` of the file (0: to its end)
/// from its current position, the tail; TF_DISCONNECT (1) shuts down.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn TransmitFile(
    socket: usize,
    file: usize,
    bytes: u32,
    _per_send: u32,
    overlapped: usize,
    buffers: usize,
    flags: u32,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let file = door_windows_abi::widen(file);
    let overlapped = door_windows_abi::widen(overlapped);
    let buffers = door_windows_abi::widen(buffers);
    {
        let result = (|| {
            let e = entry(socket)?;
            if !e.connected {
                return Err(WSAENOTCONN);
            }
            let mut data = Vec::new();
            // TRANSMIT_FILE_BUFFERS { Head, HeadLength, Tail, TailLength }.
            let (mut head, mut tail) = ((0u64, 0u32), (0u64, 0u32));
            if buffers != 0 {
                use door_windows_abi::layout::{self, Field::*};
                const TRANSMIT_FILE_BUFFERS: [layout::Field; 4] = [Pointer, U32, Pointer, U32];
                let mut b = [0u8; layout::size(&TRANSMIT_FILE_BUFFERS)];
                read(buffers, &mut b)?;
                let field = |index| layout::get(&b, &TRANSMIT_FILE_BUFFERS, index);
                head = (field(0), field(1) as u32);
                tail = (field(2), field(3) as u32);
            }
            fn append(data: &mut Vec<u8>, (pointer, length): (u64, u32)) -> Result<(), u32> {
                let at = data.len();
                data.resize(at + length as usize, 0);
                read(pointer, &mut data[at..])
            }
            append(&mut data, head)?;
            if file != 0 {
                let limit = if bytes == 0 {
                    usize::MAX
                } else {
                    bytes as usize
                };
                let mut chunk = alloc::vec![0u8; 64 * 1024];
                let mut taken = 0usize;
                while taken < limit {
                    let want = chunk.len().min(limit - taken) as u32;
                    let mut got = [0u8; 4];
                    let ok = unsafe {
                        DoorReadFile(
                            file,
                            chunk.as_mut_ptr() as u64,
                            want,
                            got.as_mut_ptr() as u64,
                            0,
                        )
                    };
                    let n = u32::from_le_bytes(got) as usize;
                    if ok == 0 {
                        return Err(WSAEINVAL);
                    }
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    taken += n;
                    if data.len() > 64 * 1024 * 1024 {
                        return Err(WSAENOBUFS);
                    }
                }
            }
            append(&mut data, tail)?;
            let disconnect = flags & 1 != 0;
            if overlapped == 0 {
                // Synchronous: send it all now.
                let mut done = 0usize;
                while done < data.len() {
                    done += send_bytes(
                        socket,
                        data[done..].as_ptr() as u64,
                        (data.len() - done) as u32,
                    )? as usize;
                }
                if disconnect {
                    unsafe { DoorNetShutdown(e.id) };
                }
                return Ok(());
            }
            start(
                socket,
                overlapped,
                0,
                Kind::Transmit {
                    data,
                    done: 0,
                    disconnect,
                },
            )
        })();
        match result {
            Ok(()) if overlapped == 0 => 1,
            other => pending(other),
        }
    }
}
/// A Microsoft extension's BOOL result: FALSE with WSA_IO_PENDING for a
/// queued operation, FALSE with the error otherwise.
fn pending(result: Result<(), u32>) -> i32 {
    let error = result.err().unwrap_or(WSA_IO_PENDING);
    unsafe { RtlSetLastWin32Error(error) };
    0
}
