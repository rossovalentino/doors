#![no_std]
//! Original cryptui.dll (P18): the certificate viewer is not available;
//! CryptUIDlgViewCertificate reports ERROR_NOT_SUPPORTED without showing a
//! dialog.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
