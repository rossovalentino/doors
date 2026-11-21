//! The COM activation broker and ncalrpc transport Init hosts (P16 M6),
//! the roles RPCSS and ALPC play on Windows. A process that exports COM
//! objects REGISTERs an endpoint name; a client CONNECTs to it and gets a
//! link. Init carries each link's packets (SEND, in order, at most
//! `PAYLOAD_BYTES` each) in bounded mailboxes: a channel end cannot leave
//! the process that made it, and every process already holds its channel
//! to Init. POLL gives a process its next event on any of its links: a
//! packet, a client that connected to one of its endpoints (NEW, with the
//! endpoint's name) or a peer that went (CLOSED). Endpoints and links
//! belong to the caller's session; those of a process Init launched go
//! when it ends.
//!
//! ACTIVATE coordinates LocalServer32 activation: the class's server has
//! registered (`READY`: connect to `class_endpoint`), another client is
//! starting it (`PENDING`: wait and ask again), or this caller should
//! start it (`LAUNCH`); a launcher that gives up says FAILED so the next
//! client may try.

/// `[name]`: own the endpoint `name` (Exists when a live process holds it).
pub const REGISTER: u32 = 0x7040;
/// `[name]`: give it up (the owner only).
pub const UNREGISTER: u32 = 0x7041;
/// `[name]`: a link to the endpoint's owner; reply `[link u32]`.
pub const CONNECT: u32 = 0x7042;
/// `[link u32][packet]`: to the link's other side (WouldBlock while its
/// mailbox is full; Closing once that side went).
pub const SEND: u32 = 0x7043;
/// Empty: reply `[link u32][event u8][data]` (`event`), or WouldBlock.
pub const POLL: u32 = 0x7044;
/// `[link u32]`: this side is done with the link.
pub const CLOSE: u32 = 0x7045;
/// `[clsid 16]`: reply `[state u32]` (`READY`, `PENDING`, `LAUNCH`).
pub const ACTIVATE: u32 = 0x7046;
/// `[clsid 16]`: the launcher could not start the server.
pub const FAILED: u32 = 0x7047;
pub const FIRST: u32 = 0x7040;
pub const LAST: u32 = 0x704f;

pub mod event {
    /// `data`: one packet.
    pub const PACKET: u8 = 0;
    /// `data`: the endpoint a client connected to.
    pub const NEW: u8 = 1;
    /// The other side closed the link or ended.
    pub const CLOSED: u8 = 2;
}

pub const READY: u32 = 0;
pub const PENDING: u32 = 1;
pub const LAUNCH: u32 = 2;

/// Endpoint names: 1..=48 bytes of `[A-Za-z0-9._-]`.
pub const NAME_BYTES: usize = 48;
/// The largest packet a link carries (SEND's body less the link).
pub const PAYLOAD_BYTES: usize = 232;
/// Endpoints and links Init keeps, and packets each mailbox holds.
pub const ENDPOINTS: usize = 16;
pub const LINKS: usize = 16;
pub const MAILBOX: usize = 4;
/// A LAUNCH is the caller's for this long (ticks); then another may try.
pub const LAUNCH_TICKS: u64 = 30_000;
// SENDs body and POLLs reply fit one service message.
const _: () = assert!(5 + PAYLOAD_BYTES <= crate::BODY_BYTES);

pub fn valid_name(name: &[u8]) -> bool {
    (1..=NAME_BYTES).contains(&name.len())
        && name
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
/// The endpoint a LocalServer32's class object listens on:
/// `OLECLASS-` and the CLSID's 32 hex digits (bytes as stored).
pub fn class_endpoint(clsid: &[u8; 16]) -> [u8; 41] {
    let mut name = [0u8; 41];
    name[..9].copy_from_slice(b"OLECLASS-");
    for (i, b) in clsid.iter().enumerate() {
        name[9 + 2 * i] = b"0123456789ABCDEF"[(b >> 4) as usize];
        name[10 + 2 * i] = b"0123456789ABCDEF"[(b & 15) as usize];
    }
    name
}
/// The endpoint of process `pid`'s object exporter: `OLE` and its id.
pub fn exporter_endpoint(pid: u64) -> ([u8; 19], usize) {
    let mut name = [0u8; 19];
    name[..3].copy_from_slice(b"OLE");
    for i in 0..16 {
        name[3 + i] = b"0123456789ABCDEF"[((pid >> (60 - 4 * i)) & 15) as usize];
    }
    (name, 19)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_sizes() {
        assert!(valid_name(b"OLE0000000000000007"));
        assert!(!valid_name(b""));
        assert!(!valid_name(&[b'a'; NAME_BYTES + 1]));
        assert!(!valid_name(b"a b"));
        let class = class_endpoint(&[0xAB; 16]);
        assert!(valid_name(&class));
        assert_eq!(&exporter_endpoint(0x2a).0, b"OLE000000000000002A");
    }
}
