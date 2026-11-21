#![no_std]
//! Win32 translation over the original ntdll adapter, with no host API fallback.
//!
//! Every export takes pointer-width parameters (P17: the same sources build
//! the x64 and the x86 DLL) and widens each one explicitly at entry
//! (`door_windows_abi::widen`). An export returning a pointer or handle runs
//! its body as an immediately called closure whose 64-bit result is
//! narrowed, so the body's own `return`s keep their meaning.
#![allow(clippy::redundant_closure_call)]
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod actctx;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod ansi_files;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod application;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod console;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod delayload;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod environment;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod fileinfo;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod formats;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod jobs;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod locale;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod machine;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod mapping;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod memory;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod message;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod named_pipes;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod namespace;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod nls;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod once;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod pipes;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod process_info;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod processes;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod runtime;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod seh;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod sync;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod system;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod system_extras;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod threadpool;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod version;
