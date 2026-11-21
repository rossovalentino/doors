//! IRQ-safe bridge between portable task ownership and actual stack switching.
//! Lock order is scheduler alone; page-table/shootdown paths never take it.
use crate::arch::{self, Context};
use core::{
    arch::asm,
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use door_kernel::task::{
    ObjectId, RetirementTicket, Scheduler, StopRequests, TaskId, WaitOutcome, WaitStatus,
};

struct Runtime {
    scheduler: Scheduler,
    idle: [u64; 4],
    retirement: [Option<RetirementTicket>; 4],
}
/// Built in place by `initialize` (the scheduler is too large for the boot
/// stack); `READY` says it is.
struct Storage(UnsafeCell<core::mem::MaybeUninit<Runtime>>);
// Every access is serialized by OWNER with this CPU's maskable IRQs disabled.
unsafe impl Sync for Storage {}
static STATE: Storage = Storage(UnsafeCell::new(core::mem::MaybeUninit::uninit()));
static READY: AtomicBool = AtomicBool::new(false);
static OWNER: AtomicUsize = AtomicUsize::new(0);
static PRELOCKED: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
static PREEMPTIONS: [AtomicU64; door_kernel::task::MAX_TASKS] =
    [const { AtomicU64::new(0) }; door_kernel::task::MAX_TASKS];
static STOPS: StopRequests = StopRequests::new();

fn irq_save() -> u64 {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", "cli", out(reg) flags);
    }
    flags
}
fn irq_restore(flags: u64) {
    if flags & 0x200 != 0 {
        arch::enable_interrupts();
    }
}
fn lock(cpu: usize) {
    assert_ne!(
        OWNER.load(Ordering::Relaxed),
        cpu + 1,
        "recursive scheduler lock"
    );
    while OWNER
        .compare_exchange(0, cpu + 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}
fn unlock(cpu: usize) {
    assert_eq!(
        OWNER.swap(0, Ordering::Release),
        cpu + 1,
        "scheduler lock owner"
    );
}
unsafe fn state() -> &'static mut Runtime {
    // Callers hold OWNER; no reference survives a callback or context switch.
    assert!(READY.load(Ordering::Acquire), "scheduler initialized");
    unsafe { (*STATE.0.get()).assume_init_mut() }
}
pub fn with<R>(f: impl FnOnce(&mut Scheduler) -> R) -> R {
    let flags = irq_save();
    let cpu = arch::current_cpu();
    lock(cpu);
    let result = f(&mut unsafe { state() }.scheduler);
    unlock(cpu);
    irq_restore(flags);
    result
}
pub fn initialize(online_cpus: u8) {
    let flags = irq_save();
    lock(0);
    unsafe {
        assert!(
            !READY.load(Ordering::Acquire),
            "scheduler initialized twice"
        );
        let runtime = (*STATE.0.get()).as_mut_ptr();
        Scheduler::initialize_at(core::ptr::addr_of_mut!((*runtime).scheduler), online_cpus)
            .expect("session scheduler");
        core::ptr::addr_of_mut!((*runtime).idle).write([0; 4]);
        core::ptr::addr_of_mut!((*runtime).retirement).write([None; 4]);
        READY.store(true, Ordering::Release);
        arch::install_schedule_hooks(schedule, commit);
    }
    unlock(0);
    irq_restore(flags);
}
unsafe extern "C" fn schedule(saved: *mut Context, cpu: usize, reason: u64) -> *mut Context {
    if !PRELOCKED[cpu].swap(false, Ordering::AcqRel) {
        lock(cpu);
    }
    assert_eq!(OWNER.load(Ordering::Relaxed), cpu + 1);
    let runtime = unsafe { state() };
    let current = runtime.scheduler.current(cpu as u8).expect("current CPU");
    if let Some(task) = current
        && let Some(code) = STOPS.requested(task)
        && unsafe { &*((*saved).frame as *const arch::TrapFrame) }.cs & 3 == 3
        && !matches!(
            runtime
                .scheduler
                .snapshot(task)
                .expect("current task")
                .state,
            door_kernel::task::State::Exiting | door_kernel::task::State::Terminated
        )
    {
        // A kernel continuation may own pinned buffers or executive leases.
        // Only a CPL3 continuation can be discarded here; in-kernel operations
        // observe the request at their bounded syscall-return safe point.
        runtime
            .scheduler
            .terminate(task, code)
            .expect("user safe-point stop");
    }
    if cpu == 0 {
        runtime
            .scheduler
            .advance_time(arch::ticks(0))
            .expect("monotonic timer");
    }
    if let Some(id) = current {
        if reason == 0 {
            PREEMPTIONS[id.slot()].fetch_add(1, Ordering::Release);
        }
    } else {
        runtime.idle[cpu] = saved as u64;
    }
    let decision = runtime
        .scheduler
        .schedule(cpu as u8, if current.is_some() { saved as u64 } else { 0 })
        .expect("schedule ownership");
    runtime.retirement[cpu] = decision.retirement;
    let selected = decision.context.unwrap_or(runtime.idle[cpu]);
    assert_ne!(selected, 0, "idle context not captured");
    // OWNER intentionally crosses the short assembly stack handoff. No Rust
    // reference from this invocation remains live when commit accesses STATE.
    selected as *mut Context
}
unsafe extern "C" fn commit(cpu: usize) {
    assert_eq!(OWNER.load(Ordering::Relaxed), cpu + 1);
    let runtime = unsafe { state() };
    if let Some(ticket) = runtime.retirement[cpu].take() {
        runtime
            .scheduler
            .acknowledge_retirement(ticket)
            .expect("stack retirement");
    }
    unlock(cpu);
}
pub fn current() -> TaskId {
    current_optional().expect("managed thread")
}
pub fn current_optional() -> Option<TaskId> {
    with(|s| s.current(arch::current_cpu() as u8).unwrap())
}
pub fn now() -> u64 {
    with(|s| s.now())
}
pub fn preemptions(id: TaskId) -> &'static AtomicU64 {
    &PREEMPTIONS[id.slot()]
}
pub fn request_stop(id: TaskId, code: i64) {
    if with(|s| STOPS.request(s, id, code)).is_err() {
        return; // The captured task was already retired and possibly replaced.
    }
    for cpu in 0..4 {
        if arch::online_mask() & (1 << cpu) != 0 {
            arch::send_reschedule(cpu).expect("stop reschedule");
        }
    }
}
pub fn stop_requested(id: TaskId) -> Option<i64> {
    STOPS.requested(id)
}

fn blocking_checked(
    f: impl FnOnce(&mut Scheduler, u8) -> Result<WaitStatus, door_kernel::task::Error>,
) -> Result<WaitOutcome, door_kernel::task::Error> {
    let flags = irq_save();
    let cpu = arch::current_cpu();
    lock(cpu);
    let (id, status) = {
        let scheduler = &mut unsafe { state() }.scheduler;
        let id = scheduler
            .current(cpu as u8)
            .unwrap()
            .expect("waiting thread");
        let status = if STOPS.requested(id).is_some() {
            Err(door_kernel::task::Error::InvalidState)
        } else {
            f(scheduler, cpu as u8)
        };
        (id, status)
    };
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            unlock(cpu);
            irq_restore(flags);
            return Err(error);
        }
    };
    if status == WaitStatus::Blocked {
        PRELOCKED[cpu].store(true, Ordering::Release);
        arch::yield_now();
        // The thread may now be on another CPU. Its previous CPU's commit
        // released the lock before allowing any other task to wake or reuse it.
    } else {
        unlock(cpu);
    }
    let result = with(|s| s.take_wake(id).unwrap().expect("one wait completion"));
    irq_restore(flags);
    Ok(result)
}
pub fn wait(objects: &[ObjectId], deadline: Option<u64>) -> WaitOutcome {
    wait_checked(objects, deadline).expect("native wait")
}
pub fn condition_wait(condition: ObjectId, mutex: ObjectId, deadline: Option<u64>) -> WaitOutcome {
    condition_wait_checked(condition, mutex, deadline).expect("condition wait")
}
pub fn wait_checked(
    objects: &[ObjectId],
    deadline: Option<u64>,
) -> Result<WaitOutcome, door_kernel::task::Error> {
    blocking_checked(|s, cpu| s.wait_current(cpu, objects, deadline))
}
pub fn condition_wait_checked(
    condition: ObjectId,
    mutex: ObjectId,
    deadline: Option<u64>,
) -> Result<WaitOutcome, door_kernel::task::Error> {
    blocking_checked(|s, cpu| s.condition_wait(cpu, condition, mutex, deadline))
}
/// A futex-style wait (`Scheduler::wait_address`).
pub fn wait_address_checked(
    key: u64,
    sequence: u64,
    deadline: Option<u64>,
) -> Result<WaitOutcome, door_kernel::task::Error> {
    blocking_checked(|s, cpu| s.wait_address(cpu, key, sequence, deadline))
}
pub fn exit(result: i64) -> ! {
    let _ = irq_save();
    let cpu = arch::current_cpu();
    lock(cpu);
    unsafe { state() }
        .scheduler
        .exit_current(cpu as u8, result)
        .expect("thread exit");
    PRELOCKED[cpu].store(true, Ordering::Release);
    arch::yield_now();
    panic!("terminated thread resumed")
}

/// Exit a user entry without nesting a software interrupt on an exception IST.
/// The architecture invokes the usual retirement commit on the selected stack.
pub unsafe fn exit_user(saved: *mut Context, result: i64) -> *mut Context {
    arch::disable_interrupts();
    let cpu = arch::current_cpu();
    lock(cpu);
    unsafe { state() }
        .scheduler
        .exit_current(cpu as u8, result)
        .expect("user thread exit");
    PRELOCKED[cpu].store(true, Ordering::Release);
    unsafe { arch::schedule_from_user(saved, 1) }
}
