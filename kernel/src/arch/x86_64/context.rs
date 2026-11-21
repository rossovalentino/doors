use super::{
    cpu::current_cpu,
    layout::{CODE_SELECTOR, Context, DATA_SELECTOR, TrapFrame},
};
use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
core::arch::global_asm!(include_str!("state_probe.S"), options(att_syntax));
unsafe extern "C" {
    fn door_state_probe(
        pattern: u64,
        preemptions: *const core::sync::atomic::AtomicU64,
        target: u64,
    ) -> u64;
}

/// # Safety
/// Run on a scheduled thread with interrupts enabled, FS:0 mapped to `pattern`,
/// and a stable aligned counter incremented only for this thread's actual timer
/// preemptions. Target must exceed its current value. No concurrent counter reset.
pub unsafe fn state_probe(
    pattern: u64,
    preemptions: *const core::sync::atomic::AtomicU64,
    target: u64,
) -> bool {
    unsafe { door_state_probe(pattern, preemptions, target) == 0 }
}

pub type ScheduleHook = unsafe extern "C" fn(*mut Context, usize, u64) -> *mut Context;
pub type CommitHook = unsafe extern "C" fn(usize);
static SCHEDULE: AtomicUsize = AtomicUsize::new(0);
static COMMIT: AtomicUsize = AtomicUsize::new(0);
static PENDING: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
pub const YIELD_VECTOR: u64 = 0x41;
pub const RESCHEDULE_VECTOR: u64 = 0x42;

/// # Safety
/// Install once on BSP before any CPU starts scheduling. Callbacks must preserve
/// context ownership, return a mapped owned context, remain allocation-free, and
/// commit outgoing-stack retirement only in the second callback on the new stack.
pub unsafe fn install_schedule_hooks(schedule: ScheduleHook, commit: CommitHook) {
    assert_eq!(
        SCHEDULE.load(Ordering::Acquire),
        0,
        "scheduler hooks already installed"
    );
    COMMIT.store(commit as usize, Ordering::Release);
    SCHEDULE.store(schedule as usize, Ordering::Release);
}

pub(super) fn schedule(context: *mut Context, reason: u64) -> *mut Context {
    let hook = SCHEDULE.load(Ordering::Acquire);
    if hook == 0 {
        return context;
    }
    let cpu = current_cpu();
    assert!(
        !PENDING[cpu].swap(true, Ordering::AcqRel),
        "nested context switch"
    );
    let hook: ScheduleHook = unsafe { core::mem::transmute(hook) };
    unsafe { hook(context, cpu, reason) }
}

/// # Safety
/// Called from a user trap/syscall hook with IRQs disabled. The execution layer
/// has prepared the same scheduler lock/state that its normal schedule hook
/// consumes. The returned context must go through the assembly switch epilogue.
pub unsafe fn schedule_from_user(context: *mut Context, reason: u64) -> *mut Context {
    schedule(context, reason)
}

pub(super) fn pending() -> bool {
    PENDING[current_cpu()].load(Ordering::Acquire)
}

#[unsafe(no_mangle)]
extern "C" fn door_switch_commit(context: *const Context) {
    let cpu = current_cpu();
    unsafe {
        super::user::select(context);
    }
    if PENDING[cpu].swap(false, Ordering::AcqRel) {
        let hook: CommitHook = unsafe { core::mem::transmute(COMMIT.load(Ordering::Acquire)) };
        unsafe {
            hook(cpu);
        }
    }
}

/// # Safety
/// Exclusive RW/NX guarded stack mapping containing at least4KiB below `stack_top`;
/// retain it and the canonical FS backing until the scheduler retires the thread.
/// Entry follows the SysV ABI and never returns. No other CPU may own this context.
pub unsafe fn create_context(
    stack_top: u64,
    entry: extern "C" fn(u64) -> !,
    argument: u64,
    fs_base: u64,
) -> Result<*mut Context, &'static str> {
    fn canonical(value: u64) -> bool {
        value <= 0x7fff_ffff_ffff || value >= 0xffff_8000_0000_0000
    }
    if stack_top < 0xffff_8000_0000_1000 || !stack_top.is_multiple_of(16) || !canonical(fs_base) {
        return Err("invalid initial thread stack or TLS");
    }
    let frame_address = stack_top - 192;
    let context_address = frame_address - core::mem::size_of::<Context>() as u64;
    let frame = TrapFrame {
        rdi: argument,
        rip: entry as *const () as u64,
        cs: u64::from(CODE_SELECTOR),
        rflags: 0x202,
        rsp: stack_top - 8,
        ss: u64::from(DATA_SELECTOR),
        ..TrapFrame::default()
    };
    unsafe {
        (stack_top as *mut u64).sub(1).write(0);
        (frame_address as *mut TrapFrame).write(frame);
        let mut context = Context::fresh(frame_address, fs_base);
        context.cr3 = super::user::kernel_cr3();
        context.kernel_stack_top = stack_top;
        (context_address as *mut Context).write(context);
    }
    Ok(context_address as *mut Context)
}

pub fn yield_now() {
    unsafe {
        asm!("int 0x41");
    }
}

pub fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nomem, nostack));
    }
}

pub fn wait_for_interrupt() {
    // STI's interrupt shadow makes the enable-and-halt pair race-free.
    unsafe {
        asm!("sti", "hlt", options(nomem, nostack));
    }
}
