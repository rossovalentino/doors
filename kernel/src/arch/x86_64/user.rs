//! Protected x64 entry, original Door ABI dispatch hooks and bounded user access.
use super::{
    Context, cpu,
    layout::{self, TrapFrame},
};
use core::{
    arch::asm,
    ptr,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub const SYSCALL_VECTOR: u64 = 0x100;
pub const INVALID_RETURN_VECTOR: u64 = 0x101;
pub const MAX_USER_COPY: usize = 64 * 1024;
pub type SyscallHook = unsafe extern "C" fn(*mut Context, usize) -> *mut Context;
pub type UserFaultHook = unsafe extern "C" fn(*mut Context, usize, u64, u64, u64) -> *mut Context;
static SYSCALL: AtomicUsize = AtomicUsize::new(0);
static FAULT: AtomicUsize = AtomicUsize::new(0);
static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// The first three offsets are consumed before assembly may use any stack.
#[repr(C, align(64))]
struct EntryState {
    kernel_stack_top: u64,
    kernel_cr3: u64,
    scratch_user_rsp: u64,
    copy_start: u64,
    copy_end: u64,
    copy_active: bool,
}
static mut ENTRY: [EntryState; 4] = [const {
    EntryState {
        kernel_stack_top: 0,
        kernel_cr3: 0,
        scratch_user_rsp: 0,
        copy_start: 0,
        copy_end: 0,
        copy_active: false,
    }
}; 4];
const _: () = {
    assert!(core::mem::offset_of!(EntryState, kernel_stack_top) == 0);
    assert!(core::mem::offset_of!(EntryState, kernel_cr3) == 8);
    assert!(core::mem::offset_of!(EntryState, scratch_user_rsp) == 16);
};

unsafe extern "C" {
    fn door_syscall_entry();
    fn door_syscall_compat_entry();
    fn door_user_copy(destination: u64, source: u64, bytes: usize, cr3: u64) -> usize;
    static door_user_copy_instruction: u8;
    static door_user_copy_fixup: u8;
}
core::arch::global_asm!(include_str!("user_copy.S"), options(att_syntax));

fn entry() -> *mut EntryState {
    unsafe {
        ptr::addr_of_mut!(ENTRY)
            .cast::<EntryState>()
            .add(cpu::current_cpu())
    }
}

pub fn kernel_cr3() -> u64 {
    KERNEL_CR3.load(Ordering::Acquire)
}

/// # Safety
/// Called only during this CPU's descriptor installation, IRQs disabled, with
/// shared supervisor kernel mappings and PCID/LA57/FSGSBASE disabled.
pub(super) unsafe fn initialize() -> Result<(), &'static str> {
    if core::arch::x86_64::__cpuid(0x8000_0000).eax < 0x8000_0001
        || core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 11) == 0
    {
        return Err("SYSCALL unavailable");
    }
    let cr3: u64;
    let cr4: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
    }
    if !layout::valid_cr3(cr3) || cr4 & ((1 << 12) | (1 << 16) | (1 << 17)) != 0 {
        return Err("unsupported syscall paging/base mode");
    }
    let expected = KERNEL_CR3.compare_exchange(0, cr3, Ordering::AcqRel, Ordering::Acquire);
    if expected.is_err_and(|old| old != cr3) {
        return Err("CPU kernel CR3 mismatch");
    }
    let state = entry();
    if (state as u64) < 0xffff_8000_0000_0000 {
        return Err("kernel GS storage outside shared supervisor half");
    }
    unsafe {
        (*state).kernel_cr3 = cr3;
        write_msr(0xc000_0101, state as u64);
        write_msr(0xc000_0102, 0);
        // STAR user base32 gives SS43/CS51. Return uses checked IRET, not SYSRET.
        write_msr(0xc000_0081, (32u64 << 48) | (8u64 << 32));
        write_msr(0xc000_0082, door_syscall_entry as *const () as u64);
        // CSTAR: SYSCALL from 32-bit code (AMD only; Intel raises #UD) is no
        // way into Door: it arrives as an invalid-opcode fault of that code.
        write_msr(0xc000_0083, door_syscall_compat_entry as *const () as u64);
        // SYSENTER (Intel, from 32-bit code) is no way in either: with a
        // null IA32_SYSENTER_CS it raises #GP in the program, whatever the
        // firmware left in these MSRs.
        write_msr(0x174, 0);
        write_msr(0x175, 0);
        write_msr(0x176, 0);
        // No ordinary IRQ, direction, single-step, alignment or nested-task
        // state may affect the untrusted-RSP transition window.
        write_msr(
            0xc000_0084,
            (1 << 8) | (1 << 9) | (1 << 10) | (3 << 12) | (1 << 14) | (1 << 18),
        );
        write_msr(0xc000_0080, read_msr(0xc000_0080) | 1);
    }
    Ok(())
}

unsafe fn read_msr(register: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!("rdmsr", in("ecx") register, out("eax") low, out("edx") high, options(nomem, nostack));
    }
    u64::from(low) | (u64::from(high) << 32)
}
unsafe fn write_msr(register: u32, value: u64) {
    unsafe {
        asm!("wrmsr", in("ecx") register, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nomem, nostack));
    }
}

/// # Safety
/// Install once before any process can run. Hooks run on the boot address space
/// with IRQs off and trusted kernel stacks. Fault hooks must retire the offending
/// process and return another owned context. A selected replacement uses
/// `schedule_from_user` so the existing post-stack-switch commit remains mandatory.
pub unsafe fn install_user_hooks(syscall: SyscallHook, fault: UserFaultHook) {
    assert_eq!(SYSCALL.load(Ordering::Acquire), 0);
    FAULT.store(fault as usize, Ordering::Release);
    SYSCALL.store(syscall as usize, Ordering::Release);
}

#[derive(Clone, Copy)]
pub struct UserContext {
    pub entry: u64,
    pub stack_pointer: u64,
    pub fs_base: u64,
    pub user_gs_base: u64,
    pub cr3: u64,
    pub kernel_stack_top: u64,
    pub arguments: layout::EntryArguments,
    /// A 32-bit (x86) program's thread (P17): it starts in compatibility
    /// mode (selector 0x3B), on a stack only 4-byte aligned, with every
    /// address below 4 GB.
    pub compat: bool,
}

/// # Safety
/// All process mappings and its CR3 are owned and retained until post-switch
/// retirement. Kernel stack is exclusive, shared supervisor RW/NX, guarded, and
/// has at least 4 KiB below its top. Entry/stack/TLS/startup were validated by VM.
pub unsafe fn create_user_context(initial: UserContext) -> Result<*mut Context, &'static str> {
    let frame_address = initial
        .kernel_stack_top
        .checked_sub(192)
        .ok_or("user kernel stack underflow")?;
    let context_address = frame_address
        .checked_sub(core::mem::size_of::<Context>() as u64)
        .ok_or("user context underflow")?;
    let mut frame = TrapFrame {
        rip: initial.entry,
        rsp: initial.stack_pointer,
        cs: u64::from(if initial.compat {
            layout::USER_CODE32_SELECTOR
        } else {
            layout::USER_CODE_SELECTOR
        }),
        ss: u64::from(layout::USER_DATA_SELECTOR),
        rflags: 0x202,
        ..TrapFrame::default()
    };
    initial.arguments.apply(&mut frame);
    let mut context = Context::fresh(frame_address, initial.fs_base);
    context.cr3 = initial.cr3;
    context.kernel_stack_top = initial.kernel_stack_top;
    context.restore_user_gs = 1;
    context.user_gs_base = initial.user_gs_base;
    let aligned = if initial.compat {
        initial.stack_pointer & 3 == 0
    } else {
        initial.stack_pointer & 15 == 8
    };
    if !layout::valid_user_return(&context, &frame, kernel_cr3()) || !aligned {
        return Err("invalid initial user context");
    }
    unsafe {
        (frame_address as *mut TrapFrame).write(frame);
        (context_address as *mut Context).write(context);
    }
    Ok(context_address as *mut Context)
}

pub(super) fn syscall(context: *mut Context) -> *mut Context {
    let hook = SYSCALL.load(Ordering::Acquire);
    if hook == 0 {
        cpu::fatal(unsafe { &*((*context).frame as *const TrapFrame) }, 0);
    }
    let hook: SyscallHook = unsafe { core::mem::transmute(hook) };
    unsafe { hook(context, cpu::current_cpu()) }
}

pub(super) fn fault(context: *mut Context, vector: u64, error: u64, cr2: u64) -> *mut Context {
    let hook = FAULT.load(Ordering::Acquire);
    if hook == 0 {
        cpu::fatal(unsafe { &*((*context).frame as *const TrapFrame) }, cr2);
    }
    let hook: UserFaultHook = unsafe { core::mem::transmute(hook) };
    unsafe { hook(context, cpu::current_cpu(), vector, error, cr2) }
}

pub(super) fn valid_return(context: *mut Context) -> bool {
    let context = unsafe { &*context };
    let frame = unsafe { &*(context.frame as *const TrapFrame) };
    frame.cs & 3 == 0 || layout::valid_user_return(context, frame, kernel_cr3())
}

pub(super) unsafe fn select(context: *const Context) {
    let top = unsafe { (*context).kernel_stack_top };
    unsafe {
        (*entry()).kernel_stack_top = top;
    }
    // Ring0 boot/idle contexts may have no privilege-transition stack yet.
    if top != 0 {
        unsafe {
            cpu::set_kernel_stack(top);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyError {
    InvalidRange,
    Fault { copied: usize },
}

/// # Safety
/// `cr3` names a live owned process. Its VM lock/pins span validation and this
/// call; mapped bytes have effective U/S+read permission. Normal callers validate
/// the complete range first. An unmapped diagnostic tail is allowed solely to
/// exercise fault recovery, and returns the copied prefix. No Rust user references.
pub unsafe fn copy_from_user(
    cr3: u64,
    address: u64,
    output: &mut [u8],
) -> Result<usize, CopyError> {
    unsafe {
        copy(
            cr3,
            address,
            output.as_mut_ptr() as u64,
            address,
            output.len(),
        )
    }
}

/// # Safety
/// As `copy_from_user`, with effective U/S+write permission for the whole output.
pub unsafe fn copy_to_user(cr3: u64, address: u64, input: &[u8]) -> Result<usize, CopyError> {
    unsafe { copy(cr3, address, address, input.as_ptr() as u64, input.len()) }
}
/// Copy a live shared Section alias into a caller's private validated output.
/// No Rust reference is formed over concurrently writable producer bytes.
/// # Safety
/// The sole VM guard, a READ Section lease and IRQ exclusion span this call.
/// `source` covers `bytes` readable supervisor bytes in the shared kernel half;
/// the complete private user destination is validated/pinned writable. Source
/// and destination cannot alias. Concurrent raw producer writes may affect this
/// capture only. External REP MOVSB observes memory without immutable assumptions.
pub unsafe fn copy_shared_to_user(
    cr3: u64,
    address: u64,
    source: u64,
    bytes: usize,
) -> Result<usize, CopyError> {
    if source < 0xffff_8000_0000_0000 || source.checked_add(bytes as u64).is_none() {
        return Err(CopyError::InvalidRange);
    }
    unsafe { copy(cr3, address, address, source, bytes) }
}

/// Copy a caller's validated user bytes into a live shared Section page
/// alias (SECTION_FILL: the storage service filling a client's section).
/// # Safety
/// The sole VM guard and IRQ exclusion span this call; `destination` covers
/// `bytes` writable supervisor bytes in the shared kernel half that no Rust
/// reference covers; the user source is validated readable.
pub unsafe fn copy_user_to_shared(
    cr3: u64,
    address: u64,
    destination: u64,
    bytes: usize,
) -> Result<usize, CopyError> {
    if destination < 0xffff_8000_0000_0000 || destination.checked_add(bytes as u64).is_none() {
        return Err(CopyError::InvalidRange);
    }
    unsafe { copy(cr3, address, destination, address, bytes) }
}

unsafe fn copy(
    cr3: u64,
    user: u64,
    destination: u64,
    source: u64,
    bytes: usize,
) -> Result<usize, CopyError> {
    let end = user
        .checked_add(bytes as u64)
        .ok_or(CopyError::InvalidRange)?;
    if !layout::valid_cr3(cr3)
        || cr3 == kernel_cr3()
        || bytes > MAX_USER_COPY
        || user < layout::USER_MIN
        || end > layout::USER_END
    {
        return Err(CopyError::InvalidRange);
    }
    if bytes == 0 {
        return Ok(0);
    }
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", "cli", out(reg) flags);
    }
    let state = entry();
    assert!(!unsafe { (*state).copy_active }, "nested user copy");
    unsafe {
        (*state).copy_start = user;
        (*state).copy_end = end;
        (*state).copy_active = true;
    }
    let remaining = unsafe { door_user_copy(destination, source, bytes, cr3) };
    unsafe {
        (*state).copy_active = false;
    }
    if flags & (1 << 9) != 0 {
        unsafe {
            asm!("sti", options(nomem, nostack));
        }
    }
    if remaining == 0 {
        Ok(bytes)
    } else {
        Err(CopyError::Fault {
            copied: bytes - remaining,
        })
    }
}

pub(super) fn fixup(frame: &mut TrapFrame, cr2: u64) -> bool {
    if frame.cs & 3 != 0
        || !matches!(frame.vector, 13 | 14)
        || frame.rip != ptr::addr_of!(door_user_copy_instruction) as u64
    {
        return false;
    }
    let state = entry();
    let active = unsafe { (*state).copy_active };
    let within = unsafe { cr2 >= (*state).copy_start && cr2 < (*state).copy_end };
    if !active || (frame.vector == 14 && !within) {
        return false;
    }
    frame.rip = ptr::addr_of!(door_user_copy_fixup) as u64;
    true
}
