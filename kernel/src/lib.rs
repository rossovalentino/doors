#![no_std]

//! Kernel mechanisms shared with host-side invariant tests.
#[path = "arch/x86_64/layout.rs"]
pub mod cpu_layout;
pub mod drivers;
pub mod entropy;
pub mod executive;
pub mod input;
pub mod keyboard;
pub mod mm;
pub mod pointer;
pub mod rtc;
pub use door_storage_core::{persistence, storage};
#[path = "process/image.rs"]
pub mod process_image;
pub mod task;
