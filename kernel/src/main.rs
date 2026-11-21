#![no_std]
#![no_main]

#[path = "arch/x86_64/mod.rs"]
mod arch;
mod block_device;
mod console;
mod entropy_source;
mod execution;
mod net_device;
mod persistence_diagnostics;
mod platform_device;
mod process;
mod sound_device;
mod startup;
mod storage_diagnostics;
mod task_diagnostics;
use core::{fmt::Write, panic::PanicInfo};
use door_boot_debug::{Serial, exit, halt};
use door_boot_protocol::{BootInfo, Header};
use sha2::{Digest, Sha256};
core::arch::global_asm!(include_str!("arch/x86_64/entry.S"), options(att_syntax));

fn token() -> ([u8; 128], usize) {
    let mut text = [0; 128];
    let count = unsafe { door_boot_debug::fw_cfg("opt/door/test-token", &mut text) }.unwrap_or(0);
    (text, count)
}
fn failure(message: &str) -> ! {
    arch::disable_interrupts();
    let (t, n) = token();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_TEST v1 {} FAIL",
        core::str::from_utf8(&t[..n]).unwrap_or("invalid")
    )
    .ok();
    writeln!(serial, "DOOR_KERNEL FAILURE: {message}").ok();
    exit(17)
}

// Keep initialization's owned allocator/table construction frames out of later
// service stacks. Foundations transfers ownership into the guarded process VM;
// none of these temporary construction values remains live afterward.
#[inline(never)]
fn initialize_core(info: &BootInfo<'static>, archive: &door_initfs::Archive<'_>, mode: &[u8]) {
    if matches!(mode, b"session" | b"desktop") {
        process::initialize_session(startup::Foundations::initialize_session(info));
        return;
    }
    let mut foundations = startup::Foundations::initialize(info);
    if matches!(
        mode,
        b"trap-ud" | b"trap-double" | b"fault-readonly" | b"fault-nx" | b"fault-guard"
    ) {
        foundations.deliberate_fault(mode);
    }
    if matches!(
        mode,
        b"normal" | b"" | b"demo" | b"smp-degraded" | b"storage-write" | b"storage-reopen"
    ) || persistence_diagnostics::is_mode(mode)
        || services_mode(mode).is_some()
    {
        task_diagnostics::verify(info, &mut foundations, mode == b"smp-degraded");
        process::verify(foundations, archive);
    }
}

fn services_mode(mode: &[u8]) -> Option<u64> {
    use door_abi::platform::mode as service;
    match mode {
        b"session" => Some(service::SESSION),
        b"desktop" => Some(service::DESKTOP),
        b"services-save" => Some(service::SAVE),
        b"services-reopen" => Some(service::REOPEN),
        b"services-restart" => Some(service::RESTART),
        b"services-failure" => Some(service::FAILURE),
        b"services-demo" => Some(service::DEMO),
        b"windows-basic" => Some(service::WINDOWS),
        b"windows-base" => Some(service::WINDOWS_BASE),
        b"windows-reopen" => Some(service::WINDOWS_REOPEN),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(pointer: u64, length: u64) -> ! {
    let mut serial = unsafe { Serial::new() };
    let (t, n) = token();
    let token = core::str::from_utf8(&t[..n]).unwrap_or("invalid");
    writeln!(serial, "DOOR_TEST v1 {token} POST_EXIT_BOOT_SERVICES").ok();
    // Exception handling is installed before consuming external tables or
    // touching the allocator. Static emergency backing remains reserved.
    unsafe { arch::initialize_early() }.expect("early CPU descriptors");
    if !(64..=1024 * 1024).contains(&length) || pointer == 0 || !pointer.is_multiple_of(4096) {
        failure("boot pointer extent");
    }
    // SAFETY: Loader entry grants this exact stable reserved mapping and bounds;
    // decoding never trusts a larger header total as permission to form a slice.
    let bytes: &'static [u8] =
        unsafe { core::slice::from_raw_parts(pointer as *const u8, length as usize) };
    let Ok(header) = Header::decode(bytes) else {
        failure("boot header");
    };
    if header.self_phys != pointer {
        failure("boot identity");
    }
    let Ok(info) = BootInfo::parse(bytes) else {
        failure("boot table validation");
    };
    if info.validate_boot().is_err() || !arch::validate_mappings(&info) {
        failure("boot ownership or mappings");
    }
    let initfs = info.module(2).unwrap_or_else(|_| failure("missing initfs"));
    let kernel = info
        .module(1)
        .unwrap_or_else(|_| failure("missing kernel image"));
    let original = unsafe {
        core::slice::from_raw_parts(kernel.physical as *const u8, kernel.file_bytes as usize)
    };
    if <[u8; 32]>::from(Sha256::digest(original)) != kernel.sha256 {
        failure("kernel file identity");
    }
    let archive_bytes: &'static [u8] = unsafe {
        core::slice::from_raw_parts(
            initfs.virtual_start as *const u8,
            initfs.file_bytes as usize,
        )
    };
    if <[u8; 32]>::from(Sha256::digest(archive_bytes)) != initfs.sha256 {
        failure("initfs content identity");
    }
    let archive =
        door_initfs::Archive::parse(archive_bytes).unwrap_or_else(|_| failure("initfs parse"));
    if archive.file("system/boot.txt").is_none() {
        failure("initfs boot configuration");
    }
    let fb = info
        .framebuffer()
        .unwrap_or_else(|_| failure("framebuffer validation"));
    let mut screen = unsafe { console::Console::new(fb) };
    let mut mode = [0; 32];
    let count = unsafe { door_boot_debug::fw_cfg("opt/door/mode", &mut mode) }.unwrap_or(0);
    let desktop = &mode[..count] == b"desktop"
        || (count == 0 && archive.file("system/desktop.conf").is_some());
    let ordinary = desktop
        || &mode[..count] == b"session"
        || (count == 0 && archive.file("system/session.conf").is_some());
    let selected = if desktop {
        b"desktop".as_slice()
    } else if ordinary {
        b"session".as_slice()
    } else {
        &mode[..count]
    };
    if desktop {
        // Shown until the window server's first composed frame replaces it.
        screen.splash();
    }
    initialize_core(&info, &archive, selected);
    if let Some(service_mode) = services_mode(selected) {
        // Health-tested, conditioned seed before any protected process runs.
        entropy_source::initialize();
        platform_device::open(&info, fb);
        process::run_services(&archive, service_mode);
        platform_device::close();
        if ordinary {
            match process::session_result() {
                81 => arch::reboot(),
                80 => {}
                code => {
                    writeln!(serial, "DOOR_SESSION INIT_EXIT code={code}").ok();
                    halt();
                }
            }
            // QEMU's ACPI S5 control; development hardware profile is Q35.
            unsafe {
                core::arch::asm!("out dx, ax",in("dx")0x604u16,in("ax")0x2000u16,options(nomem,nostack));
            }
            halt();
        }
        writeln!(serial, "DOOR_TEST v1 {token} KERNEL_DIAGNOSTIC PASS").ok();
        exit(16)
    }
    if matches!(&mode[..count], b"storage-write" | b"storage-reopen") {
        storage_diagnostics::verify(&info, &archive, &mode[..count] == b"storage-reopen");
    }
    if persistence_diagnostics::is_mode(&mode[..count]) {
        persistence_diagnostics::verify(&info, &mode[..count]);
        writeln!(serial, "DOOR_TEST v1 {token} KERNEL_DIAGNOSTIC PASS").ok();
        exit(16)
    }
    screen.screen(false);
    match &mode[..count] {
        b"panic" => panic!("intentional P02 panic"),
        b"hang" => {
            writeln!(serial, "DOOR_KERNEL INTENTIONAL HANG").ok();
            halt();
        }
        b"demo" => {
            writeln!(serial, "DOOR_TEST v1 {token} KERNEL_DIAGNOSTIC PASS").ok();
            writeln!(serial, "DOOR_DEMO READY").ok();
            loop {
                if let Some(code) = arch::keyboard() {
                    match code {
                        0x23 => {
                            screen.screen(false);
                            writeln!(serial, "DOOR_DEMO HELP").ok();
                        }
                        0x20 => {
                            screen.screen(true);
                            writeln!(serial, "DOOR_DEMO DIAGNOSTICS").ok();
                        }
                        0x19 => panic!("interactive diagnostic panic"),
                        0x13 => {
                            writeln!(serial, "DOOR_DEMO REBOOT").ok();
                            arch::reboot()
                        }
                        _ => {}
                    }
                }
                core::hint::spin_loop();
            }
        }
        b"normal" | b"" | b"smp-degraded" | b"storage-write" | b"storage-reopen" => {
            writeln!(serial, "DOOR_TEST v1 {token} KERNEL_DIAGNOSTIC PASS").ok();
            exit(16)
        }
        _ => failure("unknown diagnostic mode"),
    }
}
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::disable_interrupts();
    let (t, n) = token();
    let mut serial = unsafe { Serial::new() };
    writeln!(
        serial,
        "DOOR_TEST v1 {} KERNEL_PANIC",
        core::str::from_utf8(&t[..n]).unwrap_or("invalid")
    )
    .ok();
    writeln!(serial, "DOOR_KERNEL PANIC: {info}").ok();
    console::emergency();
    let mut mode = [0; 32];
    let count = unsafe { door_boot_debug::fw_cfg("opt/door/mode", &mut mode) }.unwrap_or(0);
    if &mode[..count] == b"demo" {
        writeln!(serial, "DOOR_DEMO PANIC_SCREEN_READY").ok();
        halt();
    }
    exit(17)
}
