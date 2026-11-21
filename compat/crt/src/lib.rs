#![no_std]
//! The C runtime's logic for Doors' Universal CRT (`ucrtbase.dll`):
//! formatting and scanning engines, number parsing, the "C" locale's
//! character classes, calendar time and sorting. Allocation-free and
//! host-tested; the DLL crate binds them to the C ABI and to Windows.
pub mod ctype;
pub mod format;
pub mod number;
pub mod scan;
pub mod sort;
pub mod time;
