#![no_std]
//! Original scoped C runtime. No host CRT implementation is bundled.
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod crt;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
