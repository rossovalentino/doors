//! Handshake messages and Noise_IKpsk2 (whitepaper sections 5.4.2–5.4.7):
//! the initiation, the response, mac1/mac2 and the cookie reply. Pure
//! functions; `peer` keeps the state between them.
use crate::crypto;
use x25519_dalek::{PublicKey, StaticSecret};

pub const INITIATION: u8 = 1;
pub const RESPONSE: u8 = 2;
pub const COOKIE_REPLY: u8 = 3;
pub const TRANSPORT: u8 = 4;
pub const INITIATION_BYTES: usize = 148;
pub const RESPONSE_BYTES: usize = 92;
pub const COOKIE_REPLY_BYTES: usize = 64;
/// Type, reserved, receiver index and counter before the ciphertext.
pub const TRANSPORT_HEADER: usize = 16;

const CONSTRUCTION: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
const IDENTIFIER: &[u8] = b"WireGuard v1 zx2c4 Jason@zx2c4.com";
const LABEL_MAC1: &[u8] = b"mac1----";
const LABEL_COOKIE: &[u8] = b"cookie--";
/// TAI64 label of 1970-01-01 (2^62 + 10, as wireguard-go stamps it).
const TAI64_BASE: u64 = (1 << 62) + 10;

/// The long-term keys of one tunnel and what derives from them once.
pub struct Statics {
    pub private: StaticSecret,
    pub public: [u8; 32],
    pub peer: [u8; 32],
    pub psk: [u8; 32],
    /// DH(S_priv, S_peer); `None` when the peer key is a low-order point.
    ss: Option<[u8; 32]>,
    /// HASH(LABEL_MAC1 || S_peer): mac1 of the messages we send.
    pub mac1_send: [u8; 32],
    /// HASH(LABEL_MAC1 || S_pub): mac1 of the messages we receive.
    pub mac1_receive: [u8; 32],
    /// HASH(LABEL_COOKIE || S_peer): opens the peer's cookie replies.
    pub cookie_open: [u8; 32],
    /// HASH(LABEL_COOKIE || S_pub): seals our cookie replies.
    pub cookie_seal: [u8; 32],
}
impl Statics {
    pub fn new(private: [u8; 32], peer: [u8; 32], psk: Option<[u8; 32]>) -> Self {
        let private = StaticSecret::from(private);
        let public = PublicKey::from(&private).to_bytes();
        let ss = crypto::dh(&private, &peer);
        Self {
            private,
            public,
            peer,
            psk: psk.unwrap_or([0; 32]),
            ss,
            mac1_send: crypto::hash(&[LABEL_MAC1, &peer]),
            mac1_receive: crypto::hash(&[LABEL_MAC1, &public]),
            cookie_open: crypto::hash(&[LABEL_COOKIE, &peer]),
            cookie_seal: crypto::hash(&[LABEL_COOKIE, &public]),
        }
    }
}
/// Transport keys of a completed handshake.
#[derive(Clone, Copy)]
pub struct Keys {
    pub send: [u8; 32],
    pub receive: [u8; 32],
}
/// An initiation we sent, waiting for its response.
pub struct Pending {
    pub sender: u32,
    chaining: [u8; 32],
    hash: [u8; 32],
    ephemeral: StaticSecret,
}
/// An initiation we accepted, ready to answer.
pub struct Accepted {
    /// The initiator's index (the response's receiver).
    pub sender: u32,
    pub timestamp: [u8; 12],
    ephemeral: [u8; 32],
    chaining: [u8; 32],
    hash: [u8; 32],
}
fn initial() -> ([u8; 32], [u8; 32]) {
    let chaining = crypto::hash(&[CONSTRUCTION]);
    let hash = crypto::hash(&[&chaining, IDENTIFIER]);
    (chaining, hash)
}
/// TAI64N of a UTC time in nanoseconds since 1970, its nanoseconds rounded
/// down to 2^24 ns as wireguard-go whitens them.
pub fn tai64n(unix_nanos: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[..8].copy_from_slice(&(TAI64_BASE + unix_nanos / 1_000_000_000).to_be_bytes());
    let nanos = (unix_nanos % 1_000_000_000) as u32 & !0x00FF_FFFF;
    out[8..].copy_from_slice(&nanos.to_be_bytes());
    out
}
/// Offsets of mac1 and mac2 at the end of a handshake message.
fn mac_offsets(len: usize) -> (usize, usize) {
    (len - 32, len - 16)
}
/// Write mac1 (and mac2 from a fresh cookie, else zeros); answers mac1.
pub fn seal_macs(message: &mut [u8], mac1_key: &[u8; 32], cookie: Option<&[u8; 16]>) -> [u8; 16] {
    let (m1, m2) = mac_offsets(message.len());
    let mac1 = crypto::mac(mac1_key, &message[..m1]);
    message[m1..m2].copy_from_slice(&mac1);
    let mac2 = cookie.map_or([0; 16], |c| crypto::mac(c, &message[..m2]));
    message[m2..].copy_from_slice(&mac2);
    mac1
}
/// mac1 of a received handshake message is valid.
pub fn mac1_valid(message: &[u8], mac1_key: &[u8; 32]) -> bool {
    let (m1, m2) = mac_offsets(message.len());
    crypto::equal(&crypto::mac(mac1_key, &message[..m1]), &message[m1..m2])
}
/// mac2 is valid for the cookie a sender at that address would hold.
pub fn mac2_valid(message: &[u8], cookie: &[u8; 16]) -> bool {
    let (_, m2) = mac_offsets(message.len());
    crypto::equal(&crypto::mac(cookie, &message[..m2]), &message[m2..])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn key_at(b: &[u8], at: usize) -> [u8; 32] {
    b[at..at + 32].try_into().unwrap()
}

/// Message 1: the initiation (without its macs: `seal_macs`).
pub fn initiate(
    st: &Statics,
    sender: u32,
    ephemeral: [u8; 32],
    timestamp: [u8; 12],
) -> Option<([u8; INITIATION_BYTES], Pending)> {
    let ss = st.ss?;
    let (mut ck, mut h) = initial();
    h = crypto::hash(&[&h, &st.peer]);
    let ephemeral = StaticSecret::from(ephemeral);
    let e_pub = PublicKey::from(&ephemeral).to_bytes();
    let mut msg = [0u8; INITIATION_BYTES];
    msg[0] = INITIATION;
    msg[4..8].copy_from_slice(&sender.to_le_bytes());
    msg[8..40].copy_from_slice(&e_pub);
    [ck] = crypto::kdf::<1>(&ck, &e_pub);
    h = crypto::hash(&[&h, &e_pub]);
    let [c, k] = crypto::kdf::<2>(&ck, &crypto::dh(&ephemeral, &st.peer)?);
    ck = c;
    let encrypted_static = crypto::seal(&k, 0, &st.public, &h);
    msg[40..88].copy_from_slice(&encrypted_static);
    h = crypto::hash(&[&h, &encrypted_static]);
    let [c, k] = crypto::kdf::<2>(&ck, &ss);
    ck = c;
    let encrypted_time = crypto::seal(&k, 0, &timestamp, &h);
    msg[88..116].copy_from_slice(&encrypted_time);
    h = crypto::hash(&[&h, &encrypted_time]);
    Some((
        msg,
        Pending {
            sender,
            chaining: ck,
            hash: h,
            ephemeral,
        },
    ))
}

/// The responder's side of message 1: the initiator must hold the
/// configured peer key. Replay and flood checks are the caller's.
pub fn accept(st: &Statics, msg: &[u8]) -> Option<Accepted> {
    if msg.len() != INITIATION_BYTES || msg[0] != INITIATION || msg[1..4] != [0; 3] {
        return None;
    }
    let ss = st.ss?;
    let (mut ck, mut h) = initial();
    h = crypto::hash(&[&h, &st.public]);
    let e_i = key_at(msg, 8);
    [ck] = crypto::kdf::<1>(&ck, &e_i);
    h = crypto::hash(&[&h, &e_i]);
    let [c, k] = crypto::kdf::<2>(&ck, &crypto::dh(&st.private, &e_i)?);
    ck = c;
    let s_i = crypto::open(&k, 0, &msg[40..88], &h)?;
    if !crypto::equal(&s_i, &st.peer) {
        return None;
    }
    h = crypto::hash(&[&h, &msg[40..88]]);
    let [c, k] = crypto::kdf::<2>(&ck, &ss);
    ck = c;
    let timestamp = crypto::open(&k, 0, &msg[88..116], &h)?;
    h = crypto::hash(&[&h, &msg[88..116]]);
    Some(Accepted {
        sender: u32_at(msg, 4),
        timestamp: timestamp.try_into().ok()?,
        ephemeral: e_i,
        chaining: ck,
        hash: h,
    })
}

/// Message 2 for an accepted initiation (without its macs) and the
/// responder's transport keys.
pub fn respond(
    st: &Statics,
    accepted: &Accepted,
    sender: u32,
    ephemeral: [u8; 32],
) -> Option<([u8; RESPONSE_BYTES], Keys)> {
    let (mut ck, mut h) = (accepted.chaining, accepted.hash);
    let ephemeral = StaticSecret::from(ephemeral);
    let e_pub = PublicKey::from(&ephemeral).to_bytes();
    let mut msg = [0u8; RESPONSE_BYTES];
    msg[0] = RESPONSE;
    msg[4..8].copy_from_slice(&sender.to_le_bytes());
    msg[8..12].copy_from_slice(&accepted.sender.to_le_bytes());
    msg[12..44].copy_from_slice(&e_pub);
    [ck] = crypto::kdf::<1>(&ck, &e_pub);
    h = crypto::hash(&[&h, &e_pub]);
    [ck] = crypto::kdf::<1>(&ck, &crypto::dh(&ephemeral, &accepted.ephemeral)?);
    [ck] = crypto::kdf::<1>(&ck, &crypto::dh(&ephemeral, &st.peer)?);
    let [c, t, k] = crypto::kdf::<3>(&ck, &st.psk);
    ck = c;
    h = crypto::hash(&[&h, &t]);
    let empty = crypto::seal(&k, 0, &[], &h);
    msg[44..60].copy_from_slice(&empty);
    let [receive, send] = crypto::kdf::<2>(&ck, &[]);
    Some((msg, Keys { send, receive }))
}

/// The initiator's side of message 2: the responder's index and the
/// initiator's transport keys.
pub fn complete(st: &Statics, pending: &Pending, msg: &[u8]) -> Option<(u32, Keys)> {
    if msg.len() != RESPONSE_BYTES || msg[0] != RESPONSE || msg[1..4] != [0; 3] {
        return None;
    }
    if u32_at(msg, 8) != pending.sender {
        return None;
    }
    let (mut ck, mut h) = (pending.chaining, pending.hash);
    let e_r = key_at(msg, 12);
    [ck] = crypto::kdf::<1>(&ck, &e_r);
    h = crypto::hash(&[&h, &e_r]);
    [ck] = crypto::kdf::<1>(&ck, &crypto::dh(&pending.ephemeral, &e_r)?);
    [ck] = crypto::kdf::<1>(&ck, &crypto::dh(&st.private, &e_r)?);
    let [c, t, k] = crypto::kdf::<3>(&ck, &st.psk);
    ck = c;
    h = crypto::hash(&[&h, &t]);
    crypto::open(&k, 0, &msg[44..60], &h)?;
    let [send, receive] = crypto::kdf::<2>(&ck, &[]);
    Some((u32_at(msg, 4), Keys { send, receive }))
}

/// A cookie reply to the handshake message whose mac1 was `mac1`: the
/// cookie for `address` (the sender's IP and port bytes) under `secret`.
pub fn cookie_reply(
    st: &Statics,
    receiver: u32,
    nonce: [u8; 24],
    secret: &[u8; 32],
    address: &[u8],
    mac1: &[u8; 16],
) -> [u8; COOKIE_REPLY_BYTES] {
    let cookie = crypto::mac(secret, address);
    let mut msg = [0u8; COOKIE_REPLY_BYTES];
    msg[0] = COOKIE_REPLY;
    msg[4..8].copy_from_slice(&receiver.to_le_bytes());
    msg[8..32].copy_from_slice(&nonce);
    msg[32..].copy_from_slice(&crypto::xseal(&st.cookie_seal, &nonce, &cookie, mac1));
    msg
}
/// The cookie in a cookie reply to our message whose mac1 was `mac1`, and
/// the index it names.
pub fn open_cookie(st: &Statics, msg: &[u8], mac1: &[u8; 16]) -> Option<(u32, [u8; 16])> {
    if msg.len() != COOKIE_REPLY_BYTES || msg[0] != COOKIE_REPLY || msg[1..4] != [0; 3] {
        return None;
    }
    let nonce: [u8; 24] = msg[8..32].try_into().unwrap();
    let cookie = crypto::xopen(&st.cookie_open, &nonce, &msg[32..], mac1)?;
    Some((u32_at(msg, 4), cookie.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::clamp;

    #[test]
    fn construction_constants_match_wireguard() {
        // HASH(CONSTRUCTION) and HASH(that || IDENTIFIER), as every
        // implementation precomputes them.
        let (ck, h) = initial();
        assert_eq!(
            ck,
            [
                96, 226, 109, 174, 243, 39, 239, 192, 46, 195, 53, 226, 160, 37, 210, 208, 22, 235,
                66, 6, 248, 114, 119, 245, 45, 56, 209, 152, 139, 120, 205, 54
            ]
        );
        assert_eq!(
            h,
            [
                34, 17, 179, 97, 8, 26, 197, 102, 105, 18, 67, 219, 69, 138, 213, 50, 45, 156, 108,
                102, 34, 147, 232, 183, 14, 225, 156, 101, 186, 7, 158, 243
            ]
        );
    }

    fn pair(psk: Option<[u8; 32]>) -> (Statics, Statics) {
        let (a, b) = (clamp([1; 32]), clamp([2; 32]));
        (
            Statics::new(a, crypto::public_key(&b), psk),
            Statics::new(b, crypto::public_key(&a), psk),
        )
    }

    #[test]
    fn both_sides_derive_matching_keys() {
        for psk in [None, Some([9u8; 32])] {
            let (i, r) = pair(psk);
            let (mut m1, pending) = initiate(&i, 0x1111, [3; 32], tai64n(5)).unwrap();
            seal_macs(&mut m1, &i.mac1_send, None);
            assert!(mac1_valid(&m1, &r.mac1_receive));
            let accepted = accept(&r, &m1).unwrap();
            assert_eq!((accepted.sender, accepted.timestamp), (0x1111, tai64n(5)));
            let (mut m2, rk) = respond(&r, &accepted, 0x2222, [4; 32]).unwrap();
            seal_macs(&mut m2, &r.mac1_send, None);
            assert!(mac1_valid(&m2, &i.mac1_receive));
            let (index, ik) = complete(&i, &pending, &m2).unwrap();
            assert_eq!(index, 0x2222);
            assert_eq!((ik.send, ik.receive), (rk.receive, rk.send));
        }
    }

    #[test]
    fn mismatched_keys_and_tampering_fail() {
        let (i, r) = pair(Some([1; 32]));
        let (_, r_other_psk) = pair(Some([2; 32]));
        let (m1, pending) = initiate(&i, 1, [3; 32], tai64n(5)).unwrap();
        // A responder configured for another peer refuses the initiator.
        let stranger = Statics::new(clamp([2; 32]), crypto::public_key(&clamp([7; 32])), None);
        assert!(accept(&stranger, &m1).is_none());
        let mut bad = m1;
        bad[100] ^= 1;
        assert!(accept(&r, &bad).is_none());
        // A different preshared key fails at the response.
        let accepted = accept(&r_other_psk, &m1).unwrap();
        let (m2, _) = respond(&r_other_psk, &accepted, 2, [4; 32]).unwrap();
        assert!(complete(&i, &pending, &m2).is_none());
        // A response to another index is not ours.
        let accepted = accept(&r, &m1).unwrap();
        let (mut m2, _) = respond(&r, &accepted, 2, [4; 32]).unwrap();
        assert!(complete(&i, &pending, &m2).is_some());
        m2[8] ^= 1;
        assert!(complete(&i, &pending, &m2).is_none());
    }

    #[test]
    fn cookies_bind_mac2() {
        let (i, r) = pair(None);
        let (mut m1, _) = initiate(&i, 7, [3; 32], tai64n(1)).unwrap();
        let mac1 = seal_macs(&mut m1, &i.mac1_send, None);
        let reply = cookie_reply(&r, 7, [8; 24], &[6; 32], &[10, 0, 2, 15, 0xCA, 0x6C], &mac1);
        let (index, cookie) = open_cookie(&i, &reply, &mac1).unwrap();
        assert_eq!(index, 7);
        assert!(open_cookie(&i, &reply, &[0; 16]).is_none());
        seal_macs(&mut m1, &i.mac1_send, Some(&cookie));
        assert!(mac2_valid(
            &m1,
            &crypto::mac(&[6; 32], &[10, 0, 2, 15, 0xCA, 0x6C])
        ));
        assert!(mac1_valid(&m1, &r.mac1_receive));
    }

    #[test]
    fn timestamps_grow_and_whiten() {
        assert!(tai64n(2_000_000_000) > tai64n(1_999_000_000));
        assert_eq!(tai64n(1_000_000_000), tai64n(1_000_000_001));
        assert_eq!(&tai64n(0)[..8], &TAI64_BASE.to_be_bytes());
    }
}
