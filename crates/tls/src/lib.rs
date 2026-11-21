//! A sans-IO TLS client for Doors OS (P15): Weather and the HTTPS helpers
//! feed it the bytes a TCP socket received and send the bytes it returns.
//! TLS 1.3 (RFC 8446): one-RTT handshakes with an X25519 key share; the
//! TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384 and
//! TLS_CHACHA20_POLY1305_SHA256 suites; CertificateVerify with ECDSA
//! P-256/P-384 or RSA-PSS; KeyUpdate. TLS 1.2 (`tls12`) for servers without
//! 1.3: ECDHE (X25519, P-256) with AES-GCM or ChaCha20-Poly1305 and the
//! extended master secret. Server certificates are checked against the
//! Mozilla roots (`verify`); close_notify. No resumption or 0-RTT, no
//! renegotiation, no client certificates (an empty one is sent when asked).
#![no_std]
extern crate alloc;
pub mod der;
pub mod rsa;
pub mod tls12;
pub mod verify;
pub mod x509;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use alloc::{string::String, vec::Vec};
use hmac::Mac;
pub use verify::CertError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The peer broke the protocol (unexpected message, bad length).
    Protocol,
    /// The peer sent this fatal alert.
    Alert(u8),
    Certificate(CertError),
    /// A record failed authentication.
    Decrypt,
    /// The server wants something this client does not do (TLS 1.2 only,
    /// another key-share group).
    Unsupported,
    /// The connection is closed.
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suite {
    Aes128,
    Aes256,
    ChaCha,
}
impl Suite {
    fn from_id(id: u16) -> Option<Self> {
        match id {
            0x1301 => Some(Self::Aes128),
            0x1302 => Some(Self::Aes256),
            0x1303 => Some(Self::ChaCha),
            _ => None,
        }
    }
    fn sha384(self) -> bool {
        self == Self::Aes256
    }
    fn hash_len(self) -> usize {
        if self.sha384() { 48 } else { 32 }
    }
    fn key_len(self) -> usize {
        if self == Self::Aes128 { 16 } else { 32 }
    }
    fn hash(self, data: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        if self.sha384() {
            sha2::Sha384::digest(data).to_vec()
        } else {
            sha2::Sha256::digest(data).to_vec()
        }
    }
    fn extract(self, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
        if self.sha384() {
            hkdf::Hkdf::<sha2::Sha384>::extract(Some(salt), ikm)
                .0
                .to_vec()
        } else {
            hkdf::Hkdf::<sha2::Sha256>::extract(Some(salt), ikm)
                .0
                .to_vec()
        }
    }
    fn expand_label(self, secret: &[u8], label: &str, context: &[u8], len: usize) -> Vec<u8> {
        let mut info = Vec::with_capacity(4 + 6 + label.len() + context.len());
        info.extend_from_slice(&(len as u16).to_be_bytes());
        info.push((6 + label.len()) as u8);
        info.extend_from_slice(b"tls13 ");
        info.extend_from_slice(label.as_bytes());
        info.push(context.len() as u8);
        info.extend_from_slice(context);
        let mut out = alloc::vec![0u8; len];
        let ok = if self.sha384() {
            hkdf::Hkdf::<sha2::Sha384>::from_prk(secret).map(|h| h.expand(&info, &mut out).is_ok())
        } else {
            hkdf::Hkdf::<sha2::Sha256>::from_prk(secret).map(|h| h.expand(&info, &mut out).is_ok())
        };
        debug_assert!(ok.unwrap_or(false));
        out
    }
    fn derive(self, secret: &[u8], label: &str, transcript: &[u8]) -> Vec<u8> {
        self.expand_label(secret, label, &self.hash(transcript), self.hash_len())
    }
    fn hmac(self, key: &[u8], data: &[u8]) -> Vec<u8> {
        if self.sha384() {
            let mut m =
                <hmac::Hmac<sha2::Sha384> as Mac>::new_from_slice(key).expect("any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        } else {
            let mut m =
                <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(key).expect("any key length");
            m.update(data);
            m.finalize().into_bytes().to_vec()
        }
    }
}

enum Cipher {
    Aes128(alloc::boxed::Box<aes_gcm::Aes128Gcm>),
    Aes256(alloc::boxed::Box<aes_gcm::Aes256Gcm>),
    ChaCha(alloc::boxed::Box<chacha20poly1305::ChaCha20Poly1305>),
}
/// One direction's traffic keys.
struct Keys {
    cipher: Cipher,
    iv: [u8; 12],
    seq: u64,
}
impl Keys {
    fn new(suite: Suite, secret: &[u8]) -> Self {
        let key = suite.expand_label(secret, "key", &[], suite.key_len());
        let iv: [u8; 12] = suite
            .expand_label(secret, "iv", &[], 12)
            .try_into()
            .unwrap();
        let cipher = match suite {
            Suite::Aes128 => Cipher::Aes128(alloc::boxed::Box::new(
                aes_gcm::Aes128Gcm::new_from_slice(&key).unwrap(),
            )),
            Suite::Aes256 => Cipher::Aes256(alloc::boxed::Box::new(
                aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap(),
            )),
            Suite::ChaCha => Cipher::ChaCha(alloc::boxed::Box::new(
                chacha20poly1305::ChaCha20Poly1305::new_from_slice(&key).unwrap(),
            )),
        };
        Self { cipher, iv, seq: 0 }
    }
    fn nonce(&mut self) -> [u8; 12] {
        let mut n = self.iv;
        for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        self.seq += 1;
        n
    }
    fn seal(&mut self, content_type: u8, data: &[u8]) -> Vec<u8> {
        let mut inner = Vec::with_capacity(data.len() + 1);
        inner.extend_from_slice(data);
        inner.push(content_type);
        let len = inner.len() + 16;
        let header = [0x17, 0x03, 0x03, (len >> 8) as u8, len as u8];
        let nonce = self.nonce();
        let payload = Payload {
            msg: &inner,
            aad: &header,
        };
        let sealed = match &self.cipher {
            Cipher::Aes128(c) => c.encrypt((&nonce).into(), payload),
            Cipher::Aes256(c) => c.encrypt((&nonce).into(), payload),
            Cipher::ChaCha(c) => c.encrypt((&nonce).into(), payload),
        }
        .expect("encryption cannot fail");
        let mut out = header.to_vec();
        out.extend_from_slice(&sealed);
        out
    }
    fn open(&mut self, header: &[u8], body: &[u8]) -> Result<(u8, Vec<u8>), Error> {
        let nonce = self.nonce();
        let payload = Payload {
            msg: body,
            aad: header,
        };
        let mut inner = match &self.cipher {
            Cipher::Aes128(c) => c.decrypt((&nonce).into(), payload),
            Cipher::Aes256(c) => c.decrypt((&nonce).into(), payload),
            Cipher::ChaCha(c) => c.decrypt((&nonce).into(), payload),
        }
        .map_err(|_| Error::Decrypt)?;
        // TLSInnerPlaintext is at most 2^14 + 1 bytes before padding.
        if inner.len() > 16384 + 1 + 255 {
            return Err(Error::Protocol);
        }
        while inner.last() == Some(&0) {
            inner.pop();
        }
        let content_type = inner.pop().ok_or(Error::Protocol)?;
        if inner.len() > 16384 {
            return Err(Error::Protocol);
        }
        Ok((content_type, inner))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    ServerHello,
    EncryptedExtensions,
    Certificate,
    CertificateVerify,
    Finished,
    /// TLS 1.2 after ServerHello: `Client::legacy` runs the handshake.
    Legacy,
    Connected,
    Closed,
}
/// The last 8 bytes of a TLS 1.3 server's random when it negotiates TLS 1.2
/// (RFC 8446 §4.1.3): a client offering 1.3 must refuse it.
const DOWNGRADE_12: [u8; 8] = *b"DOWNGRD\x01";

/// The HelloRetryRequest marker (RFC 8446 §4.1.3).
const HRR: [u8; 32] = [
    0xCF, 0x21, 0xAD, 0x74, 0xE5, 0x9A, 0x61, 0x11, 0xBE, 0x1D, 0x8C, 0x02, 0x1E, 0x65, 0xB8, 0x91,
    0xC2, 0xA2, 0x11, 0x16, 0x7A, 0xBB, 0x8C, 0x5E, 0x07, 0x9E, 0x09, 0xE2, 0xC8, 0xA8, 0x33, 0x9C,
];

pub struct Client {
    host: String,
    now: i64,
    secret: x25519_dalek::StaticSecret,
    state: State,
    suite: Suite,
    transcript: Vec<u8>,
    incoming: Vec<u8>,
    handshake: Vec<u8>,
    read: Option<Keys>,
    write: Option<Keys>,
    client_hs: Vec<u8>,
    server_hs: Vec<u8>,
    client_ap: Vec<u8>,
    server_ap: Vec<u8>,
    handshake_secret: Vec<u8>,
    certificates: Vec<Vec<u8>>,
    /// Certificate problems accepted, and those found.
    policy: verify::Policy,
    problems: verify::Problems,
    certificate_requested: bool,
    /// The legacy session id sent, which ServerHello must echo.
    session_id: [u8; 32],
    client_random: [u8; 32],
    /// The P-256 secret for a TLS 1.2 server that picks that group.
    p256: [u8; 32],
    legacy: Option<alloc::boxed::Box<tls12::Session>>,
    plaintext: Vec<u8>,
    out: Vec<u8>,
}
fn u16_at(b: &[u8], at: usize) -> Result<usize, Error> {
    Ok(usize::from(u16::from_be_bytes(
        b.get(at..at + 2)
            .ok_or(Error::Protocol)?
            .try_into()
            .unwrap(),
    )))
}
fn u24_at(b: &[u8], at: usize) -> Result<usize, Error> {
    let s = b.get(at..at + 3).ok_or(Error::Protocol)?;
    Ok((usize::from(s[0]) << 16) | (usize::from(s[1]) << 8) | usize::from(s[2]))
}
fn push_u16(v: &mut Vec<u8>, n: usize) {
    v.extend_from_slice(&(n as u16).to_be_bytes());
}
fn extension(v: &mut Vec<u8>, kind: u16, body: &[u8]) {
    v.extend_from_slice(&kind.to_be_bytes());
    push_u16(v, body.len());
    v.extend_from_slice(body);
}
impl Client {
    /// Start a handshake with `host` at Unix time `now`; `random` holds the
    /// client random (32 bytes), the X25519 secret (32) and the legacy
    /// session id (32), all from the system entropy source. Returns the
    /// ClientHello record to send.
    pub fn new(host: &str, now: i64, random: [u8; 96]) -> (Self, Vec<u8>) {
        Self::with_policy(host, now, random, verify::Policy::default())
    }
    /// `new`, accepting the certificate problems `policy` names (the
    /// handshake still records them: `problems`).
    pub fn with_policy(
        host: &str,
        now: i64,
        random: [u8; 96],
        policy: verify::Policy,
    ) -> (Self, Vec<u8>) {
        let secret =
            x25519_dalek::StaticSecret::from(<[u8; 32]>::try_from(&random[32..64]).unwrap());
        let public = x25519_dalek::PublicKey::from(&secret);
        let mut ext = Vec::new();
        // server_name
        let mut sni = Vec::new();
        push_u16(&mut sni, host.len() + 3);
        sni.push(0);
        push_u16(&mut sni, host.len());
        sni.extend_from_slice(host.as_bytes());
        extension(&mut ext, 0x0000, &sni);
        // supported_groups: X25519, then P-256 (TLS 1.2's ECDHE only; the
        // TLS 1.3 key share is X25519).
        extension(&mut ext, 0x000A, &[0x00, 0x04, 0x00, 0x1D, 0x00, 0x17]);
        // ec_point_formats (uncompressed), extended_master_secret and an
        // empty renegotiation_info, for TLS 1.2 servers.
        extension(&mut ext, 0x000B, &[0x01, 0x00]);
        extension(&mut ext, 0x0017, &[]);
        extension(&mut ext, 0xFF01, &[0x00]);
        extension(
            &mut ext,
            0x000D,
            &[
                0x00, 0x10, 0x04, 0x03, 0x05, 0x03, 0x08, 0x04, 0x08, 0x05, 0x08, 0x06, 0x04, 0x01,
                0x05, 0x01, 0x06, 0x01,
            ],
        );
        extension(
            &mut ext,
            0x0010,
            &[
                0x00, 0x09, 0x08, b'h', b't', b't', b'p', b'/', b'1', b'.', b'1',
            ],
        );
        // supported_versions: TLS 1.3, then 1.2.
        extension(&mut ext, 0x002B, &[0x04, 0x03, 0x04, 0x03, 0x03]);
        let mut share = alloc::vec![0x00, 0x24, 0x00, 0x1D, 0x00, 0x20];
        share.extend_from_slice(public.as_bytes());
        extension(&mut ext, 0x0033, &share);
        let mut body = alloc::vec![0x03, 0x03];
        body.extend_from_slice(&random[..32]);
        body.push(32);
        body.extend_from_slice(&random[64..96]);
        // The TLS 1.3 suites, then TLS 1.2's; no compression.
        let mut suites = alloc::vec![0x13, 0x01, 0x13, 0x02, 0x13, 0x03];
        for id in tls12::SUITES {
            suites.extend_from_slice(&id.to_be_bytes());
        }
        push_u16(&mut body, suites.len());
        body.extend_from_slice(&suites);
        body.extend_from_slice(&[0x01, 0x00]);
        push_u16(&mut body, ext.len());
        body.extend_from_slice(&ext);
        let mut hello = alloc::vec![
            1u8,
            (body.len() >> 16) as u8,
            (body.len() >> 8) as u8,
            body.len() as u8
        ];
        hello.extend_from_slice(&body);
        let mut record = alloc::vec![0x16, 0x03, 0x01];
        push_u16(&mut record, hello.len());
        record.extend_from_slice(&hello);
        let client = Self {
            host: host.into(),
            now,
            secret,
            state: State::ServerHello,
            suite: Suite::Aes128,
            transcript: hello,
            incoming: Vec::new(),
            handshake: Vec::new(),
            read: None,
            write: None,
            client_hs: Vec::new(),
            server_hs: Vec::new(),
            client_ap: Vec::new(),
            server_ap: Vec::new(),
            handshake_secret: Vec::new(),
            certificates: Vec::new(),
            policy,
            problems: verify::Problems::default(),
            certificate_requested: false,
            session_id: <[u8; 32]>::try_from(&random[64..96]).unwrap(),
            client_random: <[u8; 32]>::try_from(&random[..32]).unwrap(),
            p256: {
                // Derived from the X25519 secret, which is never revealed;
                // a connection uses one of the two.
                use sha2::Digest;
                let mut h = sha2::Sha256::new();
                h.update(b"door-tls P-256 ECDHE");
                h.update(&random[32..64]);
                h.finalize().into()
            },
            legacy: None,
            plaintext: Vec::new(),
            out: Vec::new(),
        };
        (client, record)
    }
    pub fn connected(&self) -> bool {
        self.state == State::Connected
    }
    /// The server certificate's problems found during the handshake
    /// (accepted ones included).
    pub fn problems(&self) -> verify::Problems {
        self.problems
    }
    /// The SHA-256 of the server's leaf certificate (DER), once received:
    /// what a client pins for a self-signed server (NoneDrive's home
    /// server, shown to the user as a fingerprint before it is trusted).
    pub fn leaf_sha256(&self) -> Option<[u8; 32]> {
        use sha2::Digest;
        self.certificates
            .first()
            .map(|leaf| sha2::Sha256::digest(leaf).into())
    }
    pub fn closed(&self) -> bool {
        self.state == State::Closed
    }
    /// Feed received bytes; returns bytes to send (the client Finished,
    /// KeyUpdate replies).
    pub fn receive(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        if self.state == State::Closed {
            return Err(Error::Closed);
        }
        self.incoming.extend_from_slice(bytes);
        loop {
            if self.incoming.len() < 5 {
                break;
            }
            let len = u16_at(&self.incoming, 3)?;
            if len > 16384 + 256 {
                return Err(Error::Protocol);
            }
            if self.incoming.len() < 5 + len {
                break;
            }
            let record: Vec<u8> = self.incoming.drain(..5 + len).collect();
            let (header, body) = record.split_at(5);
            if self.legacy.is_some() {
                if self.legacy_record(header[0], body)? {
                    break;
                }
                continue;
            }
            match header[0] {
                // Middlebox-compatibility ChangeCipherSpec: one byte, during
                // the handshake only.
                0x14 if self.state != State::Connected && body == [1] => (),
                0x15 if self.read.is_none() => {
                    return Err(Error::Alert(*body.get(1).unwrap_or(&0)));
                }
                0x16 if self.read.is_none() => {
                    self.handshake.extend_from_slice(body);
                    self.messages()?;
                }
                0x17 => {
                    let keys = self.read.as_mut().ok_or(Error::Protocol)?;
                    let (kind, content) = keys.open(header, body)?;
                    match kind {
                        0x16 => {
                            self.handshake.extend_from_slice(&content);
                            self.messages()?;
                        }
                        0x17 if self.state == State::Connected => {
                            self.plaintext.extend_from_slice(&content)
                        }
                        0x15 => {
                            if content.get(1) == Some(&0) {
                                self.state = State::Closed;
                                break;
                            }
                            return Err(Error::Alert(*content.get(1).unwrap_or(&0)));
                        }
                        _ => return Err(Error::Protocol),
                    }
                }
                _ => return Err(Error::Protocol),
            }
        }
        Ok(core::mem::take(&mut self.out))
    }
    /// Decrypted application data received so far.
    pub fn take_plaintext(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.plaintext)
    }
    /// Encrypt application data into records to send.
    pub fn send(&mut self, data: &[u8]) -> Result<Vec<u8>, Error> {
        if self.state != State::Connected {
            return Err(Error::Closed);
        }
        let mut out = Vec::new();
        if let Some(session) = self.legacy.as_mut() {
            let keys = session.write.as_mut().ok_or(Error::Closed)?;
            for chunk in data.chunks(16384) {
                out.extend_from_slice(&keys.seal(0x17, chunk));
            }
            return Ok(out);
        }
        let keys = self.write.as_mut().ok_or(Error::Closed)?;
        for chunk in data.chunks(16384) {
            out.extend_from_slice(&keys.seal(0x17, chunk));
        }
        Ok(out)
    }
    /// A close_notify alert to send before closing the socket.
    pub fn close(&mut self) -> Vec<u8> {
        let out = match self.legacy.as_mut() {
            Some(session) => session.write.as_mut().map(|k| k.seal(0x15, &[1, 0])),
            None => self.write.as_mut().map(|k| k.seal(0x15, &[1, 0])),
        }
        .unwrap_or_default();
        self.state = State::Closed;
        out
    }
    /// The negotiated protocol: 0x0304 (TLS 1.3) or 0x0303 (TLS 1.2).
    pub fn version(&self) -> u16 {
        if self.legacy.is_some() {
            0x0303
        } else {
            0x0304
        }
    }
    /// One TLS 1.2 record; true when the peer closed the connection.
    fn legacy_record(&mut self, kind: u8, body: &[u8]) -> Result<bool, Error> {
        let session = self.legacy.as_mut().ok_or(Error::Protocol)?;
        let content = match session.read.as_mut() {
            Some(keys) if kind != 0x14 => keys.open(kind, body)?,
            _ => body.to_vec(),
        };
        match kind {
            0x14 if content == [1] => session.change_cipher_spec()?,
            0x15 => {
                if content.get(1) == Some(&0) {
                    self.state = State::Closed;
                    return Ok(true);
                }
                // Warnings other than close_notify (no_renegotiation and
                // the like) do not end the connection.
                if content.first() == Some(&1) {
                    return Ok(false);
                }
                return Err(Error::Alert(*content.get(1).unwrap_or(&0)));
            }
            0x16 => {
                self.handshake.extend_from_slice(&content);
                self.messages()?;
            }
            0x17 if self.state == State::Connected => self.plaintext.extend_from_slice(&content),
            _ => return Err(Error::Protocol),
        }
        Ok(false)
    }
    /// A TLS 1.2 handshake message.
    fn legacy_message(&mut self, kind: u8, raw: &[u8]) -> Result<(), Error> {
        use tls12::Step;
        let body = &raw[4..];
        let step = self.legacy.as_ref().ok_or(Error::Protocol)?.step;
        match (step, kind) {
            (Step::Certificate, 11) => {
                // TLS 1.2: a list of certificates, no context or extensions.
                let total = u24_at(body, 0)?;
                if 3 + total != body.len() {
                    return Err(Error::Protocol);
                }
                let mut at = 3;
                while at < body.len() {
                    let len = u24_at(body, at)?;
                    let cert = body.get(at + 3..at + 3 + len).ok_or(Error::Protocol)?;
                    self.certificates.push(cert.to_vec());
                    at += 3 + len;
                }
                if self.certificates.is_empty() {
                    return Err(Error::Certificate(CertError::Malformed));
                }
                self.check_chain()?;
                let s = self.legacy.as_mut().unwrap();
                s.transcript.extend_from_slice(raw);
                s.step = Step::KeyExchange;
            }
            (Step::KeyExchange, 12) => {
                let leaf = self.certificates[0].clone();
                let s = self.legacy.as_mut().unwrap();
                s.key_exchange(body, &leaf)?;
                s.transcript.extend_from_slice(raw);
                s.step = Step::HelloDone;
            }
            (Step::HelloDone, 13) => {
                let s = self.legacy.as_mut().unwrap();
                s.certificate_requested();
                s.transcript.extend_from_slice(raw);
            }
            (Step::HelloDone, 14) if body.is_empty() => {
                let s = self.legacy.as_mut().unwrap();
                s.transcript.extend_from_slice(raw);
                let flight = s.client_flight(&self.secret, &self.p256)?;
                self.out.extend_from_slice(&flight);
            }
            (Step::Finished, 20) => {
                self.legacy.as_mut().unwrap().finished(body)?;
                self.state = State::Connected;
            }
            // HelloRequest: renegotiation is not offered; ignore it.
            (Step::Connected, 0) if body.is_empty() => (),
            _ => return Err(Error::Protocol),
        }
        Ok(())
    }
    /// The server's certificate chain against the policy (both versions).
    fn check_chain(&mut self) -> Result<(), Error> {
        let chain: Vec<&[u8]> = self.certificates.iter().map(Vec::as_slice).collect();
        if self.policy == verify::Policy::default() {
            let checked = verify::chain(&chain, &self.host, self.now);
            if checked.is_err() {
                self.problems = verify::assess(&chain, &self.host, self.now);
            }
            checked.map_err(Error::Certificate)
        } else {
            self.problems = verify::assess(&chain, &self.host, self.now);
            match self.problems.refused(self.policy) {
                Some(refused) => Err(Error::Certificate(refused)),
                None => Ok(()),
            }
        }
    }
    fn messages(&mut self) -> Result<(), Error> {
        while self.handshake.len() >= 4 {
            let len = u24_at(&self.handshake, 1)?;
            if len > 1 << 17 {
                return Err(Error::Protocol);
            }
            if self.handshake.len() < 4 + len {
                break;
            }
            let raw: Vec<u8> = self.handshake.drain(..4 + len).collect();
            let before = self.state;
            if self.legacy.is_some() {
                self.legacy_message(raw[0], &raw)?;
                continue;
            }
            self.message(raw[0], &raw)?;
            // ServerHello and Finished change keys: nothing may follow them
            // in the same record (RFC 8446 5.1).
            let rekeyed = matches!(
                (before, self.state),
                (State::ServerHello, State::EncryptedExtensions)
                    | (State::Finished, State::Connected)
            );
            if rekeyed && !self.handshake.is_empty() {
                return Err(Error::Protocol);
            }
        }
        Ok(())
    }
    fn message(&mut self, kind: u8, raw: &[u8]) -> Result<(), Error> {
        let body = &raw[4..];
        let suite = self.suite;
        match (self.state, kind) {
            (State::ServerHello, 2) => {
                if body.get(2..34) == Some(&HRR) {
                    return Err(Error::Unsupported);
                }
                let sid = usize::from(*body.get(34).ok_or(Error::Protocol)?);
                // TLS 1.3 echoes the legacy session id; a TLS 1.2 server
                // names its own session (checked once the version is known).
                let echoed = body.get(35..35 + sid) == Some(&self.session_id[..]);
                if sid > 32 || body.len() < 35 + sid {
                    return Err(Error::Protocol);
                }
                let mut at = 35 + sid;
                let suite_id = u16_at(body, at)? as u16;
                if body.get(at + 2) != Some(&0) {
                    return Err(Error::Protocol);
                }
                at += 3;
                // A TLS 1.2 ServerHello may end without extensions.
                let end = if at == body.len() {
                    at
                } else {
                    let ext_len = u16_at(body, at)?;
                    at += 2;
                    at + ext_len
                };
                if end != body.len() {
                    return Err(Error::Protocol);
                }
                let (mut version, mut share) = (false, None);
                let (mut extended_master, mut alpn_ok) = (false, true);
                let mut seen: Vec<usize> = Vec::new();
                while at + 4 <= end {
                    let kind = u16_at(body, at)?;
                    let len = u16_at(body, at + 2)?;
                    let data = body.get(at + 4..at + 4 + len).ok_or(Error::Protocol)?;
                    if seen.contains(&kind) {
                        return Err(Error::Protocol);
                    }
                    seen.push(kind);
                    match kind {
                        0x002B => version = data == [0x03, 0x04],
                        0x0033 => {
                            if u16_at(data, 0)? != 0x001D || u16_at(data, 2)? != 32 {
                                return Err(Error::Unsupported);
                            }
                            share = Some(
                                <[u8; 32]>::try_from(data.get(4..36).ok_or(Error::Protocol)?)
                                    .unwrap(),
                            );
                        }
                        // TLS 1.2's ServerHello extensions the ClientHello
                        // offered.
                        0x0000 if data.is_empty() => (),
                        0x000B => (),
                        0x0017 if data.is_empty() => extended_master = true,
                        0xFF01 if data == [0x00] => (),
                        0x0010 => {
                            alpn_ok = data
                                == [
                                    0x00, 0x09, 0x08, b'h', b't', b't', b'p', b'/', b'1', b'.',
                                    b'1',
                                ]
                        }
                        // Nothing else was offered for ServerHello.
                        _ => return Err(Error::Protocol),
                    }
                    at += 4 + len;
                }
                if !version {
                    // TLS 1.2, unless a TLS 1.3 server marks a downgrade.
                    let (Some((suite12, ecdsa)), true, true) = (
                        tls12::Suite12::from_id(suite_id),
                        body.get(..2) == Some(&[3, 3]),
                        alpn_ok,
                    ) else {
                        return Err(Error::Unsupported);
                    };
                    if body.get(26..34) == Some(&DOWNGRADE_12) || share.is_some() {
                        return Err(Error::Protocol);
                    }
                    let server_random: [u8; 32] = body[2..34].try_into().unwrap();
                    let mut transcript = core::mem::take(&mut self.transcript);
                    transcript.extend_from_slice(raw);
                    self.legacy = Some(alloc::boxed::Box::new(tls12::Session::new(
                        suite12,
                        ecdsa,
                        self.client_random,
                        server_random,
                        extended_master,
                        transcript,
                    )));
                    self.state = State::Legacy;
                    return Ok(());
                }
                let suite = Suite::from_id(suite_id).ok_or(Error::Unsupported)?;
                // TLS 1.3 sends its other extensions encrypted.
                if !echoed || seen.iter().any(|k| ![0x002B, 0x0033].contains(k)) {
                    return Err(Error::Protocol);
                }
                let peer = x25519_dalek::PublicKey::from(share.ok_or(Error::Protocol)?);
                let shared = self.secret.diffie_hellman(&peer);
                if shared.as_bytes().iter().all(|b| *b == 0) {
                    return Err(Error::Protocol);
                }
                self.suite = suite;
                self.transcript.extend_from_slice(raw);
                let zeros = alloc::vec![0u8; suite.hash_len()];
                let early = suite.extract(&zeros, &zeros);
                let derived = suite.derive(&early, "derived", &[]);
                self.handshake_secret = suite.extract(&derived, shared.as_bytes());
                self.client_hs =
                    suite.derive(&self.handshake_secret, "c hs traffic", &self.transcript);
                self.server_hs =
                    suite.derive(&self.handshake_secret, "s hs traffic", &self.transcript);
                self.read = Some(Keys::new(suite, &self.server_hs));
                self.state = State::EncryptedExtensions;
            }
            (State::EncryptedExtensions, 8) => {
                // Only extensions the ClientHello offered: server_name
                // (empty), supported_groups, ALPN (http/1.1).
                let total = u16_at(body, 0)?;
                if 2 + total != body.len() {
                    return Err(Error::Protocol);
                }
                let mut at = 2;
                let mut seen: Vec<usize> = Vec::new();
                while at < body.len() {
                    let kind = u16_at(body, at)?;
                    let len = u16_at(body, at + 2)?;
                    let data = body.get(at + 4..at + 4 + len).ok_or(Error::Protocol)?;
                    if seen.contains(&kind) {
                        return Err(Error::Protocol);
                    }
                    seen.push(kind);
                    match kind {
                        0x0000 if data.is_empty() => (),
                        0x000A => (),
                        0x0010
                            if data
                                == [
                                    0x00, 0x09, 0x08, b'h', b't', b't', b'p', b'/', b'1', b'.',
                                    b'1',
                                ] =>
                        {
                            // The ALPN the client offered (http/1.1).
                        }
                        _ => return Err(Error::Protocol),
                    }
                    at += 4 + len;
                }
                self.transcript.extend_from_slice(raw);
                self.state = State::Certificate;
            }
            (State::Certificate, 13) => {
                self.certificate_requested = true;
                self.transcript.extend_from_slice(raw);
            }
            (State::Certificate, 11) => {
                // A server's certificate_request_context is empty.
                if body.first() != Some(&0) {
                    return Err(Error::Protocol);
                }
                let mut at = 1;
                let total = u24_at(body, at)?;
                at += 3;
                let end = at + total;
                if end > body.len() {
                    return Err(Error::Protocol);
                }
                while at < end {
                    let len = u24_at(body, at)?;
                    let cert = body.get(at + 3..at + 3 + len).ok_or(Error::Protocol)?;
                    self.certificates.push(cert.to_vec());
                    at += 3 + len;
                    at += 2 + u16_at(body, at)?;
                }
                let chain: Vec<&[u8]> = self.certificates.iter().map(Vec::as_slice).collect();
                if self.policy == verify::Policy::default() {
                    let checked = verify::chain(&chain, &self.host, self.now);
                    if checked.is_err() {
                        self.problems = verify::assess(&chain, &self.host, self.now);
                    }
                    checked.map_err(Error::Certificate)?;
                } else {
                    self.problems = verify::assess(&chain, &self.host, self.now);
                    if let Some(refused) = self.problems.refused(self.policy) {
                        return Err(Error::Certificate(refused));
                    }
                }
                self.transcript.extend_from_slice(raw);
                self.state = State::CertificateVerify;
            }
            (State::CertificateVerify, 15) => {
                let alg = u16_at(body, 0)?;
                let len = u16_at(body, 2)?;
                let sig = body.get(4..4 + len).ok_or(Error::Protocol)?;
                let mut content = alloc::vec![0x20u8; 64];
                content.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
                content.extend_from_slice(&suite.hash(&self.transcript));
                let leaf = x509::parse(&self.certificates[0])
                    .ok_or(Error::Certificate(CertError::Malformed))?;
                let key = x509::spki(leaf.spki)
                    .ok_or(Error::Certificate(CertError::UnsupportedAlgorithm))?;
                let ok = match (alg, key.alg) {
                    (0x0403, x509::KeyAlg::P256) => {
                        verify::signature(&key, x509::SigAlg::EcdsaSha256, &content, sig).is_ok()
                    }
                    (0x0503, x509::KeyAlg::P384) => {
                        verify::signature(&key, x509::SigAlg::EcdsaSha384, &content, sig).is_ok()
                    }
                    (0x0804, x509::KeyAlg::Rsa) => {
                        rsa::verify_pss(key.key, rsa::Hash::Sha256, &content, sig)
                    }
                    (0x0805, x509::KeyAlg::Rsa) => {
                        rsa::verify_pss(key.key, rsa::Hash::Sha384, &content, sig)
                    }
                    (0x0806, x509::KeyAlg::Rsa) => {
                        rsa::verify_pss(key.key, rsa::Hash::Sha512, &content, sig)
                    }
                    _ => return Err(Error::Unsupported),
                };
                if !ok {
                    return Err(Error::Certificate(CertError::BadSignature));
                }
                self.transcript.extend_from_slice(raw);
                self.state = State::Finished;
            }
            (State::Finished, 20) => {
                let key = suite.expand_label(&self.server_hs, "finished", &[], suite.hash_len());
                let expected = suite.hmac(&key, &suite.hash(&self.transcript));
                if body.len() != expected.len()
                    || body
                        .iter()
                        .zip(&expected)
                        .fold(0u8, |a, (x, y)| a | (x ^ y))
                        != 0
                {
                    return Err(Error::Decrypt);
                }
                self.transcript.extend_from_slice(raw);
                let zeros = alloc::vec![0u8; suite.hash_len()];
                let derived = suite.derive(&self.handshake_secret, "derived", &[]);
                let master = suite.extract(&derived, &zeros);
                self.client_ap = suite.derive(&master, "c ap traffic", &self.transcript);
                self.server_ap = suite.derive(&master, "s ap traffic", &self.transcript);
                let mut write = Keys::new(suite, &self.client_hs);
                if self.certificate_requested {
                    let empty = [11u8, 0, 0, 4, 0, 0, 0, 0];
                    self.out.extend_from_slice(&write.seal(0x16, &empty));
                    self.transcript.extend_from_slice(&empty);
                }
                let key = suite.expand_label(&self.client_hs, "finished", &[], suite.hash_len());
                let verify = suite.hmac(&key, &suite.hash(&self.transcript));
                let mut finished = alloc::vec![20u8, 0, 0, verify.len() as u8];
                finished.extend_from_slice(&verify);
                self.out.extend_from_slice(&write.seal(0x16, &finished));
                self.read = Some(Keys::new(suite, &self.server_ap));
                self.write = Some(Keys::new(suite, &self.client_ap));
                self.transcript = Vec::new();
                self.state = State::Connected;
            }
            (State::Connected, 4) => (), // NewSessionTicket: no resumption
            (State::Connected, 24) => {
                if body.len() != 1 || body[0] > 1 {
                    return Err(Error::Protocol);
                }
                self.server_ap =
                    suite.expand_label(&self.server_ap, "traffic upd", &[], suite.hash_len());
                self.read = Some(Keys::new(suite, &self.server_ap));
                if body.first() == Some(&1) {
                    let w = self.write.as_mut().ok_or(Error::Protocol)?;
                    self.out.extend_from_slice(&w.seal(0x16, &[24, 0, 0, 1, 0]));
                    self.client_ap =
                        suite.expand_label(&self.client_ap, "traffic upd", &[], suite.hash_len());
                    self.write = Some(Keys::new(suite, &self.client_ap));
                }
            }
            _ => return Err(Error::Protocol),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_hello_is_well_formed() {
        let (c, hello) = Client::new("example.com", 1_800_000_000, [7; 96]);
        assert_eq!(&hello[..3], &[0x16, 0x03, 0x01]);
        assert_eq!(u16_at(&hello, 3).unwrap(), hello.len() - 5);
        assert_eq!(hello[5], 1);
        assert_eq!(u24_at(&hello, 6).unwrap(), hello.len() - 9);
        assert!(hello.windows(11).any(|w| w == b"example.com"));
        assert!(!c.connected());
    }
    #[test]
    fn key_schedule_matches_rfc8448_derived_secret() {
        // RFC 8448 §3: Derive-Secret(early, "derived", "") with SHA-256.
        let zeros = [0u8; 32];
        let early = Suite::Aes128.extract(&zeros, &zeros);
        assert_eq!(early[..4], [0x33, 0xad, 0x0a, 0x1c]);
        let derived = Suite::Aes128.derive(&early, "derived", &[]);
        assert_eq!(derived[..4], [0x6f, 0x26, 0x15, 0xa1]);
    }
    #[test]
    fn records_out_of_order_are_refused() {
        let (mut c, _) = Client::new("example.com", 0, [1; 96]);
        // Application data before any keys.
        assert_eq!(c.receive(&[0x17, 3, 3, 0, 2, 0, 0]), Err(Error::Protocol));
        let (mut c, _) = Client::new("example.com", 0, [1; 96]);
        // A handshake_failure alert.
        assert_eq!(c.receive(&[0x15, 3, 3, 0, 2, 2, 40]), Err(Error::Alert(40)));
    }
}
