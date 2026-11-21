#![no_std]
//! Original setupapi.dll (P18): device information sets over Doors' device
//! tree as Windows programs see it, which holds no enumerable devices:
//! sets are valid and empty, so enumeration ends at once
//! (ERROR_NO_MORE_ITEMS) and device lookups find nothing.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
