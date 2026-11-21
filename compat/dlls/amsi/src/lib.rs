#![no_std]
//! Original amsi.dll: the Antimalware Scan Interface as a Windows install
//! without an antimalware provider answers it. Applications (PowerShell
//! scans every script block, .NET loads of in-memory assemblies) open a
//! context and sessions, and every scan and notification is
//! AMSI_RESULT_NOT_DETECTED. Doors registers no scanning engine, so nothing
//! is sent anywhere and no content is kept.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const S_OK: u32 = 0;
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// AMSI_RESULT_NOT_DETECTED.
pub const NOT_DETECTED: u32 = 1;
/// The context's signature, as amsi.dll's own contexts begin.
pub const SIGNATURE: [u8; 4] = *b"AMSI";
