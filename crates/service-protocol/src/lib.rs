#![no_std]

//! Versioned user-service contracts and allocation-free orchestration policy.
//! These types confer no authority: peers come from authenticated kernel channels.
pub mod accessibility;
pub mod administration;
pub mod application;
pub mod associations;
pub mod audio;
pub mod clipboard;
pub mod com;
pub mod console;
pub mod credential;
pub mod desktop;
pub mod device;
pub mod dnd;
pub mod eventlog;
pub mod files;
pub mod firewall;
pub mod manifest;
pub mod net;
pub mod notifications;
pub mod preferences;
pub mod scm;
pub mod shell;
pub mod streams;
pub mod supervisor;
pub mod tasks;
pub mod text_input;
pub mod window;
pub mod wire;
pub use wire::{
    BODY_BYTES, CHUNK_BYTES, Code, Frame, LONG_BYTES, Message, PATH_BYTES, REGISTRY_PATH_UNITS,
    REGISTRY_VALUE_BYTES, opcode,
};
