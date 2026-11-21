//! Server certificate validation: a path from the leaf through the
//! certificates the server sent to a Mozilla trust anchor (`webpki-roots`),
//! every signature checked, the certificates on the path within their
//! validity periods, issuers that are CAs allowed to sign certificates (key
//! usage, path length, server-authentication purpose), the name
//! constraints of intermediates and anchors, and the leaf's subjectAltName
//! matching the host (one leftmost wildcard label at most; IP hosts only by
//! iPAddress).
use crate::der;
use crate::rsa::{self, Hash};
use crate::x509::{self, Certificate, KeyAlg, PublicKey, SigAlg};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertError {
    Malformed,
    /// No path to a trusted root.
    Untrusted,
    Expired,
    BadSignature,
    /// The certificate is not for this host name.
    WrongHost,
    UnsupportedAlgorithm,
}

/// Verify `signature` over `message` with `key`, as `alg` says.
pub fn signature(
    key: &PublicKey<'_>,
    alg: SigAlg,
    message: &[u8],
    sig: &[u8],
) -> Result<(), CertError> {
    let ok = match (key.alg, alg) {
        (KeyAlg::Rsa, SigAlg::RsaPkcs1Sha256) => {
            rsa::verify_pkcs1(key.key, Hash::Sha256, message, sig)
        }
        (KeyAlg::Rsa, SigAlg::RsaPkcs1Sha384) => {
            rsa::verify_pkcs1(key.key, Hash::Sha384, message, sig)
        }
        (KeyAlg::Rsa, SigAlg::RsaPkcs1Sha512) => {
            rsa::verify_pkcs1(key.key, Hash::Sha512, message, sig)
        }
        (KeyAlg::P256 | KeyAlg::P384, SigAlg::EcdsaSha256) => {
            ecdsa(key, Hash::Sha256, message, sig)
        }
        (KeyAlg::P256 | KeyAlg::P384, SigAlg::EcdsaSha384) => {
            ecdsa(key, Hash::Sha384, message, sig)
        }
        (KeyAlg::P256 | KeyAlg::P384, SigAlg::EcdsaSha512) => {
            ecdsa(key, Hash::Sha512, message, sig)
        }
        _ => return Err(CertError::UnsupportedAlgorithm),
    };
    if ok {
        Ok(())
    } else {
        Err(CertError::BadSignature)
    }
}
fn ecdsa(key: &PublicKey<'_>, hash: Hash, message: &[u8], sig: &[u8]) -> bool {
    use p256::ecdsa::signature::hazmat::PrehashVerifier;
    let digest = hash.digest(&[message]);
    match key.alg {
        KeyAlg::P256 => {
            let (Ok(k), Ok(s)) = (
                p256::ecdsa::VerifyingKey::from_sec1_bytes(key.key),
                p256::ecdsa::Signature::from_der(sig),
            ) else {
                return false;
            };
            k.verify_prehash(&digest, &s).is_ok()
        }
        KeyAlg::P384 => {
            let (Ok(k), Ok(s)) = (
                p384::ecdsa::VerifyingKey::from_sec1_bytes(key.key),
                p384::ecdsa::Signature::from_der(sig),
            ) else {
                return false;
            };
            k.verify_prehash(&digest, &s).is_ok()
        }
        KeyAlg::Rsa => false,
    }
}
/// RFC 6125 host matching against dNSName entries. An IP-address host
/// never matches a dNSName (only iPAddress entries).
pub fn host_matches(name: &[u8], host: &str) -> bool {
    if parse_ipv4(host).is_some() || parse_ipv6(host).is_some() {
        return false;
    }
    let host = host.trim_end_matches('.').as_bytes();
    let name = if name.last() == Some(&b'.') {
        &name[..name.len() - 1]
    } else {
        name
    };
    if let Some(rest) = name.strip_prefix(b"*.") {
        // The wildcard covers exactly one leftmost label.
        let Some(dot) = host.iter().position(|b| *b == b'.') else {
            return false;
        };
        return dot > 0 && host[dot + 1..].eq_ignore_ascii_case(rest) && rest.contains(&b'.');
    }
    name.eq_ignore_ascii_case(host)
}
pub fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = host.split('.');
    for b in &mut out {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *b = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// An IPv6 literal host ("2001:db8::1", or as in a URL "[2001:db8::1]"),
/// RFC 4291 section 2.2 (a trailing dotted IPv4 part included).
pub fn parse_ipv6(host: &str) -> Option<[u8; 16]> {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if !host.contains(':') {
        return None;
    }
    // A trailing dotted IPv4 part is two groups.
    let rewritten;
    let host = match host.rsplit_once(':') {
        Some((head, last)) if last.contains('.') => {
            let a = parse_ipv4(last)?;
            rewritten = alloc::format!(
                "{head}:{:x}:{:x}",
                u16::from_be_bytes([a[0], a[1]]),
                u16::from_be_bytes([a[2], a[3]])
            );
            rewritten.as_str()
        }
        _ => host,
    };
    let groups = |s: &str| -> Option<Vec<u16>> {
        if s.is_empty() {
            return Some(Vec::new());
        }
        s.split(':')
            .map(|g| {
                if g.is_empty() || g.len() > 4 {
                    return None;
                }
                u16::from_str_radix(g, 16).ok()
            })
            .collect()
    };
    let mut words = [0u16; 8];
    match host.split_once("::") {
        Some((front, back)) => {
            if back.contains("::") {
                return None;
            }
            let (front, back) = (groups(front)?, groups(back)?);
            if front.len() + back.len() >= 8 {
                return None;
            }
            words[..front.len()].copy_from_slice(&front);
            words[8 - back.len()..].copy_from_slice(&back);
        }
        None => {
            let all = groups(host)?;
            if all.len() != 8 {
                return None;
            }
            words.copy_from_slice(&all);
        }
    }
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes());
    }
    Some(out)
}
fn ip_matches(ip: &[u8], host: &str) -> bool {
    parse_ipv4(host).is_some_and(|a| ip == a) || parse_ipv6(host).is_some_and(|a| ip == a)
}
/// Whether a dNSName lies in a name-constraint subtree (RFC 5280
/// 4.2.1.10: "example.com" covers itself and its subdomains,
/// ".example.com" only subdomains).
fn dns_in_subtree(name: &[u8], base: &[u8]) -> bool {
    let name = name.to_ascii_lowercase();
    let mut base = base.to_ascii_lowercase();
    if base.last() == Some(&b'.') {
        base.pop();
    }
    if base.is_empty() {
        return true;
    }
    if base[0] == b'.' {
        return name.len() > base.len() && name.ends_with(&base);
    }
    name == base
        || (name.len() > base.len()
            && name.ends_with(&base)
            && name[name.len() - base.len() - 1] == b'.')
}
fn ip_in_subtree(ip: &[u8], base: &[u8]) -> bool {
    if base.len() != ip.len() * 2 {
        return false;
    }
    let (addr, mask) = base.split_at(ip.len());
    ip.iter()
        .zip(addr)
        .zip(mask)
        .all(|((i, a), m)| i & m == a & m)
}
type Subtrees = Vec<(u8, Vec<u8>)>;
fn subtrees(t: Option<der::Tlv<'_>>) -> Result<Subtrees, CertError> {
    let mut out = Vec::new();
    let Some(t) = t else { return Ok(out) };
    let mut list = der::Reader::new(t.value);
    while let Some(s) = list.read() {
        let mut g = der::Reader::new(s.value);
        let base = g.read().ok_or(CertError::Malformed)?;
        // minimum/maximum other than the defaults are not used on the web.
        if !g.is_empty() {
            return Err(CertError::UnsupportedAlgorithm);
        }
        out.push((base.tag, base.value.to_vec()));
    }
    Ok(out)
}
/// Apply nameConstraints contents (no outer SEQUENCE) to one certificate's
/// subject and subjectAltNames.
fn constraints_permit(
    constraints: &[u8],
    subject: &[u8],
    dns: &[&[u8]],
    ips: &[&[u8]],
) -> Result<bool, CertError> {
    let mut r = der::Reader::new(constraints);
    let (mut permitted, mut excluded) = (None, None);
    while let Some(t) = r.read() {
        match t.tag {
            0xA0 => permitted = Some(t),
            0xA1 => excluded = Some(t),
            _ => return Err(CertError::Malformed),
        }
    }
    let permitted = subtrees(permitted)?;
    let excluded = subtrees(excluded)?;
    for (kind, _) in permitted.iter().chain(&excluded) {
        // dNSName, iPAddress, directoryName, rfc822Name, URI.
        if !matches!(kind, 0x82 | 0x87 | 0xA4 | 0x81 | 0x86) {
            return Err(CertError::UnsupportedAlgorithm);
        }
    }
    let any = |kind: u8, list: &Subtrees, test: &dyn Fn(&[u8]) -> bool| {
        list.iter()
            .filter(|(k, _)| *k == kind)
            .any(|(_, b)| test(b))
    };
    let has = |kind: u8, list: &Subtrees| list.iter().any(|(k, _)| *k == kind);
    let allowed = |kind: u8, test: &dyn Fn(&[u8]) -> bool| {
        !any(kind, &excluded, test) && (!has(kind, &permitted) || any(kind, &permitted, test))
    };
    for name in dns {
        if !allowed(0x82, &|b| dns_in_subtree(name, b)) {
            return Ok(false);
        }
    }
    for ip in ips {
        if !allowed(0x87, &|b| ip_in_subtree(ip, b)) {
            return Ok(false);
        }
    }
    // directoryName: the subject's RDNs must start with the base's.
    let dn = |b: &[u8]| {
        let base = der::Reader::new(b)
            .expect(der::SEQUENCE)
            .map_or(b, |s| s.value);
        subject.starts_with(base)
    };
    if !subject.is_empty() && !allowed(0xA4, &dn) {
        return Ok(false);
    }
    Ok(true)
}
/// A trust anchor: subject Name and SPKI contents (no outer SEQUENCE) and
/// optional NameConstraints, as `webpki-roots` records them.
#[derive(Clone, Copy, Debug)]
pub struct Anchor<'a> {
    pub subject: &'a [u8],
    pub spki: &'a [u8],
    pub name_constraints: Option<&'a [u8]>,
}
/// The Mozilla root program's TLS server anchors.
pub fn mozilla_roots() -> Vec<Anchor<'static>> {
    webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .map(|a| Anchor {
            subject: a.subject.as_ref(),
            spki: a.subject_public_key_info.as_ref(),
            name_constraints: a.name_constraints.as_ref().map(|n| n.as_ref()),
        })
        .collect()
}
/// What a certificate path is validated for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// TLS server authentication (id-kp-serverAuth).
    ServerAuth,
    /// Another extended key usage (code signing, time stamping).
    Eku(&'static [u8]),
}
impl Purpose {
    fn oid(self) -> &'static [u8] {
        match self {
            Self::ServerAuth => x509::EKU_SERVER_AUTH,
            Self::Eku(oid) => oid,
        }
    }
    /// A certificate whose extended key usages (when present) allow it.
    fn allows(self, c: &Certificate<'_>) -> bool {
        c.eku.as_ref().is_none_or(|purposes| {
            purposes
                .iter()
                .any(|p| *p == self.oid() || *p == x509::EKU_ANY)
        })
    }
}
/// Up to this many certificates on a path (leaf, intermediates, anchor).
const MAX_DEPTH: usize = 8;
/// Validate `chain` (leaf first, DER) for `host` at Unix time `now`: build
/// a path from the leaf through the sent certificates to a Mozilla trust
/// anchor, backtracking over candidate issuers; only certificates on the
/// path must be within their validity periods.
pub fn chain(chain: &[&[u8]], host: &str, now: i64) -> Result<(), CertError> {
    chain_with(chain, host, now, &mozilla_roots())
}
/// `chain` against explicit trust anchors.
pub fn chain_with(
    chain: &[&[u8]],
    host: &str,
    now: i64,
    anchors: &[Anchor<'_>],
) -> Result<(), CertError> {
    let certs: Vec<Certificate<'_>> = chain
        .iter()
        .map(|c| x509::parse(c).ok_or(CertError::Malformed))
        .collect::<Result<_, _>>()?;
    let leaf = certs.first().ok_or(CertError::Malformed)?;
    if !leaf.dns_names.iter().any(|n| host_matches(n, host))
        && !leaf.ip_addresses.iter().any(|ip| ip_matches(ip, host))
    {
        return Err(CertError::WrongHost);
    }
    if now < leaf.not_before || now > leaf.not_after {
        return Err(CertError::Expired);
    }
    // An end-entity certificate is not a CA, and its listed purposes must
    // include TLS server authentication.
    if leaf.ca || !Purpose::ServerAuth.allows(leaf) {
        return Err(CertError::Untrusted);
    }
    let mut path = alloc::vec![0usize];
    extend(&certs, &mut path, now, anchors, Purpose::ServerAuth).map(|_| ())
}
/// Certificate problems a caller may choose to accept (Windows'
/// SECURITY_FLAG_IGNORE_UNKNOWN_CA, _CERT_DATE_INVALID, _CERT_CN_INVALID
/// and _CERT_WRONG_USAGE). The default accepts none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub unknown_ca: bool,
    pub date: bool,
    pub host: bool,
    pub usage: bool,
}
/// Every problem of a server chain, each checked on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Problems {
    pub malformed: bool,
    /// No path to a trusted root (dates aside).
    pub untrusted: bool,
    /// A certificate on the path is outside its validity period.
    pub date: bool,
    pub host: bool,
    /// The leaf is a CA or not for TLS server authentication.
    pub usage: bool,
    /// A signature on the path is invalid.
    pub signature: bool,
}
impl Problems {
    pub fn any(self) -> bool {
        self.malformed || self.untrusted || self.date || self.host || self.usage || self.signature
    }
    /// Problems `policy` does not accept (malformed chains and bad
    /// signatures never are).
    pub fn refused(self, policy: Policy) -> Option<CertError> {
        if self.malformed {
            Some(CertError::Malformed)
        } else if self.signature {
            Some(CertError::BadSignature)
        } else if self.host && !policy.host {
            Some(CertError::WrongHost)
        } else if self.date && !policy.date {
            Some(CertError::Expired)
        } else if (self.untrusted && !policy.unknown_ca) || (self.usage && !policy.usage) {
            Some(CertError::Untrusted)
        } else {
            None
        }
    }
}
/// Every problem of a TLS server chain for `host` at `now`.
pub fn assess(chain: &[&[u8]], host: &str, now: i64) -> Problems {
    assess_with(chain, host, now, &mozilla_roots())
}
pub fn assess_with(chain: &[&[u8]], host: &str, now: i64, anchors: &[Anchor<'_>]) -> Problems {
    let mut p = Problems::default();
    let Ok(certs) = chain
        .iter()
        .map(|c| x509::parse(c).ok_or(()))
        .collect::<Result<Vec<Certificate<'_>>, ()>>()
    else {
        p.malformed = true;
        return p;
    };
    let Some(leaf) = certs.first() else {
        p.malformed = true;
        return p;
    };
    p.host = !leaf.dns_names.iter().any(|n| host_matches(n, host))
        && !leaf.ip_addresses.iter().any(|ip| ip_matches(ip, host));
    p.usage = leaf.ca || !Purpose::ServerAuth.allows(leaf);
    // Dates count for the leaf and the path that validates it, not for
    // other certificates the server happens to send.
    p.date = now < leaf.not_before || now > leaf.not_after;
    let mut path = alloc::vec![0usize];
    let outcome = if p.date {
        Err(CertError::Expired)
    } else {
        extend(&certs, &mut path, now, anchors, Purpose::ServerAuth)
    };
    match outcome {
        Ok(_) => (),
        Err(CertError::BadSignature) => p.signature = true,
        Err(CertError::Malformed) => p.malformed = true,
        Err(_) => {
            // Checked again at a time when the certificates were all
            // issued: a path valid then is a date problem, not distrust.
            let at = certs
                .iter()
                .map(|c| c.not_before)
                .max()
                .unwrap_or(leaf.not_before)
                .max(leaf.not_before);
            let mut path = alloc::vec![0usize];
            match extend(&certs, &mut path, at, anchors, Purpose::ServerAuth) {
                Ok(_) => p.date = true,
                Err(CertError::BadSignature) => p.signature = true,
                Err(CertError::Expired) => p.date = true,
                Err(CertError::Malformed) => p.malformed = true,
                Err(_) => p.untrusted = true,
            }
        }
    }
    p
}
/// Validate `chain` (leaf first) for `purpose` at Unix time `at`: the
/// leaf is an end-entity certificate valid then, allowed that purpose (and
/// digital signatures), and a path through the other certificates reaches
/// an anchor. Code signing and time stamping use this (Authenticode).
pub fn chain_for(
    chain: &[&[u8]],
    at: i64,
    anchors: &[Anchor<'_>],
    purpose: Purpose,
) -> Result<(), CertError> {
    chain_path(chain, at, anchors, purpose).map(|_| ())
}
/// `chain_for`, giving the validated path (indices into `chain`, leaf
/// first, the anchor left out) and the index of the anchor it ends at, as
/// revocation checking walks it.
pub fn chain_path(
    chain: &[&[u8]],
    at: i64,
    anchors: &[Anchor<'_>],
    purpose: Purpose,
) -> Result<(Vec<usize>, usize), CertError> {
    let certs: Vec<Certificate<'_>> = chain
        .iter()
        .map(|c| x509::parse(c).ok_or(CertError::Malformed))
        .collect::<Result<_, _>>()?;
    let leaf = certs.first().ok_or(CertError::Malformed)?;
    if at < leaf.not_before || at > leaf.not_after {
        return Err(CertError::Expired);
    }
    if leaf.ca
        || !purpose.allows(leaf)
        || leaf
            .key_usage
            .is_some_and(|ku| ku & x509::KU_DIGITAL_SIGNATURE == 0)
    {
        return Err(CertError::WrongHost);
    }
    let mut path = alloc::vec![0usize];
    let anchor = extend(&certs, &mut path, at, anchors, purpose)?;
    Ok((path, anchor))
}
/// A CA's name constraints over every certificate below it on the path.
fn check_constraints(
    constraints: &[u8],
    certs: &[Certificate<'_>],
    path: &[usize],
) -> Result<(), CertError> {
    for (depth, &i) in path.iter().enumerate() {
        let c = &certs[i];
        // Self-issued intermediates are exempt from their own subject check.
        let subject = if depth == 0 { c.subject } else { &[] };
        if !constraints_permit(constraints, subject, &c.dns_names, &c.ip_addresses)? {
            return Err(CertError::Untrusted);
        }
    }
    Ok(())
}
fn extend(
    certs: &[Certificate<'_>],
    path: &mut Vec<usize>,
    now: i64,
    anchors: &[Anchor<'_>],
    purpose: Purpose,
) -> Result<usize, CertError> {
    let cert = &certs[*path.last().expect("leaf")];
    let mut error = CertError::Untrusted;
    // A trust anchor that signed this certificate ends the path.
    for (index, anchor) in anchors.iter().enumerate() {
        if anchor.subject != cert.issuer {
            continue;
        }
        let Some(key) = x509::spki(anchor.spki) else {
            continue;
        };
        match signature(&key, cert.sig_alg, cert.tbs, cert.signature) {
            Ok(()) => {
                if let Some(nc) = anchor.name_constraints {
                    let nc = der::Reader::new(nc)
                        .expect(der::SEQUENCE)
                        .map_or(nc, |s| s.value);
                    check_constraints(nc, certs, path)?;
                }
                return Ok(index);
            }
            Err(e) => error = e,
        }
    }
    if path.len() + 1 >= MAX_DEPTH {
        return Err(error);
    }
    // Otherwise each sent CA certificate naming this issuer, in turn.
    for (i, issuer) in certs.iter().enumerate() {
        if path.contains(&i) || issuer.subject != cert.issuer {
            continue;
        }
        let intermediates_below = path.len() - 1;
        let usable = issuer.ca
            && issuer
                .key_usage
                .is_none_or(|ku| ku & x509::KU_KEY_CERT_SIGN != 0)
            && purpose.allows(issuer)
            && issuer
                .path_len
                .is_none_or(|n| intermediates_below <= n as usize);
        if !usable {
            continue;
        }
        if now < issuer.not_before || now > issuer.not_after {
            error = CertError::Expired;
            continue;
        }
        let Some(key) = x509::spki(issuer.spki) else {
            error = CertError::UnsupportedAlgorithm;
            continue;
        };
        if let Err(e) = signature(&key, cert.sig_alg, cert.tbs, cert.signature) {
            error = e;
            continue;
        }
        if let Some(nc) = issuer.name_constraints
            && let Err(e) = check_constraints(nc, certs, path)
        {
            error = e;
            continue;
        }
        path.push(i);
        match extend(certs, path, now, anchors, purpose) {
            Ok(anchor) => return Ok(anchor),
            Err(e) => {
                error = e;
                path.pop();
            }
        }
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipv6_literal_hosts_match_ip_addresses_only() {
        let mut a = [0u8; 16];
        a[..4].copy_from_slice(&[0x20, 0x01, 0x0d, 0xb8]);
        a[15] = 1;
        assert_eq!(parse_ipv6("2001:db8::1"), Some(a));
        assert_eq!(parse_ipv6("[2001:DB8:0:0:0:0:0:1]"), Some(a));
        assert!(ip_matches(&a, "[2001:db8::1]"));
        assert!(!host_matches(b"2001:db8::1", "2001:db8::1"));
        let mut mapped = [0u8; 16];
        mapped[10..].copy_from_slice(&[0xff, 0xff, 10, 0, 2, 2]);
        assert_eq!(parse_ipv6("::ffff:10.0.2.2"), Some(mapped));
        let mut tail = [0u8; 16];
        tail[12..].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(parse_ipv6("::1.2.3.4"), Some(tail));
        for bad in [
            "2001:db8",
            "1::2::3",
            "12345::1",
            "1:2:3:4:5:6:7:8:9",
            "::g",
            "example.com",
        ] {
            assert_eq!(parse_ipv6(bad), None, "{bad}");
        }
    }
    #[test]
    fn wildcards_cover_one_label() {
        assert!(host_matches(b"example.com", "EXAMPLE.com"));
        assert!(host_matches(b"*.example.com", "www.example.com"));
        assert!(!host_matches(b"*.example.com", "a.b.example.com"));
        assert!(!host_matches(b"*.example.com", "example.com"));
        assert!(!host_matches(b"*.com", "example.com"));
        // IP literals never match dNSNames.
        assert!(!host_matches(b"*.2.3.4", "1.2.3.4"));
        assert!(!host_matches(b"1.2.3.4", "1.2.3.4"));
        assert!(ip_matches(&[1, 2, 3, 4], "1.2.3.4"));
    }
    const NOW: i64 = 1_800_000_000; // 2027-01-15
    fn root() -> Anchor<'static> {
        let c = x509::parse(include_bytes!("../tests/data/root.der")).unwrap();
        Anchor {
            subject: c.subject,
            spki: c.spki,
            name_constraints: None,
        }
    }
    fn check(certs: &[&[u8]], host: &str) -> Result<(), CertError> {
        chain_with(certs, host, NOW, &[root()])
    }
    const INTER: &[u8] = include_bytes!("../tests/data/inter.der");
    const LEAF: &[u8] = include_bytes!("../tests/data/leaf.der");
    #[test]
    fn valid_chains_and_their_names() {
        assert_eq!(check(&[LEAF, INTER], "test.doors.local"), Ok(()));
        assert_eq!(check(&[LEAF, INTER], "x.wild.doors.local"), Ok(()));
        assert_eq!(check(&[LEAF, INTER], "10.0.2.2"), Ok(()));
        // Extra unrelated or expired certificates do not matter.
        let extra = include_bytes!("../tests/data/leaf_expired.der");
        assert_eq!(check(&[LEAF, extra, INTER], "test.doors.local"), Ok(()));
    }
    #[test]
    fn invalid_chains_are_refused() {
        assert_eq!(
            check(&[LEAF, INTER], "other.doors.local"),
            Err(CertError::WrongHost)
        );
        assert_eq!(
            check(&[LEAF, INTER], "a.b.wild.doors.local"),
            Err(CertError::WrongHost)
        );
        assert_eq!(
            check(&[LEAF], "test.doors.local"),
            Err(CertError::Untrusted)
        );
        assert_eq!(
            chain(&[LEAF, INTER], "test.doors.local", NOW),
            Err(CertError::Untrusted)
        );
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_expired.der"), INTER],
                "test.doors.local"
            ),
            Err(CertError::Expired)
        );
        assert_eq!(
            chain_with(&[LEAF, INTER], "test.doors.local", 4_200_000_000, &[root()]),
            Err(CertError::Expired)
        );
        // A flipped signature byte.
        let mut bad = LEAF.to_vec();
        let n = bad.len();
        bad[n - 5] ^= 1;
        assert!(matches!(
            check(&[&bad, INTER], "test.doors.local"),
            Err(CertError::BadSignature | CertError::Malformed)
        ));
        // Client-only purpose, a CA certificate as the leaf, an issuer
        // without keyCertSign, an unknown critical extension, and a path
        // longer than pathLenConstraint 0 allows.
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_client.der"), INTER],
                "test.doors.local"
            ),
            Err(CertError::Untrusted)
        );
        let open = include_bytes!("../tests/data/inter_open.der");
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_ca.der"), open],
                "test.doors.local"
            ),
            Err(CertError::Untrusted)
        );
        assert_eq!(
            check(
                &[
                    include_bytes!("../tests/data/leaf_nosign.der"),
                    include_bytes!("../tests/data/inter_nosign.der")
                ],
                "test.doors.local"
            ),
            Err(CertError::Untrusted)
        );
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_crit.der"), open],
                "test.doors.local"
            ),
            Err(CertError::Malformed)
        );
        assert_eq!(
            check(
                &[
                    include_bytes!("../tests/data/leaf_deep.der"),
                    include_bytes!("../tests/data/deep_inter.der"),
                    INTER
                ],
                "test.doors.local"
            ),
            Err(CertError::Untrusted)
        );
    }
    #[test]
    fn problems_are_assessed_separately_and_policies_accept_them() {
        let anchors = [root()];
        let expired = include_bytes!("../tests/data/leaf_expired.der");
        assert!(!assess_with(&[LEAF, INTER], "test.doors.local", NOW, &anchors).any());
        let p = assess_with(&[LEAF, INTER], "other.doors.local", NOW, &anchors);
        assert_eq!(
            p,
            Problems {
                host: true,
                ..Problems::default()
            }
        );
        assert_eq!(p.refused(Policy::default()), Some(CertError::WrongHost));
        assert_eq!(
            p.refused(Policy {
                host: true,
                ..Policy::default()
            }),
            None
        );
        // Untrusted (no anchor) and expired, reported together.
        let p = assess_with(&[expired, INTER], "test.doors.local", NOW, &[]);
        assert!(p.date && p.untrusted && !p.host, "{p:?}");
        assert_eq!(
            p.refused(Policy {
                unknown_ca: true,
                ..Policy::default()
            }),
            Some(CertError::Expired)
        );
        assert_eq!(
            p.refused(Policy {
                unknown_ca: true,
                date: true,
                ..Policy::default()
            }),
            None
        );
        // A broken signature is never accepted.
        let mut bad = LEAF.to_vec();
        let n = bad.len();
        bad[n - 5] ^= 1;
        let p = assess_with(&[&bad, INTER], "test.doors.local", NOW, &anchors);
        assert!(p.signature || p.malformed);
        assert!(
            p.refused(Policy {
                unknown_ca: true,
                date: true,
                host: true,
                usage: true
            })
            .is_some()
        );
    }
    #[test]
    fn intermediate_name_constraints_apply() {
        let nc = include_bytes!("../tests/data/inter_nc.der");
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_in.der"), nc],
                "a.allowed.test"
            ),
            Ok(())
        );
        assert_eq!(
            check(
                &[include_bytes!("../tests/data/leaf_out.der"), nc],
                "evil.test"
            ),
            Err(CertError::Untrusted)
        );
        // A constrained anchor (TUBITAK-style) refuses names outside it.
        let constrained = Anchor {
            name_constraints: Some(b"\xa0\x070\x05\x82\x03.tr"),
            ..root()
        };
        assert_eq!(
            chain_with(&[LEAF, INTER], "test.doors.local", NOW, &[constrained]),
            Err(CertError::Untrusted)
        );
    }
    #[test]
    fn name_constraint_subtrees() {
        assert!(dns_in_subtree(b"api.open-meteo.com", b"open-meteo.com"));
        assert!(dns_in_subtree(b"open-meteo.com", b"open-meteo.com"));
        assert!(!dns_in_subtree(b"notopen-meteo.com", b"open-meteo.com"));
        assert!(!dns_in_subtree(b"open-meteo.com", b".open-meteo.com"));
        assert!(dns_in_subtree(b"x.gov.tr", b".tr"));
        assert!(!dns_in_subtree(b"api.open-meteo.com", b".tr"));
        assert!(ip_in_subtree(&[10, 1, 2, 3], &[10, 0, 0, 0, 255, 0, 0, 0]));
        assert!(!ip_in_subtree(&[11, 1, 2, 3], &[10, 0, 0, 0, 255, 0, 0, 0]));
        // TUBITAK's constraint (permitted dNSName ".tr", as webpki-roots
        // stores it) refuses names outside .tr.
        let tubitak = b"\xa0\x070\x05\x82\x03.tr";
        assert_eq!(
            constraints_permit(tubitak, &[], &[b"api.open-meteo.com"], &[]),
            Ok(false)
        );
        assert_eq!(
            constraints_permit(tubitak, &[], &[b"e-devlet.gov.tr"], &[]),
            Ok(true)
        );
        // Excluded subtree.
        let excluded = b"\xa1\x0e0\x0c\x82\x0aevil.local";
        assert_eq!(
            constraints_permit(excluded, &[], &[b"a.evil.local"], &[]),
            Ok(false)
        );
        assert_eq!(
            constraints_permit(excluded, &[], &[b"a.good.local"], &[]),
            Ok(true)
        );
    }
}
