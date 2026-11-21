//! I/O completion ports. A port is an auto-reset kernel event (so waits,
//! timeouts and CloseHandle behave as for any handle) with a bounded queue
//! of completion packets in this process. Handles associated with a port
//! (CreateIoCompletionPort on a socket or file) post their overlapped
//! completions to it through `DoorCompleteIo`, which also stores the
//! OVERLAPPED status and signals its event, as the I/O manager does.
use super::guest::{RtlSetLastWin32Error, copy};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// Packets one port holds before PostQueuedCompletionStatus fails (ntdll
/// has no heap of its own, so the queues are fixed).
const MAX_PACKETS: usize = 256;
const MAX_PORTS: usize = 16;
const MAX_ASSOCIATIONS: usize = 512;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_IO_COMPLETION: u32 = 0xc0;
const WAIT_TIMEOUT: u32 = 0x102;
const ERROR_WAIT_TIMEOUT: u32 = 258;
const ERROR_ABANDONED_WAIT_0: u32 = 735;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_ENOUGH_QUOTA: u32 = 1816;

#[derive(Clone, Copy)]
struct Packet {
    bytes: u32,
    key: u64,
    overlapped: u64,
    status: u32,
}
const NO_PACKET: Packet = Packet {
    bytes: 0,
    key: 0,
    overlapped: 0,
    status: 0,
};
/// A port's queue: a ring of packets. `handle` 0 is a free slot.
struct Port {
    handle: u64,
    head: usize,
    count: usize,
    ring: [Packet; MAX_PACKETS],
}
impl Port {
    const FREE: Self = Self {
        handle: 0,
        head: 0,
        count: 0,
        ring: [NO_PACKET; MAX_PACKETS],
    };
    fn push_back(&mut self, packet: Packet) -> bool {
        if self.count == MAX_PACKETS {
            return false;
        }
        self.ring[(self.head + self.count) % MAX_PACKETS] = packet;
        self.count += 1;
        true
    }
    fn pop_front(&mut self) -> Option<Packet> {
        if self.count == 0 {
            return None;
        }
        let packet = self.ring[self.head];
        self.head = (self.head + 1) % MAX_PACKETS;
        self.count -= 1;
        Some(packet)
    }
}
struct Ports {
    ports: [Port; MAX_PORTS],
    /// (handle, port, key) from CreateIoCompletionPort; handle 0 is free.
    associations: [(u64, u64, u64); MAX_ASSOCIATIONS],
}
impl Ports {
    fn port(&mut self, handle: u64) -> Option<&mut Port> {
        self.ports
            .iter_mut()
            .find(|p| p.handle != 0 && p.handle == handle)
    }
    fn associated(&self, handle: u64) -> Option<(u64, u64)> {
        self.associations
            .iter()
            .find(|a| a.0 != 0 && a.0 == handle)
            .map(|a| (a.1, a.2))
    }
}
struct Shared {
    held: AtomicBool,
    value: UnsafeCell<Ports>,
}
// SAFETY: `held` serializes every access.
unsafe impl Sync for Shared {}
static PORTS: Shared = Shared {
    held: AtomicBool::new(false),
    value: UnsafeCell::new(Ports {
        ports: [const { Port::FREE }; MAX_PORTS],
        associations: [(0, 0, 0); MAX_ASSOCIATIONS],
    }),
};
fn with<R>(f: impl FnOnce(&mut Ports) -> R) -> R {
    while PORTS.held.swap(true, Ordering::Acquire) {
        let _ = door_sys::yield_now();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *PORTS.value.get() });
    PORTS.held.store(false, Ordering::Release);
    r
}
fn failed<T: Default>(error: u32) -> T {
    RtlSetLastWin32Error(error);
    T::default()
}
/// CreateIoCompletionPort: a new port (`file` INVALID_HANDLE_VALUE), or
/// `file` associated with `existing` (or with a new port) under `key`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateIoCompletionPort(
    file: u64,
    existing: u64,
    key: u64,
    _threads: u32,
) -> u64 {
    let port = if existing == 0 {
        let event = super::waits::DoorCreateEventW(0, 0, 0, 0);
        if event == 0 {
            return 0;
        }
        let added = with(|p| {
            let Some(slot) = p.ports.iter_mut().find(|x| x.handle == 0) else {
                return false;
            };
            slot.handle = event;
            slot.head = 0;
            slot.count = 0;
            true
        });
        if !added {
            super::threads::DoorCloseHandle(event);
            return failed(ERROR_NOT_ENOUGH_QUOTA);
        }
        event
    } else {
        if !with(|p| p.port(existing).is_some()) {
            return failed(ERROR_INVALID_PARAMETER);
        }
        existing
    };
    if file == u64::MAX {
        if existing != 0 {
            return failed(ERROR_INVALID_PARAMETER);
        }
        return port;
    }
    // A handle belongs to one port for its life.
    let associated = with(|p| {
        if file == 0 || p.associated(file).is_some() {
            return false;
        }
        let Some(slot) = p.associations.iter_mut().find(|a| a.0 == 0) else {
            return false;
        };
        *slot = (file, port, key);
        true
    });
    if !associated {
        if existing == 0 {
            close_port(port);
            super::threads::DoorCloseHandle(port);
        }
        return failed(ERROR_INVALID_PARAMETER);
    }
    port
}
fn post(port: u64, packet: Packet) -> Result<(), u32> {
    with(|p| {
        let port = p.port(port).ok_or(ERROR_INVALID_HANDLE)?;
        if !port.push_back(packet) {
            return Err(ERROR_NOT_ENOUGH_QUOTA);
        }
        Ok(())
    })?;
    super::waits::DoorSetEvent(port);
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorPostQueuedCompletionStatus(
    port: u64,
    bytes: u32,
    key: u64,
    overlapped: u64,
) -> i32 {
    match post(
        port,
        Packet {
            bytes,
            key,
            overlapped,
            status: 0,
        },
    ) {
        Ok(()) => 1,
        Err(e) => failed(e),
    }
}
/// The next packet of `port`, waiting up to `millis` (u32::MAX: forever).
fn take(port: u64, millis: u32, alertable: bool) -> Result<Packet, u32> {
    let start = door_sys::time().unwrap_or(0);
    loop {
        let next = with(|p| {
            let port = p.port(port).ok_or(ERROR_ABANDONED_WAIT_0)?;
            let packet = port.pop_front();
            Ok::<_, u32>((packet, port.count != 0))
        })?;
        if let (Some(packet), more) = next {
            if more {
                super::waits::DoorSetEvent(port);
            }
            return Ok(packet);
        }
        if millis == 0 {
            return Err(ERROR_WAIT_TIMEOUT);
        }
        let waited = if millis == u32::MAX {
            u32::MAX
        } else {
            let elapsed = (door_sys::time().unwrap_or(0).saturating_sub(start)
                * door_abi::platform::MONOTONIC_TICK_MILLIS) as u32;
            if elapsed >= millis {
                return Err(ERROR_WAIT_TIMEOUT);
            }
            millis - elapsed
        };
        match super::waits::DoorWaitForSingleObjectEx(port, waited, i32::from(alertable)) {
            WAIT_OBJECT_0 => (),
            WAIT_TIMEOUT => return Err(ERROR_WAIT_TIMEOUT),
            WAIT_IO_COMPLETION => return Err(WAIT_IO_COMPLETION),
            _ => return Err(ERROR_ABANDONED_WAIT_0),
        }
    }
}
/// GetQueuedCompletionStatus: FALSE with a packet when its I/O failed (the
/// last error from its status), FALSE without one on timeout.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetQueuedCompletionStatus(
    port: u64,
    bytes: u64,
    key: u64,
    overlapped: u64,
    millis: u32,
) -> i32 {
    // lpCompletionKey (PULONG_PTR) and *lpOverlapped: pointer-sized (P17).
    use super::shapes::write_pointer;
    let _ = write_pointer(overlapped, 0);
    match take(port, millis, false) {
        Ok(p) => {
            let _ = copy(bytes, p.bytes.to_le_bytes().as_ptr() as u64, 4);
            let _ = write_pointer(key, p.key);
            let _ = write_pointer(overlapped, p.overlapped);
            if p.status != 0 {
                return failed(door_windows_abi::dos_error(p.status));
            }
            1
        }
        Err(e) => failed(e),
    }
}
/// GetQueuedCompletionStatusEx: up to `count` OVERLAPPED_ENTRY (32 bytes:
/// key, overlapped, Internal, bytes) at once; alertable waits run APCs
/// (WAIT_IO_COMPLETION).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetQueuedCompletionStatusEx(
    port: u64,
    entries: u64,
    count: u32,
    removed: u64,
    millis: u32,
    alertable: i32,
) -> i32 {
    if count == 0 {
        return failed(ERROR_INVALID_PARAMETER);
    }
    // OVERLAPPED_ENTRY: 32 bytes on x64, 16 on x86 (P17).
    use super::shapes::{OVERLAPPED_ENTRY, set, size};
    const ENTRY: usize = size(&OVERLAPPED_ENTRY);
    // The whole entry array must be writable before packets are taken.
    for i in 0..count as u64 {
        let zero = [0u8; ENTRY];
        if copy(entries + ENTRY as u64 * i, zero.as_ptr() as u64, ENTRY).is_err() {
            return failed(998);
        }
    }
    let mut next = match take(port, millis, alertable != 0) {
        Ok(p) => Some(p),
        Err(e) => return failed(e),
    };
    let mut n = 0u32;
    while let Some(p) = next {
        let mut b = [0u8; ENTRY];
        for (index, value) in [p.key, p.overlapped, u64::from(p.status), u64::from(p.bytes)]
            .into_iter()
            .enumerate()
        {
            set(&mut b, &OVERLAPPED_ENTRY, index, value);
        }
        let _ = copy(
            entries + ENTRY as u64 * u64::from(n),
            b.as_ptr() as u64,
            ENTRY,
        );
        n += 1;
        next = if n < count {
            with(|ports| ports.port(port).and_then(Port::pop_front))
        } else {
            None
        };
    }
    let _ = copy(removed, n.to_le_bytes().as_ptr() as u64, 4);
    1
}
/// An overlapped operation on `handle` finished: OVERLAPPED.Internal and
/// InternalHigh take the status and byte count, its event (if any) is set,
/// and the handle's port (if any, and unless the event's low bit asks for
/// no packet) gets a packet.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCompleteIo(
    handle: u64,
    overlapped: u64,
    status: u32,
    bytes: u32,
) -> i32 {
    if overlapped == 0 {
        return failed(ERROR_INVALID_PARAMETER);
    }
    // OVERLAPPED.Internal and InternalHigh, then hEvent (P17: at the
    // program's width).
    use super::shapes::{OVERLAPPED, offset, read_pointer, write_pointer};
    if write_pointer(overlapped, u64::from(status)).is_err()
        || write_pointer(overlapped + offset(&OVERLAPPED, 1), u64::from(bytes)).is_err()
    {
        return failed(998);
    }
    let event = read_pointer(overlapped + offset(&OVERLAPPED, 4)).unwrap_or(0);
    if event & !1 != 0 {
        super::waits::DoorSetEvent(event & !1);
    }
    if event & 1 == 0 {
        queue_completion(handle, overlapped, status, bytes);
    }
    1
}
/// The packet for a finished operation on `handle`, if it has a port
/// (files call this once OVERLAPPED and the event are done).
pub fn queue_completion(handle: u64, overlapped: u64, status: u32, bytes: u32) {
    if handle == 0 {
        return;
    }
    if let Some((port, key)) = with(|p| p.associated(handle)) {
        let _ = post(
            port,
            Packet {
                bytes,
                key,
                overlapped,
                status,
            },
        );
    }
}
/// CloseHandle on a completion port: its queue and associations go.
pub fn close_port(handle: u64) {
    if handle == 0 {
        return;
    }
    with(|p| {
        for port in p.ports.iter_mut().filter(|x| x.handle == handle) {
            port.handle = 0;
            port.count = 0;
        }
        for a in p.associations.iter_mut().filter(|a| a.1 == handle) {
            *a = (0, 0, 0);
        }
    });
}
/// A handle that closes leaves its port.
pub fn forget_handle(handle: u64) {
    if handle == 0 {
        return;
    }
    with(|p| {
        for a in p.associations.iter_mut().filter(|a| a.0 == handle) {
            *a = (0, 0, 0);
        }
    });
}
/// For handles that are not kernel objects (ws2_32's sockets).
#[unsafe(no_mangle)]
pub extern "system" fn DoorForgetIoHandle(handle: u64) {
    forget_handle(handle);
}

/// CancelIoEx for handles other DLLs own (sockets): each registered hook
/// gets (handle, overlapped) and answers 1 when it cancelled something.
static CANCEL_HOOKS: [core::sync::atomic::AtomicU64; 4] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 4];
#[unsafe(no_mangle)]
pub extern "system" fn DoorRegisterIoCancel(hook: u64) -> i32 {
    if hook == 0 {
        return failed(ERROR_INVALID_PARAMETER);
    }
    for slot in &CANCEL_HOOKS {
        match slot.compare_exchange(0, hook, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return 1,
            Err(existing) if existing == hook => return 1,
            Err(_) => (),
        }
    }
    failed(ERROR_NOT_ENOUGH_QUOTA)
}
/// Did a hook cancel I/O on `handle`?
pub fn cancel_hooked(handle: u64, overlapped: u64) -> bool {
    CANCEL_HOOKS.iter().any(|slot| {
        let hook = slot.load(Ordering::Acquire);
        // SAFETY: registered by a DLL of this process, which stays loaded.
        hook != 0 && {
            // A hook of one of this process's own DLLs (the same build).
            let f: extern "system" fn(u64, u64) -> i32 =
                unsafe { core::mem::transmute(hook as usize) };
            f(handle, overlapped) != 0
        }
    })
}
