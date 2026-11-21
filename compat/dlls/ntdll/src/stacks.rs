//! Thread stacks as Windows makes them (P18 M0h). Each x64 thread, the
//! first one included, runs its entry on a stack reserved at the size the
//! executable's headers (SizeOfStackReserve/Commit) or CreateThread ask
//! for: the top pages committed and a PAGE_GUARD page below them, which the
//! kernel moves down as the thread touches it (`windows_stack_guard`),
//! raising STATUS_STACK_OVERFLOW at the reservation's last page. The TEB's
//! StackBase/StackLimit describe it, and VirtualQuery's AllocationBase is
//! its bottom (CoreCLR reads both). The platform's fixed start-up stack only
//! carries the switch; the reservation is released when the thread ends.
//! 32-bit threads keep the platform stack.
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const PAGE: u64 = 4096;
const GRANULARITY: u64 = 64 * 1024;
/// The largest reservation honoured (a header or request beyond is capped).
const MAX_RESERVE: u64 = 256 << 20;
/// Windows' defaults when the headers give none.
const DEFAULT_RESERVE: u64 = 1 << 20;
pub(super) const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x1_0000;
const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_GUARD: u32 = 0x100;

/// A thread running on its reserved stack.
struct Slot {
    /// Windows thread ID (0: free).
    id: AtomicU32,
    base: AtomicU64,
    top: AtomicU64,
    /// Where the platform stack resumes for ExitThread's release.
    original: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            id: AtomicU32::new(0),
            base: AtomicU64::new(0),
            top: AtomicU64::new(0),
            original: AtomicU64::new(0),
        }
    }
}
static SLOTS: [Slot; 256] = [const { Slot::new() }; 256];

/// SizeOfStackReserve and SizeOfStackCommit of the executable.
fn image_sizes() -> (u64, u64) {
    let image = super::modules::executable_base();
    if image == 0 {
        return (DEFAULT_RESERVE, PAGE);
    }
    // SAFETY: the executable's headers are mapped read-only for the
    // process's life; offsets come from its own validated header.
    unsafe {
        let nt = image
            + u64::from(
                (image as *const u8)
                    .add(0x3c)
                    .cast::<u32>()
                    .read_unaligned(),
            );
        let optional = nt + 24;
        if (optional as *const u16).read_unaligned() != 0x20b {
            return (DEFAULT_RESERVE, PAGE);
        }
        (
            ((optional + 72) as *const u64).read_unaligned(),
            ((optional + 80) as *const u64).read_unaligned(),
        )
    }
}
/// (reserve, commit) for a thread, by RtlCreateUserStack's rules: the
/// image's sizes; a CreateThread size is the commit, or with
/// STACK_SIZE_PARAM_IS_A_RESERVATION the reservation; a commit at least the
/// reservation grows the reservation to the next MiB.
pub(super) fn sizes(requested: u64, flags: u32) -> (u64, u64) {
    let (mut reserve, mut commit) = image_sizes();
    if requested != 0 {
        if flags & STACK_SIZE_PARAM_IS_A_RESERVATION != 0 {
            reserve = requested;
        } else {
            commit = requested;
        }
    }
    if commit >= reserve {
        reserve = commit.saturating_add(1 << 20) & !((1 << 20) - 1);
    }
    let reserve = reserve
        .clamp(GRANULARITY, MAX_RESERVE)
        .next_multiple_of(GRANULARITY);
    // At least a page, and room left below for the guard and one more page.
    let commit = commit
        .max(PAGE)
        .next_multiple_of(PAGE)
        .min(reserve - 2 * PAGE);
    (reserve, commit)
}
/// A reserved stack: (base, top), its top `commit` bytes committed and a
/// guard page below them.
fn allocate(reserve: u64, commit: u64) -> Option<(u64, u64)> {
    use super::memory::{DoorVirtualAlloc, DoorVirtualFree};
    let base = DoorVirtualAlloc(0, reserve, MEM_RESERVE, PAGE_READWRITE);
    if base == 0 {
        return None;
    }
    let top = base + reserve;
    let committed = DoorVirtualAlloc(top - commit, commit, MEM_COMMIT, PAGE_READWRITE) != 0
        && DoorVirtualAlloc(
            top - commit - PAGE,
            PAGE,
            MEM_COMMIT,
            PAGE_READWRITE | PAGE_GUARD,
        ) != 0;
    if !committed {
        DoorVirtualFree(base, 0, MEM_RELEASE);
        return None;
    }
    Some((base, top))
}
fn stack_pointer() -> u64 {
    let rsp: u64;
    // SAFETY: reads the register only.
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack, preserves_flags))
    };
    rsp
}
fn teb_word(offset: u64) -> u64 {
    // SAFETY: the TEB is this thread's, mapped for its life.
    unsafe { ((super::arch::teb() + offset) as *const u64).read_unaligned() }
}
fn set_teb_word(offset: u64, value: u64) {
    // SAFETY: as teb_word; NT_TIB's stack words belong to this thread.
    unsafe { ((super::arch::teb() + offset) as *mut u64).write_unaligned(value) }
}

/// `function(argument)` on a stack of `reserve` bytes (`commit` committed),
/// then back on the platform stack with the reservation released. Without
/// address space for it, on the platform stack as before.
pub(super) fn run(
    function: extern "system" fn(u64) -> u64,
    argument: u64,
    reserve: u64,
    commit: u64,
) -> u64 {
    let id = super::arch::thread_id();
    let Some((base, top)) = allocate(reserve, commit) else {
        return function(argument);
    };
    let Some(slot) = SLOTS.iter().find(|s| {
        s.id.compare_exchange(0, id.max(1), Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    }) else {
        super::memory::DoorVirtualFree(base, 0, MEM_RELEASE);
        return function(argument);
    };
    slot.base.store(base, Ordering::Relaxed);
    slot.top.store(top, Ordering::Relaxed);
    // Room for the release call below the frames this one leaves behind.
    slot.original
        .store((stack_pointer() - 4096) & !15, Ordering::Relaxed);
    let (old_base, old_limit) = (teb_word(8), teb_word(16));
    set_teb_word(8, top);
    set_teb_word(16, top - commit);
    // SAFETY: `top` ends a committed, 16-byte-aligned stack of this thread.
    let result = unsafe { call_on_stack(argument, function as usize as u64, top) };
    set_teb_word(8, old_base);
    set_teb_word(16, old_limit);
    release(slot);
    result
}
fn release(slot: &Slot) {
    let base = slot.base.swap(0, Ordering::AcqRel);
    slot.id.store(0, Ordering::Release);
    if base != 0 {
        super::memory::DoorVirtualFree(base, 0, MEM_RELEASE);
    }
}
fn own_slot() -> Option<&'static Slot> {
    let id = super::arch::thread_id().max(1);
    SLOTS.iter().find(|s| s.id.load(Ordering::Acquire) == id)
}

/// ExitThread's end: a thread still on its reserved stack goes back to the
/// platform stack, releases the reservation and exits; one already back
/// (its entry returned) has nothing left to release.
pub(super) fn exit(code: u32) -> ! {
    if let Some(slot) = own_slot() {
        let rsp = stack_pointer();
        let (base, top) = (
            slot.base.load(Ordering::Relaxed),
            slot.top.load(Ordering::Relaxed),
        );
        if rsp >= base && rsp < top {
            let original = slot.original.load(Ordering::Relaxed);
            // SAFETY: the platform stack below the frames `run` left is
            // committed and unused; nothing returns to the reserved stack.
            unsafe {
                exit_on_stack(
                    u64::from(code),
                    release_and_exit as *const () as usize as u64,
                    original,
                )
            }
        }
        release(slot);
    }
    door_sys::thread_exit(i64::from(code))
}
extern "system" fn release_and_exit(code: u64) -> ! {
    if let Some(slot) = own_slot() {
        release(slot);
    }
    door_sys::thread_exit(i64::from(code as u32))
}

/// `function(argument)` with RSP at `top` (Win64 shadow space below it);
/// the platform stack comes back on return.
#[unsafe(naked)]
unsafe extern "system" fn call_on_stack(argument: u64, function: u64, top: u64) -> u64 {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "mov rsp, r8",
        "sub rsp, 32",
        "call rdx",
        "mov rsp, rbp",
        "pop rbp",
        "ret",
    )
}
/// `function(code)` with RSP at `rsp`; it never returns.
#[unsafe(naked)]
unsafe extern "system" fn exit_on_stack(code: u64, function: u64, rsp: u64) -> ! {
    core::arch::naked_asm!("mov rsp, r8", "sub rsp, 32", "call rdx", "ud2")
}
