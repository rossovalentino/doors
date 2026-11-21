//! msiexec.exe: Windows Installer's program. Everything it does is
//! msi.dll's (`DoorMsiexecMain` parses the command line, runs the request
//! with the UI it asks for and returns Windows Installer's exit code).
#![cfg_attr(all(feature = "guest-pe", target_os = "windows"), no_std, no_main)]

#[cfg(all(feature = "guest-pe", target_os = "windows"))]
mod guest {
    // The freestanding runtime: panic handler, _fltused.
    use door_windows_abi as _;
    #[link(name = "msi", kind = "dylib")]
    unsafe extern "system" {
        // Doors' private exports take 64-bit integers at every width.
        fn DoorMsiexecMain(command_line: u64) -> u32;
    }
    #[link(name = "kernel32", kind = "dylib")]
    unsafe extern "system" {
        fn GetCommandLineW() -> *const u16;
        fn ExitProcess(code: u32) -> !;
    }
    /// The process entry (/ENTRY:msiexec_main).
    #[unsafe(no_mangle)]
    pub extern "system" fn msiexec_main() -> ! {
        // SAFETY: kernel32 and msi.dll are this program's imports.
        unsafe { ExitProcess(DoorMsiexecMain(GetCommandLineW() as usize as u64)) }
    }
}

#[cfg(not(all(feature = "guest-pe", target_os = "windows")))]
fn main() {}
