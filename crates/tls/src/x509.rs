//! X.509 v3 certificates: the fields chain validation needs (RFC 5280).
use crate::der::{self, Reader, Tlv};
use alloc::vec::Vec;

/// Signature and public key algorithms Doors verifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigAlg {
    /// Parsed so legacy roots load as trust anchors; never accepted as a
    /// certificate signature (`verify::signature` refuses it).
    RsaPkcs1Sha1,
    RsaPkcs1Sha256,
    RsaPkcs1Sha384,
    RsaPkcs1Sha512,
    RsaPss,
    EcdsaSha256,
    EcdsaSha384,
    EcdsaSha512,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAlg {
    Rsa,
    P256,
    P384,
}
const OID_RSA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01];
const OID_RSA_SHA1: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x05];
const OID_RSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
const OID_RSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0C];
const OID_RSA_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0D];
const OID_RSA_PSS: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0A];
const OID_EC: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
const OID_P384: &[u8] = &[0x2B, 0x81, 0x04, 0x00, 0x22];
const OID_ECDSA_SHA256: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x02];
const OID_ECDSA_SHA384: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x03];
const OID_ECDSA_SHA512: &[u8] = &[0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x04, 0x03, 0x04];
const OID_SAN: &[u8] = &[0x55, 0x1D, 0x11];
const OID_BASIC: &[u8] = &[0x55, 0x1D, 0x13];
const OID_KEY_USAGE: &[u8] = &[0x55, 0x1D, 0x0F];
const OID_EKU: &[u8] = &[0x55, 0x1D, 0x25];
const OID_NAME_CONSTRAINTS: &[u8] = &[0x55, 0x1D, 0x1E];
/// Extensions read but deliberately not enforced (key identifiers, policy
/// processing, CRL/OCSP locations): their criticality is acceptable.
const UNDERSTOOD: [&[u8]; 6] = [
    &[0x55, 0x1D, 0x0E], // subjectKeyIdentifier
    &[0x55, 0x1D, 0x23], // authorityKeyIdentifier
    &[0x55, 0x1D, 0x20], // certificatePolicies
    &[0x55, 0x1D, 0x24], // policyConstraints
    &[0x55, 0x1D, 0x36], // inhibitAnyPolicy
    &[0x55, 0x1D, 0x21], // policyMappings
];
/// id-kp-serverAuth and anyExtendedKeyUsage.
pub const EKU_SERVER_AUTH: &[u8] = &[0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
pub const EKU_ANY: &[u8] = &[0x55, 0x1D, 0x25, 0x00];
/// keyUsage bits (RFC 5280 numbering: bit 0 is the most significant).
pub const KU_DIGITAL_SIGNATURE: u16 = 1 << 15;
pub const KU_KEY_CERT_SIGN: u16 = 1 << 10;

/// A subjectPublicKeyInfo's algorithm and key bytes (an RSAPublicKey
/// SEQUENCE, or an uncompressed SEC1 point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKey<'a> {
    pub alg: KeyAlg,
    pub key: &'a [u8],
}
/// Parse SPKI contents (without the outer SEQUENCE tag).
pub fn spki(contents: &[u8]) -> Option<PublicKey<'_>> {
    let mut r = Reader::new(contents);
    let alg = r.expect(der::SEQUENCE)?;
    let key = der::bits(r.read()?)?;
    let mut a = Reader::new(alg.value);
    let oid = a.expect(der::OID)?.value;
    let alg = if oid == OID_RSA {
        KeyAlg::Rsa
    } else if oid == OID_EC {
        match a.expect(der::OID)?.value {
            OID_P256 => KeyAlg::P256,
            OID_P384 => KeyAlg::P384,
            _ => return None,
        }
    } else {
        return None;
    };
    Some(PublicKey { alg, key })
}
/// An AlgorithmIdentifier's signature algorithm (certificates and CRLs).
pub fn sig_alg(t: Tlv<'_>) -> Option<SigAlg> {
    let mut r = Reader::new(t.value);
    Some(match r.expect(der::OID)?.value {
        OID_RSA_SHA1 => SigAlg::RsaPkcs1Sha1,
        OID_RSA_SHA256 => SigAlg::RsaPkcs1Sha256,
        OID_RSA_SHA384 => SigAlg::RsaPkcs1Sha384,
        OID_RSA_SHA512 => SigAlg::RsaPkcs1Sha512,
        OID_RSA_PSS => SigAlg::RsaPss,
        OID_ECDSA_SHA256 => SigAlg::EcdsaSha256,
        OID_ECDSA_SHA384 => SigAlg::EcdsaSha384,
        OID_ECDSA_SHA512 => SigAlg::EcdsaSha512,
        _ => return None,
    })
}

/// The parts of a certificate the verifier uses.
#[derive(Clone, Debug)]
pub struct Certificate<'a> {
    pub tbs: &'a [u8],
    pub sig_alg: SigAlg,
    pub signature: &'a [u8],
    /// Issuer and subject Name contents (no outer SEQUENCE).
    pub issuer: &'a [u8],
    pub subject: &'a [u8],
    pub not_before: i64,
    pub not_after: i64,
    /// SPKI contents (no outer SEQUENCE).
    pub spki: &'a [u8],
    pub ca: bool,
    /// basicConstraints pathLenConstraint.
    pub path_len: Option<u32>,
    /// keyUsage bits, when present.
    pub key_usage: Option<u16>,
    /// extendedKeyUsage purposes (OIDs), when present.
    pub eku: Option<Vec<&'a [u8]>>,
    /// nameConstraints contents (no outer SEQUENCE), when present.
    pub name_constraints: Option<&'a [u8]>,
    /// subjectAltName dNSName entries.
    pub dns_names: Vec<&'a [u8]>,
    /// subjectAltName iPAddress entries.
    pub ip_addresses: Vec<&'a [u8]>,
}
pub fn parse(der_bytes: &[u8]) -> Option<Certificate<'_>> {
    let mut outer = Reader::new(der_bytes);
    let cert = outer.expect(der::SEQUENCE)?;
    if !outer.is_empty() {
        return None;
    }
    let mut c = Reader::new(cert.value);
    let tbs = c.expect(der::SEQUENCE)?;
    let alg = sig_alg(c.expect(der::SEQUENCE)?)?;
    let signature = der::bits(c.read()?)?;
    let mut t = Reader::new(tbs.value);
    // [0] version, serial, signature (must match), issuer, validity, subject, spki.
    t.optional(0xA0);
    t.expect(der::INTEGER)?;
    if sig_alg(t.expect(der::SEQUENCE)?)? != alg {
        return None;
    }
    let issuer = t.expect(der::SEQUENCE)?.value;
    let mut validity = Reader::new(t.expect(der::SEQUENCE)?.value);
    let not_before = der::time(validity.read()?)?;
    let not_after = der::time(validity.read()?)?;
    let subject = t.expect(der::SEQUENCE)?.value;
    let spki = t.expect(der::SEQUENCE)?.value;
    t.optional(0x81);
    t.optional(0x82);
    let mut ca = false;
    let mut path_len = None;
    let mut key_usage = None;
    let mut eku = None;
    let mut name_constraints = None;
    let mut dns_names = Vec::new();
    let mut ip_addresses = Vec::new();
    let mut seen: Vec<&[u8]> = Vec::new();
    if let Some(ext) = t.optional(0xA3) {
        let mut list = Reader::new(Reader::new(ext.value).expect(der::SEQUENCE)?.value);
        while let Some(e) = list.read() {
            let mut f = Reader::new(e.value);
            let oid = f.expect(der::OID)?.value;
            // An extension may appear once (RFC 5280 4.2).
            if seen.contains(&oid) {
                return None;
            }
            seen.push(oid);
            let critical = f.optional(der::BOOLEAN).is_some_and(|b| b.value == [0xFF]);
            let value = f.expect(der::OCTET_STRING)?.value;
            if oid == OID_SAN {
                let mut names = Reader::new(Reader::new(value).expect(der::SEQUENCE)?.value);
                while let Some(n) = names.read() {
                    match n.tag {
                        0x82 => dns_names.push(n.value),
                        0x87 => ip_addresses.push(n.value),
                        _ => (),
                    }
                }
            } else if oid == OID_BASIC {
                let mut b = Reader::new(Reader::new(value).expect(der::SEQUENCE)?.value);
                if let Some(flag) = b.optional(der::BOOLEAN) {
                    ca = flag.value == [0xFF];
                }
                if let Some(n) = b.optional(der::INTEGER) {
                    if n.value.len() > 4 || n.value.first().is_some_and(|b| b & 0x80 != 0) {
                        return None;
                    }
                    path_len = Some(n.value.iter().fold(0u32, |a, b| (a << 8) | u32::from(*b)));
                }
            } else if oid == OID_KEY_USAGE {
                let bits = Reader::new(value).expect(der::BIT_STRING)?.value;
                let (unused, data) = bits.split_first()?;
                if *unused > 7 || data.is_empty() {
                    return None;
                }
                key_usage = Some((u16::from(data[0]) << 8) | u16::from(*data.get(1).unwrap_or(&0)));
            } else if oid == OID_EKU {
                let mut list = Reader::new(Reader::new(value).expect(der::SEQUENCE)?.value);
                let mut purposes = Vec::new();
                while let Some(p) = list.read() {
                    if p.tag != der::OID {
                        return None;
                    }
                    purposes.push(p.value);
                }
                eku = Some(purposes);
            } else if oid == OID_NAME_CONSTRAINTS {
                name_constraints = Some(Reader::new(value).expect(der::SEQUENCE)?.value);
            } else if critical && !UNDERSTOOD.contains(&oid) {
                // An unknown critical extension: the certificate must not be
                // used (RFC 5280 4.2).
                return None;
            }
        }
    }
    Some(Certificate {
        tbs: tbs.raw,
        sig_alg: alg,
        signature,
        issuer,
        subject,
        not_before,
        not_after,
        spki,
        ca,
        path_len,
        key_usage,
        eku,
        name_constraints,
        dns_names,
        ip_addresses,
    })
}
