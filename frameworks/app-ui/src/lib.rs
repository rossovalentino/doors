#![no_std]
//! Shared native application UI facade. All native apps use the same runtime.
pub use door_workspace::runtime::{Application, run};
pub use door_workspace::{ImageCache, View, fluent, icons, ui};
