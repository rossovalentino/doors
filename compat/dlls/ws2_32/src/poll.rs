//! Readiness: select, WSAPoll, WSAEventSelect and WSAAsyncSelect (a watcher
//! thread signals each socket's event, or posts its window message, and
//! records its network events, as the AFD driver does for Windows).
use crate::error::*;
use crate::guest::*;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateEventW(attributes: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorResetEvent(handle: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorWaitForMultipleObjectsEx(
        count: u32,
        handles: u64,
        all: i32,
        millis: u32,
        alert: i32,
    ) -> u32;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
}

/// What a socket is ready for, as select and the FD_* events see it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ready {
    pub read: bool,
    pub write: bool,
    pub except: bool,
    /// FD_* events now true.
    pub events: u32,
    pub connect_error: u32,
}
const TCP_BUFFER: u32 = 32 * 1024;
pub(crate) fn readiness(socket: u64) -> Result<Ready, u32> {
    let e = entry(socket)?;
    let mut r = Ready::default();
    if e.listening {
        let s = net_state(e.id)?;
        if s.queued_in > 0 {
            r.read = true;
            r.events |= FD_ACCEPT;
        }
        return Ok(r);
    }
    if !e.stream {
        let s = net_state(e.id)?;
        r.read = s.queued_in > 0;
        r.write = true;
        r.events = if r.read { FD_READ } else { 0 } | FD_WRITE;
        return Ok(r);
    }
    if e.connecting.is_some() {
        match finish_connect(socket) {
            Ok(true) => (),
            Ok(false) | Err(WSAEWOULDBLOCK) => return Ok(r),
            Err(error) => {
                r.except = true;
                r.events = FD_CONNECT;
                r.connect_error = error;
                return Ok(r);
            }
        }
        r.events |= FD_CONNECT;
    }
    let e = entry(socket)?;
    if !e.connected {
        return Ok(r);
    }
    let s = net_state(e.id)?;
    match s.state {
        state::ESTABLISHED => {
            r.read = s.queued_in > 0;
            r.write = !e.shut_send && s.queued_out < TCP_BUFFER;
        }
        // The peer finished: data, then end of stream, are readable.
        state::PEER_CLOSED => {
            r.read = true;
            r.write = !e.shut_send;
            if s.queued_in == 0 {
                r.events |= FD_CLOSE;
            }
        }
        state::CLOSING => r.read = s.queued_in > 0,
        // Reset or closed: recv reports it.
        _ => {
            r.read = true;
            r.events |= FD_CLOSE;
        }
    }
    if r.read && s.queued_in > 0 {
        r.events |= FD_READ;
    }
    if r.write {
        r.events |= FD_WRITE;
    }
    Ok(r)
}

/// fd_set: fd_count u32, then SOCKET fd_array[FD_SETSIZE] (at 8 on x64, after
/// padding; at 4 on x86, a pointer each). Programs may define FD_SETSIZE
/// above the headers' 64; Windows reads all fd_count entries, so this reads
/// up to a bound no real program reaches.
const MAX_SET: usize = 4096;
fn read_set(pointer: u64) -> Result<Vec<u64>, u32> {
    if pointer == 0 {
        return Ok(Vec::new());
    }
    let mut count = [0u8; 4];
    read(pointer, &mut count)?;
    let count = u32::from_le_bytes(count) as usize;
    if count > MAX_SET {
        return Err(WSAEINVAL);
    }
    (0..count)
        .map(|i| read_pointer(pointer + (POINTER * (1 + i)) as u64))
        .collect()
}
fn write_set(pointer: u64, sockets: &[u64]) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    write(pointer, &(sockets.len() as u32).to_le_bytes())?;
    let raw: Vec<u8> = sockets
        .iter()
        .flat_map(|s| s.to_le_bytes()[..POINTER].to_vec())
        .collect();
    write(pointer + POINTER as u64, &raw)
}
#[unsafe(no_mangle)]
pub extern "system" fn select(
    _nfds: i32,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    timeout: usize,
) -> i32 {
    let readfds = door_windows_abi::widen(readfds);
    let writefds = door_windows_abi::widen(writefds);
    let exceptfds = door_windows_abi::widen(exceptfds);
    let timeout = door_windows_abi::widen(timeout);
    {
        let result = (|| -> Result<i32, u32> {
            started()?;
            let sets = [
                read_set(readfds)?,
                read_set(writefds)?,
                read_set(exceptfds)?,
            ];
            if sets.iter().all(Vec::is_empty) {
                return Err(WSAEINVAL);
            }
            // TIMEVAL: seconds and microseconds (32-bit longs); null waits forever.
            let limit = if timeout == 0 {
                None
            } else {
                let mut t = [0u8; 8];
                read(timeout, &mut t)?;
                let (seconds, micros) = (
                    i32::from_le_bytes(t[..4].try_into().unwrap()),
                    i32::from_le_bytes(t[4..].try_into().unwrap()),
                );
                if seconds < 0 || micros < 0 {
                    return Err(WSAEINVAL);
                }
                Some(seconds as u64 * 1000 + micros as u64 / 1000)
            };
            let start = tick();
            loop {
                let mut ready: [Vec<u64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
                for (which, set) in sets.iter().enumerate() {
                    for &socket in set {
                        let r = readiness(socket)?;
                        let hit = match which {
                            0 => r.read,
                            1 => r.write,
                            _ => r.except,
                        };
                        if hit && !ready[which].contains(&socket) {
                            ready[which].push(socket);
                        }
                    }
                }
                let total: usize = ready.iter().map(Vec::len).sum();
                let expired = limit.is_some_and(|l| tick().saturating_sub(start) >= l);
                if total > 0 || expired {
                    write_set(readfds, &ready[0])?;
                    write_set(writefds, &ready[1])?;
                    write_set(exceptfds, &ready[2])?;
                    return Ok(total as i32);
                }
                unsafe { DoorSleep(1) };
            }
        })();
        result.unwrap_or_else(fail)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn __WSAFDIsSet(socket: usize, set: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let set = door_windows_abi::widen(set);
    read_set(set).map_or(0, |s| i32::from(s.contains(&socket)))
}
const POLLERR: i16 = 0x0001;
const POLLHUP: i16 = 0x0002;
const POLLNVAL: i16 = 0x0004;
const POLLWRNORM: i16 = 0x0010;
const POLLRDNORM: i16 = 0x0100;
const POLLRDBAND: i16 = 0x0200;
/// WSAPOLLFD: SOCKET, events i16, revents i16 (16 bytes on x64, 8 on x86).
const POLLFD: usize = 2 * POINTER;
#[unsafe(no_mangle)]
pub extern "system" fn WSAPoll(fds: usize, count: u32, timeout: i32) -> i32 {
    let fds = door_windows_abi::widen(fds);
    {
        let result = (|| -> Result<i32, u32> {
            started()?;
            if count == 0 || count > 1024 {
                return Err(WSAEINVAL);
            }
            let mut raw = alloc::vec![0u8; count as usize * POLLFD];
            read(fds, &mut raw)?;
            let start = tick();
            loop {
                let mut hits = 0;
                for record in raw.chunks_exact_mut(POLLFD) {
                    let socket = door_windows_abi::layout::get(
                        record,
                        &[door_windows_abi::layout::Field::Pointer],
                        0,
                    );
                    let wanted = i16::from_le_bytes([record[POINTER], record[POINTER + 1]]);
                    let mut revents = 0i16;
                    if socket != INVALID_SOCKET {
                        match readiness(socket) {
                            Ok(r) => {
                                if r.read && wanted & (POLLRDNORM | POLLRDBAND) != 0 {
                                    revents |= wanted & (POLLRDNORM | POLLRDBAND);
                                }
                                if r.write && wanted & POLLWRNORM != 0 {
                                    revents |= POLLWRNORM;
                                }
                                if r.except {
                                    revents |= POLLERR;
                                }
                                if r.events & FD_CLOSE != 0 {
                                    revents |= POLLHUP;
                                }
                            }
                            Err(_) => revents = POLLNVAL,
                        }
                    }
                    record[POINTER + 2..POINTER + 4].copy_from_slice(&revents.to_le_bytes());
                    if revents != 0 {
                        hits += 1;
                    }
                }
                if hits > 0
                    || timeout == 0
                    || (timeout > 0 && tick().saturating_sub(start) >= timeout as u64)
                {
                    write(fds, &raw)?;
                    return Ok(hits);
                }
                unsafe { DoorSleep(1) };
            }
        })();
        result.unwrap_or_else(fail)
    }
}

// ---- WSAEventSelect -----------------------------------------------------
static WATCHING: AtomicBool = AtomicBool::new(false);
/// The watcher: every couple of milliseconds, each event-selected socket's
/// newly true events are recorded and its event set (re-enabling functions
/// clear `reported`).
extern "system" fn watch(_argument: usize) -> u32 {
    {
        loop {
            let sockets = Table::lock().map(|mut t| t.sockets()).unwrap_or_default();
            for (socket, e) in sockets
                .into_iter()
                .filter(|(_, e)| e.mask != 0 && (e.event != 0 || e.window != 0))
            {
                let Ok(r) = readiness(socket) else { continue };
                let fresh = r.events & e.mask & !e.reported;
                if fresh == 0 {
                    continue;
                }
                // Each event stays reported until its re-enabling call (recv,
                // send, accept); FD_CONNECT and FD_CLOSE are reported once.
                let signalled = with_entry(socket, |e| {
                    e.pending |= fresh;
                    e.reported |= fresh;
                    if fresh & FD_CONNECT != 0 {
                        e.connect_error = r.connect_error;
                    }
                    Ok((e.event, e.window, e.message))
                });
                match signalled {
                    Ok((event, 0, _)) if event != 0 => {
                        unsafe { DoorSetEvent(event) };
                    }
                    // One message per event: WSAMAKESELECTREPLY(event, error).
                    Ok((_, window, message)) if window != 0 => {
                        for bit in 0..10 {
                            let event = fresh & (1 << bit);
                            if event != 0 {
                                let error = if event == FD_CONNECT {
                                    r.connect_error
                                } else {
                                    0
                                };
                                crate::messages::post(
                                    window,
                                    message,
                                    socket,
                                    u64::from(error << 16 | event),
                                );
                            }
                        }
                    }
                    _ => (),
                }
            }
            unsafe { DoorSleep(2) };
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAEventSelect(socket: usize, event: usize, mask: i32) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let event = door_windows_abi::widen(event);
    {
        let result = with_entry(socket, |e| {
            e.event = event;
            e.window = 0;
            e.mask = mask as u32;
            e.reported = 0;
            e.pending = 0;
            // Event selection makes the socket nonblocking.
            if mask != 0 {
                e.nonblocking = true;
            }
            Ok(())
        });
        if result.is_ok() && mask != 0 && !start_watcher() {
            return fail(WSAENOBUFS);
        }
        result.map_or_else(fail, |()| ok())
    }
}
/// Starts the one watcher thread of the process if it is not running.
pub(crate) fn start_watcher() -> bool {
    if WATCHING.swap(true, Ordering::AcqRel) {
        return true;
    }
    let mut id = [0u8; 4];
    if unsafe { DoorCreateThreadEx(watch as *const () as u64, 0, id.as_mut_ptr() as u64) } == 0 {
        WATCHING.store(false, Ordering::Release);
        return false;
    }
    true
}
/// WSANETWORKEVENTS: lNetworkEvents i32, iErrorCode[10] i32.
#[unsafe(no_mangle)]
pub extern "system" fn WSAEnumNetworkEvents(socket: usize, event: usize, out: usize) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let event = door_windows_abi::widen(event);
    let out = door_windows_abi::widen(out);
    {
        let result = (|| {
            let (events, connect_error) = with_entry(socket, |e| {
                let taken = (e.pending, e.connect_error);
                e.pending = 0;
                Ok(taken)
            })?;
            let mut b = [0u8; 44];
            b[..4].copy_from_slice(&events.to_le_bytes());
            // FD_CONNECT_BIT is 4.
            b[4 + 4 * 4..4 + 5 * 4].copy_from_slice(&connect_error.to_le_bytes());
            write(out, &b)?;
            if event != 0 {
                unsafe { DoorResetEvent(event) };
            }
            Ok(())
        })();
        result.map_or_else(fail, |()| ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSACreateEvent() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        // Manual reset, initially non-signalled.
        let event = unsafe { DoorCreateEventW(0, 1, 0, 0) };
        if event == 0 {
            fail(WSA_NOT_ENOUGH_MEMORY);
        }
        event
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSACloseEvent(event: usize) -> i32 {
    let event = door_windows_abi::widen(event);
    unsafe { DoorCloseHandle(event) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSASetEvent(event: usize) -> i32 {
    let event = door_windows_abi::widen(event);
    unsafe { DoorSetEvent(event) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAResetEvent(event: usize) -> i32 {
    let event = door_windows_abi::widen(event);
    unsafe { DoorResetEvent(event) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAWaitForMultipleEvents(
    count: u32,
    events: usize,
    all: i32,
    timeout: u32,
    alertable: i32,
) -> u32 {
    let events = door_windows_abi::widen(events);
    {
        // WSA_WAIT_* values are the WAIT_* values of the wait itself.
        unsafe { DoorWaitForMultipleObjectsEx(count, events, all, timeout, alertable) }
    }
}
