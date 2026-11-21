//! TLS 1.2 (RFC 5246) for servers that do not speak TLS 1.3: ECDHE key
//! exchange over X25519 or P-256 (RFC 8422), AES-GCM (RFC 5288) and
//! ChaCha20-Poly1305 (RFC 7905) records, the extended master secret (RFC
//! 7627) and secure renegotiation's empty renegotiation_info (RFC 5746;
//! renegotiation itself is refused). The server's ECDHE parameters are
//! signed with its certificate key: RSA PKCS#1 v1.5 or PSS, or ECDSA.
use super::{Cipher, Error, rsa, verify, x509};
use aes_gcm::aead::{Aead, KeyInit, Payload};
use alloc::{boxed::Box, vec::Vec};
use hmac::Mac;

/// The TLS 1.2 suites offered, most preferred first.
pub const SUITES: [u16; 6] = [0xC02B, 0xC02F, 0xCCA9, 0xCCA8, 0xC02C, 0xC030];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite12 {
    /// ECDHE with AES-128-GCM and SHA-256 (ECDSA or RSA certificate).
    Aes128,
    Aes256,
    ChaCha,
}
impl Suite12 {
    /// The suite and whether its certificate key is ECDSA.
    pub fn from_id(id: u16) -> Option<(Self, bool)> {
        Some(match id {
            0xC02B => (Self::Aes128, true),
            0xC02F => (Self::Aes128, false),
            0xC02C => (Self::Aes256, true),
            0xC030 => (Self::Aes256, false),
            0xCCA9 => (Self::ChaCha, true),
            0xCCA8 => (Self::ChaCha, false),
            _ => return None,
        })
    }
    fn sha384(self) -> bool {
        self == Self::Aes256
    }
    fn key_len(self) -> usize {
        if self == Self::Aes128 { 16 } else { 32 }
    }
    /// The implicit IV: GCM's 4-byte salt, ChaCha20's whole 12-byte nonce.
    fn iv_len(self) -> usize {
        if self == Self::ChaCha { 12 } else { 4 }
    }
    pub fn hash(self, data: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        if self.sha384() {
            sha2::Sha384::digest(data).to_vec()
        } else {
            sha2::Sha256::digest(data).to_vec()
        }
    }
    fn hmac(self, key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
        if self.sha384() {
            let mut m = <hmac::Hmac<sha2::Sha384> as Mac>::new_from_slice(key).expect("any key");
            for p in parts {
                m.update(p);
            }
            m.finalize().into_bytes().to_vec()
        } else {
            let mut m = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("any key");
            for p in parts {
                m.update(p);
            }
            m.finalize().into_bytes().to_vec()
        }
    }
    /// PRF(secret, label, seed) of RFC 5246 §5: P_<hash> for the suite.
    pub fn prf(self, secret: &[u8], label: &[u8], seed: &[u8], len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 48);
        let mut a = self.hmac(secret, &[label, seed]);
        while out.len() < len {
            out.extend_from_slice(&self.hmac(secret, &[&a, label, seed]));
            a = self.hmac(secret, &[&a]);
        }
        out.truncate(len);
        out
    }
}

/// One direction's record protection.
pub struct Keys12 {
    cipher: Cipher,
    iv: [u8; 12],
    chacha: bool,
    seq: u64,
}
impl Keys12 {
    fn new(suite: Suite12, key: &[u8], iv: &[u8]) -> Self {
        let cipher = match suite {
            Suite12::Aes128 => {
                Cipher::Aes128(Box::new(aes_gcm::Aes128Gcm::new_from_slice(key).unwrap()))
            }
            Suite12::Aes256 => {
                Cipher::Aes256(Box::new(aes_gcm::Aes256Gcm::new_from_slice(key).unwrap()))
            }
            Suite12::ChaCha => Cipher::ChaCha(Box::new(
                chacha20poly1305::ChaCha20Poly1305::new_from_slice(key).unwrap(),
            )),
        };
        let mut fixed = [0u8; 12];
        fixed[..iv.len()].copy_from_slice(iv);
        Self {
            cipher,
            iv: fixed,
            chacha: suite == Suite12::ChaCha,
            seq: 0,
        }
    }
    /// The record nonce for the current sequence number (and GCM's
    /// explicit part, which is that number).
    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        if self.chacha {
            for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
                n[4 + i] ^= b;
            }
        } else {
            n[4..].copy_from_slice(&self.seq.to_be_bytes());
        }
        n
    }
    fn aad(&self, kind: u8, length: usize) -> [u8; 13] {
        let mut a = [0u8; 13];
        a[..8].copy_from_slice(&self.seq.to_be_bytes());
        a[8] = kind;
        a[9] = 3;
        a[10] = 3;
        a[11..].copy_from_slice(&(length as u16).to_be_bytes());
        a
    }
    pub fn seal(&mut self, kind: u8, data: &[u8]) -> Vec<u8> {
        let nonce = self.nonce();
        let aad = self.aad(kind, data.len());
        let payload = Payload {
            msg: data,
            aad: &aad,
        };
        let sealed = match &self.cipher {
            Cipher::Aes128(c) => c.encrypt((&nonce).into(), payload),
            Cipher::Aes256(c) => c.encrypt((&nonce).into(), payload),
            Cipher::ChaCha(c) => c.encrypt((&nonce).into(), payload),
        }
        .expect("encryption cannot fail");
        let explicit = if self.chacha { 0 } else { 8 };
        let len = explicit + sealed.len();
        let mut out = alloc::vec![kind, 3, 3, (len >> 8) as u8, len as u8];
        if !self.chacha {
            out.extend_from_slice(&self.seq.to_be_bytes());
        }
        out.extend_from_slice(&sealed);
        self.seq += 1;
        out
    }
    pub fn open(&mut self, kind: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
        let explicit = if self.chacha { 0 } else { 8 };
        if body.len() < explicit + 16 {
            return Err(Error::Decrypt);
        }
        let mut nonce = self.nonce();
        if !self.chacha {
            nonce[4..].copy_from_slice(&body[..8]);
        }
        let sealed = &body[explicit..];
        let aad = self.aad(kind, sealed.len() - 16);
        let payload = Payload {
            msg: sealed,
            aad: &aad,
        };
        let plain = match &self.cipher {
            Cipher::Aes128(c) => c.decrypt((&nonce).into(), payload),
            Cipher::Aes256(c) => c.decrypt((&nonce).into(), payload),
            Cipher::ChaCha(c) => c.decrypt((&nonce).into(), payload),
        }
        .map_err(|_| Error::Decrypt)?;
        if plain.len() > 16384 {
            return Err(Error::Protocol);
        }
        self.seq += 1;
        Ok(plain)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Certificate,
    KeyExchange,
    HelloDone,
    ChangeCipherSpec,
    Finished,
    Connected,
}
/// The TLS 1.2 half of a handshake once ServerHello chose it.
pub struct Session {
    pub suite: Suite12,
    ecdsa: bool,
    pub step: Step,
    client_random: [u8; 32],
    server_random: [u8; 32],
    extended_master: bool,
    /// The server's ECDHE share: (named group, point).
    share: Option<(u16, Vec<u8>)>,
    certificate_requested: bool,
    master: Vec<u8>,
    pub transcript: Vec<u8>,
    pub read: Option<Keys12>,
    pub write: Option<Keys12>,
    /// Keys installed by the server's ChangeCipherSpec.
    pending_read: Option<Keys12>,
}
const MAX_POINT: usize = 133;
impl Session {
    pub fn new(
        suite: Suite12,
        ecdsa: bool,
        client_random: [u8; 32],
        server_random: [u8; 32],
        extended_master: bool,
        transcript: Vec<u8>,
    ) -> Self {
        Self {
            suite,
            ecdsa,
            step: Step::Certificate,
            client_random,
            server_random,
            extended_master,
            share: None,
            certificate_requested: false,
            master: Vec::new(),
            transcript,
            read: None,
            write: None,
            pending_read: None,
        }
    }
    /// ServerKeyExchange: named-curve ECDHE parameters signed by the
    /// certificate key in `leaf`.
    pub fn key_exchange(&mut self, body: &[u8], leaf: &[u8]) -> Result<(), Error> {
        if body.first() != Some(&3) {
            return Err(Error::Unsupported);
        }
        let group = super::u16_at(body, 1)? as u16;
        if group != 0x001D && group != 0x0017 {
            return Err(Error::Unsupported);
        }
        let point_len = usize::from(*body.get(3).ok_or(Error::Protocol)?);
        if point_len == 0 || point_len > MAX_POINT {
            return Err(Error::Protocol);
        }
        let params = body.get(..4 + point_len).ok_or(Error::Protocol)?;
        let point = params[4..].to_vec();
        let at = 4 + point_len;
        let alg = super::u16_at(body, at)?;
        let len = super::u16_at(body, at + 2)?;
        let sig = body.get(at + 4..at + 4 + len).ok_or(Error::Protocol)?;
        if at + 4 + len != body.len() {
            return Err(Error::Protocol);
        }
        let mut content = Vec::with_capacity(64 + params.len());
        content.extend_from_slice(&self.client_random);
        content.extend_from_slice(&self.server_random);
        content.extend_from_slice(params);
        let cert = x509::parse(leaf).ok_or(Error::Certificate(verify::CertError::Malformed))?;
        let key = x509::spki(cert.spki)
            .ok_or(Error::Certificate(verify::CertError::UnsupportedAlgorithm))?;
        let ok = match (alg, key.alg) {
            (0x0403, x509::KeyAlg::P256) | (0x0503, x509::KeyAlg::P384) if self.ecdsa => {
                let hash = if alg == 0x0403 {
                    x509::SigAlg::EcdsaSha256
                } else {
                    x509::SigAlg::EcdsaSha384
                };
                verify::signature(&key, hash, &content, sig).is_ok()
            }
            (0x0401 | 0x0501 | 0x0601, x509::KeyAlg::Rsa) if !self.ecdsa => {
                let hash = match alg {
                    0x0401 => rsa::Hash::Sha256,
                    0x0501 => rsa::Hash::Sha384,
                    _ => rsa::Hash::Sha512,
                };
                rsa::verify_pkcs1(key.key, hash, &content, sig)
            }
            (0x0804..=0x0806, x509::KeyAlg::Rsa) if !self.ecdsa => {
                let hash = match alg {
                    0x0804 => rsa::Hash::Sha256,
                    0x0805 => rsa::Hash::Sha384,
                    _ => rsa::Hash::Sha512,
                };
                rsa::verify_pss(key.key, hash, &content, sig)
            }
            _ => return Err(Error::Unsupported),
        };
        if !ok {
            return Err(Error::Certificate(verify::CertError::BadSignature));
        }
        self.share = Some((group, point));
        Ok(())
    }
    pub fn certificate_requested(&mut self) {
        self.certificate_requested = true;
    }
    /// ServerHelloDone: the client's flight (an empty Certificate if one was
    /// asked for, ClientKeyExchange, ChangeCipherSpec, Finished) as records.
    /// `x25519` and `p256` are this connection's secrets.
    pub fn client_flight(
        &mut self,
        x25519: &x25519_dalek::StaticSecret,
        p256: &[u8; 32],
    ) -> Result<Vec<u8>, Error> {
        let (group, point) = self.share.take().ok_or(Error::Protocol)?;
        let (public, premaster) = match group {
            0x001D => {
                let peer = <[u8; 32]>::try_from(point.as_slice()).map_err(|_| Error::Protocol)?;
                let shared = x25519.diffie_hellman(&x25519_dalek::PublicKey::from(peer));
                if shared.as_bytes().iter().all(|b| *b == 0) {
                    return Err(Error::Protocol);
                }
                (
                    x25519_dalek::PublicKey::from(x25519).as_bytes().to_vec(),
                    shared.as_bytes().to_vec(),
                )
            }
            _ => {
                let secret = p256::SecretKey::from_slice(p256).map_err(|_| Error::Protocol)?;
                let peer = p256::PublicKey::from_sec1_bytes(&point).map_err(|_| Error::Protocol)?;
                let shared =
                    p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
                use p256::elliptic_curve::sec1::ToEncodedPoint;
                (
                    secret
                        .public_key()
                        .to_encoded_point(false)
                        .as_bytes()
                        .to_vec(),
                    shared.raw_secret_bytes().to_vec(),
                )
            }
        };
        let mut out = Vec::new();
        let mut flight = Vec::new();
        if self.certificate_requested {
            flight.extend_from_slice(&[11, 0, 0, 3, 0, 0, 0]);
        }
        let mut exchange = alloc::vec![16u8, 0, 0, (public.len() + 1) as u8, public.len() as u8];
        exchange.extend_from_slice(&public);
        flight.extend_from_slice(&exchange);
        self.transcript.extend_from_slice(&flight);
        let suite = self.suite;
        self.master = if self.extended_master {
            let session_hash = suite.hash(&self.transcript);
            suite.prf(&premaster, b"extended master secret", &session_hash, 48)
        } else {
            let mut seed = self.client_random.to_vec();
            seed.extend_from_slice(&self.server_random);
            suite.prf(&premaster, b"master secret", &seed, 48)
        };
        let mut seed = self.server_random.to_vec();
        seed.extend_from_slice(&self.client_random);
        let (k, i) = (suite.key_len(), suite.iv_len());
        let block = suite.prf(&self.master, b"key expansion", &seed, 2 * k + 2 * i);
        let write = Keys12::new(suite, &block[..k], &block[2 * k..2 * k + i]);
        self.pending_read = Some(Keys12::new(suite, &block[k..2 * k], &block[2 * k + i..]));
        // The plaintext flight, then ChangeCipherSpec, then the sealed Finished.
        out.extend_from_slice(&[22, 3, 3, (flight.len() >> 8) as u8, flight.len() as u8]);
        out.extend_from_slice(&flight);
        out.extend_from_slice(&[20, 3, 3, 0, 1, 1]);
        let verify = suite.prf(
            &self.master,
            b"client finished",
            &suite.hash(&self.transcript),
            12,
        );
        let mut finished = alloc::vec![20u8, 0, 0, 12];
        finished.extend_from_slice(&verify);
        self.transcript.extend_from_slice(&finished);
        let mut write = write;
        out.extend_from_slice(&write.seal(22, &finished));
        self.write = Some(write);
        self.step = Step::ChangeCipherSpec;
        Ok(out)
    }
    /// The server's ChangeCipherSpec: its records are protected from now.
    pub fn change_cipher_spec(&mut self) -> Result<(), Error> {
        if self.step != Step::ChangeCipherSpec {
            return Err(Error::Protocol);
        }
        self.read = Some(self.pending_read.take().ok_or(Error::Protocol)?);
        self.step = Step::Finished;
        Ok(())
    }
    /// The server's Finished.
    pub fn finished(&mut self, body: &[u8]) -> Result<(), Error> {
        let expected = self.suite.prf(
            &self.master,
            b"server finished",
            &self.suite.hash(&self.transcript),
            12,
        );
        if body.len() != 12
            || body
                .iter()
                .zip(&expected)
                .fold(0u8, |a, (x, y)| a | (x ^ y))
                != 0
        {
            return Err(Error::Decrypt);
        }
        self.transcript = Vec::new();
        self.step = Step::Connected;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prf_matches_the_published_sha256_vector() {
        // The TLS 1.2 PRF test vector (SHA-256) from the IETF TLS list.
        let secret = [
            0x9b, 0xbe, 0x43, 0x6b, 0xa9, 0x40, 0xf0, 0x17, 0xb1, 0x76, 0x52, 0x84, 0x9a, 0x71,
            0xdb, 0x35,
        ];
        let seed = [
            0xa0, 0xba, 0x9f, 0x93, 0x6c, 0xda, 0x31, 0x18, 0x27, 0xa6, 0xf7, 0x96, 0xff, 0xd5,
            0x19, 0x8c,
        ];
        let out = Suite12::Aes128.prf(&secret, b"test label", &seed, 100);
        assert_eq!(out.len(), 100);
        assert_eq!(
            out[..16],
            [
                0xe3, 0xf2, 0x29, 0xba, 0x72, 0x7b, 0xe1, 0x7b, 0x8d, 0x12, 0x26, 0x20, 0x55, 0x7c,
                0xd4, 0x53
            ]
        );
    }
    #[test]
    fn records_round_trip_and_reject_tampering() {
        for suite in [Suite12::Aes128, Suite12::Aes256, Suite12::ChaCha] {
            let key = alloc::vec![7u8; suite.key_len()];
            let iv = alloc::vec![9u8; suite.iv_len()];
            let (mut a, mut b) = (Keys12::new(suite, &key, &iv), Keys12::new(suite, &key, &iv));
            for message in [&b"hello"[..], &[0u8; 300][..]] {
                let record = a.seal(23, message);
                assert_eq!(record[0], 23);
                assert_eq!(b.open(23, &record[5..]).unwrap(), message);
            }
            let mut record = a.seal(23, b"x");
            let last = record.len() - 1;
            record[last] ^= 1;
            assert_eq!(b.open(23, &record[5..]), Err(Error::Decrypt));
        }
    }
}
