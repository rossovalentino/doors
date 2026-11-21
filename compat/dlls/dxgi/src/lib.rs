#![no_std]
//! Original dxgi.dll (P18): Doors exposes no graphics adapter to Windows
//! programs until its GPU stack (P19), so DXGI factory creation reports
//! DXGI_ERROR_UNSUPPORTED, as Windows does where DXGI is unavailable.
//! Chromium then renders in software (`--disable-gpu`).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// DXGI_ERROR_UNSUPPORTED: the call needs a graphics device Doors does not
/// expose yet.
pub const DXGI_ERROR_UNSUPPORTED: u32 = 0x887A_0004;
