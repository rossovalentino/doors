//! The WireGuard primitives (whitepaper section 5.4): BLAKE2s HASH, keyed
//! BLAKE2s MAC, HMAC-BLAKE2s KDF, ChaCha20-Poly1305 AEAD with a 64-bit
//! counter nonce, XChaCha20-Poly1305 for cookies, and X25519.
use alloc::vec::Vec;
use blake2::{
    Blake2s256, Blake2sMac,
    digest::{Digest, Mac, consts::U16},
};
use chacha20poly1305::{
    ChaCha20Poly1305, XChaCha20Poly1305,
    aead::{Aead, KeyInit, Payload},
};
use hmac::SimpleHmac;
use x25519_dalek::{PublicKey, StaticSecret};

pub const KEY: usize = 32;
pub const TAG: usize = 16;
/// BLAKE2s-256 over the concatenation of `parts`.
pub fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Blake2s256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}
/// Keyed BLAKE2s with a 16-byte output (mac1, mac2, cookies).
pub fn mac(key: &[u8], data: &[u8]) -> [u8; 16] {
    // Keys are 32 bytes (mac1) or 16 bytes (cookies): always valid.
    let mut m = <Blake2sMac<U16> as Mac>::new_from_slice(key).expect("BLAKE2s key length");
    m.update(data);
    m.finalize().into_bytes().into()
}
fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <SimpleHmac<Blake2s256> as Mac>::new_from_slice(key).expect("HMAC accepts any key");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}
/// KDF_n(key, input): the first `N` outputs of the HKDF-style chain.
pub fn kdf<const N: usize>(key: &[u8; 32], input: &[u8]) -> [[u8; 32]; N] {
    let prk = hmac(key, &[input]);
    let mut out = [[0u8; 32]; N];
    for i in 0..N {
        let previous: &[u8] = if i == 0 { &[] } else { &out[i - 1] };
        let next = hmac(&prk, &[previous, &[i as u8 + 1]]);
        out[i] = next;
    }
    out
}
fn nonce(counter: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&counter.to_le_bytes());
    n
}
/// AEAD(key, counter, plaintext, aad).
pub fn seal(key: &[u8; 32], counter: u64, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key.into())
        .encrypt(
            &nonce(counter).into(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("ChaCha20-Poly1305 seals any length")
}
/// The plaintext, or `None` when the tag does not verify.
pub fn open(key: &[u8; 32], counter: u64, ciphertext: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(key.into())
        .decrypt(
            &nonce(counter).into(),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}
pub fn xseal(key: &[u8; 32], nonce: &[u8; 24], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(
            nonce.into(),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 seals any length")
}
pub fn xopen(key: &[u8; 32], nonce: &[u8; 24], ciphertext: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            nonce.into(),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}
/// X25519; `None` for an all-zero result (a low-order public key).
pub fn dh(private: &StaticSecret, public: &[u8; 32]) -> Option<[u8; 32]> {
    let shared = private.diffie_hellman(&PublicKey::from(*public));
    shared.was_contributory().then(|| shared.to_bytes())
}
/// The public key of a private key (the private key is clamped as X25519
/// does).
pub fn public_key(private: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*private)).to_bytes()
}
/// A private key as `wg genkey` makes it: random bytes, clamped.
pub fn clamp(mut key: [u8; 32]) -> [u8; 32] {
    key[0] &= 248;
    key[31] = (key[31] & 127) | 64;
    key
}
/// Constant-time comparison of equal-length byte strings.
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake2s_matches_rfc_7693() {
        // RFC 7693 appendix B: BLAKE2s-256("abc").
        assert_eq!(
            hash(&[b"a", b"bc"]),
            [
                0x50, 0x8C, 0x5E, 0x8C, 0x32, 0x7C, 0x14, 0xE2, 0xE1, 0xA7, 0x2B, 0xA3, 0x4E, 0xEB,
                0x45, 0x2F, 0x37, 0x45, 0x8B, 0x20, 0x9E, 0xD6, 0x3A, 0x29, 0x4D, 0x99, 0x9B, 0x4C,
                0x86, 0x67, 0x59, 0x82
            ]
        );
    }

    #[test]
    fn kdf_outputs_chain() {
        let key = [7u8; 32];
        let [a] = kdf::<1>(&key, b"input");
        let [b, c] = kdf::<2>(&key, b"input");
        let [d, e, f] = kdf::<3>(&key, b"input");
        assert_eq!((a, b, d), (b, d, a));
        assert_eq!(c, e);
        assert_ne!(f, e);
        // τ2 = HMAC(τ0, τ1 || 0x2)
        let prk = hmac(&key, &[b"input"]);
        assert_eq!(c, hmac(&prk, &[&a, &[2]]));
    }

    #[test]
    fn aead_round_trips_and_rejects_tampering() {
        let key = [3u8; 32];
        let mut sealed = seal(&key, 9, b"payload", b"aad");
        assert_eq!(sealed.len(), 7 + TAG);
        assert_eq!(open(&key, 9, &sealed, b"aad").unwrap(), b"payload");
        assert!(open(&key, 8, &sealed, b"aad").is_none());
        assert!(open(&key, 9, &sealed, b"other").is_none());
        sealed[0] ^= 1;
        assert!(open(&key, 9, &sealed, b"aad").is_none());
        let nonce = [5u8; 24];
        let x = xseal(&key, &nonce, &[1; 16], b"m");
        assert_eq!(xopen(&key, &nonce, &x, b"m").unwrap(), [1; 16]);
        assert!(xopen(&key, &[6; 24], &x, b"m").is_none());
    }

    #[test]
    fn dh_rejects_low_order_points() {
        let private = StaticSecret::from(clamp([9u8; 32]));
        assert!(dh(&private, &[0u8; 32]).is_none());
        let other = clamp([4u8; 32]);
        let shared = dh(&private, &public_key(&other)).unwrap();
        assert_eq!(
            shared,
            dh(&StaticSecret::from(other), &public_key(&clamp([9u8; 32]))).unwrap()
        );
    }
}
