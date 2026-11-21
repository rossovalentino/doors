#![no_std]
//! Original wevtapi.dll (P18): the Windows Event Log query API. Doors'
//! event log is read through advapi32's classic API; the channel API
//! finds no channels (ERROR_EVT_CHANNEL_NOT_FOUND).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
