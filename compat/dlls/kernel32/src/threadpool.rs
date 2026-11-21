//! The Windows thread pool (P18): work, simple callbacks, timers and waits
//! (CreateThreadpool*), callback-instance actions, cleanup groups, and the
//! legacy pool over the same machinery (QueueUserWorkItem,
//! RegisterWaitForSingleObject, timer queues).
//!
//! Every object is a slot of one table; its TP_* pointer (or HANDLE) is the
//! address of the slot's cell. Callbacks run on worker threads started on
//! demand (up to `WORKERS`), fed through a ring of queued calls and a
//! semaphore. One timer thread fires timers (sleeping on an event until
//! the next due time); wait threads, each watching up to `PER_WAIT_THREAD`
//! handles plus its own wake event, fire waits. A queued call carries its
//! object's epoch, so WaitFor*Callbacks(cancel) drops calls already queued.
//! Pools and their thread limits, callback priorities and stack sizes are
//! accepted and share the one process pool, as the default pool does.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use door_windows_abi::{self as abi, layout::POINTER};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(code: u32);
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
}

const OBJECTS: usize = 512;
const WORKERS: usize = 16;
const WAIT_THREADS: usize = 8;
const PER_WAIT_THREAD: usize = 15;
const QUEUE: usize = 2048;
const INFINITE: u32 = u32::MAX;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = u32::MAX;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_IO_PENDING: u32 = 997;
/// WT_EXECUTEONLYONCE: a registered wait or queue timer fires once.
const WT_EXECUTEONLYONCE: u32 = 0x8;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Free,
    Work,
    /// TrySubmitThreadpoolCallback: freed after its one call.
    Simple,
    Timer,
    Wait,
    Pool,
    Group,
    TimerQueue,
    QueueTimer,
    RegisteredWait,
    /// QueueUserWorkItem: freed after its one call.
    UserWorkItem,
}
#[derive(Clone, Copy)]
struct Object {
    kind: Kind,
    generation: u32,
    callback: u64,
    context: u64,
    /// Cleanup group slot + 1 (0: none); a queue timer's queue slot + 1.
    owner: u32,
    flags: u32,
    armed: bool,
    /// Timers: due time (ms of GetTickCount64). Waits: their deadline
    /// (u64::MAX: none).
    due: u64,
    /// Timers: period (ms). Registered waits: their timeout (ms).
    period: u32,
    handle: u64,
    /// Wait thread + 1.
    thread: u8,
    pending: u32,
    running: u32,
    epoch: u32,
    closing: bool,
    /// Signaled once the closed object's callbacks are over.
    done_event: u64,
}
const FREE: Object = Object {
    kind: Kind::Free,
    generation: 0,
    callback: 0,
    context: 0,
    owner: 0,
    flags: 0,
    armed: false,
    due: 0,
    period: 0,
    handle: 0,
    thread: 0,
    pending: 0,
    running: 0,
    epoch: 0,
    closing: false,
    done_event: 0,
};
#[derive(Clone, Copy)]
struct Call {
    slot: u16,
    epoch: u32,
    generation: u32,
    /// A wait's result (WAIT_OBJECT_0 / WAIT_TIMEOUT).
    result: u32,
}
struct Pool {
    objects: [Object; OBJECTS],
    queue: [Call; QUEUE],
    head: usize,
    length: usize,
    started: bool,
    semaphore: u64,
    timer_event: u64,
    timer_thread: bool,
    wait_events: [u64; WAIT_THREADS],
    wait_threads: usize,
    workers: usize,
}
struct Shared {
    held: AtomicBool,
    pool: UnsafeCell<Pool>,
}
unsafe impl Sync for Shared {}
static POOL: Shared = Shared {
    held: AtomicBool::new(false),
    pool: UnsafeCell::new(Pool {
        objects: [FREE; OBJECTS],
        queue: [Call {
            slot: 0,
            epoch: 0,
            generation: 0,
            result: 0,
        }; QUEUE],
        head: 0,
        length: 0,
        started: false,
        semaphore: 0,
        timer_event: 0,
        timer_thread: false,
        wait_events: [0; WAIT_THREADS],
        wait_threads: 0,
        workers: 0,
    }),
};
/// Workers waiting for a call.
static IDLE: AtomicUsize = AtomicUsize::new(0);
fn with<R>(f: impl FnOnce(&mut Pool) -> R) -> R {
    while POOL.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *POOL.pool.get() });
    POOL.held.store(false, Ordering::Release);
    r
}

/// The objects' cells: an object's pointer is its cell's address.
#[repr(C, align(16))]
struct Cell([u8; 16]);
static CELLS: [Cell; OBJECTS] = [const { Cell([0; 16]) }; OBJECTS];
fn pointer(slot: usize) -> u64 {
    &CELLS[slot] as *const Cell as u64
}
fn slot_of(pointer: u64) -> Option<usize> {
    let base = CELLS.as_ptr() as u64;
    let offset = pointer.checked_sub(base)?;
    let slot = usize::try_from(offset / 16).ok()?;
    (offset.is_multiple_of(16) && slot < OBJECTS).then_some(slot)
}
/// The slot of a live object of one of `kinds`.
fn find(pool: &Pool, pointer: u64, kinds: &[Kind]) -> Option<usize> {
    let slot = slot_of(pointer)?;
    let o = &pool.objects[slot];
    (kinds.contains(&o.kind) && !o.closing).then_some(slot)
}

fn now() -> u64 {
    crate::guest::GetTickCount64()
}
fn last_error(code: u32) {
    unsafe { RtlSetLastWin32Error(code) };
}
fn read_bytes(pointer: u64, out: &mut [u8]) -> bool {
    pointer != 0 && unsafe { DoorUserCopy(out.as_mut_ptr() as u64, pointer, out.len()) } == 0
}
fn read_i64(pointer: u64) -> Option<i64> {
    let mut b = [0u8; 8];
    read_bytes(pointer, &mut b).then(|| i64::from_le_bytes(b))
}
fn read_pointer(pointer: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    read_bytes(pointer, &mut b[..POINTER]).then(|| abi::widen(u64::from_le_bytes(b) as usize))
}
fn write_pointer(pointer: u64, value: u64) -> bool {
    let b = value.to_le_bytes();
    pointer != 0 && unsafe { DoorUserCopy(pointer, b.as_ptr() as u64, POINTER) } == 0
}
/// A FILETIME due time as GetTickCount64 milliseconds: negative values are
/// relative (100 ns units), positive ones absolute UTC, 0 now.
fn due_from(filetime: i64) -> u64 {
    let base = now();
    if filetime <= 0 {
        return base.saturating_add(filetime.unsigned_abs() / 10_000);
    }
    let mut current = [0u64; 1];
    crate::application::GetSystemTimeAsFileTime(abi::narrow(current.as_mut_ptr() as u64));
    base.saturating_add((filetime as u64).saturating_sub(current[0]) / 10_000)
}
/// TP_CALLBACK_ENVIRON's cleanup group (Version, Pool, CleanupGroup, ...).
fn group_of(pool: &Pool, environment: u64) -> u32 {
    if environment == 0 {
        return 0;
    }
    read_pointer(environment + 2 * POINTER as u64)
        .and_then(|g| find(pool, g, &[Kind::Group]))
        .map_or(0, |s| s as u32 + 1)
}

// ---- threads --------------------------------------------------------------------------
fn start(pool: &mut Pool) -> bool {
    if pool.started {
        return true;
    }
    let semaphore = crate::sync::CreateSemaphoreW(0, 0, i32::MAX, 0);
    let event = crate::guest::CreateEventW(0, 0, 0, 0);
    if semaphore == 0 || event == 0 {
        return false;
    }
    pool.semaphore = abi::widen(semaphore);
    pool.timer_event = abi::widen(event);
    pool.started = true;
    true
}
fn spawn(entry: extern "system" fn(usize) -> u32, argument: usize) -> bool {
    let thread = crate::guest::CreateThread(0, 0, entry as usize, argument, 0, 0);
    if thread == 0 {
        return false;
    }
    crate::guest::CloseHandle(thread);
    true
}
/// Queues a call of `slot`; a worker is started when none is idle.
fn enqueue(pool: &mut Pool, slot: usize, result: u32) -> bool {
    if pool.length == QUEUE {
        return false;
    }
    let o = &mut pool.objects[slot];
    o.pending += 1;
    let at = (pool.head + pool.length) % QUEUE;
    pool.queue[at] = Call {
        slot: slot as u16,
        epoch: o.epoch,
        generation: o.generation,
        result,
    };
    pool.length += 1;
    if IDLE.load(Ordering::Acquire) == 0 && pool.workers < WORKERS && spawn(worker, pool.workers) {
        pool.workers += 1;
    }
    crate::sync::ReleaseSemaphore(abi::narrow(pool.semaphore), 1, 0);
    true
}
fn wake_timers(pool: &mut Pool) -> bool {
    if !pool.timer_thread {
        if !spawn(timer_thread, 0) {
            return false;
        }
        pool.timer_thread = true;
    }
    crate::guest::SetEvent(abi::narrow(pool.timer_event));
    true
}
/// Assigns an armed wait to a wait thread with room (starting one).
fn place_wait(pool: &mut Pool, slot: usize) -> bool {
    let mut counts = [0usize; WAIT_THREADS];
    for o in pool.objects.iter() {
        if o.armed && o.thread != 0 && matches!(o.kind, Kind::Wait | Kind::RegisteredWait) {
            counts[o.thread as usize - 1] += 1;
        }
    }
    let thread = match (0..pool.wait_threads).find(|&t| counts[t] < PER_WAIT_THREAD) {
        Some(t) => t,
        None if pool.wait_threads < WAIT_THREADS => {
            let t = pool.wait_threads;
            let event = abi::widen(crate::guest::CreateEventW(0, 0, 0, 0));
            if event == 0 || !spawn(wait_thread, t) {
                return false;
            }
            pool.wait_events[t] = event;
            pool.wait_threads += 1;
            t
        }
        None => return false,
    };
    pool.objects[slot].thread = thread as u8 + 1;
    crate::guest::SetEvent(abi::narrow(pool.wait_events[thread]));
    true
}
/// Takes a wait off its thread (which drops the handle).
fn unplace_wait(pool: &mut Pool, slot: usize) {
    let o = &mut pool.objects[slot];
    o.armed = false;
    if o.thread != 0 {
        let event = pool.wait_events[o.thread as usize - 1];
        o.thread = 0;
        crate::guest::SetEvent(abi::narrow(event));
    }
}

/// A worker's callback instance: what to do when the callback returns.
#[repr(C)]
struct Instance {
    event: u64,
    semaphore: u64,
    count: u32,
    section: u64,
    library: u64,
}
static INSTANCES: [Instances; WORKERS] = [const { Instances(UnsafeCell::new(None)) }; WORKERS];
struct Instances(UnsafeCell<Option<Instance>>);
unsafe impl Sync for Instances {}
fn instance(pointer: u64) -> Option<&'static mut Instance> {
    let base = INSTANCES.as_ptr() as u64;
    let size = core::mem::size_of::<Instances>() as u64;
    let index = pointer.checked_sub(base)? / size;
    if index as usize >= WORKERS || !(pointer - base).is_multiple_of(size) {
        return None;
    }
    // SAFETY: an instance is used only by its worker, during a callback.
    unsafe { (*INSTANCES[index as usize].0.get()).as_mut() }
}

extern "system" fn worker(index: usize) -> u32 {
    let semaphore = with(|p| p.semaphore);
    let cell = &INSTANCES[index];
    let instance_pointer = cell as *const Instances as u64;
    loop {
        IDLE.fetch_add(1, Ordering::AcqRel);
        crate::guest::WaitForSingleObject(abi::narrow(semaphore), INFINITE);
        IDLE.fetch_sub(1, Ordering::AcqRel);
        let call = with(|p| {
            if p.length == 0 {
                return None;
            }
            let call = p.queue[p.head];
            p.head = (p.head + 1) % QUEUE;
            p.length -= 1;
            let o = &mut p.objects[call.slot as usize];
            if o.kind == Kind::Free || o.generation != call.generation || o.epoch != call.epoch {
                return None;
            }
            o.pending -= 1;
            o.running += 1;
            Some((call, *o))
        });
        let Some((call, o)) = call else {
            continue;
        };
        // SAFETY: this worker's own instance.
        unsafe {
            *cell.0.get() = Some(Instance {
                event: 0,
                semaphore: 0,
                count: 0,
                section: 0,
                library: 0,
            })
        };
        let me = abi::narrow(instance_pointer);
        let object = abi::narrow(pointer(call.slot as usize));
        let context = abi::narrow(o.context);
        // SAFETY: the callbacks the program registered, with their types.
        unsafe {
            match o.kind {
                Kind::Work | Kind::Timer => {
                    let f: extern "system" fn(usize, usize, usize) =
                        core::mem::transmute(o.callback as usize);
                    f(me, context, object);
                }
                Kind::Simple => {
                    let f: extern "system" fn(usize, usize) =
                        core::mem::transmute(o.callback as usize);
                    f(me, context);
                }
                Kind::Wait => {
                    let f: extern "system" fn(usize, usize, usize, u32) =
                        core::mem::transmute(o.callback as usize);
                    f(me, context, object, call.result);
                }
                Kind::QueueTimer | Kind::RegisteredWait => {
                    let f: extern "system" fn(usize, u8) =
                        core::mem::transmute(o.callback as usize);
                    f(
                        context,
                        u8::from(call.result == WAIT_TIMEOUT || o.kind == Kind::QueueTimer),
                    );
                }
                Kind::UserWorkItem => {
                    let f: extern "system" fn(usize) -> u32 =
                        core::mem::transmute(o.callback as usize);
                    f(context);
                }
                _ => (),
            }
        }
        // SAFETY: as above.
        if let Some(actions) = unsafe { (*cell.0.get()).take() } {
            if actions.section != 0 {
                crate::sync::LeaveCriticalSection(abi::narrow(actions.section));
            }
            if actions.semaphore != 0 {
                crate::sync::ReleaseSemaphore(
                    abi::narrow(actions.semaphore),
                    actions.count as i32,
                    0,
                );
            }
            if actions.event != 0 {
                crate::guest::SetEvent(abi::narrow(actions.event));
            }
            if actions.library != 0 {
                crate::guest::FreeLibrary(abi::narrow(actions.library));
            }
        }
        finished(call.slot as usize);
    }
}
/// A callback of `slot` returned: a closed object is freed once its last
/// callback is over.
fn finished(slot: usize) {
    let done = with(|p| {
        let o = &mut p.objects[slot];
        o.running -= 1;
        if (o.closing || matches!(o.kind, Kind::Simple | Kind::UserWorkItem))
            && o.running == 0
            && o.pending == 0
        {
            let event = o.done_event;
            free(o);
            Some(event)
        } else {
            None
        }
    });
    if let Some(event) = done.filter(|&e| e != 0) {
        crate::guest::SetEvent(abi::narrow(event));
    }
}
fn free(o: &mut Object) {
    let generation = o.generation.wrapping_add(1);
    *o = FREE;
    o.generation = generation;
}

extern "system" fn timer_thread(_: usize) -> u32 {
    let event = with(|p| p.timer_event);
    loop {
        let next = with(|p| {
            let t = now();
            let mut next = u64::MAX;
            for slot in 0..OBJECTS {
                let o = p.objects[slot];
                if !o.armed || !matches!(o.kind, Kind::Timer | Kind::QueueTimer) {
                    continue;
                }
                if o.due <= t {
                    let _ = enqueue(p, slot, 0);
                    let o = &mut p.objects[slot];
                    if o.period == 0 {
                        o.armed = false;
                        continue;
                    }
                    o.due = (o.due + u64::from(o.period)).max(t + 1);
                }
                next = next.min(p.objects[slot].due);
            }
            next
        });
        let timeout = if next == u64::MAX {
            INFINITE
        } else {
            next.saturating_sub(now()).min(u64::from(INFINITE - 1)) as u32
        };
        crate::guest::WaitForSingleObject(abi::narrow(event), timeout);
    }
}

extern "system" fn wait_thread(index: usize) -> u32 {
    let event = with(|p| p.wait_events[index]);
    loop {
        let mut handles = [0usize; PER_WAIT_THREAD + 1];
        let mut slots = [0usize; PER_WAIT_THREAD + 1];
        handles[0] = abi::narrow(event);
        let (count, timeout) = with(|p| {
            let t = now();
            let mut count = 1;
            let mut next = u64::MAX;
            for slot in 0..OBJECTS {
                let o = p.objects[slot];
                if !o.armed || o.thread as usize != index + 1 {
                    continue;
                }
                if o.due <= t {
                    let _ = enqueue(p, slot, WAIT_TIMEOUT);
                    rearm_or_disarm(p, slot, t);
                    continue;
                }
                if count <= PER_WAIT_THREAD {
                    handles[count] = abi::narrow(o.handle);
                    slots[count] = slot;
                    count += 1;
                }
                next = next.min(o.due);
            }
            let timeout = if next == u64::MAX {
                INFINITE
            } else {
                next.saturating_sub(t).min(u64::from(INFINITE - 1)) as u32
            };
            (count, timeout)
        });
        let r = crate::guest::WaitForMultipleObjectsEx(
            count as u32,
            abi::narrow(handles.as_ptr() as u64),
            0,
            timeout,
            0,
        );
        if r != 0 && (r as usize) < count {
            let slot = slots[r as usize];
            let handle = abi::widen(handles[r as usize]);
            with(|p| {
                let o = p.objects[slot];
                if o.armed && o.handle == handle && o.thread as usize == index + 1 {
                    let _ = enqueue(p, slot, 0);
                    rearm_or_disarm(p, slot, now());
                }
            });
        } else if r == WAIT_FAILED {
            // A handle was closed under its wait: drop the waits that fail.
            with(|p| {
                for &slot in &slots[1..count] {
                    let o = p.objects[slot];
                    if o.armed
                        && crate::guest::WaitForSingleObject(abi::narrow(o.handle), 0)
                            == WAIT_FAILED
                    {
                        p.objects[slot].armed = false;
                        p.objects[slot].thread = 0;
                    }
                }
            });
        }
    }
}
/// After a wait fired: a pool wait is done; a registered wait waits again
/// (with its timeout restarted) unless WT_EXECUTEONLYONCE.
fn rearm_or_disarm(p: &mut Pool, slot: usize, t: u64) {
    let o = &mut p.objects[slot];
    if o.kind == Kind::RegisteredWait && o.flags & WT_EXECUTEONLYONCE == 0 {
        o.due = if o.period == INFINITE {
            u64::MAX
        } else {
            t + u64::from(o.period)
        };
    } else {
        o.armed = false;
        o.thread = 0;
    }
}

// ---- creating and closing ----------------------------------------------------------------
fn create(kind: Kind, callback: u64, context: u64, environment: u64) -> u64 {
    if callback == 0 && !matches!(kind, Kind::Pool | Kind::Group | Kind::TimerQueue) {
        last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let r = with(|p| {
        if !start(p) {
            return None;
        }
        let owner = group_of(p, environment);
        let slot = p.objects.iter().position(|o| o.kind == Kind::Free)?;
        let o = &mut p.objects[slot];
        let generation = o.generation;
        *o = Object {
            kind,
            generation,
            callback,
            context,
            owner,
            due: u64::MAX,
            ..FREE
        };
        Some(slot)
    });
    match r {
        Some(slot) => pointer(slot),
        None => {
            last_error(ERROR_NOT_ENOUGH_MEMORY);
            0
        }
    }
}
/// Waits until `slot`'s callbacks are over (`cancel`: drops queued ones).
fn wait_callbacks(object: u64, kinds: &[Kind], cancel: bool) {
    let Some(slot) = with(|p| {
        let slot = find(p, object, kinds)?;
        if cancel {
            let o = &mut p.objects[slot];
            o.epoch = o.epoch.wrapping_add(1);
            o.pending = 0;
        }
        Some((slot, p.objects[slot].generation))
    }) else {
        return;
    };
    let mut step = 1;
    while with(|p| {
        let o = &p.objects[slot.0];
        o.generation == slot.1 && (o.pending > 0 || o.running > 0)
    }) {
        crate::guest::Sleep(step);
        step = (step * 2).min(20);
    }
}
/// Closes `object`: disarmed now, freed once its callbacks are over (then
/// `done_event` is set, if any). Returns whether callbacks were outstanding.
fn close(object: u64, kinds: &[Kind], done_event: u64) -> Option<bool> {
    let (busy, event) = with(|p| {
        let slot = find(p, object, kinds)?;
        if matches!(p.objects[slot].kind, Kind::Wait | Kind::RegisteredWait) {
            unplace_wait(p, slot);
        }
        let o = &mut p.objects[slot];
        o.armed = false;
        if o.running == 0 && o.pending == 0 {
            free(o);
            Some((false, done_event))
        } else {
            o.closing = true;
            o.done_event = done_event;
            Some((true, 0))
        }
    })?;
    if event != 0 && event != INVALID_HANDLE_VALUE {
        crate::guest::SetEvent(abi::narrow(event));
    }
    Some(busy)
}
/// Closes and, for INVALID_HANDLE_VALUE, waits until the callbacks are
/// over (UnregisterWaitEx, DeleteTimerQueueTimer).
fn close_waiting(object: u64, kinds: &[Kind], completion: u64) -> i32 {
    let generation = slot_of(object).map(|s| with(|p| p.objects[s].generation));
    match close(object, kinds, completion) {
        None => {
            last_error(ERROR_INVALID_HANDLE);
            0
        }
        Some(false) => 1,
        Some(true) if completion == INVALID_HANDLE_VALUE => {
            let slot = slot_of(object).unwrap();
            let mut step = 1;
            while with(|p| Some(p.objects[slot].generation) == generation) {
                crate::guest::Sleep(step);
                step = (step * 2).min(20);
            }
            1
        }
        Some(true) if completion == 0 => {
            last_error(ERROR_IO_PENDING);
            0
        }
        Some(true) => 1,
    }
}

// ---- work ---------------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CreateThreadpoolWork(
    callback: usize,
    context: usize,
    environment: usize,
) -> usize {
    abi::narrow(create(
        Kind::Work,
        abi::widen(callback),
        abi::widen(context),
        abi::widen(environment),
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn SubmitThreadpoolWork(work: usize) {
    with(|p| {
        if let Some(slot) = find(p, abi::widen(work), &[Kind::Work]) {
            let _ = enqueue(p, slot, 0);
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForThreadpoolWorkCallbacks(work: usize, cancel: i32) {
    wait_callbacks(abi::widen(work), &[Kind::Work], cancel != 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpoolWork(work: usize) {
    let _ = close(abi::widen(work), &[Kind::Work], 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn TrySubmitThreadpoolCallback(
    callback: usize,
    context: usize,
    environment: usize,
) -> i32 {
    submit_once(
        Kind::Simple,
        abi::widen(callback),
        abi::widen(context),
        abi::widen(environment),
    )
}
fn submit_once(kind: Kind, callback: u64, context: u64, environment: u64) -> i32 {
    let object = create(kind, callback, context, environment);
    if object == 0 {
        return 0;
    }
    let queued = with(|p| {
        let slot = slot_of(object).unwrap();
        let queued = enqueue(p, slot, 0);
        if !queued {
            free(&mut p.objects[slot]);
        }
        queued
    });
    if !queued {
        last_error(ERROR_NOT_ENOUGH_MEMORY);
    }
    i32::from(queued)
}
/// QueueUserWorkItem(LPTHREAD_START_ROUTINE, context, WT_* flags).
#[unsafe(no_mangle)]
pub extern "system" fn QueueUserWorkItem(function: usize, context: usize, _flags: u32) -> i32 {
    submit_once(
        Kind::UserWorkItem,
        abi::widen(function),
        abi::widen(context),
        0,
    )
}

// ---- timers ------------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CreateThreadpoolTimer(
    callback: usize,
    context: usize,
    environment: usize,
) -> usize {
    abi::narrow(create(
        Kind::Timer,
        abi::widen(callback),
        abi::widen(context),
        abi::widen(environment),
    ))
}
/// Arms (`due` a FILETIME pointer) or disarms (0) a timer: whether it was
/// armed before.
fn set_timer(timer: u64, kinds: &[Kind], due: Option<u64>, period: u32) -> bool {
    with(|p| {
        let Some(slot) = find(p, timer, kinds) else {
            return false;
        };
        let o = &mut p.objects[slot];
        let was = o.armed;
        match due {
            Some(at) => {
                o.armed = true;
                o.due = at;
                o.period = period;
                wake_timers(p);
            }
            None => {
                o.armed = false;
                // Queued expirations of the old setting are dropped.
                o.epoch = o.epoch.wrapping_add(1);
                o.pending = 0;
            }
        }
        was
    })
}
fn filetime_due(pointer: u64) -> Option<u64> {
    (pointer != 0).then(|| read_i64(pointer).map_or(now(), due_from))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolTimer(timer: usize, due: usize, period: u32, _window: u32) {
    let _ = set_timer(
        abi::widen(timer),
        &[Kind::Timer],
        filetime_due(abi::widen(due)),
        period,
    );
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolTimerEx(
    timer: usize,
    due: usize,
    period: u32,
    _window: u32,
) -> i32 {
    i32::from(set_timer(
        abi::widen(timer),
        &[Kind::Timer],
        filetime_due(abi::widen(due)),
        period,
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn IsThreadpoolTimerSet(timer: usize) -> i32 {
    with(|p| find(p, abi::widen(timer), &[Kind::Timer]).is_some_and(|s| p.objects[s].armed)).into()
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForThreadpoolTimerCallbacks(timer: usize, cancel: i32) {
    wait_callbacks(abi::widen(timer), &[Kind::Timer], cancel != 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpoolTimer(timer: usize) {
    let _ = close(abi::widen(timer), &[Kind::Timer], 0);
}

// ---- waits -------------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CreateThreadpoolWait(
    callback: usize,
    context: usize,
    environment: usize,
) -> usize {
    abi::narrow(create(
        Kind::Wait,
        abi::widen(callback),
        abi::widen(context),
        abi::widen(environment),
    ))
}
fn set_wait(wait: u64, handle: u64, timeout: u64) -> bool {
    let deadline = if timeout == 0 {
        u64::MAX
    } else {
        read_i64(timeout).map_or(u64::MAX, due_from)
    };
    with(|p| {
        let Some(slot) = find(p, wait, &[Kind::Wait]) else {
            return false;
        };
        let was = p.objects[slot].armed;
        unplace_wait(p, slot);
        if handle != 0 {
            let o = &mut p.objects[slot];
            o.handle = handle;
            o.due = deadline;
            o.armed = true;
            if !place_wait(p, slot) {
                p.objects[slot].armed = false;
            }
        } else {
            let o = &mut p.objects[slot];
            o.epoch = o.epoch.wrapping_add(1);
            o.pending = 0;
        }
        was
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolWait(wait: usize, handle: usize, timeout: usize) {
    let _ = set_wait(abi::widen(wait), abi::widen(handle), abi::widen(timeout));
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolWaitEx(
    wait: usize,
    handle: usize,
    timeout: usize,
    _reserved: usize,
) -> i32 {
    i32::from(set_wait(
        abi::widen(wait),
        abi::widen(handle),
        abi::widen(timeout),
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForThreadpoolWaitCallbacks(wait: usize, cancel: i32) {
    wait_callbacks(abi::widen(wait), &[Kind::Wait], cancel != 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpoolWait(wait: usize) {
    let _ = close(abi::widen(wait), &[Kind::Wait], 0);
}

// ---- pools, cleanup groups, callback instances --------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CreateThreadpool(_reserved: usize) -> usize {
    abi::narrow(create(Kind::Pool, 0, 0, 0))
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpool(pool: usize) {
    let _ = close(abi::widen(pool), &[Kind::Pool], 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolThreadMaximum(_pool: usize, _most: u32) {}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolThreadMinimum(_pool: usize, _least: u32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadpoolStackInformation(_pool: usize, _information: usize) -> i32 {
    1
}
/// TP_POOL_STACK_INFORMATION { StackReserve, StackCommit }: the 256 KiB
/// every Doors thread has.
#[unsafe(no_mangle)]
pub extern "system" fn QueryThreadpoolStackInformation(_pool: usize, information: usize) -> i32 {
    let information = abi::widen(information);
    i32::from(
        write_pointer(information, 256 * 1024)
            && write_pointer(information + POINTER as u64, 256 * 1024),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateThreadpoolCleanupGroup() -> usize {
    abi::narrow(create(Kind::Group, 0, 0, 0))
}
/// Closes every member (waiting for their callbacks; `cancel` drops queued
/// ones first).
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpoolCleanupGroupMembers(
    group: usize,
    cancel: i32,
    _context: usize,
) {
    let group = abi::widen(group);
    let Some(id) = with(|p| find(p, group, &[Kind::Group]).map(|s| s as u32 + 1)) else {
        return;
    };
    let members: [Option<u64>; OBJECTS] = with(|p| {
        let mut m = [None; OBJECTS];
        for (slot, o) in p.objects.iter().enumerate() {
            if o.owner == id
                && !o.closing
                && matches!(o.kind, Kind::Work | Kind::Timer | Kind::Wait | Kind::Simple)
            {
                m[slot] = Some(pointer(slot));
            }
        }
        m
    });
    for object in members.iter().flatten() {
        let kinds = [Kind::Work, Kind::Timer, Kind::Wait, Kind::Simple];
        wait_callbacks(*object, &kinds, cancel != 0);
        let _ = close(*object, &kinds, 0);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseThreadpoolCleanupGroup(group: usize) {
    let _ = close(abi::widen(group), &[Kind::Group], 0);
}
#[unsafe(no_mangle)]
pub extern "system" fn CallbackMayRunLong(_instance: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DisassociateCurrentThreadFromCallback(_instance: usize) {}
#[unsafe(no_mangle)]
pub extern "system" fn SetEventWhenCallbackReturns(instance: usize, event: usize) {
    if let Some(i) = self::instance(abi::widen(instance)) {
        i.event = abi::widen(event);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseSemaphoreWhenCallbackReturns(
    instance: usize,
    semaphore: usize,
    count: u32,
) {
    if let Some(i) = self::instance(abi::widen(instance)) {
        i.semaphore = abi::widen(semaphore);
        i.count = count;
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LeaveCriticalSectionWhenCallbackReturns(instance: usize, section: usize) {
    if let Some(i) = self::instance(abi::widen(instance)) {
        i.section = abi::widen(section);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeLibraryWhenCallbackReturns(instance: usize, module: usize) {
    if let Some(i) = self::instance(abi::widen(instance)) {
        i.library = abi::widen(module);
    }
}

// ---- the legacy pool: registered waits and timer queues ------------------------------------
/// RegisterWaitForSingleObject(out, object, WAITORTIMERCALLBACK, context,
/// timeout ms, WT_* flags).
#[unsafe(no_mangle)]
pub extern "system" fn RegisterWaitForSingleObject(
    out: usize,
    handle: usize,
    callback: usize,
    context: usize,
    millis: u32,
    flags: u32,
) -> i32 {
    let out = abi::widen(out);
    let handle = abi::widen(handle);
    if out == 0 || handle == 0 {
        last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let object = create(
        Kind::RegisteredWait,
        abi::widen(callback),
        abi::widen(context),
        0,
    );
    if object == 0 {
        return 0;
    }
    let placed = with(|p| {
        let slot = slot_of(object).unwrap();
        let o = &mut p.objects[slot];
        o.handle = handle;
        o.flags = flags;
        o.period = millis;
        o.due = if millis == INFINITE {
            u64::MAX
        } else {
            now() + u64::from(millis)
        };
        o.armed = true;
        let placed = place_wait(p, slot);
        if !placed {
            free(&mut p.objects[slot]);
        }
        placed
    });
    if !placed || !write_pointer(out, object) {
        let _ = close(object, &[Kind::RegisteredWait], 0);
        last_error(ERROR_NOT_ENOUGH_MEMORY);
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterWait(wait: usize) -> i32 {
    close_waiting(abi::widen(wait), &[Kind::RegisteredWait], 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterWaitEx(wait: usize, completion: usize) -> i32 {
    close_waiting(
        abi::widen(wait),
        &[Kind::RegisteredWait],
        abi::widen(completion),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateTimerQueue() -> usize {
    abi::narrow(create(Kind::TimerQueue, 0, 0, 0))
}
/// CreateTimerQueueTimer(out, queue (0: the default), WAITORTIMERCALLBACK,
/// parameter, due ms, period ms, WT_* flags).
#[unsafe(no_mangle)]
pub extern "system" fn CreateTimerQueueTimer(
    out: usize,
    queue: usize,
    callback: usize,
    parameter: usize,
    due: u32,
    period: u32,
    flags: u32,
) -> i32 {
    let out = abi::widen(out);
    let queue = abi::widen(queue);
    let owner = if queue == 0 {
        Some(0)
    } else {
        with(|p| find(p, queue, &[Kind::TimerQueue]).map(|s| s as u32 + 1))
    };
    let Some(owner) = owner.filter(|_| out != 0) else {
        last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    let timer = create(
        Kind::QueueTimer,
        abi::widen(callback),
        abi::widen(parameter),
        0,
    );
    if timer == 0 {
        return 0;
    }
    with(|p| {
        let slot = slot_of(timer).unwrap();
        let o = &mut p.objects[slot];
        o.owner = owner;
        o.flags = flags;
    });
    let period = if flags & WT_EXECUTEONLYONCE != 0 {
        0
    } else {
        period
    };
    set_timer(
        timer,
        &[Kind::QueueTimer],
        Some(now() + u64::from(due)),
        period,
    );
    if !write_pointer(out, timer) {
        let _ = close(timer, &[Kind::QueueTimer], 0);
        last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeTimerQueueTimer(
    _queue: usize,
    timer: usize,
    due: u32,
    period: u32,
) -> i32 {
    let timer = abi::widen(timer);
    let once = with(|p| {
        find(p, timer, &[Kind::QueueTimer]).map(|s| p.objects[s].flags & WT_EXECUTEONLYONCE != 0)
    });
    let Some(once) = once else {
        last_error(ERROR_INVALID_PARAMETER);
        return 0;
    };
    set_timer(
        timer,
        &[Kind::QueueTimer],
        Some(now() + u64::from(due)),
        if once { 0 } else { period },
    );
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteTimerQueueTimer(
    _queue: usize,
    timer: usize,
    completion: usize,
) -> i32 {
    close_waiting(
        abi::widen(timer),
        &[Kind::QueueTimer],
        abi::widen(completion),
    )
}
/// Deletes every timer of the queue, then the queue.
#[unsafe(no_mangle)]
pub extern "system" fn DeleteTimerQueueEx(queue: usize, completion: usize) -> i32 {
    let queue = abi::widen(queue);
    let completion = abi::widen(completion);
    let Some(id) = with(|p| find(p, queue, &[Kind::TimerQueue]).map(|s| s as u32 + 1)) else {
        last_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    let timers: [Option<u64>; OBJECTS] = with(|p| {
        let mut m = [None; OBJECTS];
        for (slot, o) in p.objects.iter().enumerate() {
            if o.kind == Kind::QueueTimer && o.owner == id && !o.closing {
                m[slot] = Some(pointer(slot));
            }
        }
        m
    });
    let wait = if completion == INVALID_HANDLE_VALUE {
        INVALID_HANDLE_VALUE
    } else {
        0
    };
    for timer in timers.iter().flatten() {
        let _ = close_waiting(*timer, &[Kind::QueueTimer], wait);
    }
    let _ = close(queue, &[Kind::TimerQueue], 0);
    if completion != 0 && completion != INVALID_HANDLE_VALUE {
        crate::guest::SetEvent(abi::narrow(completion));
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteTimerQueue(queue: usize) -> i32 {
    DeleteTimerQueueEx(queue, 0)
}
