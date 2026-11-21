#![no_std]
//! Original pdh.dll (P18): performance data helper queries. Queries open,
//! but Doors publishes no performance counter objects, so adding a counter
//! reports PDH_CSTATUS_NO_OBJECT.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
