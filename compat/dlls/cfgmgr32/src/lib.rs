#![no_std]
//! Original cfgmgr32.dll (P18): configuration manager queries; no device
//! node can be located (CR_NO_SUCH_DEVNODE).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
