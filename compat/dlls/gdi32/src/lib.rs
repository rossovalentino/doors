#![no_std]
//! Original gdi32: device contexts with saved state, generational objects,
//! regions and clipping, ROP2/ROP3 raster operations, DIB sections and text
//! through Door Text. The semantics live in the host-tested
//! `door-windows-gui` core; exported PE entry points exist only in guest
//! builds, where application pointers pass the checked-copy boundary first.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
