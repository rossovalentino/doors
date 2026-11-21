#![no_std]
//! Original winspool.drv (P18): the print spooler API. Doors has no
//! printers: enumeration returns none, there is no default printer
//! (GetDefaultPrinter, ordinal 203, ERROR_FILE_NOT_FOUND), and no printer
//! opens (ERROR_INVALID_PRINTER_NAME).
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
