#![no_std]
//! Original mf.dll (P18): Media Foundation's media sources. Doors exposes
//! no capture devices (cameras) to Windows programs: enumeration finds
//! none.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
