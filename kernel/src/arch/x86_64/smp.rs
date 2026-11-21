use super::{cpu, timer};
use core::{
    arch::asm,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
};
use door_boot_protocol::{Platform, Startup};

core::arch::global_asm!(include_str!("ap_start.S"), options(att_syntax));
pub const SHOOTDOWN_VECTOR: u64 = 0x43;
#[derive(Clone, Copy, Default)]
pub struct ApConfig {
    pub stack_top: u64,
    pub ist_tops: [u64; 4],
}
#[derive(Clone, Copy, Debug)]
pub struct StartupResult {
    pub requested_mask: u32,
    pub online_mask: u32,
    pub missing_mask: u32,
}
static PREPARED: AtomicBool = AtomicBool::new(false);
static ENTRY: AtomicUsize = AtomicUsize::new(0);
static VECTOR: AtomicU32 = AtomicU32::new(0);
static REQUESTED: AtomicU32 = AtomicU32::new(1);
static ONLINE: AtomicU32 = AtomicU32::new(1);
static STATE: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];
static SHOOTDOWN_BUSY: AtomicBool = AtomicBool::new(false);
static SHOOTDOWN_ADDRESS: AtomicU64 = AtomicU64::new(0);
static SHOOTDOWN_GENERATION: AtomicU64 = AtomicU64::new(0);
static SHOOTDOWN_ACK: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

unsafe extern "C" {
    static door_ap_start_begin: u8;
    static door_ap_start_end: u8;
    static door_ap_start_protected: u8;
    static door_ap_start_long: u8;
}

pub fn online_mask() -> u32 {
    ONLINE.load(Ordering::Acquire)
}

/// Copy the relocatable bootstrap while the code page is still RW/NX. The caller
/// must promote only its code page to RX and finish local invalidation before start.
///
/// # Safety
/// BSP alone before AP startup. `startup` is the validated retained BootInfo pair;
/// both pages currently have exclusive identity RW/NX mappings. Each config index
/// owns permanent guarded stacks. `entry` never returns and enables IRQ only after
/// completing its per-CPU scheduler state. Index0 is BSP; other indices follow the
/// MADT processor order after skipping the BSP's APIC ID.
pub unsafe fn prepare_secondary_cpus(
    platform: &Platform,
    startup: &Startup,
    configs: &[ApConfig; 4],
    entry: extern "C" fn(usize) -> !,
) -> Result<(), &'static str> {
    cpu::disable_interrupts();
    if cpu::current_cpu() != 0 || PREPARED.swap(true, Ordering::AcqRel) {
        return Err("AP startup already prepared");
    }
    if !(1..=4).contains(&platform.processor_count)
        || startup.code_physical == 0
        || !startup.code_physical.is_multiple_of(4096)
        || startup.data_physical != startup.code_physical + 4096
        || startup.data_physical + 4096 > 0x10_0000
    {
        return Err("invalid low AP startup pair");
    }
    let root: u64;
    unsafe {
        asm!("mov {}, cr3",out(reg)root,options(nomem,nostack));
    }
    if root > u64::from(u32::MAX) || !root.is_multiple_of(4096) {
        return Err("AP CR3 must fit32bits");
    }
    let begin = core::ptr::addr_of!(door_ap_start_begin) as u64;
    let length = core::ptr::addr_of!(door_ap_start_end) as u64 - begin;
    if length == 0 || length > 4096 {
        return Err("AP bootstrap exceeds its codepage");
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            begin as *const u8,
            startup.code_physical as *mut u8,
            length as usize,
        );
        core::ptr::write_bytes(startup.data_physical as *mut u8, 0, 4096);
    }
    let data = startup.data_physical as *mut u8;
    let write64 = |offset: usize, value: u64| unsafe {
        data.add(offset).cast::<u64>().write_unaligned(value)
    };
    let write32 = |offset: usize, value: u32| unsafe {
        data.add(offset).cast::<u32>().write_unaligned(value)
    };
    let write16 = |offset: usize, value: u16| unsafe {
        data.add(offset).cast::<u16>().write_unaligned(value)
    };
    write64(0, root);
    write64(8, door_ap_kernel_entry as *const () as u64);
    write16(16, 31);
    write32(18, (startup.data_physical + 64) as u32);
    write32(
        24,
        (startup.code_physical + core::ptr::addr_of!(door_ap_start_protected) as u64 - begin)
            as u32,
    );
    write16(28, 8);
    write32(
        32,
        (startup.code_physical + core::ptr::addr_of!(door_ap_start_long) as u64 - begin) as u32,
    );
    write16(36, 24);
    for (index, descriptor) in [
        0,
        0x00cf_9a00_0000_ffff,
        0x00cf_9200_0000_ffff,
        0x00af_9a00_0000_ffff,
    ]
    .into_iter()
    .enumerate()
    {
        write64(64 + index * 8, descriptor);
    }
    for index in 0..4 {
        write32(128 + index * 64, u32::MAX);
    }
    let bsp = cpu::apic_id(0);
    let mut slot = 1;
    for processor in &platform.processors[..usize::from(platform.processor_count)] {
        if processor.apic_id == bsp {
            continue;
        }
        let config = configs[slot];
        if config.stack_top < 0xffff_8000_0000_1000 || !config.stack_top.is_multiple_of(16) {
            return Err("invalid AP bootstack");
        }
        cpu::register_cpu(slot, processor.apic_id)?;
        let base = 128 + slot * 64;
        write32(base, processor.apic_id);
        write32(base + 4, slot as u32);
        write64(base + 8, config.stack_top);
        for (index, top) in config.ist_tops.into_iter().enumerate() {
            write64(base + 16 + index * 8, top);
        }
        slot += 1;
    }
    ENTRY.store(entry as usize, Ordering::Release);
    VECTOR.store((startup.code_physical >> 12) as u32, Ordering::Release);
    REQUESTED.store((1 << slot) - 1, Ordering::Release);
    Ok(())
}

/// # Safety
/// Call once on BSP after promoting the prepared code page to RX. All mappings,
/// CPU config storage and global scheduler callbacks must remain valid permanently.
pub unsafe fn start_secondary_cpus(skip_last: bool) -> Result<StartupResult, &'static str> {
    cpu::disable_interrupts();
    if !PREPARED.load(Ordering::Acquire) || ENTRY.load(Ordering::Acquire) == 0 {
        return Err("AP startup unprepared");
    }
    let requested = REQUESTED.load(Ordering::Acquire);
    let last = 31 - requested.leading_zeros();
    for (index, state) in STATE.iter().enumerate().skip(1) {
        if requested & (1 << index) == 0 {
            continue;
        }
        if state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("AP started twice");
        }
        if !(skip_last && index == last as usize) {
            let id = cpu::apic_id(index);
            timer::send_ipi(id, 0xc500)?; // INIT level assert
            timer::wait_micros(10_000)?;
            timer::send_ipi(id, 0x8500)?; // INIT level deassert
            timer::wait_micros(200)?;
            timer::send_ipi(id, 0x600 | VECTOR.load(Ordering::Acquire))?;
            timer::wait_micros(200)?;
            if online_mask() & (1 << index) == 0 {
                timer::send_ipi(id, 0x600 | VECTOR.load(Ordering::Acquire))?;
            }
        }
        let start = timer::pm_read();
        for _ in 0..10_000_000 {
            if online_mask() & (1 << index) != 0 || state.load(Ordering::Acquire) == 3 {
                break;
            }
            if timer::elapsed(start) >= 1_789_773 {
                break;
            } //500ms
            core::hint::spin_loop();
        }
        if online_mask() & (1 << index) == 0 {
            match state.compare_exchange(1, 4, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) | Err(3 | 4) => {}
                Err(_) => return Err("AP online publication stalled"),
            }
        }
    }
    let online = online_mask();
    Ok(StartupResult {
        requested_mask: requested,
        online_mask: online,
        missing_mask: requested & !online,
    })
}

extern "C" fn door_ap_kernel_entry(index: usize, tops: *const [u64; 4]) -> ! {
    if index == 0 || index >= 4 || cpu::current_cpu() != index {
        door_boot_debug::halt();
    }
    let initialized =
        unsafe { cpu::initialize_ap(tops.read()) }.and_then(|()| timer::initialize_ap_timer());
    if initialized.is_err() {
        STATE[index].store(3, Ordering::Release);
        door_boot_debug::halt();
    }
    if STATE[index]
        .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        door_boot_debug::halt();
    }
    ONLINE.fetch_or(1 << index, Ordering::AcqRel);
    let entry: extern "C" fn(usize) -> ! =
        unsafe { core::mem::transmute(ENTRY.load(Ordering::Acquire)) };
    entry(index)
}

pub fn send_reschedule(cpu: usize) -> Result<(), &'static str> {
    if cpu >= 4 || online_mask() & (1 << cpu) == 0 {
        return Err("reschedule target is offline");
    }
    timer::send_ipi(cpu::apic_id(cpu), super::context::RESCHEDULE_VECTOR as u32)
}

/// Retire a shared4KiB translation on every observed online CPU before its frame
/// can be reused. One mapping owner serializes calls; handlers acquire no locks.
/// The caller must not hold a lock needed by a remote CPU with IRQ disabled.
pub fn shootdown(address: u64) -> Result<(), &'static str> {
    if !address.is_multiple_of(4096) {
        return Err("unaligned shootdown");
    }
    if SHOOTDOWN_BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("concurrent shootdown owners");
    }
    let result = (|| {
        let current = cpu::current_cpu();
        let mask = online_mask();
        SHOOTDOWN_ADDRESS.store(address, Ordering::Relaxed);
        let generation = SHOOTDOWN_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
        unsafe {
            asm!("invlpg [{}]",in(reg)address,options(nostack,preserves_flags));
        }
        SHOOTDOWN_ACK[current].store(generation, Ordering::Release);
        for index in 0..4 {
            if index != current && mask & (1 << index) != 0 {
                timer::send_ipi(cpu::apic_id(index), SHOOTDOWN_VECTOR as u32)?;
            }
        }
        let start = timer::pm_read();
        for _ in 0..10_000_000 {
            if (0..4).all(|index| {
                mask & (1 << index) == 0
                    || SHOOTDOWN_ACK[index].load(Ordering::Acquire) == generation
            }) {
                return Ok(());
            }
            if timer::elapsed(start) >= 715_909 {
                break;
            }
            core::hint::spin_loop();
        }
        Err("remote TLB acknowledgement timeout")
    })();
    SHOOTDOWN_BUSY.store(false, Ordering::Release);
    result
}

pub(super) fn shootdown_interrupt() {
    let generation = SHOOTDOWN_GENERATION.load(Ordering::Acquire);
    let address = SHOOTDOWN_ADDRESS.load(Ordering::Relaxed);
    unsafe {
        asm!("invlpg [{}]",in(reg)address,options(nostack,preserves_flags));
    }
    SHOOTDOWN_ACK[cpu::current_cpu()].store(generation, Ordering::Release);
    timer::eoi();
}
