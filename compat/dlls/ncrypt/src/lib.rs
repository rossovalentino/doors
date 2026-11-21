#![no_std]
//! Original ncrypt.dll (P18): CNG key storage. Doors has no key storage
//! provider (software or TPM) yet: opening one reports
//! NTE_PROV_DLL_NOT_FOUND, and every key or provider handle is invalid.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
