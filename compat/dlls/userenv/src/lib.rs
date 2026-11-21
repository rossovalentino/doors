#![no_std]
//! Original userenv.dll (P18): the user's profile directory, and
//! environment blocks copied from the calling process's environment (the
//! session's user is the only user a token can name). AppContainer SIDs
//! are not supported.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
