#![no_std]
//! Original dcomp.dll (P18): no DirectComposition device until Doors' GPU
//! stack (P19); Chromium composites its windows in software meanwhile.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// DXGI_ERROR_UNSUPPORTED: the call needs a graphics device Doors does not
/// expose yet.
pub const DXGI_ERROR_UNSUPPORTED: u32 = 0x887A_0004;
