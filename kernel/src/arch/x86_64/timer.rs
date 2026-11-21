//! BSP xAPIC timer, calibrated from the validated ACPI fixed PM timer. No timer
//! frequency or APIC physical address is inferred from a QEMU default.
use super::{
    cpu::{current_cpu, disable_interrupts},
    layout::pm_elapsed,
};
use core::{
    arch::asm,
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use door_boot_debug::Serial;
use door_boot_protocol::Platform;

pub const TIMER_VECTOR: u64 = 0x40;
pub const SPURIOUS_VECTOR: u64 = 0xff;
const MASKED: u32 = 1 << 16;
const PM_TICKS_10MS: u32 = 35_795;
const PM_TICKS_200MS: u32 = 715_909;
const POLL_LIMIT: usize = 10_000_000;
static LAPIC: AtomicU64 = AtomicU64::new(0);
static PM_PORT: AtomicU32 = AtomicU32::new(0);
static PM_BITS: AtomicU32 = AtomicU32::new(0);
static APIC_PHYSICAL: AtomicU64 = AtomicU64::new(0);
//0 preserves the historical explicitly managed P03/P04 timer lifecycle.
//1/2 request enabled/disabled service on the next reschedule IPI per CPU.
static TIMER_SERVICE: AtomicU32 = AtomicU32::new(0);
struct TimerState {
    count: AtomicU32,
    ticks: AtomicU64,
    running: AtomicBool,
}
static TIMERS: [TimerState; 4] = [const {
    TimerState {
        count: AtomicU32::new(0),
        ticks: AtomicU64::new(0),
        running: AtomicBool::new(false),
    }
}; 4];

pub(super) fn read_register(offset: u64) -> u32 {
    let base = LAPIC.load(Ordering::Acquire);
    // The sole initialization owner publishes only the checked uncached mapping.
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}
pub(super) fn write_register(offset: u64, value: u32) {
    let base = LAPIC.load(Ordering::Acquire);
    unsafe {
        core::ptr::write_volatile((base + offset) as *mut u32, value);
    }
    // Read-back serializes an MMIO APIC write before subsequent register use.
    let _ = read_register(0x20);
}
pub(super) fn pm_read() -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", in("dx") PM_PORT.load(Ordering::Relaxed) as u16,
        out("eax") value, options(nomem, nostack, preserves_flags));
    }
    value
}
pub(super) fn elapsed(start: u32) -> u32 {
    pm_elapsed(start, pm_read(), PM_BITS.load(Ordering::Relaxed) as u8)
}
/// PM timer ticks counted by `pm_millis` and its last reading.
static PM_TOTAL: AtomicU64 = AtomicU64::new(0);
static PM_LAST: AtomicU32 = AtomicU32::new(0);
/// Milliseconds from the ACPI PM timer (3.579545 MHz), which runs while
/// interrupts are disabled, unlike the timer tick count. Device drivers that
/// poll with interrupts off measure their timeouts with it. Callers must
/// read it at least every 4.6 s while measuring (a 24-bit counter wraps
/// then) and are serialized (the platform device lock, or boot). Before the
/// timer is initialized it falls back to the tick count.
pub fn pm_millis() -> u64 {
    if PM_PORT.load(Ordering::Relaxed) == 0 {
        return ticks(0).saturating_mul(10);
    }
    let now = pm_read();
    let delta = pm_elapsed(
        PM_LAST.swap(now, Ordering::Relaxed),
        now,
        PM_BITS.load(Ordering::Relaxed) as u8,
    );
    let total = PM_TOTAL.fetch_add(u64::from(delta), Ordering::Relaxed) + u64::from(delta);
    total.saturating_mul(1000) / 3_579_545
}

/// Invariant-TSC frequency in Hz (0: the TSC is not invariant) and its
/// reading at calibration.
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static TSC_BASE: AtomicU64 = AtomicU64::new(0);
fn rdtsc() -> u64 {
    let (low, high): (u32, u32);
    unsafe { asm!("lfence", "rdtsc", out("eax") low, out("edx") high, options(nomem, nostack)) };
    (u64::from(high) << 32) | u64::from(low)
}
/// Measure the invariant TSC against 10 ms of ACPI PM time (bare-metal
/// CPUs and KVM have one; without it the precise clock is the tick).
fn calibrate_tsc() {
    let extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if extended < 0x8000_0007 || core::arch::x86_64::__cpuid(0x8000_0007).edx & (1 << 8) == 0 {
        return;
    }
    let bits = PM_BITS.load(Ordering::Relaxed) as u8;
    let start = pm_read();
    let first = rdtsc();
    let mut spins = 0;
    while pm_elapsed(start, pm_read(), bits) < PM_TICKS_10MS {
        spins += 1;
        if spins > POLL_LIMIT {
            return;
        }
    }
    let pm = pm_elapsed(start, pm_read(), bits);
    let cycles = rdtsc().wrapping_sub(first);
    let hz = (u128::from(cycles) * 3_579_545 / u128::from(pm.max(1))) as u64;
    TSC_BASE.store(first, Ordering::Relaxed);
    TSC_HZ.store(hz, Ordering::Release);
}
/// Time since boot in 100 ns units (QueryPerformanceCounter at 10 MHz, as
/// Windows reports it): the invariant TSC when there is one, else the tick.
pub fn precise_100ns() -> u64 {
    let hz = TSC_HZ.load(Ordering::Acquire);
    if hz == 0 {
        return ticks(0).saturating_mul(100_000);
    }
    let elapsed = rdtsc().wrapping_sub(TSC_BASE.load(Ordering::Relaxed));
    (u128::from(elapsed) * 10_000_000 / u128::from(hz)) as u64
}

/// Discover/initialize one xAPIC and calibrate its timer against ACPI PM time.
///
/// # Safety
/// The caller is CPU0 at CPL0, after installing owned GDT/TSS/IDT. `platform`
/// must be the validated copied BootInfo platform summary. `lapic_virtual`
/// maps its physical LAPIC page RW/NX with uncached PCD/PWT attributes, without
/// a conflicting RAM alias, and remains reserved for this CPU's lifetime.
pub unsafe fn initialize_timer(
    lapic_virtual: u64,
    platform: &Platform,
) -> Result<(), &'static str> {
    disable_interrupts();
    if LAPIC.load(Ordering::Acquire) != 0 {
        return Err("LAPIC already initialized");
    }
    if lapic_virtual < 0xffff_8000_0000_0000
        || !lapic_virtual.is_multiple_of(4096)
        || platform.lapic_physical == 0
        || !platform.lapic_physical.is_multiple_of(4096)
        || platform.pm_timer_port == 0
        || !matches!(platform.pm_timer_bits, 24 | 32)
    {
        return Err("LAPIC mapping or ACPI PM timer absent");
    }
    let cpu = core::arch::x86_64::__cpuid(1);
    if cpu.edx & (1 << 9) == 0 {
        return Err("CPU has no APIC");
    }
    if !(1..=4).contains(&platform.processor_count)
        || !platform.processors[..usize::from(platform.processor_count)]
            .iter()
            .any(|processor| processor.apic_id == cpu.ebx >> 24)
    {
        return Err("unsupported CPU discovery or missing BSP");
    }
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") 0x1bu32, out("eax") lo, out("edx") hi,
        options(nomem, nostack));
    }
    let apic_base = (u64::from(hi) << 32) | u64::from(lo);
    if apic_base & (1 << 10) != 0
        || apic_base & (1 << 8) == 0
        || apic_base & 0x000f_ffff_ffff_f000 != platform.lapic_physical
    {
        return Err("APIC mode/base/BSP mismatch");
    }
    if apic_base & (1 << 11) == 0 {
        unsafe {
            asm!("wrmsr", in("ecx") 0x1bu32, in("eax") lo | (1 << 11), in("edx") hi,
            options(nomem, nostack));
        }
    }
    LAPIC.store(lapic_virtual, Ordering::Release);
    APIC_PHYSICAL.store(platform.lapic_physical, Ordering::Release);
    if read_register(0x20) >> 24 != cpu.ebx >> 24 {
        return Err("LAPIC identity mismatch");
    }
    if platform.pcat_compat {
        // MADT declares the legacy dual PIC. Stop all routed legacy IRQs before
        // enabling local timer interrupts; device routing belongs to later I/O.
        unsafe {
            door_boot_debug::out8(0x21, 0xff);
            door_boot_debug::out8(0xa1, 0xff);
        }
    }
    PM_PORT.store(u32::from(platform.pm_timer_port), Ordering::Relaxed);
    PM_BITS.store(u32::from(platform.pm_timer_bits), Ordering::Relaxed);
    initialize_local()?;
    calibrate_tsc();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P03 TIMER calibrated source=ACPI_PM port={:#x} bits={} lapic={:#x} bsp={} count={}",
        platform.pm_timer_port,
        platform.pm_timer_bits,
        platform.lapic_physical,
        cpu.ebx >> 24,
        TIMERS[current_cpu()].count.load(Ordering::Acquire)
    )
    .ok();
    Ok(())
}

pub(super) fn initialize_ap_timer() -> Result<(), &'static str> {
    disable_interrupts();
    if LAPIC.load(Ordering::Acquire) == 0 {
        return Err("BSP LAPIC mapping missing");
    }
    let (pat_lo, pat_hi): (u32, u32);
    unsafe {
        asm!("rdmsr",in("ecx")0x277u32,out("eax")pat_lo,out("edx")pat_hi,options(nomem,nostack));
    }
    let pat = (u64::from(pat_hi) << 32) | u64::from(pat_lo);
    if pat & 255 != 6 || (pat >> 24) & 255 != 0 {
        return Err("AP PAT memory types differ from mapper contract");
    }
    let (lo, hi): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") 0x1bu32, out("eax") lo, out("edx") hi, options(nomem,nostack));
    }
    let base = (u64::from(hi) << 32) | u64::from(lo);
    if base & (1 << 10) != 0
        || base & 0x000f_ffff_ffff_f000 != APIC_PHYSICAL.load(Ordering::Acquire)
    {
        return Err("AP LAPIC base/mode mismatch");
    }
    if base & (1 << 11) == 0 {
        unsafe {
            asm!("wrmsr",in("ecx") 0x1bu32,in("eax") lo|(1<<11),in("edx") hi,options(nomem,nostack));
        }
    }
    if read_register(0x20) >> 24 != super::cpu::apic_id(current_cpu()) {
        return Err("AP LAPIC identity mismatch");
    }
    initialize_local()
}

fn initialize_local() -> Result<(), &'static str> {
    let max_lvt = (read_register(0x30) >> 16) & 255;
    if max_lvt < 3 {
        return Err("incomplete xAPIC LVT");
    }
    write_register(0x320, MASKED | TIMER_VECTOR as u32);
    write_register(0x350, MASKED); // LINT0 / firmware ExtINT
    write_register(0x360, MASKED); // LINT1; NMI routes are not exposed in P03.
    write_register(0x370, MASKED); // APIC error, retained in ESR
    if max_lvt >= 4 {
        write_register(0x340, MASKED);
    }
    if max_lvt >= 5 {
        write_register(0x330, MASKED);
    }
    if max_lvt >= 6 {
        write_register(0x2f0, MASKED);
    }
    write_register(0x80, 0); // Task priority: permit the owned timer vector.
    write_register(0xf0, 0x100 | SPURIOUS_VECTOR as u32);
    write_register(0x3e0, 3); // Intel SDM11.5.4: divide by16.
    let start = pm_read();
    write_register(0x380, u32::MAX);
    let mut finished = false;
    for _ in 0..POLL_LIMIT {
        if elapsed(start) >= PM_TICKS_10MS {
            finished = true;
            break;
        }
        core::hint::spin_loop();
    }
    let current = read_register(0x390);
    let pm_ticks = elapsed(start);
    write_register(0x380, 0);
    if !finished || current == 0 || current == u32::MAX || pm_ticks > PM_TICKS_200MS {
        return Err("LAPIC/ACPI calibration timeout");
    }
    let elapsed_count = u32::MAX - current;
    // Normalize the measured interval; port-read/TCG latency may exceed10ms.
    let count = u64::from(elapsed_count) * u64::from(PM_TICKS_10MS) / u64::from(pm_ticks);
    if count < 100 || count > u64::from(u32::MAX) {
        return Err("invalid LAPIC timer rate");
    }
    TIMERS[current_cpu()]
        .count
        .store(count as u32, Ordering::Release);
    Ok(())
}

pub(super) fn interrupt() -> bool {
    if LAPIC.load(Ordering::Acquire) == 0 {
        return false;
    }
    let timer = &TIMERS[current_cpu()];
    if timer.running.load(Ordering::Acquire) {
        timer.ticks.fetch_add(1, Ordering::Release);
        if current_cpu() == 0 {
            crate::platform_device::capture_display_input(timer.ticks.load(Ordering::Acquire));
        }
    }
    eoi(); // A stopped timer may still have one interrupt pending in IRR.
    true
}

pub(super) fn eoi() {
    write_register(0xb0, 0);
}

pub fn ticks(cpu: usize) -> u64 {
    TIMERS[cpu].ticks.load(Ordering::Acquire)
}

pub fn start_timer() -> Result<(), &'static str> {
    let timer = &TIMERS[current_cpu()];
    let count = timer.count.load(Ordering::Acquire);
    if count == 0 {
        return Err("uncalibrated CPU timer");
    }
    timer.running.store(true, Ordering::Release);
    write_register(0x320, (1 << 17) | TIMER_VECTOR as u32);
    write_register(0x380, count);
    Ok(())
}

pub fn stop_timer() {
    write_register(0x320, MASKED | TIMER_VECTOR as u32);
    write_register(0x380, 0);
    TIMERS[current_cpu()]
        .running
        .store(false, Ordering::Release);
}

/// Publish the desired timer service state, then send reschedule IPIs to every
/// online CPU (including the caller) to apply it without remote MMIO writes.
pub fn enable_timer_service(enabled: bool) {
    TIMER_SERVICE.store(if enabled { 1 } else { 2 }, Ordering::Release);
}

pub(super) fn apply_timer_service() {
    let running = TIMERS[current_cpu()].running.load(Ordering::Acquire);
    match TIMER_SERVICE.load(Ordering::Acquire) {
        1 if !running => start_timer().expect("online CPU timer calibration"),
        2 if running => stop_timer(),
        _ => {}
    }
}

pub(super) fn wait_micros(micros: u32) -> Result<(), &'static str> {
    let desired = (u64::from(micros) * 3_579_545 / 1_000_000).max(1) as u32;
    let start = pm_read();
    for _ in 0..POLL_LIMIT {
        if elapsed(start) >= desired {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err("ACPI clock wait exceeded iteration bound")
}

pub(super) fn send_ipi(apic_id: u32, command: u32) -> Result<(), &'static str> {
    let flags: u64;
    unsafe {
        asm!("pushfq", "pop {}", "cli", out(reg) flags);
    }
    let result = send_ipi_locked(apic_id, command);
    if flags & (1 << 9) != 0 {
        unsafe {
            asm!("sti", options(nomem, nostack));
        }
    }
    result
}

fn send_ipi_locked(apic_id: u32, command: u32) -> Result<(), &'static str> {
    let start = pm_read();
    for _ in 0..POLL_LIMIT {
        if read_register(0x300) & (1 << 12) == 0 {
            write_register(0x310, apic_id << 24);
            write_register(0x300, command);
            return Ok(());
        }
        if elapsed(start) >= PM_TICKS_200MS {
            break;
        }
        core::hint::spin_loop();
    }
    Err("APIC ICR delivery timeout")
}

/// Prove real periodic delivery. Returns with IRQs disabled and the timer stopped.
/// P04 will introduce the scheduler's lifecycle instead of leaving an idle IRQ on.
pub fn timer_diagnostics() -> Result<(), &'static str> {
    disable_interrupts();
    let timer = &TIMERS[current_cpu()];
    let count = timer.count.load(Ordering::Acquire);
    if count == 0 {
        return Err("LAPIC timer uncalibrated");
    }
    let start_ticks = timer.ticks.load(Ordering::Acquire);
    let start_time = pm_read();
    timer.running.store(true, Ordering::Release);
    write_register(0x320, (1 << 17) | TIMER_VECTOR as u32);
    write_register(0x380, count);
    unsafe {
        asm!("sti", options(nomem, nostack));
    }
    for _ in 0..POLL_LIMIT {
        if timer
            .ticks
            .load(Ordering::Acquire)
            .wrapping_sub(start_ticks)
            >= 3
            || elapsed(start_time) >= PM_TICKS_200MS
        {
            break;
        }
        core::hint::spin_loop();
    }
    disable_interrupts();
    write_register(0x320, MASKED | TIMER_VECTOR as u32);
    write_register(0x380, 0);
    timer.running.store(false, Ordering::Release);
    let delivered = timer
        .ticks
        .load(Ordering::Acquire)
        .wrapping_sub(start_ticks);
    if delivered < 3 {
        return Err("LAPIC interrupt delivery timeout");
    }
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_P03 TIMER PASS ticks={delivered} source=ACPI_PM period_ms=10"
    )
    .ok();
    Ok(())
}
