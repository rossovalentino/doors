#![no_std]
//! Original urlmon.dll (P18): URL monikers and security zones are not
//! available; CoInternetCreateSecurityManager reports E_NOTIMPL, which
//! callers treat as "no zone information".
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
