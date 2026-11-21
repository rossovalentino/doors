#![no_std]
//! Original Winsock 2 (ws2_32.dll) over door.network, reached through
//! ntdll's `DoorNet*` exports: sockets with blocking, nonblocking and
//! timeout semantics, select/WSAPoll/WSAEventSelect, getaddrinfo and the
//! legacy name databases, overlapped I/O with completion ports and routines
//! (AcceptEx, ConnectEx, DisconnectEx, TransmitFile), and Winsock's error
//! codes. IPv4 and IPv6 (AF_INET6 sockets, IPV6_V6ONLY, dual-stack
//! sockets with IPv4-mapped peers).
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod catalog;
pub mod error;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod messages;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod names;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod overlapped;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod poll;
pub mod text;
#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod wsa;
