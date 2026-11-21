#![no_std]
//! Original bthprops.cpl (P18): the Bluetooth API where no radio is
//! present: radio and device searches find none (ERROR_NO_MORE_ITEMS).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
