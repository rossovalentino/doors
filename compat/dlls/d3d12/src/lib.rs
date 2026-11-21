#![no_std]
//! Original d3d12.dll (P18): no Direct3D 12 device until Doors' GPU stack
//! (P19). Chromium imports D3D12CreateDevice by ordinal (101), so the
//! exports keep Windows' ordinals.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// DXGI_ERROR_UNSUPPORTED: the call needs a graphics device Doors does not
/// expose yet.
pub const DXGI_ERROR_UNSUPPORTED: u32 = 0x887A_0004;
/// The exports Windows numbers (d3d12.dll's export table) that importers
/// use by ordinal.
pub const ORDINALS: [(&str, u16); 6] = [
    ("D3D12CreateDevice", 101),
    ("D3D12GetDebugInterface", 102),
    ("D3D12EnableExperimentalFeatures", 109),
    ("D3D12GetInterface", 110),
    ("D3D12SerializeRootSignature", 115),
    ("D3D12SerializeVersionedRootSignature", 116),
];
