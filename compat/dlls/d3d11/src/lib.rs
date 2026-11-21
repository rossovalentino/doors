#![no_std]
//! Original d3d11.dll (P18): no Direct3D 11 device until Doors' GPU stack
//! (P19); device creation reports DXGI_ERROR_UNSUPPORTED for every driver
//! type (WARP and reference included), so callers fall back to software.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// DXGI_ERROR_UNSUPPORTED: the call needs a graphics device Doors does not
/// expose yet.
pub const DXGI_ERROR_UNSUPPORTED: u32 = 0x887A_0004;
