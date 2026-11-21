#![no_std]

//! Allocation-free filesystem, redo recovery and persistent namespace mechanisms.
//! Kernel diagnostics and the protected storage service use this one implementation.
pub mod persistence;
pub mod storage;
