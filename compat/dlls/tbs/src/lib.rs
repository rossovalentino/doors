#![no_std]
//! Original tbs.dll (P18): TPM Base Services where no TPM is present:
//! device queries report TBS_E_TPM_NOT_FOUND, as Windows does on such a
//! computer.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
