//! Authenticode signature verification for Windows executables, as
//! `WinVerifyTrust` (WINTRUST_ACTION_GENERIC_VERIFY_V2) does for an
//! embedded signature:
//!
//! 1. The attribute certificate table (PE optional header data directory
//!    4, a file offset) holds a `WIN_CERTIFICATE` of revision 2.0 and type
//!    `WIN_CERT_TYPE_PKCS_SIGNED_DATA`: a PKCS #7 SignedData whose content is
//!    `SpcIndirectDataContent` (1.3.6.1.4.1.311.2.1.4) carrying the image's
//!    digest.
//! 2. The image digest covers the whole file except the optional header's
//!    CheckSum, the Certificate Table directory entry and the certificate
//!    table itself (Microsoft's "Windows Authenticode Portable Executable
//!    Signature Format"); it must equal the signed digest. Data after the
//!    certificate table, or nonzero padding after the signature inside it,
//!    is refused (the hardened `EnableCertPaddingCheck` behaviour).
//! 3. The one SignerInfo's authenticated attributes name that content type
//!    and carry the digest of the `SpcIndirectDataContent` (its contents,
//!    without tag and length); the signer certificate's key signs their DER.
//! 4. The signer certificate chains to a trusted root through the
//!    certificates in the signature, each allowed to sign code (extended key
//!    usage codeSigning 1.3.6.1.5.5.7.3.3, or none/any).
//! 5. A valid timestamp (RFC 3161 `1.3.6.1.4.1.311.3.3.1` or a PKCS #9
//!    countersignature) by a time-stamping authority chained to a trusted
//!    root lets an expired signing certificate count at the signing time;
//!    without one, every certificate must be valid now.
//!
//! The verified publisher is the signer certificate's subject common name
//! (else its organization), as `CertGetNameString(CERT_NAME_SIMPLE_DISPLAY_TYPE)`
//! gives it to the consent prompt. Revocation is checked only when asked
//! (`verify_full` with a CRL source, `revocation`), as WinVerifyTrust
//! checks it only under WTD_REVOKE_WHOLECHAIN. Security catalogs
//! (`catalog`) vouch for files by hash under a signature checked the same
//! way.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;
use alloc::{string::String, vec::Vec};
use door_tls::der::{self, Reader, Tlv};
use door_tls::verify::{Anchor, CertError, Purpose};
use door_tls::x509;
use sha2::Digest;

const OID_SIGNED_DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02];
const OID_SPC_INDIRECT_DATA: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x01, 0x04];
const OID_CONTENT_TYPE: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x03];
const OID_MESSAGE_DIGEST: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x04];
const OID_SIGNING_TIME: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x05];
const OID_COUNTERSIGNATURE: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x06];
const OID_RFC3161: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x03, 0x03, 0x01];
const OID_TST_INFO: &[u8] = &[
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x01, 0x04,
];
const OID_SHA1: &[u8] = &[0x2B, 0x0E, 0x03, 0x02, 0x1A];
const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const OID_SHA384: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02];
const OID_SHA512: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03];
const OID_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
const OID_RSA_SHA1: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x05];
const OID_RSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
const OID_RSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0C];
const OID_RSA_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0D];
const OID_ECDSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
const OID_ECDSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x03];
const OID_EC_PUBLIC_KEY: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
const OID_O: &[u8] = &[0x55, 0x04, 0x0A];
const OID_OU: &[u8] = &[0x55, 0x04, 0x0B];
const OID_EMAIL: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x01];
/// Extended key usages: code signing and time stamping.
pub const EKU_CODE_SIGNING: &[u8] = &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03];
pub const EKU_TIME_STAMPING: &[u8] = &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x08];
/// Microsoft's "lifetime signing" EKU: the signature ends with the
/// certificate, whatever the timestamp says.
const EKU_LIFETIME_SIGNING: &[u8] = &[0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x0A, 0x03, 0x0D];
/// The largest signature blob examined.
pub const MAX_SIGNATURE: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a PE image.
    NotPe,
    /// No embedded signature (the file is unsigned).
    Unsigned,
    /// The signature or the image layout is malformed.
    Malformed,
    /// The image changed after it was signed (TRUST_E_BAD_DIGEST).
    Modified,
    /// The signer's signature over its attributes is wrong.
    BadSignature,
    /// The chain ends at a root no anchor trusts (CERT_E_UNTRUSTEDROOT).
    Untrusted,
    /// No chain could be built to a root: an issuer is missing
    /// (CERT_E_CHAINING).
    Chaining,
    /// A certificate on the chain was not valid at the signing time
    /// (CERT_E_EXPIRED).
    Expired,
    /// The signer is not allowed to sign code (CERT_E_WRONG_USAGE).
    WrongUsage,
    /// A digest or key algorithm this verifier does not support.
    Unsupported,
    /// A certificate on the chain is revoked (CERT_E_REVOKED).
    Revoked,
    /// Revocation could not be checked (CERT_E_REVOCATION_FAILURE).
    RevocationOffline,
}
impl Error {
    /// The `WinVerifyTrust` result Windows reports for it.
    pub fn hresult(self) -> u32 {
        match self {
            Self::NotPe | Self::Malformed => 0x800B_0003, // TRUST_E_SUBJECT_FORM_UNKNOWN
            Self::Unsigned => 0x800B_0100,                // TRUST_E_NOSIGNATURE
            Self::Modified => 0x8009_6010,                // TRUST_E_BAD_DIGEST
            Self::BadSignature => 0x8009_6010,
            Self::Untrusted => 0x800B_0109,   // CERT_E_UNTRUSTEDROOT
            Self::Chaining => 0x800B_010A,    // CERT_E_CHAINING
            Self::Expired => 0x800B_0101,     // CERT_E_EXPIRED
            Self::WrongUsage => 0x800B_0110,  // CERT_E_WRONG_USAGE
            Self::Unsupported => 0x8009_6002, // TRUST_E_NO_SIGNER_CERT
            Self::Revoked => 0x800B_010C,
            Self::RevocationOffline => 0x800B_010E,
        }
    }
}
/// A verified signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// The verified publisher (signer subject CN, else O).
    pub publisher: String,
    /// The signer certificate's issuer (its CN).
    pub issuer: String,
    /// The image digest algorithm ("SHA256", "SHA1", ...).
    pub digest: &'static str,
    /// Unix seconds of a verified timestamp, if any.
    pub timestamp: Option<i64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}
impl Hash {
    fn from_oid(oid: &[u8]) -> Option<Self> {
        Some(match oid {
            OID_SHA1 => Self::Sha1,
            OID_SHA256 => Self::Sha256,
            OID_SHA384 => Self::Sha384,
            OID_SHA512 => Self::Sha512,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha384 => "SHA384",
            Self::Sha512 => "SHA512",
        }
    }
    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Self::Sha1 => run::<sha1::Sha1>(parts),
            Self::Sha256 => run::<sha2::Sha256>(parts),
            Self::Sha384 => run::<sha2::Sha384>(parts),
            Self::Sha512 => run::<sha2::Sha512>(parts),
        }
    }
    fn tls(self) -> door_tls::rsa::Hash {
        match self {
            Self::Sha1 => door_tls::rsa::Hash::Sha1,
            Self::Sha256 => door_tls::rsa::Hash::Sha256,
            Self::Sha384 => door_tls::rsa::Hash::Sha384,
            Self::Sha512 => door_tls::rsa::Hash::Sha512,
        }
    }
}
/// Where the PE's checksum, certificate directory and certificate table lie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub checksum: usize,
    pub directory: usize,
    pub table: Option<(usize, usize)>,
}
fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
/// Locate the fields the image digest skips.
pub fn layout(file: &[u8]) -> Result<Layout, Error> {
    if file.get(..2) != Some(b"MZ") {
        return Err(Error::NotPe);
    }
    let pe = u32_at(file, 0x3C).ok_or(Error::NotPe)? as usize;
    if file.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(Error::NotPe);
    }
    let optional = pe + 24;
    let size = u16_at(file, pe + 20).ok_or(Error::NotPe)? as usize;
    let (directories, count_at) = match u16_at(file, optional).ok_or(Error::NotPe)? {
        0x10B => (optional + 96, optional + 92),
        0x20B => (optional + 112, optional + 108),
        _ => return Err(Error::NotPe),
    };
    let count = u32_at(file, count_at).ok_or(Error::NotPe)? as usize;
    if count <= 4 || directories + 5 * 8 > optional + size || directories + 5 * 8 > file.len() {
        return Err(Error::Unsigned);
    }
    let directory = directories + 4 * 8;
    let offset = u32_at(file, directory).ok_or(Error::Malformed)? as usize;
    let length = u32_at(file, directory + 4).ok_or(Error::Malformed)? as usize;
    let table = if offset == 0 && length == 0 {
        None
    } else {
        let end = offset.checked_add(length).ok_or(Error::Malformed)?;
        // Quadword aligned, after the headers, and the end of the file:
        // nothing may follow the certificate table.
        // After the certificate table's own directory entry (the digest
        // skips that entry, then hashes up to the table).
        if !offset.is_multiple_of(8) || offset < directory + 8 || end != file.len() || length < 8 {
            return Err(Error::Malformed);
        }
        Some((offset, length))
    };
    Ok(Layout {
        checksum: optional + 64,
        directory,
        table,
    })
}
/// The Authenticode image digest.
fn image_digest(file: &[u8], l: &Layout, hash: Hash) -> Vec<u8> {
    let end = l.table.map_or(file.len(), |t| t.0);
    hash.digest(&[
        &file[..l.checksum],
        &file[l.checksum + 4..l.directory],
        &file[l.directory + 8..end],
    ])
}
/// The PKCS #7 blob of the first `WIN_CERT_TYPE_PKCS_SIGNED_DATA` entry,
/// with its padding checked.
fn pkcs7<'a>(file: &'a [u8], l: &Layout) -> Result<&'a [u8], Error> {
    let (offset, length) = l.table.ok_or(Error::Unsigned)?;
    let table = &file[offset..offset + length];
    let mut at = 0;
    while at + 8 <= table.len() {
        let size = u32_at(table, at).ok_or(Error::Malformed)? as usize;
        let revision = u16_at(table, at + 4).ok_or(Error::Malformed)?;
        let kind = u16_at(table, at + 6).ok_or(Error::Malformed)?;
        if size < 8 || at + size > table.len() {
            return Err(Error::Malformed);
        }
        if revision == 0x0200 && kind == 0x0002 {
            let blob = &table[at + 8..at + size];
            let used = Reader::new(blob).read().ok_or(Error::Malformed)?.raw.len();
            if used > MAX_SIGNATURE || blob[used..].iter().any(|b| *b != 0) || blob.len() - used > 7
            {
                return Err(Error::Malformed);
            }
            return Ok(&blob[..used]);
        }
        at += (size + 7) & !7;
    }
    Err(Error::Unsigned)
}
/// A required DER element.
fn m<T>(o: Option<T>) -> Result<T, Error> {
    o.ok_or(Error::Malformed)
}
/// The parts of a PKCS #7 SignedData this verifier reads.
struct SignedData<'a> {
    /// encapContentInfo content type and its [0] content (the inner TLV).
    content_type: &'a [u8],
    content: Tlv<'a>,
    certificates: Vec<&'a [u8]>,
    signer: SignerInfo<'a>,
}
struct SignerInfo<'a> {
    issuer: &'a [u8],
    serial: &'a [u8],
    digest: Hash,
    /// The [0] IMPLICIT authenticated attributes, whole TLV.
    attributes: Option<Tlv<'a>>,
    encryption: &'a [u8],
    signature: &'a [u8],
    unauthenticated: Option<Tlv<'a>>,
}
fn signed_data(blob: &[u8]) -> Result<SignedData<'_>, Error> {
    let mut outer = Reader::new(m(Reader::new(blob).expect(der::SEQUENCE))?.value);
    if m(outer.expect(der::OID))?.value != OID_SIGNED_DATA {
        return Err(Error::Malformed);
    }
    let explicit = m(outer.expect(0xA0))?;
    let mut sd = Reader::new(m(Reader::new(explicit.value).expect(der::SEQUENCE))?.value);
    m(sd.expect(der::INTEGER))?;
    m(sd.expect(der::SET))?;
    let mut encap = Reader::new(m(sd.expect(der::SEQUENCE))?.value);
    let content_type = m(encap.expect(der::OID))?.value;
    let content = m(Reader::new(m(encap.expect(0xA0))?.value).read())?;
    let mut certificates = Vec::new();
    if let Some(set) = sd.optional(0xA0) {
        let mut r = Reader::new(set.value);
        while let Some(c) = r.read() {
            if c.tag == der::SEQUENCE {
                certificates.push(c.raw);
            }
        }
    }
    sd.optional(0xA1);
    let mut signers = Reader::new(m(sd.expect(der::SET))?.value);
    let signer = signer_info(m(signers.expect(der::SEQUENCE))?.value)?;
    // Authenticode allows exactly one signer.
    if !signers.is_empty() {
        return Err(Error::Malformed);
    }
    Ok(SignedData {
        content_type,
        content,
        certificates,
        signer,
    })
}
fn signer_info(value: &[u8]) -> Result<SignerInfo<'_>, Error> {
    let mut s = Reader::new(value);
    m(s.expect(der::INTEGER))?;
    let mut sid = Reader::new(m(s.expect(der::SEQUENCE))?.value);
    let issuer = m(sid.expect(der::SEQUENCE))?.value;
    let serial = m(sid.expect(der::INTEGER))?.value;
    let digest_oid = m(Reader::new(m(s.expect(der::SEQUENCE))?.value).expect(der::OID))?.value;
    let digest = Hash::from_oid(digest_oid).ok_or(Error::Unsupported)?;
    let attributes = s.optional(0xA0);
    let encryption = m(Reader::new(m(s.expect(der::SEQUENCE))?.value).expect(der::OID))?.value;
    let signature = m(s.expect(der::OCTET_STRING))?.value;
    let unauthenticated = s.optional(0xA1);
    Ok(SignerInfo {
        issuer,
        serial,
        digest,
        attributes,
        encryption,
        signature,
        unauthenticated,
    })
}
/// An attribute's single value (the first of its SET).
fn attribute<'a>(attrs: &Tlv<'a>, oid: &[u8]) -> Option<Tlv<'a>> {
    let mut r = Reader::new(attrs.value);
    while let Some(a) = r.read() {
        let mut f = Reader::new(a.value);
        if f.expect(der::OID)?.value == oid {
            return Reader::new(f.expect(der::SET)?.value).read();
        }
    }
    None
}
/// Every value of an attribute (countersignatures may repeat).
fn attribute_values<'a>(attrs: &Tlv<'a>, oid: &[u8]) -> Vec<Tlv<'a>> {
    let mut out = Vec::new();
    let mut r = Reader::new(attrs.value);
    while let Some(a) = r.read() {
        let mut f = Reader::new(a.value);
        if f.expect(der::OID).is_some_and(|o| o.value == oid)
            && let Some(set) = f.expect(der::SET)
        {
            let mut v = Reader::new(set.value);
            while let Some(x) = v.read() {
                out.push(x);
            }
        }
    }
    out
}
/// The signer certificate among `certs`: issuer and serial match.
fn find_signer<'a>(
    certs: &[&'a [u8]],
    issuer: &[u8],
    serial: &[u8],
) -> Option<(usize, x509::Certificate<'a>)> {
    certs.iter().enumerate().find_map(|(i, raw)| {
        let c = x509::parse(raw)?;
        (c.issuer == issuer && serial_of(raw) == Some(serial)).then_some((i, c))
    })
}
fn serial_of(raw: &[u8]) -> Option<&[u8]> {
    let cert = Reader::new(raw).expect(der::SEQUENCE)?;
    let tbs = Reader::new(cert.value).expect(der::SEQUENCE)?;
    let mut t = Reader::new(tbs.value);
    t.optional(0xA0);
    Some(t.expect(der::INTEGER)?.value)
}
/// Verify the signer's signature over the DER of its authenticated
/// attributes (re-tagged from [0] IMPLICIT to SET OF).
fn check_signer(
    cert: &x509::Certificate<'_>,
    s: &SignerInfo<'_>,
    attrs: &Tlv<'_>,
) -> Result<(), Error> {
    let mut der_set = Vec::with_capacity(attrs.raw.len());
    der_set.push(der::SET);
    der_set.extend_from_slice(&attrs.raw[1..]);
    let key = x509::spki(cert.spki).ok_or(Error::Unsupported)?;
    let ok = match (key.alg, s.encryption) {
        (
            x509::KeyAlg::Rsa,
            OID_RSA | OID_RSA_SHA1 | OID_RSA_SHA256 | OID_RSA_SHA384 | OID_RSA_SHA512,
        ) => door_tls::rsa::verify_pkcs1(key.key, s.digest.tls(), &der_set, s.signature),
        (
            x509::KeyAlg::P256 | x509::KeyAlg::P384,
            OID_ECDSA_SHA256 | OID_ECDSA_SHA384 | OID_EC_PUBLIC_KEY,
        ) => {
            let alg = match s.digest {
                Hash::Sha256 => x509::SigAlg::EcdsaSha256,
                Hash::Sha384 => x509::SigAlg::EcdsaSha384,
                Hash::Sha512 => x509::SigAlg::EcdsaSha512,
                Hash::Sha1 => return Err(Error::Unsupported),
            };
            door_tls::verify::signature(&key, alg, &der_set, s.signature).is_ok()
        }
        _ => return Err(Error::Unsupported),
    };
    if ok { Ok(()) } else { Err(Error::BadSignature) }
}
/// A verified timestamp's time (Unix seconds) over `signature`, or None.
fn timestamp(s: &SignerInfo<'_>, certs: &[&[u8]], anchors: &[Anchor<'_>], now: i64) -> Option<i64> {
    let unauth = s.unauthenticated.as_ref()?;
    // RFC 3161: a SignedData whose TSTInfo imprints the signature.
    for token in attribute_values(unauth, OID_RFC3161) {
        if let Some(t) = rfc3161(token.raw, s.signature, anchors, now) {
            return Some(t);
        }
    }
    // PKCS #9 countersignature: a SignerInfo over the signature, signed
    // with a certificate from the outer set.
    for cs in attribute_values(unauth, OID_COUNTERSIGNATURE) {
        let Ok(c) = signer_info(cs.value) else {
            continue;
        };
        let Some(attrs) = c.attributes.as_ref() else {
            continue;
        };
        let Some(digest) = attribute(attrs, OID_MESSAGE_DIGEST) else {
            continue;
        };
        if digest.value != c.digest.digest(&[s.signature]) {
            continue;
        }
        let Some(time) = attribute(attrs, OID_SIGNING_TIME).and_then(|t| der::time(t)) else {
            continue;
        };
        let Some((i, cert)) = find_signer(certs, c.issuer, c.serial) else {
            continue;
        };
        if check_signer(&cert, &c, attrs).is_ok()
            && chain(certs, i, time, anchors, EKU_TIME_STAMPING).is_ok()
        {
            return Some(time);
        }
    }
    None
}
fn rfc3161(token: &[u8], signature: &[u8], anchors: &[Anchor<'_>], now: i64) -> Option<i64> {
    let sd = signed_data(token).ok()?;
    if sd.content_type != OID_TST_INFO || sd.content.tag != der::OCTET_STRING {
        return None;
    }
    let tst = sd.content.value;
    let mut info = Reader::new(Reader::new(tst).expect(der::SEQUENCE)?.value);
    info.expect(der::INTEGER)?;
    info.expect(der::OID)?;
    let mut imprint = Reader::new(info.expect(der::SEQUENCE)?.value);
    let hash = Hash::from_oid(
        Reader::new(imprint.expect(der::SEQUENCE)?.value)
            .expect(der::OID)?
            .value,
    )?;
    if imprint.expect(der::OCTET_STRING)?.value != hash.digest(&[signature]) {
        return None;
    }
    info.expect(der::INTEGER)?;
    let time = generalized_time(info.expect(der::GENERALIZED_TIME)?.value)?;
    let attrs = sd.signer.attributes.as_ref()?;
    if attribute(attrs, OID_MESSAGE_DIGEST)?.value != sd.signer.digest.digest(&[tst]) {
        return None;
    }
    let (i, cert) = find_signer(&sd.certificates, sd.signer.issuer, sd.signer.serial)?;
    check_signer(&cert, &sd.signer, attrs).ok()?;
    let _ = now;
    chain(&sd.certificates, i, time, anchors, EKU_TIME_STAMPING).ok()?;
    Some(time)
}
/// GeneralizedTime with optional fractional seconds (`YYYYMMDDhhmmss[.f]Z`).
fn generalized_time(v: &[u8]) -> Option<i64> {
    let s = core::str::from_utf8(v).ok()?;
    let s = s.strip_suffix('Z')?;
    let whole = s.split('.').next()?;
    if whole.len() != 14 {
        return None;
    }
    let mut bytes = Vec::from(whole.as_bytes());
    bytes.push(b'Z');
    der::time(Tlv {
        tag: der::GENERALIZED_TIME,
        value: &bytes,
        raw: &bytes,
    })
}
/// A validated path: the certificates considered (leaf first), the path
/// through them (indices, leaf first) and the anchor it ends at.
type Path<'a> = (Vec<&'a [u8]>, Vec<usize>, usize);
/// Chain certificate `leaf` of `certs` to an anchor for `purpose` at `time`.
/// Other certificates the signature carries are candidate intermediates;
/// ones this verifier cannot parse are left out, as Windows' chain engine
/// ignores certificates it cannot use.
fn chain<'a>(
    certs: &[&'a [u8]],
    leaf: usize,
    time: i64,
    anchors: &[Anchor<'_>],
    purpose: &'static [u8],
) -> Result<Path<'a>, CertError> {
    let mut ordered: Vec<&[u8]> = alloc::vec![certs[leaf]];
    ordered.extend(
        certs
            .iter()
            .enumerate()
            .filter(|(i, c)| *i != leaf && x509::parse(c).is_some())
            .map(|(_, c)| *c),
    );
    let (path, anchor) =
        door_tls::verify::chain_path(&ordered, time, anchors, Purpose::Eku(purpose))?;
    Ok((ordered, path, anchor))
}
/// Whether the certificates a signature carries lead by issuer name from
/// `leaf` to a self-signed one: an untrusted root rather than a missing
/// issuer, as Windows' chain engine tells CERT_E_UNTRUSTEDROOT from
/// CERT_E_CHAINING.
fn reaches_root(certs: &[&[u8]], leaf: usize) -> bool {
    let Some(mut cert) = x509::parse(certs[leaf]) else {
        return false;
    };
    for _ in 0..8 {
        if cert.subject == cert.issuer {
            return true;
        }
        let Some(parent) = certs
            .iter()
            .filter_map(|c| x509::parse(c))
            .find(|c| c.subject == cert.issuer)
        else {
            return false;
        };
        cert = parent;
    }
    false
}
/// A Name's simple display form: its CN, else O, OU or e-mail.
pub fn display_name(name: &[u8]) -> String {
    let mut found: [Option<String>; 4] = [None, None, None, None];
    let mut rdns = Reader::new(name);
    while let Some(rdn) = rdns.read() {
        let mut set = Reader::new(rdn.value);
        while let Some(atv) = set.read() {
            let mut f = Reader::new(atv.value);
            let (Some(oid), Some(v)) = (f.expect(der::OID), f.read()) else {
                continue;
            };
            let slot = match oid.value {
                OID_CN => 0,
                OID_O => 1,
                OID_OU => 2,
                OID_EMAIL => 3,
                _ => continue,
            };
            let text = match v.tag {
                // BMPString: UTF-16BE.
                0x1E => String::from_utf16_lossy(
                    &v.value
                        .chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect::<Vec<_>>(),
                ),
                _ => String::from_utf8_lossy(v.value).into_owned(),
            };
            found[slot].get_or_insert(text);
        }
    }
    found.into_iter().flatten().next().unwrap_or_default()
}
/// Verify a SignedData's one signer for content of type `content_type` at
/// Unix time `now`: its authenticated attributes name that type and the
/// content's digest, its key signs them, a verified timestamp (unless the
/// certificate is for lifetime signing) sets the time its chain must be
/// valid at, the chain reaches an anchor for code signing and, given
/// `revocation`, no certificate on it is revoked. `digest` names the
/// content's own digest algorithm in the result.
fn verify_signer(
    sd: &SignedData<'_>,
    content_type: &[u8],
    digest: &'static str,
    now: i64,
    anchors: &[Anchor<'_>],
    revocation: Option<&mut dyn revocation::Source>,
) -> Result<Signature, Error> {
    let s = &sd.signer;
    let attrs = s.attributes.as_ref().ok_or(Error::Malformed)?;
    if attribute(attrs, OID_CONTENT_TYPE).map(|t| t.value) != Some(content_type) {
        return Err(Error::Malformed);
    }
    if attribute(attrs, OID_MESSAGE_DIGEST).map(|t| t.value)
        != Some(&s.digest.digest(&[sd.content.value])[..])
    {
        return Err(Error::Modified);
    }
    let (i, cert) = find_signer(&sd.certificates, s.issuer, s.serial).ok_or(Error::Untrusted)?;
    check_signer(&cert, s, attrs)?;
    let lifetime = cert
        .eku
        .as_ref()
        .is_some_and(|e| e.contains(&EKU_LIFETIME_SIGNING));
    let stamp = if lifetime {
        None
    } else {
        timestamp(s, &sd.certificates, anchors, now)
    };
    let at = stamp.unwrap_or(now);
    let (ordered, path, anchor) = chain(&sd.certificates, i, at, anchors, EKU_CODE_SIGNING)
        .map_err(|e| match e {
            CertError::Expired => Error::Expired,
            CertError::WrongHost => Error::WrongUsage,
            CertError::BadSignature => Error::BadSignature,
            CertError::UnsupportedAlgorithm => Error::Unsupported,
            CertError::Malformed => Error::Malformed,
            CertError::Untrusted if reaches_root(&sd.certificates, i) => Error::Untrusted,
            CertError::Untrusted => Error::Chaining,
        })?;
    if let Some(source) = revocation {
        revocation::check(&ordered, &path, &anchors[anchor], now, stamp, source)?;
    }
    Ok(Signature {
        publisher: display_name(cert.subject),
        issuer: display_name(cert.issuer),
        digest,
        timestamp: stamp,
    })
}
/// Verify `file`'s embedded Authenticode signature at Unix time `now`
/// (revocation unchecked).
pub fn verify(file: &[u8], now: i64, anchors: &[Anchor<'_>]) -> Result<Signature, Error> {
    verify_full(file, now, anchors, None)
}
/// `verify`, checking revocation with CRLs from `revocation` when given.
pub fn verify_full(
    file: &[u8],
    now: i64,
    anchors: &[Anchor<'_>],
    revocation: Option<&mut dyn revocation::Source>,
) -> Result<Signature, Error> {
    let l = layout(file)?;
    let blob = pkcs7(file, &l)?;
    let sd = signed_data(blob)?;
    if sd.content_type != OID_SPC_INDIRECT_DATA || sd.content.tag != der::SEQUENCE {
        return Err(Error::Malformed);
    }
    // SpcIndirectDataContent { data, messageDigest DigestInfo }.
    let mut spc = Reader::new(sd.content.value);
    spc.expect(der::SEQUENCE).ok_or(Error::Malformed)?;
    let mut info = Reader::new(spc.expect(der::SEQUENCE).ok_or(Error::Malformed)?.value);
    let alg = Reader::new(info.expect(der::SEQUENCE).ok_or(Error::Malformed)?.value)
        .expect(der::OID)
        .ok_or(Error::Malformed)?
        .value;
    let hash = Hash::from_oid(alg).ok_or(Error::Unsupported)?;
    let signed_digest = info
        .expect(der::OCTET_STRING)
        .ok_or(Error::Malformed)?
        .value;
    if image_digest(file, &l, hash) != signed_digest {
        return Err(Error::Modified);
    }
    verify_signer(
        &sd,
        OID_SPC_INDIRECT_DATA,
        hash.name(),
        now,
        anchors,
        revocation,
    )
}

pub mod catalog;
pub mod revocation;
pub mod roots;
#[cfg(test)]
mod tests;
