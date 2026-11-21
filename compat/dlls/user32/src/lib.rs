#![no_std]
//! Original user32: window classes and HWNDs, per-thread message queues with
//! posted, sent (nested and cross-thread, with timeouts) and input messages,
//! window procedures run on their owning thread with no lock held, painting
//! into back buffers committed to the Door window service, menus,
//! accelerators, dialogs, message boxes and the standard controls. The
//! semantics live in the host-tested `door-windows-gui` core; exported PE
//! entry points exist only in guest builds.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports and window procedures take pointer-width parameters, widened at
//! entry; structures that hold pointers or handles are read and written by
//! field through `door_windows_abi::layout`, at the build's width.
#![allow(clippy::redundant_closure_call)]
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
extern crate alloc;
