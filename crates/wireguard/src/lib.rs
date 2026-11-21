//! door-wireguard (P15 VPN): the WireGuard protocol, sans-IO.
//!
//! - `crypto`: BLAKE2s, HMAC-BLAKE2s KDF, keyed BLAKE2s, ChaCha20-Poly1305,
//!   XChaCha20-Poly1305 and X25519, as the whitepaper (section 5.4) fixes
//!   them;
//! - `handshake`: Noise_IKpsk2 messages, mac1/mac2 and cookie replies;
//! - `replay`: the receive counter window;
//! - `tunnel`: one peer's sessions, transport data, cryptokey routing and
//!   the timers of section 6;
//! - `config`: wg-quick files (WireGuard for Windows' import/export
//!   format);
//! - `ip`: IPv4 prefixes, headers and TCP MSS clamping.
//!
//! door-network runs a `Tunnel` over its UDP socket and its IP stack;
//! interoperability is tested against boringtun (`tests/interop.rs`).
#![no_std]
extern crate alloc;

pub mod config;
pub mod crypto;
pub mod handshake;
pub mod ip;
pub mod replay;
pub mod tunnel;

pub use config::{Config, Endpoint};
pub use ip::Cidr;
pub use tunnel::{Drop, Io, Now, Random, Stats, Tunnel};

/// UDP, IPv4 and WireGuard transport overhead around a tunnelled packet:
/// 20 + 8 + 16 header + 16 tag.
pub const OVERHEAD: usize = 60;
/// The TCP MSS that fits a tunnel MTU (40 bytes of IPv4 and TCP headers).
pub const fn mss_for(mtu: u16) -> u16 {
    mtu.saturating_sub(40)
}
/// A new private key from 32 random bytes (`wg genkey`).
pub fn private_key(random: [u8; 32]) -> [u8; 32] {
    crypto::clamp(random)
}
pub use crypto::public_key;
