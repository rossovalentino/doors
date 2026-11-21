use core::arch::asm;
use door_boot_protocol::{BootInfo, MAPPINGS_KIND, Mapping};

mod context;
mod cpu;
mod layout;
mod smp;
mod timer;
mod user;
pub use context::{
    create_context, enable_interrupts, install_schedule_hooks, schedule_from_user, state_probe,
    wait_for_interrupt, yield_now,
};
pub use cpu::{
    current_cpu, deliberate_double_fault, deliberate_ud, diagnostics, disable_interrupts,
    initialize, initialize_early, seal_single_cpu_profile,
};
pub use layout::{
    Context, EntryArguments, TrapFrame, USER_CODE32_SELECTOR, USER_DATA_SELECTOR, valid_user_return,
};
pub use smp::{
    ApConfig, online_mask, prepare_secondary_cpus, send_reschedule, shootdown, start_secondary_cpus,
};
pub use timer::{
    enable_timer_service, initialize_timer, pm_millis, precise_100ns, start_timer, stop_timer,
    ticks, timer_diagnostics,
};
pub use user::{
    CopyError, MAX_USER_COPY, UserContext, copy_from_user, copy_shared_to_user, copy_to_user,
    copy_user_to_shared, create_user_context, install_user_hooks, kernel_cr3,
};

pub fn validate_mappings(info: &BootInfo<'_>) -> bool {
    let root: u64;
    let cr0: u64;
    unsafe {
        asm!("mov {}, cr3",out(reg) root,options(nomem,nostack));
        asm!("mov {}, cr0",out(reg) cr0,options(nomem,nostack));
    }
    if cr0 & (1 << 16) == 0 {
        return false;
    }
    let Some(bytes) = info.table(MAPPINGS_KIND) else {
        return false;
    };
    let Some(pool) = bytes
        .chunks_exact(40)
        .find_map(|b| Mapping::decode(b).ok().filter(|m| m.role == 5))
    else {
        return false;
    };
    if root & !4095 != pool.physical {
        return false;
    }
    for mapping in bytes.chunks_exact(40) {
        let Ok(m) = Mapping::decode(mapping) else {
            return false;
        };
        for offset in (0..m.bytes).step_by(4096) {
            let v = m.virtual_start + offset;
            let mut table = root & !4095;
            for shift in [39, 30, 21, 12] {
                if table < pool.physical
                    || table
                        .checked_add(4096)
                        .is_none_or(|end| end > pool.physical + pool.bytes)
                {
                    return false;
                }
                // SAFETY: BootInfo verified reserved identity mapping of the whole
                // table pool; every table pointer remains within that allocation.
                let entry = unsafe {
                    core::ptr::read_volatile((table + ((v >> shift) & 511) * 8) as *const u64)
                };
                if entry & 1 == 0 || (shift != 12 && entry & (1 << 7) != 0) {
                    return false;
                }
                table = entry & 0x000f_ffff_ffff_f000;
                if shift == 12
                    && (table != m.physical + offset
                        || (entry & 2 != 0) != (m.permissions == 6)
                        || (entry & (1 << 63) == 0) != (m.permissions == 5))
                {
                    return false;
                }
            }
        }
    }
    true
}

pub fn keyboard() -> Option<u8> {
    // Sole CPU polls the explicitly selected diagnostic PS/2 controller. IRQs off.
    let status = unsafe { door_boot_debug::in8(0x64) };
    if status & 1 == 0 {
        return None;
    }
    let byte = unsafe { door_boot_debug::in8(0x60) };
    if status & 0xe0 != 0 || byte & 0x80 != 0 {
        None
    } else {
        Some(byte)
    }
}
/// Restarts the machine: the chipset's reset control register (0xCF9, a
/// full reset), then the keyboard controller's reset line (0xFE to 0x64)
/// once its input buffer is empty, then a triple fault (an empty IDT and a
/// breakpoint), as firmware-independent kernels do when one way is not
/// wired.
pub fn reboot() -> ! {
    let pause = || {
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
    };
    unsafe {
        door_boot_debug::out8(0xcf9, 0x02);
        door_boot_debug::out8(0xcf9, 0x06);
    }
    pause();
    unsafe {
        for _ in 0..0x10000 {
            if door_boot_debug::in8(0x64) & 0x02 == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        door_boot_debug::out8(0x64, 0xfe);
    }
    pause();
    unsafe {
        let empty: [u16; 5] = [0; 5];
        core::arch::asm!(
            "cli",
            "lidt [{0}]",
            "int3",
            in(reg) empty.as_ptr(),
            options(nostack)
        );
    }
    door_boot_debug::halt()
}
