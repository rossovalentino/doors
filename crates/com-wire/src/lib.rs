#![no_std]
//! Out-of-process COM's wire formats (P16 M6), shared by ole32 and
//! oleaut32:
//!
//! - `objref`: OBJREF as [MS-DCOM] 2.2.18 lays it out (standard and custom),
//!   its exporter reached through one ncalrpc string binding naming the
//!   endpoint Init brokers.
//! - `message`: the ORPC messages ole32 exchanges over a Doors channel
//!   (calls, replies, IRemUnknown's QueryInterface/AddRef/Release), cut into
//!   the kernel's 256-byte packets and joined again.
//! - `value`: the little-endian writer and reader the marshalers build
//!   their buffers with.
extern crate alloc;

pub mod message;
pub mod objref;
pub mod value;

pub type Guid = [u8; 16];
