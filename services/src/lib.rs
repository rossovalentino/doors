#![no_std]
//! Protected native services. Policies and protocol parsers remain host-testable;
//! production adapters use only the native kernel ABI.
pub mod console;
pub mod credentials;
pub mod init;
pub mod notifications;
pub mod server;
pub mod session;
pub mod storage;
