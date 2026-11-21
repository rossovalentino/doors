//! Named pipes, the server side (P18: CoreCLR's diagnostics server listens
//! on `\\.\pipe\dotnet-diagnostic-<pid>-...` at start-up). Instances live
//! in this process's table: CreateNamedPipe makes a listening instance,
//! ConnectNamedPipe waits for a client (overlapped: pending until a client
//! connects or the wait is cancelled), DisconnectNamedPipe, the
//! information and state calls, WaitNamedPipe for instances of this
//! process. Clients in other processes need a name broker between
//! processes (P18 M0k); until then no client connects, so a waiting server
//! keeps waiting and CreateFile of `\\.\pipe\...` finds no pipe.
use door_windows_abi::{self as abi, layout::POINTER};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorGetCurrentProcessId() -> u32;
    fn RtlSetLastWin32Error(code: u32);
}
const SLOTS: usize = 32;
const NAME_UNITS: usize = 256;
const PREFIX: &str = "\\\\.\\pipe\\";

const INVALID_HANDLE: u32 = 6;
const INVALID_PARAMETER: u32 = 87;
const INVALID_NAME: u32 = 123;
const ACCESS_DENIED: u32 = 5;
const FILE_NOT_FOUND: u32 = 2;
const PIPE_BUSY: u32 = 231;
const PIPE_CONNECTED: u32 = 535;
const PIPE_LISTENING: u32 = 536;
const PIPE_NOT_CONNECTED: u32 = 233;
const IO_PENDING: u32 = 997;
const IO_INCOMPLETE: u32 = 996;
const OPERATION_ABORTED: u32 = 995;
const NOT_FOUND: u32 = 1168;
const CANNOT_IMPERSONATE: u32 = 1368;
const SEM_TIMEOUT: u32 = 121;
const STATUS_PENDING: u64 = 0x103;
const STATUS_CANCELLED: u64 = 0xC000_0120;

const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
const OPEN_MODE_VALID: u32 = PIPE_ACCESS_DUPLEX
    | FILE_FLAG_FIRST_PIPE_INSTANCE
    | FILE_FLAG_OVERLAPPED
    | FILE_FLAG_WRITE_THROUGH
    // WRITE_DAC, WRITE_OWNER, ACCESS_SYSTEM_SECURITY.
    | 0x0004_0000
    | 0x0008_0000
    | 0x0100_0000;
const PIPE_TYPE_MESSAGE: u32 = 4;
const PIPE_READMODE_MESSAGE: u32 = 2;
const PIPE_NOWAIT: u32 = 1;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_MODE_VALID: u32 =
    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const PIPE_SERVER_END: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Listening,
    // Set once a client connects (the broker of M0k).
    #[allow(dead_code)]
    Connected,
    Disconnected,
}
#[derive(Clone, Copy)]
struct Instance {
    name: [u16; NAME_UNITS],
    length: usize,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_bytes: u32,
    in_bytes: u32,
    state: State,
    /// A pending overlapped ConnectNamedPipe: (OVERLAPPED, its event).
    pending: Option<(u64, u64)>,
    /// Handles naming the instance (DuplicateHandle adds one).
    references: u32,
}
struct Table {
    slots: [Option<Instance>; SLOTS],
    generations: [u32; SLOTS],
}
struct Shared {
    held: core::sync::atomic::AtomicBool,
    table: core::cell::UnsafeCell<Table>,
}
unsafe impl Sync for Shared {}
static TABLE: Shared = Shared {
    held: core::sync::atomic::AtomicBool::new(false),
    table: core::cell::UnsafeCell::new(Table {
        slots: [None; SLOTS],
        generations: [0; SLOTS],
    }),
};
fn with<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    use core::sync::atomic::Ordering;
    while TABLE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *TABLE.table.get() });
    TABLE.held.store(false, Ordering::Release);
    r
}
fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn fail_handle(code: u32) -> u64 {
    unsafe { RtlSetLastWin32Error(code) };
    u64::MAX
}
fn put_u32(pointer: u64, value: u32) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    let b = value.to_le_bytes();
    match unsafe { DoorUserCopy(pointer, b.as_ptr() as u64, 4) } {
        0 => Ok(()),
        _ => Err(998),
    }
}
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    if unsafe { DoorUserCopy(b.as_mut_ptr() as u64, pointer, POINTER) } != 0 {
        return Err(998);
    }
    Ok(abi::widen(u64::from_le_bytes(b) as usize))
}
fn put_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    let b = value.to_le_bytes();
    match unsafe { DoorUserCopy(pointer, b.as_ptr() as u64, POINTER) } {
        0 => Ok(()),
        _ => Err(998),
    }
}
/// OVERLAPPED: Internal (the status) at 0, InternalHigh, Offset and
/// OffsetHigh, then hEvent.
fn overlapped_event(overlapped: u64) -> Result<u64, u32> {
    read_pointer(overlapped + 2 * POINTER as u64 + 8)
}
/// Completes a pending wait with `status` (and its event).
fn complete(pending: (u64, u64), status: u64) {
    let (overlapped, event) = pending;
    let _ = put_pointer(overlapped, status);
    let _ = put_pointer(overlapped + POINTER as u64, 0);
    if event != 0 {
        unsafe { DoorSetEvent(event) };
    }
}

pub(crate) fn is(handle: u64) -> bool {
    abi::tagged::NAMED_PIPE.is(handle)
}
fn slot(table: &Table, handle: u64) -> Result<usize, u32> {
    let (slot, generation) = abi::tagged::NAMED_PIPE
        .decode(handle)
        .filter(|&(s, _)| s < SLOTS)
        .ok_or(INVALID_HANDLE)?;
    if table.slots[slot].is_none() || !abi::tagged::same(table.generations[slot], generation) {
        return Err(INVALID_HANDLE);
    }
    Ok(slot)
}
fn with_instance<R>(handle: u64, f: impl FnOnce(&mut Instance) -> R) -> Result<R, u32> {
    with(|t| {
        let s = slot(t, handle)?;
        Ok(f(t.slots[s].as_mut().unwrap()))
    })
}
/// A NUL-terminated UTF-16 pipe name: its units after `\\.\pipe\`
/// (case-insensitive prefix), or ERROR_INVALID_NAME.
fn pipe_name(pointer: u64) -> Result<([u16; NAME_UNITS], usize), u32> {
    if pointer == 0 {
        return Err(INVALID_PARAMETER);
    }
    let mut units = [0u16; NAME_UNITS];
    let mut length = 0;
    loop {
        let mut b = [0u8; 2];
        if unsafe { DoorUserCopy(b.as_mut_ptr() as u64, pointer + 2 * length as u64, 2) } != 0 {
            return Err(998);
        }
        let unit = u16::from_le_bytes(b);
        if unit == 0 {
            break;
        }
        if length == NAME_UNITS {
            return Err(INVALID_NAME);
        }
        units[length] = unit;
        length += 1;
    }
    let prefix: [u16; 9] = {
        let mut p = [0u16; 9];
        for (i, c) in PREFIX.encode_utf16().enumerate() {
            p[i] = c;
        }
        p
    };
    let matches = length > prefix.len()
        && units[..prefix.len()]
            .iter()
            .zip(prefix.iter())
            .all(|(&a, &b)| fold(a) == fold(b));
    if !matches {
        return Err(INVALID_NAME);
    }
    Ok((units, length))
}
fn fold(unit: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&unit) {
        unit - 32
    } else {
        unit
    }
}
fn same_name(a: &Instance, units: &[u16]) -> bool {
    a.length == units.len()
        && a.name[..a.length]
            .iter()
            .zip(units)
            .all(|(&x, &y)| fold(x) == fold(y))
}

pub(crate) fn create(
    name: u64,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_bytes: u32,
    in_bytes: u32,
) -> u64 {
    let (units, length) = match pipe_name(name) {
        Ok(n) => n,
        Err(e) => return fail_handle(e),
    };
    if open_mode & !OPEN_MODE_VALID != 0
        || open_mode & PIPE_ACCESS_DUPLEX == 0
        || pipe_mode & !PIPE_MODE_VALID != 0
        || max_instances == 0
        || max_instances > PIPE_UNLIMITED_INSTANCES
        // A byte-stream pipe cannot be read as messages.
        || (pipe_mode & PIPE_TYPE_MESSAGE == 0 && pipe_mode & PIPE_READMODE_MESSAGE != 0)
    {
        return fail_handle(INVALID_PARAMETER);
    }
    let result = with(|t| {
        let mut existing = 0u32;
        let mut limit = max_instances;
        for i in t.slots.iter().flatten() {
            if same_name(i, &units[..length]) {
                existing += 1;
                limit = i.max_instances;
            }
        }
        if existing > 0 && open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0 {
            return Err(ACCESS_DENIED);
        }
        if existing > 0 && (limit != max_instances) {
            return Err(ACCESS_DENIED);
        }
        if limit != PIPE_UNLIMITED_INSTANCES && existing >= limit {
            return Err(PIPE_BUSY);
        }
        let s = t.slots.iter().position(Option::is_none).ok_or(PIPE_BUSY)?;
        let generation = t.generations[s].wrapping_add(1).max(1);
        t.generations[s] = generation;
        t.slots[s] = Some(Instance {
            name: units,
            length,
            open_mode,
            pipe_mode,
            max_instances,
            out_bytes,
            in_bytes,
            state: State::Listening,
            pending: None,
            references: 1,
        });
        Ok(abi::tagged::NAMED_PIPE.encode(s, generation))
    });
    result.unwrap_or_else(fail_handle)
}
/// ConnectNamedPipe: overlapped, the wait is pending (ERROR_IO_PENDING);
/// otherwise the call returns once a client connects.
pub(crate) fn connect(handle: u64, overlapped: u64) -> i32 {
    let state = match with_instance(handle, |i| (i.state, i.open_mode)) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    match state.0 {
        State::Connected => return fail(PIPE_CONNECTED),
        // A disconnected instance listens again.
        State::Disconnected | State::Listening => (),
    }
    if overlapped != 0 {
        let event = match overlapped_event(overlapped) {
            Ok(e) => e,
            Err(e) => return fail(e),
        };
        if put_pointer(overlapped, STATUS_PENDING).is_err() {
            return fail(998);
        }
        let _ = with_instance(handle, |i| {
            i.state = State::Listening;
            i.pending = Some((overlapped, event));
        });
        return fail(IO_PENDING);
    }
    let _ = with_instance(handle, |i| i.state = State::Listening);
    let mut step = 1;
    loop {
        match with_instance(handle, |i| i.state) {
            Ok(State::Connected) => return 1,
            Ok(State::Listening) => (),
            // Disconnected or closed by another thread.
            Ok(State::Disconnected) | Err(_) => return fail(OPERATION_ABORTED),
        }
        unsafe { DoorSleep(step) };
        step = (step * 2).min(100);
    }
}
pub(crate) fn disconnect(handle: u64) -> i32 {
    let r = with_instance(handle, |i| {
        let was = i.state;
        if let Some(p) = i.pending.take() {
            complete(p, STATUS_CANCELLED);
        }
        i.state = State::Disconnected;
        was
    });
    match r {
        Ok(State::Connected) => 1,
        Ok(_) => fail(PIPE_NOT_CONNECTED),
        Err(e) => fail(e),
    }
}
/// DuplicateHandle: the same handle names the instance once more; it
/// closes when every reference has.
pub(crate) fn duplicate(handle: u64) -> Result<u64, u32> {
    with_instance(handle, |i| {
        i.references += 1;
        handle
    })
}
/// CloseHandle: the last reference closes the instance, and a pending
/// wait completes as cancelled.
pub(crate) fn close(handle: u64) -> i32 {
    let r = with(|t| {
        let s = slot(t, handle)?;
        let instance = t.slots[s].as_mut().unwrap();
        instance.references -= 1;
        if instance.references > 0 {
            return Ok(None);
        }
        let instance = t.slots[s].take().unwrap();
        Ok(instance.pending)
    });
    match r {
        Ok(pending) => {
            if let Some(p) = pending {
                complete(p, STATUS_CANCELLED);
            }
            1
        }
        Err(e) => fail(e),
    }
}
/// CancelIoEx: the instance's pending wait (0 = any).
pub(crate) fn cancel(handle: u64, overlapped: u64) -> i32 {
    let r = with_instance(handle, |i| match i.pending {
        Some(p) if overlapped == 0 || p.0 == overlapped => i.pending.take(),
        _ => None,
    });
    match r {
        Ok(Some(p)) => {
            complete(p, STATUS_CANCELLED);
            1
        }
        Ok(None) => fail(NOT_FOUND),
        Err(e) => fail(e),
    }
}
/// GetOverlappedResult of a ConnectNamedPipe.
pub(crate) fn overlapped_result(handle: u64, overlapped: u64, count: u64, wait: bool) -> i32 {
    if let Err(e) = with_instance(handle, |_| ()) {
        return fail(e);
    }
    let mut step = 1;
    loop {
        let status = match read_pointer(overlapped) {
            Ok(s) => s & 0xFFFF_FFFF,
            Err(e) => return fail(e),
        };
        match status {
            STATUS_PENDING if !wait => return fail(IO_INCOMPLETE),
            STATUS_PENDING => {
                unsafe { DoorSleep(step) };
                step = (step * 2).min(100);
            }
            STATUS_CANCELLED => return fail(OPERATION_ABORTED),
            _ => {
                let _ = put_u32(count, 0);
                return 1;
            }
        }
    }
}
/// ReadFile/WriteFile on a server end no client has connected to.
pub(crate) fn io(handle: u64) -> i32 {
    match with_instance(handle, |i| i.state) {
        Ok(State::Listening) => fail(PIPE_LISTENING),
        Ok(_) => fail(PIPE_NOT_CONNECTED),
        Err(e) => fail(e),
    }
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateNamedPipeW(
    name: usize,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_bytes: u32,
    in_bytes: u32,
    _timeout: u32,
    _security: usize,
) -> usize {
    abi::narrow(create(
        abi::widen(name),
        open_mode,
        pipe_mode,
        max_instances,
        out_bytes,
        in_bytes,
    ))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateNamedPipeA(
    name: usize,
    open_mode: u32,
    pipe_mode: u32,
    max_instances: u32,
    out_bytes: u32,
    in_bytes: u32,
    _timeout: u32,
    _security: usize,
) -> usize {
    let mut units = [0u16; crate::environment::UNITS];
    abi::narrow(
        match crate::environment::widen(abi::widen(name), &mut units) {
            Ok(wide) => create(
                wide,
                open_mode,
                pipe_mode,
                max_instances,
                out_bytes,
                in_bytes,
            ),
            Err(e) => fail_handle(e),
        },
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn ConnectNamedPipe(handle: usize, overlapped: usize) -> i32 {
    connect(abi::widen(handle), abi::widen(overlapped))
}
#[unsafe(no_mangle)]
pub extern "system" fn DisconnectNamedPipe(handle: usize) -> i32 {
    disconnect(abi::widen(handle))
}
fn wait_named(name: u64, timeout: u32) -> i32 {
    let (units, length) = match pipe_name(name) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let found = with(|t| {
        t.slots
            .iter()
            .flatten()
            .any(|i| same_name(i, &units[..length]))
    });
    match (found, timeout) {
        (false, _) => fail(FILE_NOT_FOUND),
        // Instances listen but no client of another process can reach
        // them yet; an in-process client would connect at once.
        (true, 0) => 1,
        (true, _) => fail(SEM_TIMEOUT),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitNamedPipeW(name: usize, timeout: u32) -> i32 {
    wait_named(abi::widen(name), timeout)
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitNamedPipeA(name: usize, timeout: u32) -> i32 {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(abi::widen(name), &mut units) {
        Ok(wide) => wait_named(wide, timeout),
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNamedPipeInfo(
    handle: usize,
    flags: usize,
    out_bytes: usize,
    in_bytes: usize,
    max_instances: usize,
) -> i32 {
    let i = match with_instance(abi::widen(handle), |i| *i) {
        Ok(i) => i,
        Err(e) => return fail(e),
    };
    let kind = PIPE_SERVER_END | (i.pipe_mode & PIPE_TYPE_MESSAGE);
    let r = put_u32(abi::widen(flags), kind)
        .and_then(|_| put_u32(abi::widen(out_bytes), i.out_bytes))
        .and_then(|_| put_u32(abi::widen(in_bytes), i.in_bytes))
        .and_then(|_| put_u32(abi::widen(max_instances), i.max_instances));
    match r {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetNamedPipeHandleState(
    handle: usize,
    mode: usize,
    _collection: usize,
    _timeout: usize,
) -> i32 {
    let handle = abi::widen(handle);
    let mode = abi::widen(mode);
    let mut wanted = None;
    if mode != 0 {
        let mut b = [0u8; 4];
        if unsafe { DoorUserCopy(b.as_mut_ptr() as u64, mode, 4) } != 0 {
            return fail(998);
        }
        wanted = Some(u32::from_le_bytes(b));
    }
    let r = with_instance(handle, |i| {
        if let Some(m) = wanted {
            if m & !(PIPE_READMODE_MESSAGE | PIPE_NOWAIT) != 0
                || (m & PIPE_READMODE_MESSAGE != 0 && i.pipe_mode & PIPE_TYPE_MESSAGE == 0)
            {
                return Err(INVALID_PARAMETER);
            }
            i.pipe_mode = (i.pipe_mode & !(PIPE_READMODE_MESSAGE | PIPE_NOWAIT)) | m;
        }
        Ok(())
    });
    match r {
        Ok(Ok(())) => 1,
        Ok(Err(e)) | Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn GetNamedPipeHandleStateW(
    handle: usize,
    state: usize,
    instances: usize,
    _collection: usize,
    _timeout: usize,
    user: usize,
    _user_units: u32,
) -> i32 {
    let handle = abi::widen(handle);
    let r = with(|t| {
        let s = slot(t, handle)?;
        let me = t.slots[s].unwrap();
        let count = t
            .slots
            .iter()
            .flatten()
            .filter(|i| same_name(i, &me.name[..me.length]))
            .count() as u32;
        Ok((me.pipe_mode & (PIPE_READMODE_MESSAGE | PIPE_NOWAIT), count))
    });
    let (mode, count) = match r {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    // The client's user name is only known once a client connects.
    if user != 0 {
        return fail(CANNOT_IMPERSONATE);
    }
    match put_u32(abi::widen(state), mode).and_then(|_| put_u32(abi::widen(instances), count)) {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateNamedPipeClient(handle: usize) -> i32 {
    match with_instance(abi::widen(handle), |i| i.state) {
        Ok(_) => fail(CANNOT_IMPERSONATE),
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNamedPipeServerProcessId(handle: usize, id: usize) -> i32 {
    match with_instance(abi::widen(handle), |_| ()) {
        Ok(()) => match put_u32(abi::widen(id), unsafe { DoorGetCurrentProcessId() }) {
            Ok(()) => 1,
            Err(e) => fail(e),
        },
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNamedPipeClientProcessId(handle: usize, _id: usize) -> i32 {
    match with_instance(abi::widen(handle), |i| i.state) {
        Ok(_) => fail(PIPE_NOT_CONNECTED),
        Err(e) => fail(e),
    }
}
/// A client open (CreateFile of `\\.\pipe\name`): no instance of another
/// process is reachable yet, so ERROR_FILE_NOT_FOUND unless this process
/// listens under the name (ERROR_PIPE_BUSY: no in-process connection yet).
pub(crate) fn client_open(path: u64) -> Option<u64> {
    // Most opens are files: only the prefix is read for them.
    let mut head = [0u8; 18];
    if path == 0 || unsafe { DoorUserCopy(head.as_mut_ptr() as u64, path, 18) } != 0 {
        return None;
    }
    let pipe = PREFIX
        .encode_utf16()
        .enumerate()
        .all(|(i, c)| fold(u16::from_le_bytes([head[2 * i], head[2 * i + 1]])) == fold(c));
    if !pipe {
        return None;
    }
    let (units, length) = pipe_name(path).ok()?;
    let found = with(|t| {
        t.slots
            .iter()
            .flatten()
            .any(|i| same_name(i, &units[..length]))
    });
    Some(fail_handle(if found { PIPE_BUSY } else { FILE_NOT_FOUND }))
}
