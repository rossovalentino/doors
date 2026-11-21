#![no_std]
//! Original mfreadwrite.dll (P18): source readers need the Media Foundation
//! platform, which Doors does not provide yet (E_NOTIMPL).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
