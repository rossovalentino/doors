use super::layout::{
    Context, DescriptorPointer, Gate, TSS_SELECTOR, TrapFrame, tss_bytes, tss_descriptor,
};
use core::{
    arch::asm,
    fmt::Write,
    ptr,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use door_boot_debug::{Serial, exit};

core::arch::global_asm!(include_str!("interrupts.S"),
    code_selector = const super::layout::CODE_SELECTOR,
    data_selector = const super::layout::DATA_SELECTOR,
    user_code_selector = const super::layout::USER_CODE_SELECTOR,
    user_code32_selector = const super::layout::USER_CODE32_SELECTOR,
    user_data_selector = const super::layout::USER_DATA_SELECTOR,
    options(att_syntax));

#[repr(C, align(16))]
struct DescriptorStorage {
    gdt: [u64; 8],
    tss: [u8; 104],
    idt: [Gate; 256],
}
static mut DESCRIPTORS: [DescriptorStorage; 4] = [const {
    DescriptorStorage {
        gdt: [0; 8],
        tss: [0; 104],
        idt: [Gate::EMPTY; 256],
    }
}; 4];

#[repr(C, align(4096))]
struct BootstrapStack([u8; 16 * 1024]);
// This reserved image backing is never freed. Owned guarded stacks replace its
// TSS pointers after the frame allocator/page-table manager become available.
static mut BOOTSTRAP_STACKS: [BootstrapStack; 4] = [const { BootstrapStack([0; 16 * 1024]) }; 4];
struct CpuState {
    initialized: AtomicBool,
    breakpoint_armed: AtomicBool,
    breakpoints: AtomicU64,
    ist_tops: [AtomicU64; 4],
    breakpoint_stack: AtomicU64,
}
static STATE: [CpuState; 4] = [const {
    CpuState {
        initialized: AtomicBool::new(false),
        breakpoint_armed: AtomicBool::new(false),
        breakpoints: AtomicU64::new(0),
        ist_tops: [const { AtomicU64::new(0) }; 4],
        breakpoint_stack: AtomicU64::new(0),
    }
}; 4];
static APIC_IDS: [AtomicU32; 4] = [const { AtomicU32::new(u32::MAX) }; 4];
static FATAL_OWNER: AtomicU32 = AtomicU32::new(u32::MAX);
static SINGLE_CPU_SEALED: AtomicBool = AtomicBool::new(false);

pub fn current_cpu() -> usize {
    if SINGLE_CPU_SEALED.load(Ordering::Acquire) {
        return 0;
    }
    lookup_cpu()
}
fn lookup_cpu() -> usize {
    let id = core::arch::x86_64::__cpuid(1).ebx >> 24;
    APIC_IDS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == id)
        .unwrap_or_else(|| door_boot_debug::halt())
}

/// Boot owner only, before admitting services. This permanently seals a known
/// one-CPU platform; AP registration/hotplug is forbidden afterward. Avoiding
/// intercepted CPUID in every lock/copy/timer lookup matters under WHPX.
pub fn seal_single_cpu_profile() -> Result<(), &'static str> {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", out(reg) flags, options(nomem));
    }
    if flags & 0x200 != 0
        || lookup_cpu() != 0
        || !STATE[0].initialized.load(Ordering::Acquire)
        || APIC_IDS[0].load(Ordering::Acquire) == u32::MAX
        || APIC_IDS[1..]
            .iter()
            .any(|id| id.load(Ordering::Acquire) != u32::MAX)
        || super::smp::online_mask() != 1
    {
        return Err("CPU indexing can seal only an initialized BSP-only platform");
    }
    SINGLE_CPU_SEALED.store(true, Ordering::Release);
    Ok(())
}

pub(super) fn register_cpu(index: usize, apic_id: u32) -> Result<(), &'static str> {
    if index != 0 && SINGLE_CPU_SEALED.load(Ordering::Acquire) {
        return Err("single CPU profile is sealed; AP admission is forbidden");
    }
    if index >= 4 || apic_id > 255 {
        return Err("unsupported CPU slot/APIC ID");
    }
    for (other, id) in APIC_IDS.iter().enumerate() {
        if other != index && id.load(Ordering::Acquire) == apic_id {
            return Err("duplicate APIC ID");
        }
    }
    let old = APIC_IDS[index].load(Ordering::Acquire);
    if old != u32::MAX && old != apic_id {
        return Err("CPU slot already assigned");
    }
    APIC_IDS[index].store(apic_id, Ordering::Release);
    Ok(())
}

pub(super) fn apic_id(index: usize) -> u32 {
    APIC_IDS[index].load(Ordering::Acquire)
}

fn descriptor_storage() -> *mut DescriptorStorage {
    // Slots are registered before a CPU starts and each CPU exclusively owns its
    // descriptor region. No mutable Rust references are shared between CPUs.
    unsafe {
        ptr::addr_of_mut!(DESCRIPTORS)
            .cast::<DescriptorStorage>()
            .add(current_cpu())
    }
}

unsafe extern "C" {
    static door_vector_addresses: [u64; 256];
    fn door_load_segments();
    fn door_breakpoint_probe() -> u64;
}

pub fn disable_interrupts() {
    // CPU0 privileged kernel operation; used before serial and fatal paths.
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
}

/// Install kernel-owned descriptors before physical allocation/page-table edits.
///
/// # Safety
/// Exactly once on the boot processor at CPL0, with interrupts disabled. The
/// kernel image, entry stack and static storage must be mapped and retained.
pub unsafe fn initialize_early() -> Result<(), &'static str> {
    register_cpu(0, core::arch::x86_64::__cpuid(1).ebx >> 24)?;
    let base = ptr::addr_of_mut!(BOOTSTRAP_STACKS) as u64;
    let tops = core::array::from_fn(|index| base + ((index + 1) * 16 * 1024) as u64);
    unsafe { initialize_cpu(tops) }
}

/// # Safety
/// The current registered AP exclusively owns its valid guarded stack tops and
/// high kernel mapping. No interrupt has been enabled before this call.
pub(super) unsafe fn initialize_ap(tops: [u64; 4]) -> Result<(), &'static str> {
    validate_tops(tops)?;
    unsafe { initialize_cpu(tops) }
}

unsafe fn initialize_cpu(tops: [u64; 4]) -> Result<(), &'static str> {
    disable_interrupts();
    let state = &STATE[current_cpu()];
    let storage = descriptor_storage();
    let features = core::arch::x86_64::__cpuid(1);
    if features.edx & ((1 << 24) | (1 << 25) | (1 << 26)) != ((1 << 24) | (1 << 25) | (1 << 26)) {
        return Err("FXSAVE/SSE2 unsupported");
    }
    if state.initialized.swap(true, Ordering::AcqRel) {
        return Err("CPU descriptors already initialized");
    }
    // FXSAVE is the complete enabled extended-state contract in P03. Disabling
    // OSXSAVE prevents AVX/XSAVE-only state from being silently exposed.
    unsafe {
        let mut cr0: u64;
        let mut cr4: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
        asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr0 = (cr0 & !((1 << 2) | (1 << 3))) | (1 << 1) | (1 << 5);
        // User WRGSBASE must not forge the trusted high-half GS domain. The
        // enabled state remains FXSAVE-only; no PKRU/CET state is exposed.
        cr4 = (cr4 & !((1 << 16) | (1 << 18) | (1 << 22) | (1 << 23))) | (1 << 9) | (1 << 10);
        asm!("mov cr0, {}", in(reg) cr0, options(nomem, nostack));
        asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
        let mxcsr = 0x1f80u32;
        asm!("fninit", "ldmxcsr [{}]", in(reg) &mxcsr, options(nostack));
    }
    let tss_base = unsafe { ptr::addr_of_mut!((*storage).tss) as u64 };
    let descriptor = tss_descriptor(tss_base);
    // No live descriptor points at this memory until LGDT/LTR/LIDT below.
    unsafe {
        ptr::addr_of_mut!((*storage).gdt).write([
            0,
            0x00af_9a00_0000_ffff,
            0x00cf_9200_0000_ffff,
            descriptor[0],
            descriptor[1],
            0x00cf_f200_0000_ffff,
            0x00af_fa00_0000_ffff,
            super::layout::USER_CODE32_DESCRIPTOR,
        ]);
        ptr::addr_of_mut!((*storage).tss).write(tss_bytes(tops));
        for (slot, top) in state.ist_tops.iter().zip(tops) {
            slot.store(top, Ordering::Release);
        }
        for (vector, &address) in door_vector_addresses.iter().enumerate() {
            let ist = match vector {
                8 => 2,
                2 => 3,
                18 => 4,
                0..=31 => 1,
                _ => 0,
            };
            ptr::addr_of_mut!((*storage).idt)
                .cast::<Gate>()
                .add(vector)
                .write(if vector == 3 {
                    // int3 from user mode (debug breaks, CHECK failures).
                    Gate::user_interrupt(address, ist)
                } else {
                    Gate::interrupt(address, ist)
                });
        }
        let gdtr = DescriptorPointer {
            limit: (core::mem::size_of::<[u64; 8]>() - 1) as u16,
            base: ptr::addr_of!((*storage).gdt) as u64,
        };
        let idtr = DescriptorPointer {
            limit: (core::mem::size_of::<[Gate; 256]>() - 1) as u16,
            base: ptr::addr_of!((*storage).idt) as u64,
        };
        asm!("lgdt [{}]", in(reg) &gdtr, options(nostack, preserves_flags));
        door_load_segments();
        super::user::initialize()?;
        asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
        asm!("lidt [{}]", in(reg) &idtr, options(nostack, preserves_flags));
    }
    Ok(())
}

pub(super) unsafe fn set_kernel_stack(top: u64) {
    let storage = descriptor_storage();
    unsafe {
        ptr::addr_of_mut!((*storage).tss)
            .cast::<u8>()
            .add(4)
            .cast::<u64>()
            .write_unaligned(top);
    }
}

/// Replace bootstrap IST storage with four independently guarded stacks.
///
/// # Safety
/// CPU0 only, outside every interrupt handler. Each supplied top ends a distinct
/// permanent RW/NX mapping of at least32KiB with absent lower/upper guard pages.
/// The caller owns and retains all frames and mappings for this CPU's lifetime.
pub unsafe fn initialize(stack_tops: [u64; 4]) -> Result<(), &'static str> {
    disable_interrupts();
    let state = &STATE[current_cpu()];
    if !state.initialized.load(Ordering::Acquire) {
        return Err("CPU early descriptors absent");
    }
    validate_tops(stack_tops)?;
    let storage = descriptor_storage();
    for (index, top) in stack_tops.into_iter().enumerate() {
        unsafe {
            ptr::addr_of_mut!((*storage).tss)
                .cast::<u8>()
                .add(36 + index * 8)
                .cast::<u64>()
                .write_unaligned(top);
        }
        state.ist_tops[index].store(top, Ordering::Release);
    }
    Ok(())
}

fn validate_tops(stack_tops: [u64; 4]) -> Result<(), &'static str> {
    for (index, &top) in stack_tops.iter().enumerate() {
        if top < 0xffff_8000_0000_8000
            || !top.is_multiple_of(4096)
            || stack_tops[..index]
                .iter()
                .any(|other| other.abs_diff(top) < 40 * 1024)
        {
            return Err("invalid independent guarded IST stacks");
        }
    }
    Ok(())
}

/// Exercise a recoverable hardware breakpoint plus GPR/SSE state preservation.
pub fn diagnostics() -> Result<(), &'static str> {
    disable_interrupts();
    let state = &STATE[current_cpu()];
    if !state.initialized.load(Ordering::Acquire) {
        return Err("CPU descriptors absent");
    }
    let before = state.breakpoints.load(Ordering::Acquire);
    state.breakpoint_armed.store(true, Ordering::Release);
    let preserved = unsafe { door_breakpoint_probe() } == 1;
    if !preserved
        || state.breakpoint_armed.load(Ordering::Acquire)
        || state.breakpoints.load(Ordering::Acquire) != before + 1
    {
        return Err("breakpoint context restoration");
    }
    let frame_address = state.breakpoint_stack.load(Ordering::Acquire);
    if frame_address.checked_add(core::mem::size_of::<TrapFrame>() as u64)
        != Some(state.ist_tops[0].load(Ordering::Acquire))
    {
        return Err("breakpoint did not switch to its owned IST");
    }
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P03 CPU PASS breakpoint=3 gpr=preserved sse=preserved ist=4"
    )
    .ok();
    Ok(())
}

pub fn deliberate_ud() -> ! {
    unsafe {
        asm!("ud2", options(noreturn));
    }
}

/// Force an exception-delivery fault while retaining the separate double-fault
/// IST. The hardware must enter vector8 on IST2, not reset or reuse IST1.
///
/// # Safety
/// CPU0 at CPL0 after guarded IST initialization. The whole4KiB page immediately
/// below `unmapped_stack_top` must be absent; the independent IST2 remains mapped.
/// This development fixture irreversibly corrupts IST1 and cannot return.
pub unsafe fn deliberate_double_fault(unmapped_stack_top: u64) -> ! {
    disable_interrupts();
    let storage = descriptor_storage();
    unsafe {
        ptr::addr_of_mut!((*storage).tss)
            .cast::<u8>()
            .add(36)
            .cast::<u64>()
            .write_unaligned(unmapped_stack_top);
        asm!("ud2", options(noreturn));
    }
}

#[unsafe(no_mangle)]
extern "C" fn door_trap_dispatch(context: *mut Context, cr2: u64) -> *mut Context {
    let selected = dispatch(context, cr2);
    if super::user::valid_return(selected) {
        return selected;
    }
    // A malformed user SYSCALL return is a process fault. An already-selected
    // corrupt context while the scheduler holds its lock is a kernel invariant
    // failure, not another scheduling attempt against that same lock.
    if super::context::pending() {
        fatal(unsafe { &*((*selected).frame as *const TrapFrame) }, cr2);
    }
    let replacement = super::user::fault(selected, super::user::INVALID_RETURN_VECTOR, 0, 0);
    if !super::user::valid_return(replacement) {
        fatal(unsafe { &*((*replacement).frame as *const TrapFrame) }, cr2);
    }
    replacement
}

fn dispatch(context: *mut Context, cr2: u64) -> *mut Context {
    let frame = unsafe { &mut *((*context).frame as *mut TrapFrame) };
    if super::user::fixup(frame, cr2) {
        return context;
    }
    if frame.cs & 3 == 3 {
        if !super::user::valid_return(context) {
            return super::user::fault(context, super::user::INVALID_RETURN_VECTOR, 0, 0);
        }
        if frame.vector == super::user::SYSCALL_VECTOR {
            return super::user::syscall(context);
        }
        if frame.vector < 32 {
            return super::user::fault(context, frame.vector, frame.error, cr2);
        }
    }
    let frame = unsafe { &*((*context).frame as *const TrapFrame) };
    let state = &STATE[current_cpu()];
    // Every entry is an interrupt gate: no ordinary interrupt can nest here.
    // No heap, locks, frame allocator or filesystem belongs in this handler.
    match frame.vector {
        3 if state.breakpoint_armed.swap(false, Ordering::AcqRel) => {
            state
                .breakpoint_stack
                .store(frame as *const TrapFrame as u64, Ordering::Release);
            state.breakpoints.fetch_add(1, Ordering::Release);
        }
        super::timer::TIMER_VECTOR if super::timer::interrupt() => {
            return super::context::schedule(context, 0);
        }
        super::context::YIELD_VECTOR => return super::context::schedule(context, 1),
        super::context::RESCHEDULE_VECTOR => {
            super::timer::eoi();
            super::timer::apply_timer_service();
            return super::context::schedule(context, 2);
        }
        super::smp::SHOOTDOWN_VECTOR => super::smp::shootdown_interrupt(),
        super::timer::SPURIOUS_VECTOR => {} // A spurious interrupt has no ISR bit/EOI.
        _ => fatal(frame, cr2),
    }
    context
}

pub(super) fn fatal(frame: &TrapFrame, cr2: u64) -> ! {
    disable_interrupts();
    let cpu = current_cpu() as u32;
    let owner = FATAL_OWNER.compare_exchange(u32::MAX, cpu, Ordering::AcqRel, Ordering::Acquire);
    if let Err(other) = owner {
        if other != cpu {
            door_boot_debug::halt();
        }
        // Never format or touch a display on recursive failure. The original
        // failure is already fatal; bounded UART output and debug exit only.
        let mut serial = unsafe { Serial::new() };
        serial.write_str("DOOR_TRAP RECURSIVE_FATAL\n").ok();
        exit(17);
    }
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_TRAP vector={} error={:#x} rip={:#x} cr2={:#x}",
        frame.vector, frame.error, frame.rip, cr2
    )
    .ok();
    writeln!(
        serial,
        "DOOR_TRAP context rsp={:#x} cs={:#x} flags={:#x} rax={:#x} rbx={:#x}",
        frame.rsp, frame.cs, frame.rflags, frame.rax, frame.rbx
    )
    .ok();
    let (token, count) = crate::token();
    writeln!(
        serial,
        "DOOR_TEST v1 {} KERNEL_PANIC",
        core::str::from_utf8(&token[..count]).unwrap_or("invalid")
    )
    .ok();
    crate::console::emergency();
    exit(17)
}
