#![no_std]
//! Original bcryptprimitives.dll: `ProcessPrng`, Windows' process random
//! number generator (Chromium's `base::RandBytes` requires it). Every
//! request is filled from the kernel entropy source, which is what the
//! Windows generator is seeded from.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// The largest piece one entropy call fills.
pub const CHUNK: usize = 1 << 20;
