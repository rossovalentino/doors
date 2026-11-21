//! X.509 certificates as CryptoAPI presents them: CERT_INFO's fields from
//! the DER encoding, the CERT_CONTEXT block crypt32 hands out (built for
//! either pointer width, with absolute pointers into the block itself), and
//! Name rendering for CertGetNameString.
use alloc::{string::String, vec::Vec};
use door_tls::der::{self, Reader};

/// X509_ASN_ENCODING | PKCS_7_ASN_ENCODING.
pub const ENCODING: u32 = 0x0001_0001;
/// Seconds between 1601-01-01 and 1970-01-01.
const EPOCH_1601: i64 = 11_644_473_600;

pub struct Extension<'a> {
    pub oid: String,
    pub critical: bool,
    pub value: &'a [u8],
}
/// CERT_INFO's fields. Byte slices borrow the encoding.
pub struct Parts<'a> {
    pub version: u32,
    /// The serial number's INTEGER contents (big-endian).
    pub serial: &'a [u8],
    pub signature_oid: String,
    /// The encoded parameters (empty when absent).
    pub signature_params: &'a [u8],
    /// The encoded issuer and subject Names (CERT_NAME_BLOB).
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    /// FILETIMEs.
    pub not_before: u64,
    pub not_after: u64,
    pub key_oid: String,
    pub key_params: &'a [u8],
    pub key: &'a [u8],
    pub key_unused: u32,
    pub issuer_uid: Option<(&'a [u8], u32)>,
    pub subject_uid: Option<(&'a [u8], u32)>,
    pub extensions: Vec<Extension<'a>>,
}

/// The dotted text of an OBJECT IDENTIFIER's contents.
pub fn oid_text(bytes: &[u8]) -> Option<String> {
    use core::fmt::Write;
    let mut out = String::new();
    let mut value = 0u64;
    let mut first = true;
    for (i, b) in bytes.iter().enumerate() {
        if value > (u64::MAX >> 7) {
            return None;
        }
        value = (value << 7) | u64::from(b & 0x7F);
        if b & 0x80 != 0 {
            if i + 1 == bytes.len() {
                return None;
            }
            continue;
        }
        if first {
            let (a, rest) = match value {
                0..40 => (0, value),
                40..80 => (1, value - 40),
                _ => (2, value - 80),
            };
            let _ = write!(out, "{a}.{rest}");
            first = false;
        } else {
            let _ = write!(out, ".{value}");
        }
        value = 0;
    }
    (!first).then_some(out)
}
/// The contents of the OBJECT IDENTIFIER `text` names.
pub fn oid_der(text: &[u8]) -> Option<Vec<u8>> {
    let mut arcs = Vec::new();
    for part in text.split(|b| *b == b'.') {
        if part.is_empty() || !part.iter().all(u8::is_ascii_digit) || part.len() > 19 {
            return None;
        }
        arcs.push(part.iter().fold(0u64, |a, d| a * 10 + u64::from(d - b'0')));
    }
    if arcs.len() < 2 || arcs[0] > 2 || (arcs[0] < 2 && arcs[1] >= 40) {
        return None;
    }
    let mut out = Vec::new();
    let mut put = |mut v: u64| {
        let mut tmp = [0u8; 10];
        let mut n = 0;
        loop {
            tmp[n] = (v & 0x7F) as u8;
            n += 1;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            out.push(tmp[i] | if i > 0 { 0x80 } else { 0 });
        }
    };
    put(arcs[0] * 40 + arcs[1]);
    for a in &arcs[2..] {
        put(*a);
    }
    Some(out)
}
fn filetime(t: der::Tlv<'_>) -> Option<u64> {
    let seconds = der::time(t)?;
    u64::try_from((seconds + EPOCH_1601).checked_mul(10_000_000)?).ok()
}
/// An AlgorithmIdentifier's OID and encoded parameters.
fn algorithm(t: der::Tlv<'_>) -> Option<(String, &[u8])> {
    let mut r = Reader::new(t.value);
    let oid = oid_text(r.expect(der::OID)?.value)?;
    let params = r.read().map_or(&[][..], |p| p.raw);
    Some((oid, params))
}
/// A BIT STRING's bytes and unused-bit count.
fn bit_string(value: &[u8]) -> Option<(&[u8], u32)> {
    let (unused, data) = value.split_first()?;
    (*unused < 8).then_some((data, u32::from(*unused)))
}
/// Parse an encoded certificate.
pub fn parse(encoded: &[u8]) -> Option<Parts<'_>> {
    let mut outer = Reader::new(encoded);
    let certificate = outer.expect(der::SEQUENCE)?;
    let mut c = Reader::new(certificate.value);
    let tbs = c.expect(der::SEQUENCE)?;
    let mut t = Reader::new(tbs.value);
    let version = match t.optional(0xA0) {
        Some(v) => {
            let n = Reader::new(v.value).expect(der::INTEGER)?;
            u32::from(*n.value.last()?)
        }
        None => 0,
    };
    let serial = t.expect(der::INTEGER)?.value;
    let (signature_oid, signature_params) = algorithm(t.expect(der::SEQUENCE)?)?;
    let issuer = t.expect(der::SEQUENCE)?.raw;
    let mut validity = Reader::new(t.expect(der::SEQUENCE)?.value);
    let not_before = filetime(validity.read()?)?;
    let not_after = filetime(validity.read()?)?;
    let subject = t.expect(der::SEQUENCE)?.raw;
    let mut spki = Reader::new(t.expect(der::SEQUENCE)?.value);
    let (key_oid, key_params) = algorithm(spki.expect(der::SEQUENCE)?)?;
    let (key, key_unused) = bit_string(spki.expect(der::BIT_STRING)?.value)?;
    let issuer_uid = t.optional(0x81).and_then(|u| bit_string(u.value));
    let subject_uid = t.optional(0x82).and_then(|u| bit_string(u.value));
    let mut extensions = Vec::new();
    if let Some(list) = t.optional(0xA3) {
        let mut items = Reader::new(Reader::new(list.value).expect(der::SEQUENCE)?.value);
        while let Some(e) = items.read() {
            let mut f = Reader::new(e.value);
            let oid = oid_text(f.expect(der::OID)?.value)?;
            let critical = f.optional(der::BOOLEAN).is_some_and(|b| b.value == [0xFF]);
            let value = f.expect(der::OCTET_STRING)?.value;
            extensions.push(Extension {
                oid,
                critical,
                value,
            });
        }
    }
    Some(Parts {
        version,
        serial,
        signature_oid,
        signature_params,
        issuer,
        subject,
        not_before,
        not_after,
        key_oid,
        key_params,
        key,
        key_unused,
        issuer_uid,
        subject_uid,
        extensions,
    })
}
/// The keyUsage bytes of an extension value (the BIT STRING's data).
pub fn key_usage(value: &[u8]) -> Option<&[u8]> {
    let bits = Reader::new(value).expect(der::BIT_STRING)?;
    bit_string(bits.value).map(|(data, _)| data)
}

/// The SubjectPublicKeyInfo contents (no outer SEQUENCE) of a certificate.
pub fn spki_contents(encoded: &[u8]) -> Option<&[u8]> {
    let certificate = Reader::new(encoded).expect(der::SEQUENCE)?;
    let tbs = Reader::new(certificate.value).expect(der::SEQUENCE)?;
    let mut t = Reader::new(tbs.value);
    t.optional(0xA0);
    for _ in 0..5 {
        t.read()?;
    }
    Some(t.expect(der::SEQUENCE)?.value)
}
/// SubjectPublicKeyInfo contents from CERT_PUBLIC_KEY_INFO's parts.
pub fn spki_from(oid: &[u8], params: &[u8], key: &[u8], unused: u32) -> Option<Vec<u8>> {
    let oid = oid_der(oid)?;
    let mut algorithm = Vec::new();
    push_tlv(&mut algorithm, der::OID, &oid);
    algorithm.extend_from_slice(params);
    let mut out = Vec::new();
    push_tlv(&mut out, der::SEQUENCE, &algorithm);
    let mut bits = Vec::with_capacity(key.len() + 1);
    bits.push(u8::try_from(unused).ok()?);
    bits.extend_from_slice(key);
    push_tlv(&mut out, der::BIT_STRING, &bits);
    Some(out)
}
fn push_tlv(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag);
    let n = value.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes = n.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(value);
}
/// Why a signature check failed: the algorithm is unknown, or the
/// signature does not verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signature {
    Malformed,
    Algorithm,
    Bad,
}
/// Checks the signature of an encoded certificate (or any signed
/// SEQUENCE { tbs, algorithm, BIT STRING }) with an issuer's key.
pub fn verify_signature(subject: &[u8], issuer_spki: &[u8]) -> Result<(), Signature> {
    use door_tls::x509::{self, KeyAlg, SigAlg};
    let signed = Reader::new(subject)
        .expect(der::SEQUENCE)
        .ok_or(Signature::Malformed)?;
    let mut r = Reader::new(signed.value);
    let tbs = r.expect(der::SEQUENCE).ok_or(Signature::Malformed)?.raw;
    let algorithm = r.expect(der::SEQUENCE).ok_or(Signature::Malformed)?;
    let signature = r.read().and_then(der::bits).ok_or(Signature::Malformed)?;
    let alg = x509::sig_alg(algorithm).ok_or(Signature::Algorithm)?;
    let key = x509::spki(issuer_spki).ok_or(Signature::Algorithm)?;
    let ok = match (key.alg, alg) {
        // Windows checks SHA-1 signatures (old Authenticode and roots).
        (KeyAlg::Rsa, SigAlg::RsaPkcs1Sha1) => {
            door_tls::rsa::verify_pkcs1(key.key, door_tls::rsa::Hash::Sha1, tbs, signature)
        }
        _ => match door_tls::verify::signature(&key, alg, tbs, signature) {
            Ok(()) => true,
            Err(door_tls::CertError::BadSignature) => false,
            Err(_) => return Err(Signature::Algorithm),
        },
    };
    if ok { Ok(()) } else { Err(Signature::Bad) }
}
/// Base64 (RFC 4648) with whitespace ignored.
pub fn base64(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}
/// The certificates of PEM text ("-----BEGIN CERTIFICATE-----" blocks).
pub fn pem_certificates(text: &[u8]) -> Vec<Vec<u8>> {
    const BEGIN: &[u8] = b"-----BEGIN CERTIFICATE-----";
    const END: &[u8] = b"-----END CERTIFICATE-----";
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = find(rest, BEGIN) {
        rest = &rest[start + BEGIN.len()..];
        let Some(end) = find(rest, END) else { break };
        if let Some(bytes) = base64(&rest[..end])
            && parse(&bytes).is_some()
        {
            out.push(bytes);
        }
        rest = &rest[end + END.len()..];
    }
    out
}
/// The certificates of a DER PKCS #7 SignedData ContentInfo.
pub fn pkcs7_certificates(encoded: &[u8]) -> Option<Vec<&[u8]>> {
    const SIGNED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02];
    let info = Reader::new(encoded).expect(der::SEQUENCE)?;
    let mut r = Reader::new(info.value);
    if r.expect(der::OID)?.value != SIGNED_DATA {
        return None;
    }
    let explicit = r.expect(0xA0)?;
    let signed = Reader::new(explicit.value).expect(der::SEQUENCE)?;
    let mut s = Reader::new(signed.value);
    s.expect(der::INTEGER)?;
    s.expect(der::SET)?;
    s.expect(der::SEQUENCE)?;
    let mut out = Vec::new();
    if let Some(list) = s.optional(0xA0) {
        let mut certificates = Reader::new(list.value);
        while let Some(c) = certificates.read() {
            if c.tag == der::SEQUENCE && parse(c.raw).is_some() {
                out.push(c.raw);
            }
        }
    }
    Some(out)
}

/// Writes C structures for one pointer width, recording field offsets.
struct Writer {
    out: Vec<u8>,
    width: usize,
}
impl Writer {
    fn align(&mut self, a: usize) {
        while !self.out.len().is_multiple_of(a) {
            self.out.push(0);
        }
    }
    fn u32(&mut self, v: u32) -> usize {
        self.align(4);
        let at = self.out.len();
        self.out.extend_from_slice(&v.to_le_bytes());
        at
    }
    fn ptr(&mut self, v: u64) -> usize {
        self.align(self.width);
        let at = self.out.len();
        self.out.extend_from_slice(&v.to_le_bytes()[..self.width]);
        at
    }
    /// CRYPT_*_BLOB { cbData, pbData }.
    fn blob(&mut self, length: usize, pointer: u64) -> usize {
        self.align(self.width);
        let at = self.u32(length as u32);
        self.ptr(if length == 0 { 0 } else { pointer });
        at
    }
    /// CRYPT_BIT_STRING_BLOB { cbData, pbData, cUnusedBits }.
    fn bits(&mut self, length: usize, pointer: u64, unused: u32) {
        self.blob(length, pointer);
        self.u32(unused);
        self.align(self.width);
    }
    /// CRYPT_ALGORITHM_IDENTIFIER { pszObjId, Parameters }.
    fn algorithm(&mut self, oid: u64, length: usize, pointer: u64) {
        self.align(self.width);
        self.ptr(oid);
        self.blob(length, pointer);
    }
}

/// Offsets of CERT_CONTEXT's and CERT_INFO's fields for a pointer width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub context: usize,
    pub encoded: usize,
    pub encoded_length: usize,
    pub info_pointer: usize,
    pub store: usize,
    pub info: usize,
    pub serial: usize,
    pub issuer: usize,
    pub not_before: usize,
    pub not_after: usize,
    pub subject: usize,
    pub public_key_info: usize,
    pub extension_count: usize,
    pub extensions: usize,
    pub extension: usize,
}
/// The shape of the structures for `width`-byte pointers.
pub const fn shape(width: usize) -> Shape {
    if width == 8 {
        Shape {
            context: 40,
            encoded: 8,
            encoded_length: 16,
            info_pointer: 24,
            store: 32,
            info: 208,
            serial: 8,
            issuer: 48,
            not_before: 64,
            not_after: 72,
            subject: 80,
            public_key_info: 96,
            extension_count: 192,
            extensions: 200,
            extension: 32,
        }
    } else {
        Shape {
            context: 20,
            encoded: 4,
            encoded_length: 8,
            info_pointer: 12,
            store: 16,
            info: 112,
            serial: 4,
            issuer: 24,
            not_before: 32,
            not_after: 40,
            subject: 48,
            public_key_info: 56,
            extension_count: 104,
            extensions: 108,
            extension: 16,
        }
    }
}
/// The CERT_CONTEXT block for `encoded` placed at `base`: the context,
/// its CERT_INFO, the CERT_EXTENSION array, then the encoding, the
/// little-endian serial number and the OID strings.
pub fn context_block(encoded: &[u8], base: u64, width: usize, store: u64) -> Option<Vec<u8>> {
    let parts = parse(encoded)?;
    let s = shape(width);
    let fixed = s.context + s.info + parts.extensions.len() * s.extension;
    // Data area: encoding, serial (reversed), then NUL-terminated OIDs.
    let encoding_at = fixed;
    let serial_at = encoding_at + encoded.len();
    let mut strings = Vec::new();
    let string_at = |text: &str, strings: &mut Vec<u8>| -> u64 {
        let at = serial_at + parts.serial.len() + strings.len();
        strings.extend_from_slice(text.as_bytes());
        strings.push(0);
        base + at as u64
    };
    let signature_oid = string_at(&parts.signature_oid, &mut strings);
    let key_oid = string_at(&parts.key_oid, &mut strings);
    let extension_oids: Vec<u64> = parts
        .extensions
        .iter()
        .map(|e| string_at(&e.oid, &mut strings))
        .collect();
    // The address of a slice of `encoded` within the block's copy.
    let at = |slice: &[u8]| -> u64 {
        if slice.is_empty() {
            return 0;
        }
        let offset = slice.as_ptr() as usize - encoded.as_ptr() as usize;
        base + (encoding_at + offset) as u64
    };
    let mut w = Writer {
        out: Vec::with_capacity(fixed + encoded.len() + strings.len() + 32),
        width,
    };
    // CERT_CONTEXT.
    w.u32(ENCODING);
    w.ptr(base + encoding_at as u64);
    w.u32(encoded.len() as u32);
    w.ptr(base + s.context as u64);
    w.ptr(store);
    w.align(width);
    // CERT_INFO.
    w.u32(parts.version);
    w.blob(parts.serial.len(), base + serial_at as u64);
    w.algorithm(
        signature_oid,
        parts.signature_params.len(),
        at(parts.signature_params),
    );
    w.blob(parts.issuer.len(), at(parts.issuer));
    w.u32(parts.not_before as u32);
    w.u32((parts.not_before >> 32) as u32);
    w.u32(parts.not_after as u32);
    w.u32((parts.not_after >> 32) as u32);
    w.blob(parts.subject.len(), at(parts.subject));
    w.algorithm(key_oid, parts.key_params.len(), at(parts.key_params));
    w.bits(parts.key.len(), at(parts.key), parts.key_unused);
    let (uid, unused) = parts.issuer_uid.unwrap_or((&[], 0));
    w.bits(uid.len(), at(uid), unused);
    let (uid, unused) = parts.subject_uid.unwrap_or((&[], 0));
    w.bits(uid.len(), at(uid), unused);
    w.u32(parts.extensions.len() as u32);
    w.ptr(if parts.extensions.is_empty() {
        0
    } else {
        base + (s.context + s.info) as u64
    });
    w.align(width);
    // CERT_EXTENSION array.
    for (e, oid) in parts.extensions.iter().zip(&extension_oids) {
        w.ptr(*oid);
        w.u32(u32::from(e.critical));
        w.blob(e.value.len(), at(e.value));
    }
    debug_assert_eq!(w.out.len(), fixed);
    w.out.extend_from_slice(encoded);
    w.out.extend(parts.serial.iter().rev());
    w.out.extend_from_slice(&strings);
    Some(w.out)
}

/// The attribute values of an encoded Name, in encoding order, as
/// (OID text, value text).
pub fn name_attributes(name: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(sequence) = Reader::new(name).expect(der::SEQUENCE) else {
        return out;
    };
    let mut rdns = Reader::new(sequence.value);
    while let Some(set) = rdns.read() {
        let mut attributes = Reader::new(set.value);
        while let Some(pair) = attributes.read() {
            let mut p = Reader::new(pair.value);
            let (Some(oid), Some(value)) = (p.expect(der::OID), p.read()) else {
                continue;
            };
            let Some(oid) = oid_text(oid.value) else {
                continue;
            };
            out.push((oid, string_value(value.tag, value.value)));
        }
    }
    out
}
/// A directory string's text (UTF8, Printable, IA5, T61, BMP, Universal).
fn string_value(tag: u8, value: &[u8]) -> String {
    match tag {
        0x1E => char::decode_utf16(
            value
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]])),
        )
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect(),
        0x1C => value
            .chunks_exact(4)
            .map(|c| {
                char::from_u32(u32::from_be_bytes([c[0], c[1], c[2], c[3]])).unwrap_or('\u{FFFD}')
            })
            .collect(),
        0x14 => value.iter().map(|b| char::from(*b)).collect(),
        _ => String::from_utf8_lossy(value).into_owned(),
    }
}
pub const OID_COMMON_NAME: &str = "2.5.4.3";
pub const OID_ORGANIZATIONAL_UNIT: &str = "2.5.4.11";
pub const OID_ORGANIZATION: &str = "2.5.4.10";
pub const OID_EMAIL: &str = "1.2.840.113549.1.9.1";
/// X.500 keys CertNameToStr uses.
fn x500_key(oid: &str) -> Option<&'static str> {
    Some(match oid {
        "2.5.4.3" => "CN",
        "2.5.4.6" => "C",
        "2.5.4.7" => "L",
        "2.5.4.8" => "S",
        "2.5.4.9" => "STREET",
        "2.5.4.10" => "O",
        "2.5.4.11" => "OU",
        "2.5.4.12" => "T",
        "2.5.4.42" => "G",
        "2.5.4.4" => "SN",
        "2.5.4.5" => "SERIALNUMBER",
        "1.2.840.113549.1.9.1" => "E",
        "0.9.2342.19200300.100.1.25" => "DC",
        _ => return None,
    })
}
/// CERT_X500_NAME_STR rendering ("CN=a, O=b"); values holding separators
/// are quoted.
pub fn x500_text(name: &[u8]) -> String {
    let mut out = String::new();
    for (i, (oid, value)) in name_attributes(name).iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match x500_key(oid) {
            Some(key) => out.push_str(key),
            None => {
                out.push_str("OID.");
                out.push_str(oid);
            }
        }
        out.push('=');
        if value.contains([',', '+', '=', '"', '\n', '<', '>', '#', ';']) {
            out.push('"');
            out.push_str(&value.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(value);
        }
    }
    out
}
/// The first value of attribute `oid` in an encoded Name.
pub fn attribute(name: &[u8], oid: &str) -> Option<String> {
    name_attributes(name)
        .into_iter()
        .find(|(o, _)| o == oid)
        .map(|(_, v)| v)
}
/// CERT_NAME_SIMPLE_DISPLAY_TYPE: the common name, else the first
/// organizational unit, organization or e-mail address.
pub fn simple_display(name: &[u8]) -> String {
    [
        OID_COMMON_NAME,
        OID_ORGANIZATIONAL_UNIT,
        OID_ORGANIZATION,
        OID_EMAIL,
    ]
    .iter()
    .find_map(|oid| attribute(name, oid))
    .unwrap_or_default()
}
/// subjectAltName entries of `kind` (2 dNSName, 1 rfc822Name).
pub fn alt_names<'a>(parts: &Parts<'a>, kind: u8) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    for e in parts.extensions.iter().filter(|e| e.oid == "2.5.29.17") {
        let Some(list) = Reader::new(e.value).expect(der::SEQUENCE) else {
            continue;
        };
        let mut names = Reader::new(list.value);
        while let Some(n) = names.read() {
            if n.tag == 0x80 | kind {
                out.push(n.value);
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A self-signed P-256 certificate ("CN=Doors Test, O=Macrohard"),
    /// signed by its own key: SKI, AKI, keyUsage digitalSignature and
    /// keyCertSign (critical), SAN DNS doors.test, basicConstraints CA.
    pub(crate) fn sample() -> Vec<u8> {
        let hex = "308201cd30820174a003020102021401a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4300a06082a8648ce3d04030230293113301106035504030c0a446f6f7273205465737431123010060355040a0c094d6163726f68617264301e170d3236313131363138303032375a170d3336313131333138303032375a30293113301106035504030c0a446f6f7273205465737431123010060355040a0c094d6163726f686172643059301306072a8648ce3d020106082a8648ce3d0301070342000482ccccb3192d596f12db717bc7f04548fbe423bbbc71fe2f4b5128762a329c8cea5a91f03bdd46b36d180bb64c0f815cb9c6db1cf9235cc22fa634b3b0bcb234a37a3078301d0603551d0e041604140e4ddeb872ae7ec870c63edceb4e891385940572301f0603551d230418301680140e4ddeb872ae7ec870c63edceb4e891385940572300e0603551d0f0101ff04040302028430150603551d11040e300c820a646f6f72732e74657374300f0603551d130101ff040530030101ff300a06082a8648ce3d0403020347003044022029f2205cb1fc7dad2967a862d025e526ddd67cd876bf21fea46d166d6abab4c80220648bbdbe4532683b92e00632b507f7d61ec2e2681c51d1e2c52656e7597a7832";
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn object_identifiers_round_trip() {
        for text in [
            "1.2.840.113549.1.1.11",
            "2.5.29.15",
            "1.3.6.1.5.5.7.3.1",
            "2.999.3",
        ] {
            let der = oid_der(text.as_bytes()).unwrap();
            assert_eq!(oid_text(&der).unwrap(), text);
        }
        assert!(oid_der(b"3.1").is_none());
        assert!(oid_der(b"1.40").is_none());
        assert!(oid_der(b"1..2").is_none());
    }
    #[test]
    fn certificate_fields_match_cert_info() {
        let der = sample();
        let p = parse(&der).expect("parses");
        assert_eq!(p.version, 2);
        assert_eq!(p.serial.len(), 20);
        assert_eq!(p.signature_oid, "1.2.840.10045.4.3.2");
        assert!(p.signature_params.is_empty());
        assert_eq!(p.key_oid, "1.2.840.10045.2.1");
        assert_eq!(
            p.key_params,
            &[6, 8, 0x2a, 0x86, 0x48, 0xce, 0x3d, 3, 1, 7][..]
        );
        assert_eq!(p.key.len(), 65);
        assert_eq!(p.issuer, p.subject);
        // 2026-11-16T18:00:27Z.
        assert_eq!(p.not_before, (1_794_852_027 + 11_644_473_600) * 10_000_000);
        assert_eq!(p.extensions.len(), 5);
        assert!(p.extensions[2].critical && !p.extensions[0].critical);
        assert_eq!(key_usage(p.extensions[2].value), Some(&[0x84][..]));
        assert_eq!(alt_names(&p, 2), [b"doors.test".as_slice()]);
        assert_eq!(x500_text(p.subject), "CN=Doors Test, O=Macrohard");
        assert_eq!(simple_display(p.subject), "Doors Test");
    }
    #[test]
    fn signatures_verify_with_the_issuer_key() {
        let der = sample();
        let spki = spki_contents(&der).unwrap();
        assert_eq!(verify_signature(&der, spki), Ok(()));
        // The same key rebuilt from CERT_PUBLIC_KEY_INFO's parts.
        let p = parse(&der).unwrap();
        let rebuilt = spki_from(p.key_oid.as_bytes(), p.key_params, p.key, p.key_unused).unwrap();
        assert_eq!(rebuilt, spki);
        let mut tampered = der.clone();
        let at = tampered.len() - 70;
        tampered[at] ^= 1;
        assert!(verify_signature(&tampered, spki).is_err());
    }
    #[test]
    fn pem_and_pkcs7_hold_certificates() {
        let der = sample();
        let encoded = {
            const T: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut s = Vec::new();
            for chunk in der.chunks(3) {
                let n = chunk.iter().fold(0u32, |a, b| (a << 8) | u32::from(*b))
                    << (8 * (3 - chunk.len()));
                for i in 0..4 {
                    s.push(if i <= chunk.len() {
                        T[(n >> (18 - 6 * i)) as usize & 63]
                    } else {
                        b'='
                    });
                }
            }
            s
        };
        let mut pem = b"junk\n-----BEGIN CERTIFICATE-----\n".to_vec();
        for line in encoded.chunks(64) {
            pem.extend_from_slice(line);
            pem.push(b'\n');
        }
        pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
        assert_eq!(pem_certificates(&pem), core::slice::from_ref(&der));
        // ContentInfo { signedData, [0] { 1, {}, {data}, [0] { cert } } }.
        let mut certificates = Vec::new();
        push_tlv(&mut certificates, 0xA0, &der);
        let mut signed = alloc::vec![2, 1, 1, 0x31, 0];
        let mut content = Vec::new();
        push_tlv(
            &mut content,
            der::OID,
            &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01],
        );
        push_tlv(&mut signed, der::SEQUENCE, &content);
        signed.extend_from_slice(&certificates);
        signed.extend_from_slice(&[0x31, 0]);
        let mut sequence = Vec::new();
        push_tlv(&mut sequence, der::SEQUENCE, &signed);
        let mut info = Vec::new();
        push_tlv(
            &mut info,
            der::OID,
            &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02],
        );
        push_tlv(&mut info, 0xA0, &sequence);
        let mut p7 = Vec::new();
        push_tlv(&mut p7, der::SEQUENCE, &info);
        assert_eq!(pkcs7_certificates(&p7).unwrap(), [der.as_slice()]);
        assert!(pkcs7_certificates(&der).is_none());
    }
    #[test]
    fn context_blocks_have_windows_layout() {
        let der = sample();
        for width in [8usize, 4] {
            let s = shape(width);
            let base = 0x1000_0000u64;
            let store = 0x2000_0000u64;
            let block = context_block(&der, base, width, store).unwrap();
            let read = |at: usize, n: usize| -> u64 {
                let mut b = [0u8; 8];
                b[..n].copy_from_slice(&block[at..at + n]);
                u64::from_le_bytes(b)
            };
            let p = |at: usize| read(at, width);
            assert_eq!(read(0, 4), u64::from(ENCODING));
            let encoded = p(s.encoded);
            assert_eq!(read(s.encoded_length, 4) as usize, der.len());
            let copy = (encoded - base) as usize;
            assert_eq!(&block[copy..copy + der.len()], &der[..]);
            assert_eq!(p(s.info_pointer), base + s.context as u64);
            assert_eq!(p(s.store), store);
            let info = s.context;
            assert_eq!(read(info, 4), 2);
            // The serial number is little-endian.
            let serial = (p(info + s.serial + width) - base) as usize;
            assert_eq!(block[serial], 0xc4);
            assert_eq!(
                read(info + s.not_before, 4) | (read(info + s.not_before + 4, 4) << 32),
                (1_794_852_027 + 11_644_473_600) * 10_000_000
            );
            let subject = (p(info + s.subject + width) - base) as usize;
            let length = read(info + s.subject, 4) as usize;
            assert_eq!(
                simple_display(&block[subject..subject + length]),
                "Doors Test"
            );
            assert_eq!(read(info + s.extension_count, 4), 5);
            let first = (p(info + s.extensions) - base) as usize;
            assert_eq!(first, s.context + s.info);
            let oid = (p(first) - base) as usize;
            assert_eq!(&block[oid..oid + 10], b"2.5.29.14\0");
            // keyUsage, the third extension, is critical.
            assert_eq!(read(first + width, 4), 0);
            assert_eq!(read(first + 2 * s.extension + width, 4), 1);
        }
    }
}
