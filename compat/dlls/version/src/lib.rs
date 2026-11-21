#![no_std]
//! Original version.dll. Windows 11's version.dll forwards its file version
//! functions to api-ms-win-core-version-l1-1-0 (kernelbase); Doors hosts that
//! contract in kernel32 (`compat/dlls/kernel32/src/version.rs`), so every
//! export here is a PE forwarder to kernel32, listed with the build
//! (`tools/xtask/src/windows/dlls.rs`, `forwarders`). The crate holds no
//! code of its own: programs that load version.dll by name, or delay-load
//! it as CoreCLR does, find the same functions as on Windows, and the DLL
//! imports nothing.

/// Nothing here can panic; the handler a `no_std` DLL must name.
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
