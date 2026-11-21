#![no_std]
//! Original dhcpcsvc.dll (P18): door.network runs the DHCP client and
//! exposes no option queries to programs; DhcpRequestParams reports
//! ERROR_NOT_SUPPORTED (Chromium's WPAD then uses DNS).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
