//! The image host (P18 M0h): the executable Init starts for a Windows
//! program too large to capture whole (Init marks the environment with
//! `DOOR_IMAGE=1` and gives the process the program's own path and
//! subsystem). ntdll's loader lays the program out from its file and runs
//! its entry instead of this one, so this entry runs only when the host is
//! started on its own: it exits with ERROR_BAD_EXE_FORMAT.
#![cfg_attr(all(feature = "guest-pe", target_os = "windows"), no_std, no_main)]

#[cfg(all(feature = "guest-pe", target_os = "windows"))]
mod guest {
    // The freestanding runtime: panic handler, _fltused.
    use door_windows_abi as _;
    #[link(name = "kernel32", kind = "dylib")]
    unsafe extern "system" {
        fn ExitProcess(code: u32) -> !;
    }
    /// The process entry (/ENTRY:imagehost_main).
    #[unsafe(no_mangle)]
    pub extern "system" fn imagehost_main() -> ! {
        // SAFETY: kernel32 is this program's import.
        unsafe { ExitProcess(193) }
    }
}

#[cfg(not(all(feature = "guest-pe", target_os = "windows")))]
fn main() {}
