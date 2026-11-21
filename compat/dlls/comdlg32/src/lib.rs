#![no_std]
//! Original comdlg32: GetOpenFileNameW/GetSaveFileNameW over the
//! host-tested Open/Save model, with Explorer-style hook pages (CDN_INITDONE,
//! CDN_FOLDERCHANGE, CDN_FILEOK, CDM_* messages, control ids edt1/lst1/cmb1)
//! and CommDlgExtendedError. Exported PE entry points exist only in guest
//! builds.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
extern crate alloc;
