#![no_std]
#![no_main]

mod boot_info;
mod elf_loader;
mod firmware;
mod handoff;
mod paging;
mod platform;
mod ramfb;
use core::{fmt::Write, panic::PanicInfo};
use door_boot_debug::{Serial, exit};
use door_boot_protocol::{
    BootInfo, Mapping, Module, STARTUP_CODE_ROLE, STARTUP_DATA_ROLE, Startup,
};
use sha2::{Digest, Sha256};

#[uefi::entry]
fn main() -> uefi::Status {
    let mut serial = unsafe { Serial::new() };
    writeln!(serial, "DOOR_LOADER v1 START").ok();
    let (mut raw, kernel_length) =
        firmware::read(uefi::cstr16!("\\Door\\kernel.elf"), 8 * 1024 * 1024);
    let mut fixture_mode = [0; 32];
    let fixture_length =
        unsafe { door_boot_debug::fw_cfg("opt/door/mode", &mut fixture_mode) }.unwrap_or(0);
    if &fixture_mode[..fixture_length] == b"bad-kernel" {
        raw.data_mut()[0] = 0;
    }
    let kernel = elf_loader::load(&raw.data()[..kernel_length]);
    let (mut initfs, initfs_length) = firmware::read(
        uefi::cstr16!("\\Door\\initfs.tar"),
        door_initfs::MAX_ARCHIVE,
    );
    if &fixture_mode[..fixture_length] == b"bad-initfs" {
        initfs.data_mut()[0] ^= 1;
    }
    let archive =
        door_initfs::Archive::parse(&initfs.data()[..initfs_length]).expect("initfs validation");
    assert!(
        archive.file("system/boot.txt").is_some(),
        "missing boot configuration"
    );
    let mut display_request = [0; 16];
    let display_length =
        unsafe { door_boot_debug::fw_cfg("opt/door/display", &mut display_request) }.unwrap_or(0);
    let (fb, ram_backed) = firmware::framebuffer(&display_request[..display_length]);
    let platform = platform::acquire();
    let startup_backing = firmware::Allocation::secondary_startup();
    let startup = Startup {
        code_physical: startup_backing.base(),
        data_physical: startup_backing.base() + 4096,
    };
    let mut blob = firmware::Allocation::new(1024 * 1024);
    let stack = firmware::Allocation::new(128 * 1024);
    let mut tables = paging::PageTables::new();
    let mut mappings = [Mapping {
        virtual_start: 0,
        physical: 0,
        bytes: 4096,
        permissions: 4,
        role: 1,
    }; 32];
    let mut count = 0;
    for mapping in kernel.mappings.into_iter().flatten() {
        mappings[count] = mapping;
        count += 1;
    }
    let transfer = handoff::enter as *const () as u64 & !4095;
    for mapping in [
        Mapping {
            virtual_start: initfs.base(),
            physical: initfs.base(),
            bytes: initfs.bytes() as u64,
            permissions: 4,
            role: 2,
        },
        Mapping {
            virtual_start: blob.base(),
            physical: blob.base(),
            bytes: blob.bytes() as u64,
            permissions: 4,
            role: 3,
        },
        Mapping {
            virtual_start: paging::STACK_BASE,
            physical: stack.base(),
            bytes: stack.bytes() as u64,
            permissions: 6,
            role: 4,
        },
        Mapping {
            virtual_start: tables.backing.base(),
            physical: tables.backing.base(),
            bytes: tables.backing.bytes() as u64,
            permissions: 6,
            role: 5,
        },
        Mapping {
            virtual_start: transfer,
            physical: transfer,
            bytes: 8192,
            permissions: 5,
            role: 6,
        },
        Mapping {
            virtual_start: fb.physical,
            physical: fb.physical,
            bytes: fb.bytes.div_ceil(4096) * 4096,
            permissions: 6,
            role: 7,
        },
        Mapping {
            virtual_start: raw.base(),
            physical: raw.base(),
            bytes: raw.bytes() as u64,
            permissions: 4,
            role: 8,
        },
    ] {
        mappings[count] = mapping;
        count += 1;
    }
    for (physical, role) in [
        (startup.code_physical, STARTUP_CODE_ROLE),
        (startup.data_physical, STARTUP_DATA_ROLE),
    ] {
        mappings[count] = Mapping {
            virtual_start: physical,
            physical,
            bytes: 4096,
            permissions: 6,
            role,
        };
        count += 1;
    }
    for mapping in &mappings[..count] {
        tables.map(*mapping, ram_backed);
    }
    let modules = [
        Module {
            role: 1,
            physical: raw.base(),
            storage_bytes: raw.bytes() as u64,
            file_bytes: kernel_length as u64,
            virtual_start: kernel.base,
            virtual_bytes: kernel.bytes,
            sha256: Sha256::digest(&raw.data()[..kernel_length]).into(),
        },
        Module {
            role: 2,
            physical: initfs.base(),
            storage_bytes: initfs.bytes() as u64,
            file_bytes: initfs_length as u64,
            virtual_start: initfs.base(),
            virtual_bytes: initfs.bytes() as u64,
            sha256: Sha256::digest(&initfs.data()[..initfs_length]).into(),
        },
    ];
    // SAFETY: CPU feature query cannot dereference memory and this profile needs NX.
    assert!(
        core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) != 0,
        "CPU NX required"
    );
    writeln!(serial, "DOOR_LOADER v1 EXIT_BOOT_SERVICES").ok();
    // All firmware-owned file/GOP/protocol guards dropped inside the helpers. No
    // allocation, protocol call or console output follows this successful exit.
    let memory = unsafe { uefi::boot::exit_boot_services(None) };
    let physical = blob.base();
    let length = boot_info::encode(
        blob.data_mut(),
        physical,
        memory,
        boot_info::Payload {
            modules: &modules,
            maps: &mappings[..count],
            fb,
            platform: &platform,
            startup,
        },
    );
    let info = BootInfo::parse(&blob.data()[..length]).expect("produced BootInfo");
    info.validate_boot().expect("produced boot contract");
    info.validate_platform()
        .expect("produced platform contract");
    info.validate_startup().expect("produced startup contract");
    let mut mode = [0; 32];
    let size = unsafe { door_boot_debug::fw_cfg("opt/door/mode", &mut mode) }.unwrap_or(0);
    if &mode[..size] == b"bad-handoff" {
        blob.data_mut()[8] = 2;
    }
    unsafe {
        handoff::enter(
            kernel.entry,
            blob.base(),
            length as u64,
            paging::STACK_BASE + stack.bytes() as u64,
            tables.backing.base(),
        )
    }
}
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut serial = unsafe { Serial::new() };
    writeln!(serial, "DOOR_LOADER v1 FAILURE: {info}").ok();
    exit(17)
}
