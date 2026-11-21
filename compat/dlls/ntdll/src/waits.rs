//! Native event authority and bounded user APC delivery. No metadata lock spans
//! an application callback. Alertable waits poll at the native 10ms tick period.
use super::guest::RtlSetLastWin32Error;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_abi::{HandleKind, Status};
use door_windows_abi::apc::{Callback, Queue};
#[derive(Clone, Copy)]
struct Thread {
    id: u32,
    handle: u64,
    active: bool,
    queue: Queue,
}
impl Thread {
    const EMPTY: Self = Self {
        id: 0,
        handle: 0,
        active: false,
        queue: Queue::new(),
    };
}
/// Threads one process keeps at once (the kernel runs 128 tasks in all; a
/// .NET process starts a dozen, PowerShell and Chromium dozens).
pub(super) const THREADS: usize = 128;
struct Registry {
    locked: AtomicBool,
    threads: UnsafeCell<[Thread; THREADS]>,
}
unsafe impl Sync for Registry {}
static REGISTRY: Registry = Registry {
    locked: AtomicBool::new(false),
    threads: UnsafeCell::new([Thread::EMPTY; THREADS]),
};
/// The registry's lock: held for a few table updates; a waiter gives up
/// after 30 seconds (only a stuck holder makes it).
struct Guard;
impl Guard {
    fn acquire() -> Result<Self, u32> {
        let until = door_sys::time().map_err(native)?.saturating_add(
            door_abi::platform::ticks_from_millis(30_000).expect("constant duration"),
        );
        while REGISTRY
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            if door_sys::time().map_err(native)? >= until {
                return Err(1460);
            }
            door_sys::yield_now().map_err(native)?;
        }
        HOLDER.store(super::threads::DoorGetCurrentThreadId(), Ordering::Relaxed);
        Ok(Self)
    }
    fn table(&mut self) -> &mut [Thread; THREADS] {
        unsafe { &mut *REGISTRY.threads.get() }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        REGISTRY.locked.store(false, Ordering::Release);
    }
}
pub fn prepare_thread(id: u32) -> bool {
    let Ok(mut guard) = Guard::acquire() else {
        report(b"lock", 0, 0);
        return false;
    };
    let Some(thread) = guard.table().iter_mut().find(|thread| thread.id == 0) else {
        let table = guard.table();
        let running = table.iter().filter(|t| t.active).count();
        report(b"full", running, THREADS - running);
        return false;
    };
    *thread = Thread {
        id,
        active: true,
        ..Thread::EMPTY
    };
    true
}
pub fn register_thread(handle: u64, id: u32) {
    let Ok(mut guard) = Guard::acquire() else {
        return;
    };
    if let Some(thread) = guard.table().iter_mut().find(|thread| thread.id == id) {
        thread.handle = handle;
    }
}
pub fn retire_thread(id: u32) {
    // Mutexes it still holds are abandoned (their next wait reports it).
    super::named::abandon(id);
    let Ok(mut guard) = Guard::acquire() else {
        return;
    };
    if let Some(thread) = guard.table().iter_mut().find(|thread| thread.id == id) {
        thread.active = false;
        thread.queue.take();
        if thread.handle == 0 {
            *thread = Thread::EMPTY;
        }
    }
}
pub fn close_thread(handle: u64) {
    let Ok(mut guard) = Guard::acquire() else {
        return;
    };
    if let Some(thread) = guard
        .table()
        .iter_mut()
        .find(|thread| thread.handle == handle)
    {
        thread.handle = 0;
        if !thread.active {
            *thread = Thread::EMPTY;
        }
    }
}
/// GetThreadId: the id of the thread `handle` names (the current-thread
/// pseudo-handle included), or 0 with ERROR_INVALID_HANDLE.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetThreadId(handle: u64) -> u32 {
    if handle == u64::MAX - 1 || handle == u32::MAX as u64 - 1 {
        return super::threads::DoorGetCurrentThreadId();
    }
    let found = Guard::acquire().ok().and_then(|mut guard| {
        guard
            .table()
            .iter()
            .find(|t| t.handle == handle && handle != 0)
            .map(|t| t.id)
    });
    match found {
        Some(id) => id,
        None => {
            RtlSetLastWin32Error(6);
            0
        }
    }
}
fn fail(error: u32) -> u32 {
    RtlSetLastWin32Error(error);
    u32::MAX
}
fn native(error: Status) -> u32 {
    match error {
        Status::BadHandle => 6,
        Status::AccessDenied => 5,
        Status::ResourceExhausted => 8,
        Status::BadAddress => 998,
        _ => 87,
    }
}
fn deadline(millis: u32) -> Result<u64, u32> {
    if millis == u32::MAX {
        return Ok(u64::MAX);
    }
    Ok(door_sys::time().map_err(native)?.saturating_add(
        door_abi::platform::ticks_from_millis(u64::from(millis)).expect("u32 duration"),
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateEventW(
    attributes: u64,
    manual: i32,
    initial: i32,
    name: u64,
) -> u64 {
    // SECURITY_ATTRIBUTES: a security descriptor and handle inheritance,
    // neither of which an event's use here depends on.
    let _ = attributes;
    let create = || {
        door_sys::wait_create(1, u64::from(initial != 0), u64::from(manual != 0)).map_err(native)
    };
    named_create(super::named::EVENT, name, create)
}
/// Create* of a kernel object: unnamed, or found again by `name`
/// (ERROR_ALREADY_EXISTS with the existing object; 0 for a new one).
fn named_create(kind: u8, name: u64, create: impl FnOnce() -> Result<u64, u32>) -> u64 {
    let result = if name == 0 {
        create().map(|handle| (handle, false))
    } else {
        super::named::create_or_open(kind, name, create)
    };
    match result {
        Ok((handle, existed)) => {
            RtlSetLastWin32Error(if existed { 183 } else { 0 });
            handle
        }
        Err(code) => {
            fail(code);
            0
        }
    }
}
/// OpenEvent/OpenSemaphore/OpenMutex: `kind` 1, 2 or 3.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenNamedObject(kind: u32, name: u64) -> u64 {
    match super::named::open(kind as u8, name) {
        Ok(handle) => handle,
        Err(code) => {
            fail(code);
            0
        }
    }
}
/// CreateMutex(Ex): owned by the caller when `owned`; a name finds an
/// existing mutex (ERROR_ALREADY_EXISTS, its ownership unchanged).
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateMutexW(attributes: u64, owned: i32, name: u64) -> u64 {
    let _ = attributes;
    named_create(super::named::MUTEX, name, || {
        super::named::mutex_create(owned != 0)
    })
}
/// ReleaseMutex: ERROR_NOT_OWNER unless the caller holds it.
#[unsafe(no_mangle)]
pub extern "system" fn DoorReleaseMutex(handle: u64) -> i32 {
    match super::named::release(handle) {
        Ok(()) => 1,
        Err(code) => {
            fail(code);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetEvent(handle: u64) -> i32 {
    if kind(handle) != Ok(HandleKind::Event) {
        fail(6);
        return 0;
    }
    match door_sys::signal(handle, 1) {
        Ok(()) => 1,
        Err(error) => {
            fail(native(error));
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorResetEvent(handle: u64) -> i32 {
    if kind(handle) != Ok(HandleKind::Event) {
        fail(6);
        return 0;
    }
    match door_sys::reset(handle) {
        Ok(()) => 1,
        Err(error) => {
            fail(native(error));
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorQueueUserAPC(function: u64, handle: u64, argument: u64) -> u32 {
    if function == 0 {
        fail(87);
        return 0;
    }
    let id = super::threads::DoorGetCurrentThreadId();
    if handle != u64::MAX - 1 {
        match door_sys::thread_join(handle, 0) {
            Err(Status::Timeout | Status::WouldBlock) => {}
            _ => {
                fail(6);
                return 0;
            }
        }
    }
    let mut guard = match Guard::acquire() {
        Ok(guard) => guard,
        Err(error) => {
            fail(error);
            return 0;
        }
    };
    if handle == u64::MAX - 1
        && !guard.table().iter().any(|thread| thread.id == id)
        && let Some(thread) = guard.table().iter_mut().find(|thread| thread.id == 0)
    {
        *thread = Thread {
            id,
            active: true,
            ..Thread::EMPTY
        };
    }
    let Some(thread) = guard.table().iter_mut().find(|thread| {
        thread.active
            && ((handle == u64::MAX - 1 && thread.id == id)
                || (handle != 0 && thread.handle == handle))
    }) else {
        fail(6);
        return 0;
    };
    if !thread.queue.push(Callback { function, argument }) {
        fail(8);
        return 0;
    }
    1
}
/// Queue an APC to the thread with Windows ID `id` (overlapped Winsock's
/// completion routines run on the thread that started the operation). The
/// first thread, which no CreateThread registered, joins on first use.
#[unsafe(no_mangle)]
pub extern "system" fn DoorQueueUserAPCById(function: u64, id: u32, argument: u64) -> u32 {
    if function == 0 || id == 0 || id == u32::MAX {
        fail(87);
        return 0;
    }
    let mut guard = match Guard::acquire() {
        Ok(guard) => guard,
        Err(error) => {
            fail(error);
            return 0;
        }
    };
    if id == 1
        && !guard.table().iter().any(|thread| thread.id == 1)
        && let Some(thread) = guard.table().iter_mut().find(|thread| thread.id == 0)
    {
        *thread = Thread {
            id: 1,
            active: true,
            ..Thread::EMPTY
        };
    }
    let Some(thread) = guard
        .table()
        .iter_mut()
        .find(|thread| thread.active && thread.id == id)
    else {
        fail(6);
        return 0;
    };
    if !thread.queue.push(Callback { function, argument }) {
        fail(8);
        return 0;
    }
    1
}
fn dispatch_apcs() -> Result<bool, u32> {
    let id = super::threads::DoorGetCurrentThreadId();
    // Snapshot prevents a callback that queues itself from starving the wait.
    let queue = {
        let mut guard = Guard::acquire()?;
        let Some(thread) = guard
            .table()
            .iter_mut()
            .find(|thread| thread.active && thread.id == id)
        else {
            return Ok(false);
        };
        thread.queue.take()
    };
    let mut delivered = false;
    for apc in queue.into_iter().flatten() {
        delivered = true;
        super::arch::call_one(apc.function, apc.argument);
    }
    Ok(delivered)
}
fn kind(handle: u64) -> Result<HandleKind, u32> {
    let info = door_sys::inspect(handle).map_err(native)?;
    match HandleKind::from_raw(info[0]) {
        Some(
            kind @ (HandleKind::Event
            | HandleKind::Thread
            | HandleKind::Process
            | HandleKind::Semaphore),
        ) => Ok(kind),
        _ => Err(6),
    }
}
/// A wait that succeeded on `handle`: a mutex becomes the caller's
/// (WAIT_ABANDONED once after its owner died); anything else is signaled.
fn acquired(handle: u64, kind: HandleKind) -> u32 {
    if kind == HandleKind::Semaphore {
        super::named::acquired(handle)
    } else {
        0
    }
}
fn probe(handle: u64, kind: HandleKind, deadline: u64) -> Result<bool, u32> {
    let result = if kind == HandleKind::Thread {
        door_sys::thread_join(handle, deadline).map(|_| ())
    } else if kind == HandleKind::Process {
        // A process handle is signaled once the program has exited.
        door_sys::process_wait(handle, deadline).map(|_| ())
    } else {
        door_sys::wait(handle, deadline).map(|_| ())
    };
    match result {
        Ok(()) => Ok(true),
        Err(Status::Timeout | Status::WouldBlock) => Ok(false),
        Err(error) => Err(native(error)),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorWaitForSingleObjectEx(handle: u64, millis: u32, alertable: i32) -> u32 {
    let result = (|| {
        let kind = kind(handle)?;
        // A mutex its owner waits on again: held once more, no wait.
        if kind == HandleKind::Semaphore
            && let Some(code) = super::named::owned_again(handle)
        {
            return Ok(code);
        }
        let until = deadline(millis)?;
        if alertable == 0 {
            return Ok(if probe(handle, kind, until)? {
                acquired(handle, kind)
            } else {
                258
            });
        }
        loop {
            if dispatch_apcs()? {
                return Ok(192);
            }
            let now = door_sys::time().map_err(native)?;
            if probe(handle, kind, now)? {
                return Ok(acquired(handle, kind));
            }
            if now >= until {
                return Ok(258);
            }
            door_sys::sleep(now.saturating_add(1).min(until)).map_err(native)?;
        }
    })();
    result.unwrap_or_else(fail)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorWaitForMultipleObjectsEx(
    count: u32,
    pointers: u64,
    all: i32,
    millis: u32,
    alertable: i32,
) -> u32 {
    let result = (|| {
        if count == 0 || count > 16 {
            return Err(87);
        }
        // The program's HANDLE array, a pointer per entry (P17).
        let mut handles = [0u64; 16];
        for (index, handle) in handles.iter_mut().take(count as usize).enumerate() {
            *handle =
                super::shapes::read_pointer(pointers + (index * super::shapes::POINTER) as u64)
                    .map_err(|_| 998u32)?;
        }
        let mut kinds = [HandleKind::Event; 16];
        for index in 0..count as usize {
            if handles[..index].contains(&handles[index]) {
                return Err(87);
            }
            kinds[index] = kind(handles[index])?;
        }
        for index in 0..count as usize {
            if all == 0
                && kinds[index] == HandleKind::Semaphore
                && let Some(code) = super::named::owned_again(handles[index])
            {
                return Ok(code + index as u32);
            }
        }
        let until = deadline(millis)?;
        if all != 0 {
            return wait_all(
                &handles[..count as usize],
                &kinds[..count as usize],
                until,
                alertable,
            );
        }
        loop {
            if alertable != 0 && dispatch_apcs()? {
                return Ok(192);
            }
            let now = door_sys::time().map_err(native)?;
            for index in 0..count as usize {
                if probe(handles[index], kinds[index], now)? {
                    return Ok(acquired(handles[index], kinds[index]) + index as u32);
                }
            }
            if now >= until {
                return Ok(258);
            }
            door_sys::sleep(now.saturating_add(1).min(until)).map_err(native)?;
        }
    })();
    result.unwrap_or_else(fail)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorSleepEx(millis: u32, alertable: i32) -> u32 {
    let result = (|| {
        let until = deadline(millis)?;
        loop {
            if alertable != 0 && dispatch_apcs()? {
                return Ok(192);
            }
            let now = door_sys::time().map_err(native)?;
            if now >= until {
                if millis == 0 {
                    let _ = door_sys::yield_now();
                }
                return Ok(0);
            }
            door_sys::sleep(if alertable == 0 {
                until
            } else {
                now.saturating_add(1).min(until)
            })
            .map_err(native)?;
        }
    })();
    result.unwrap_or_else(fail)
}

/// CreateSemaphore(Ex): an unnamed counting semaphore (P18). Named
/// semaphores fail with ERROR_NOT_SUPPORTED (no object namespace yet).
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateSemaphoreW(
    attributes: u64,
    initial: i32,
    maximum: i32,
    name: u64,
) -> u64 {
    let _ = attributes;
    if maximum <= 0 || initial < 0 || initial > maximum {
        fail(87);
        return 0;
    }
    named_create(super::named::SEMAPHORE, name, || {
        door_sys::wait_create(4, initial as u64, maximum as u64).map_err(native)
    })
}
/// ReleaseSemaphore: the previous count into `previous` (if non-null);
/// ERROR_TOO_MANY_POSTS past the maximum.
#[unsafe(no_mangle)]
pub extern "system" fn DoorReleaseSemaphore(handle: u64, amount: i32, previous: u64) -> i32 {
    if kind(handle) != Ok(HandleKind::Semaphore) {
        fail(6);
        return 0;
    }
    if amount <= 0 {
        fail(87);
        return 0;
    }
    match unsafe {
        door_sys::raw(
            door_abi::operation::SIGNAL,
            [handle, amount as u64, 0, 0, 0, 0],
        )
    }
    .result()
    {
        Ok(count) => {
            if previous != 0 {
                let count = count as i32;
                if super::guest::copy(previous, &count as *const i32 as u64, 4).is_err() {
                    fail(998);
                    return 0;
                }
            }
            1
        }
        Err(Status::InvalidArgument) => {
            fail(298);
            0
        }
        Err(error) => {
            fail(native(error));
            0
        }
    }
}
/// Why a thread could not be registered, on the serial log
/// (`DOOR_THREADS prepare failed reason=lock|full running=N retired=M
/// holder=T`): the registry's lock stayed taken for a second (`holder`
/// had it), or every record names a thread (`retired` ones keep theirs
/// until their handle closes).
fn report(reason: &[u8], running: usize, retired: usize) {
    let mut line = [0u8; 120];
    let mut n = 0;
    let mut push = |bytes: &[u8]| {
        for &b in bytes {
            if n < line.len() {
                line[n] = b;
                n += 1;
            }
        }
    };
    let number = |value: usize, push: &mut dyn FnMut(&[u8])| {
        let mut digits = [0u8; 20];
        let mut d = digits.len();
        let mut v = value;
        loop {
            d -= 1;
            digits[d] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        push(&digits[d..]);
    };
    push(b"DOOR_THREADS prepare failed reason=");
    push(reason);
    push(b" running=");
    number(running, &mut push);
    push(b" retired=");
    number(retired, &mut push);
    push(b" holder=");
    number(HOLDER.load(Ordering::Relaxed) as usize, &mut push);
    push(b"\n");
    let _ = door_sys::write(&line[..n]);
}
/// The thread holding the registry's lock (diagnostics only).
static HOLDER: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// WaitForMultipleObjects(Ex) with bWaitAll: every object acquired. Each is
/// taken as it becomes signaled (a recursive mutex its owner already holds
/// counts once more); if the time runs out or an APC is delivered first, the
/// ones taken are given back (auto-reset events and semaphores signaled
/// again, mutexes released), so the caller holds none. WAIT_OBJECT_0, or
/// WAIT_ABANDONED_0 when one was an abandoned mutex.
fn wait_all(handles: &[u64], kinds: &[HandleKind], until: u64, alertable: i32) -> Result<u32, u32> {
    let mut taken = [false; 16];
    let mut abandoned = false;
    let give_back = |taken: &[bool; 16]| {
        for (index, handle) in handles.iter().enumerate() {
            if !taken[index] {
                continue;
            }
            match kinds[index] {
                HandleKind::Semaphore if super::named::release(*handle).is_ok() => {}
                HandleKind::Semaphore | HandleKind::Event => {
                    let _ = door_sys::signal(*handle, 1);
                }
                _ => {}
            }
        }
    };
    loop {
        if alertable != 0 && dispatch_apcs()? {
            give_back(&taken);
            return Ok(192);
        }
        let now = door_sys::time().map_err(native)?;
        for (index, handle) in handles.iter().enumerate() {
            if taken[index] {
                continue;
            }
            let owned = kinds[index] == HandleKind::Semaphore
                && super::named::owned_again(*handle).is_some();
            if owned || probe(*handle, kinds[index], now)? {
                taken[index] = true;
                if !owned && acquired(*handle, kinds[index]) != 0 {
                    abandoned = true;
                }
            }
        }
        if taken[..handles.len()].iter().all(|t| *t) {
            return Ok(if abandoned { 0x80 } else { 0 });
        }
        if now >= until {
            give_back(&taken);
            return Ok(258);
        }
        door_sys::sleep(now.saturating_add(1).min(until)).map_err(native)?;
    }
}
/// The handle CreateThread returned for the running thread `id`.
pub fn thread_handle(id: u32) -> Option<u64> {
    let mut guard = Guard::acquire().ok()?;
    guard
        .table()
        .iter()
        .find(|t| t.id == id && t.active && t.handle != 0)
        .map(|t| t.handle)
}
