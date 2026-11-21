#![no_std]
//! Original dbghelp.dll (P18): the symbol handler initialises and keeps its
//! options and search path, but Doors ships no symbol files and reads no
//! PDBs: symbol, line and module lookups report ERROR_MOD_NOT_FOUND, stack
//! walks end at once, and minidumps are not written (ERROR_NOT_SUPPORTED).
//! UnDecorateSymbolName returns the name unchanged.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
