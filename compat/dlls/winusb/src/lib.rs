#![no_std]
//! Original winusb.dll (P18): Doors exposes no WinUSB devices; every
//! handle is invalid (ERROR_INVALID_HANDLE).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
