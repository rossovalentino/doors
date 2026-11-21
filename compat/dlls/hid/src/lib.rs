#![no_std]
//! Original hid.dll (P18): Doors exposes no HID devices to Windows
//! programs, so no handle a program holds is a HID device: attribute and
//! string queries fail and the parsing functions report
//! HIDP_STATUS_INVALID_PREPARSED_DATA.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
