//! CONNECT-A's HTTPS endpoints (P15): a minimal TLS 1.3 server (RFC 8446)
//! on this host's loopback, reached by the guest at 10.0.2.2, so WinHTTP's
//! secure path is exercised end to end: an X25519 key share,
//! TLS_AES_128_GCM_SHA256 and a self-signed ECDSA P-256 certificate.
//! Three listeners present three certificates for 127.0.0.1 and 10.0.2.2:
//! a valid one, one for another host name and an expired one. None chains
//! to a trusted root, so a client must refuse each unless told to ignore
//! that error (WinHTTP's SECURITY_FLAG_IGNORE_*). Test-only: the key and
//! the server's randomness are derived deterministically, nothing else
//! (resumption, HelloRetryRequest, client certificates) is offered, and
//! every connection answers one HTTP/1.1 request with `BODY`.
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// What every HTTPS request gets.
pub const BODY: &str = "hello https";

/// The three listeners: `good` (valid for 127.0.0.1 and 10.0.2.2),
/// `wrong_host` (for elsewhere.doors.test only) and `expired` (2020-2021).
pub struct Server {
    pub good: u16,
    pub wrong_host: u16,
    pub expired: u16,
    stop: Arc<AtomicBool>,
    pub log: Arc<Mutex<Vec<String>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// ---- the certificates ------------------------------------------------------

const ECDSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const PRIME256V1: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];
const KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x0f];
const EXTENDED_KEY_USAGE: &[u8] = &[0x55, 0x1d, 0x25];
const BASIC_CONSTRAINTS: &[u8] = &[0x55, 0x1d, 0x13];
const SERVER_AUTH: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend([0x81, n as u8]);
    } else {
        out.extend([0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}
fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    der(0x30, &parts.concat())
}
fn oid(id: &[u8]) -> Vec<u8> {
    der(0x06, id)
}
fn extension(id: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let mut parts = oid(id);
    if critical {
        parts.extend(der(0x01, &[0xff]));
    }
    parts.extend(der(0x04, value));
    der(0x30, &parts)
}
/// The test key (deterministic: the certificates are reproducible).
fn key() -> SigningKey {
    SigningKey::from_bytes(&Sha256::digest(b"Doors CONNECT-A TLS test key"))
        .expect("a P-256 scalar")
}
/// A self-signed X.509 v3 certificate for `names` and `addresses`, valid
/// from `not_before` to `not_after` (UTCTime, YYMMDDhhmmssZ), for TLS
/// server authentication.
fn certificate(
    key: &SigningKey,
    serial: u8,
    names: &[&str],
    addresses: &[[u8; 4]],
    not_before: &str,
    not_after: &str,
) -> Vec<u8> {
    let algorithm = sequence(&[&oid(ECDSA_SHA256)]);
    let name = sequence(&[&der(
        0x31,
        &sequence(&[&oid(COMMON_NAME), &der(0x0c, b"Doors TLS Test")]),
    )]);
    let validity = sequence(&[
        &der(0x17, not_before.as_bytes()),
        &der(0x17, not_after.as_bytes()),
    ]);
    let point = key.verifying_key().to_encoded_point(false);
    let mut bits = vec![0];
    bits.extend_from_slice(point.as_bytes());
    let public_key = sequence(&[
        &sequence(&[&oid(EC_PUBLIC_KEY), &oid(PRIME256V1)]),
        &der(0x03, &bits),
    ]);
    let mut alternatives = Vec::new();
    for name in names {
        alternatives.extend(der(0x82, name.as_bytes()));
    }
    for address in addresses {
        alternatives.extend(der(0x87, address));
    }
    let extensions = der(
        0xa3,
        &sequence(&[
            &extension(SUBJECT_ALT_NAME, false, &der(0x30, &alternatives)),
            // digitalSignature.
            &extension(KEY_USAGE, true, &[0x03, 0x02, 0x07, 0x80]),
            &extension(EXTENDED_KEY_USAGE, false, &sequence(&[&oid(SERVER_AUTH)])),
            // Not a certification authority.
            &extension(BASIC_CONSTRAINTS, true, &[0x30, 0x00]),
        ]),
    );
    let tbs = sequence(&[
        &[0xa0, 0x03, 0x02, 0x01, 0x02],
        &der(0x02, &[serial]),
        &algorithm,
        &name,
        &validity,
        &name,
        &public_key,
        &extensions,
    ]);
    let signature: Signature = key.sign(&tbs);
    let mut signature_bits = vec![0];
    signature_bits.extend_from_slice(signature.to_der().as_bytes());
    sequence(&[&tbs, &algorithm, &der(0x03, &signature_bits)])
}
const LOOPBACK: [u8; 4] = [127, 0, 0, 1];
const GUEST_HOST: [u8; 4] = [10, 0, 2, 2];

// ---- TLS 1.3 ----------------------------------------------------------------

fn extract(salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    Hkdf::<Sha256>::extract(Some(salt), ikm).0.to_vec()
}
fn expand_label(secret: &[u8], label: &str, context: &[u8], length: usize) -> Vec<u8> {
    let full = format!("tls13 {label}");
    let mut info = Vec::new();
    info.extend((length as u16).to_be_bytes());
    info.push(full.len() as u8);
    info.extend(full.as_bytes());
    info.push(context.len() as u8);
    info.extend(context);
    let mut out = vec![0; length];
    Hkdf::<Sha256>::from_prk(secret)
        .expect("a SHA-256 secret")
        .expand(&info, &mut out)
        .expect("a short label");
    out
}
fn hash(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}
fn finished(secret: &[u8], transcript: &[u8]) -> Vec<u8> {
    let key = expand_label(secret, "finished", &[], 32);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("an HMAC key");
    mac.update(&hash(transcript));
    mac.finalize().into_bytes().to_vec()
}
/// One direction's record protection.
struct Protection {
    cipher: aes_gcm::Aes128Gcm,
    iv: Vec<u8>,
    sequence: u64,
}
impl Protection {
    fn new(secret: &[u8]) -> Self {
        use aes_gcm::KeyInit;
        let key = expand_label(secret, "key", &[], 16);
        Self {
            cipher: aes_gcm::Aes128Gcm::new_from_slice(&key).expect("an AES-128 key"),
            iv: expand_label(secret, "iv", &[], 12),
            sequence: 0,
        }
    }
    fn nonce(&mut self) -> aes_gcm::Nonce<aes_gcm::aead::consts::U12> {
        let mut nonce = self.iv.clone();
        for (n, s) in nonce[4..].iter_mut().zip(self.sequence.to_be_bytes()) {
            *n ^= s;
        }
        self.sequence += 1;
        *aes_gcm::Nonce::from_slice(&nonce)
    }
    /// A protected record carrying `data` of content type `kind`.
    fn seal(&mut self, kind: u8, data: &[u8]) -> Vec<u8> {
        use aes_gcm::aead::{Aead, Payload};
        let mut plain = data.to_vec();
        plain.push(kind);
        let length = plain.len() + 16;
        let header = [23, 3, 3, (length >> 8) as u8, length as u8];
        let nonce = self.nonce();
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: &plain,
                    aad: &header,
                },
            )
            .expect("AES-GCM sealing");
        [&header[..], &sealed].concat()
    }
    /// A protected record's content type and data.
    fn open(&mut self, header: &[u8; 5], body: &[u8]) -> Result<(u8, Vec<u8>), String> {
        use aes_gcm::aead::{Aead, Payload};
        let nonce = self.nonce();
        let mut plain = self
            .cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: body,
                    aad: header,
                },
            )
            .map_err(|_| String::from("a record failed authentication"))?;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        let kind = plain.pop().ok_or("an empty protected record")?;
        Ok((kind, plain))
    }
}
fn read_record(stream: &mut TcpStream) -> Result<([u8; 5], Vec<u8>), String> {
    let mut header = [0; 5];
    stream
        .read_exact(&mut header)
        .map_err(|e| format!("record header: {e}"))?;
    let length = usize::from(u16::from_be_bytes([header[3], header[4]]));
    if length > 16384 + 256 {
        return Err(format!("record of {length} bytes"));
    }
    let mut body = vec![0; length];
    stream
        .read_exact(&mut body)
        .map_err(|e| format!("record body: {e}"))?;
    Ok((header, body))
}
fn handshake_message(kind: u8, body: &[u8]) -> Vec<u8> {
    let n = body.len();
    let mut out = vec![kind, (n >> 16) as u8, (n >> 8) as u8, n as u8];
    out.extend_from_slice(body);
    out
}
/// The parts of a ClientHello this server needs.
struct Hello {
    random: Vec<u8>,
    session: Vec<u8>,
    share: [u8; 32],
}
fn client_hello(message: &[u8]) -> Result<Hello, String> {
    let bad = || String::from("a malformed ClientHello");
    let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
        let end = at.checked_add(n).ok_or_else(bad)?;
        let part = message.get(*at..end).ok_or_else(bad)?;
        *at = end;
        Ok(part)
    };
    let mut at = 4;
    take(&mut at, 2)?;
    let random = take(&mut at, 32)?.to_vec();
    let n = usize::from(take(&mut at, 1)?[0]);
    let session = take(&mut at, n)?.to_vec();
    let n = usize::from(u16::from_be_bytes(take(&mut at, 2)?.try_into().unwrap()));
    let suites = take(&mut at, n)?;
    if !suites.chunks(2).any(|s| s == [0x13, 0x01]) {
        return Err("no TLS_AES_128_GCM_SHA256".into());
    }
    let n = usize::from(take(&mut at, 1)?[0]);
    take(&mut at, n)?;
    let n = usize::from(u16::from_be_bytes(take(&mut at, 2)?.try_into().unwrap()));
    let end = at + n;
    let (mut tls13, mut share) = (false, None);
    while at < end {
        let kind = u16::from_be_bytes(take(&mut at, 2)?.try_into().unwrap());
        let n = usize::from(u16::from_be_bytes(take(&mut at, 2)?.try_into().unwrap()));
        let data = take(&mut at, n)?;
        match kind {
            // supported_versions
            0x2b => {
                tls13 = data
                    .get(1..)
                    .is_some_and(|v| v.chunks(2).any(|v| v == [3, 4]))
            }
            // key_share
            0x33 => {
                let mut i = 2;
                while i + 4 <= data.len() {
                    let group = u16::from_be_bytes([data[i], data[i + 1]]);
                    let length = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
                    let key = data.get(i + 4..i + 4 + length).ok_or_else(bad)?;
                    if group == 0x1d && length == 32 {
                        share = Some(<[u8; 32]>::try_from(key).unwrap());
                    }
                    i += 4 + length;
                }
            }
            _ => {}
        }
    }
    if !tls13 {
        return Err("TLS 1.3 not offered".into());
    }
    Ok(Hello {
        random,
        session,
        share: share.ok_or("no X25519 key share")?,
    })
}
/// One connection: the handshake with `chain`'s certificate, one request,
/// `BODY`, close_notify. What happened goes to the log.
fn serve_connection(mut stream: TcpStream, certificate: &[u8], key: &SigningKey) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    match exchange(&mut stream, certificate, key) {
        Ok(line) => line,
        Err(e) => e,
    }
}
fn exchange(
    stream: &mut TcpStream,
    certificate: &[u8],
    key: &SigningKey,
) -> Result<String, String> {
    // The ClientHello, perhaps over several records.
    let mut hello = Vec::new();
    loop {
        let (header, body) = read_record(stream)?;
        if header[0] != 22 {
            return Err(format!("record type {} before the ClientHello", header[0]));
        }
        hello.extend(body);
        if hello.len() >= 4 {
            let n = (usize::from(hello[1]) << 16)
                | (usize::from(hello[2]) << 8)
                | usize::from(hello[3]);
            if hello.len() >= 4 + n {
                hello.truncate(4 + n);
                break;
            }
        }
    }
    if hello[0] != 1 {
        return Err("not a ClientHello".into());
    }
    let client = client_hello(&hello)?;
    // Test-only randomness, derived from the client's.
    let secret_bytes: [u8; 32] =
        Sha256::digest([b"share".as_slice(), &client.random].concat()).into();
    let secret = x25519_dalek::StaticSecret::from(secret_bytes);
    let public = x25519_dalek::PublicKey::from(&secret);
    let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(client.share));
    let random = Sha256::digest([b"random".as_slice(), &client.random].concat());
    let mut body = vec![3, 3];
    body.extend_from_slice(&random);
    body.push(client.session.len() as u8);
    body.extend_from_slice(&client.session);
    body.extend([0x13, 0x01, 0]);
    let mut extensions = vec![0, 0x2b, 0, 2, 3, 4, 0, 0x33, 0, 36, 0, 0x1d, 0, 32];
    extensions.extend_from_slice(public.as_bytes());
    body.extend((extensions.len() as u16).to_be_bytes());
    body.extend(extensions);
    let server_hello = handshake_message(2, &body);
    let mut transcript = [hello.clone(), server_hello.clone()].concat();
    let mut out = vec![22, 3, 3];
    out.extend((server_hello.len() as u16).to_be_bytes());
    out.extend(&server_hello);
    // Middlebox compatibility (RFC 8446 D.4).
    out.extend([20, 3, 3, 0, 1, 1]);
    let zero = [0u8; 32];
    let early = extract(&zero, &zero);
    let derived = expand_label(&early, "derived", &hash(&[]), 32);
    let handshake = extract(&derived, shared.as_bytes());
    let client_secret = expand_label(&handshake, "c hs traffic", &hash(&transcript), 32);
    let server_secret = expand_label(&handshake, "s hs traffic", &hash(&transcript), 32);
    let mut sealing = Protection::new(&server_secret);
    let mut opening = Protection::new(&client_secret);
    let encrypted_extensions = handshake_message(8, &[0, 0]);
    let mut list = vec![
        (certificate.len() >> 16) as u8,
        (certificate.len() >> 8) as u8,
        certificate.len() as u8,
    ];
    list.extend_from_slice(certificate);
    list.extend([0, 0]);
    let mut body = vec![
        0,
        (list.len() >> 16) as u8,
        (list.len() >> 8) as u8,
        list.len() as u8,
    ];
    body.extend(list);
    let certificate_message = handshake_message(11, &body);
    transcript.extend(&encrypted_extensions);
    transcript.extend(&certificate_message);
    let mut signed = vec![0x20; 64];
    signed.extend(b"TLS 1.3, server CertificateVerify\0");
    signed.extend(hash(&transcript));
    let signature: Signature = key.sign(&signed);
    let signature = signature.to_der();
    let mut body = vec![4, 3];
    body.extend((signature.as_bytes().len() as u16).to_be_bytes());
    body.extend(signature.as_bytes());
    let verify = handshake_message(15, &body);
    transcript.extend(&verify);
    let server_finished = handshake_message(20, &finished(&server_secret, &transcript));
    transcript.extend(&server_finished);
    let flight = [
        encrypted_extensions,
        certificate_message,
        verify,
        server_finished,
    ]
    .concat();
    out.extend(sealing.seal(22, &flight));
    stream
        .write_all(&out)
        .map_err(|e| format!("server flight: {e}"))?;
    let master_derived = expand_label(&handshake, "derived", &hash(&[]), 32);
    let master = extract(&master_derived, &zero);
    let client_application = expand_label(&master, "c ap traffic", &hash(&transcript), 32);
    let server_application = expand_label(&master, "s ap traffic", &hash(&transcript), 32);
    // The client's Finished (after its compatibility ChangeCipherSpec), or
    // the alert that refused the certificate.
    loop {
        let (header, body) = read_record(stream)?;
        match header[0] {
            20 => continue,
            21 => return Ok(format!("plain alert {body:02x?}")),
            23 => {}
            other => return Err(format!("record type {other} in the handshake")),
        }
        let (kind, message) = opening.open(&header, &body)?;
        if kind == 21 {
            return Ok(format!("alert {message:02x?}"));
        }
        if kind != 22 || message.first() != Some(&20) {
            return Err(format!("expected the client's Finished, got {kind}"));
        }
        if message[4..] != finished(&client_secret, &transcript)[..] {
            return Err("the client's Finished does not verify".into());
        }
        break;
    }
    let mut opening = Protection::new(&client_application);
    let mut sealing = Protection::new(&server_application);
    let mut request = Vec::new();
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let (header, body) = read_record(stream)?;
        if header[0] != 23 {
            return Err(format!("record type {} after the handshake", header[0]));
        }
        let (kind, data) = opening.open(&header, &body)?;
        match kind {
            23 => request.extend(data),
            21 => return Ok(format!("alert after the handshake {data:02x?}")),
            // NewSessionTicket or KeyUpdate from a client is not expected.
            other => return Err(format!("content type {other} after the handshake")),
        }
        if request.len() > 8192 {
            return Err("an oversized request".into());
        }
    }
    let line = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .unwrap_or("")
        .to_owned();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
        BODY.len()
    );
    let mut out = sealing.seal(23, response.as_bytes());
    out.extend(sealing.seal(21, &[1, 0]));
    stream
        .write_all(&out)
        .map_err(|e| format!("response: {e}"))?;
    let _ = stream.flush();
    Ok(format!("served {line}"))
}

/// Serves until dropped.
pub fn serve() -> Result<Server, String> {
    let key = key();
    let certificates = [
        (
            "good",
            certificate(
                &key,
                1,
                &["tls.doors.test"],
                &[LOOPBACK, GUEST_HOST],
                "250101000000Z",
                "491231235959Z",
            ),
        ),
        (
            "wrong-host",
            certificate(
                &key,
                2,
                &["elsewhere.doors.test"],
                &[],
                "250101000000Z",
                "491231235959Z",
            ),
        ),
        (
            "expired",
            certificate(
                &key,
                3,
                &["tls.doors.test"],
                &[LOOPBACK, GUEST_HOST],
                "200101000000Z",
                "210101000000Z",
            ),
        ),
    ];
    let stop = Arc::new(AtomicBool::new(false));
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut ports = Vec::new();
    for (name, certificate) in certificates {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        ports.push(listener.local_addr().map_err(|e| e.to_string())?.port());
        let (s, l, key) = (stop.clone(), log.clone(), key.clone());
        let certificate = Arc::new(certificate);
        thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let (l, key, certificate) = (l.clone(), key.clone(), certificate.clone());
                        thread::spawn(move || {
                            let line = serve_connection(stream, &certificate, &key);
                            if let Ok(mut log) = l.lock() {
                                log.push(format!("{name}: {line}"));
                            }
                        });
                    }
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        });
    }
    Ok(Server {
        good: ports[0],
        wrong_host: ports[1],
        expired: ports[2],
        stop,
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_tls::{CertError, Client, Error, verify::Policy};
    /// Doors' TLS client against one listener: the body, or the error.
    fn fetch(port: u16, policy: Policy) -> Result<String, Error> {
        let now = 1_794_400_000; // 11 November 2026
        let mut random = [0u8; 96];
        random
            .iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = i as u8 ^ port as u8);
        let (mut client, hello) = Client::with_policy("127.0.0.1", now, random, policy);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(&hello).unwrap();
        let mut sent = false;
        let mut body = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = stream.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            let reply = client.receive(&buffer[..n])?;
            stream.write_all(&reply).unwrap();
            body.extend(client.take_plaintext());
            if client.connected() && !sent {
                let request = client.send(b"GET /hello HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")?;
                stream.write_all(&request).unwrap();
                sent = true;
            }
            if client.closed() {
                break;
            }
        }
        assert_eq!(client.version(), 0x0304);
        let text = String::from_utf8_lossy(&body).into_owned();
        Ok(text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned())
    }
    #[test]
    fn doors_client_completes_the_handshake_and_refuses_each_certificate() {
        let server = serve().unwrap();
        let unknown = Policy {
            unknown_ca: true,
            ..Policy::default()
        };
        assert_eq!(fetch(server.good, unknown).unwrap(), BODY);
        assert_eq!(
            fetch(server.good, Policy::default()),
            Err(Error::Certificate(CertError::Untrusted))
        );
        assert_eq!(
            fetch(server.wrong_host, unknown),
            Err(Error::Certificate(CertError::WrongHost))
        );
        let host = Policy {
            host: true,
            ..unknown
        };
        assert_eq!(fetch(server.wrong_host, host).unwrap(), BODY);
        assert_eq!(
            fetch(server.expired, unknown),
            Err(Error::Certificate(CertError::Expired))
        );
        let date = Policy {
            date: true,
            ..unknown
        };
        assert_eq!(fetch(server.expired, date).unwrap(), BODY);
    }
    #[test]
    fn the_certificates_parse() {
        let key = key();
        let der = certificate(
            &key,
            1,
            &["tls.doors.test"],
            &[LOOPBACK],
            "250101000000Z",
            "491231235959Z",
        );
        assert!(door_tls::x509::parse(&der).is_some());
    }
    #[test]
    fn the_key_schedule_matches_rfc_8448() {
        // RFC 8448 section 3: the early secret of no PSK, then "derived".
        let zero = [0u8; 32];
        let early = extract(&zero, &zero);
        assert_eq!(early[..4], [0x33, 0xad, 0x0a, 0x1c]);
        let derived = expand_label(&early, "derived", &hash(&[]), 32);
        assert_eq!(derived[..4], [0x6f, 0x26, 0x15, 0xa1]);
    }
}
