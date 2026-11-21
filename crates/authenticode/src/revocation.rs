//! Revocation by certificate revocation lists (RFC 5280 section 5), as
//! Windows' chain engine checks a code-signing chain when WinVerifyTrust is
//! asked to (WTD_REVOKE_WHOLECHAIN): every certificate below the root names
//! a CRL in its CRL distribution points; a CRL its issuer signed, current
//! now, must not list it. A certificate revoked after a signature's
//! verified timestamp still vouches for that signature (Windows' rule for
//! timestamped code). CRLs come from the caller (`Source`: a cache and the
//! network, which this crate does not touch). A certificate naming no CRL,
//! or whose CRLs cannot be had, is a revocation failure
//! (CERT_E_REVOCATION_FAILURE), as Windows reports an offline check.
use super::*;

/// Where CRLs come from.
pub trait Source {
    /// The DER CRL `url` names, or None when it cannot be had (offline,
    /// or nothing cached under cache-only retrieval).
    fn crl(&mut self, url: &str) -> Option<Vec<u8>>;
}

const OID_CRL_DISTRIBUTION_POINTS: &[u8] = &[0x55, 0x1D, 0x1F];

/// A parsed CRL.
pub struct Crl<'a> {
    tbs: &'a [u8],
    sig_alg: x509::SigAlg,
    signature: &'a [u8],
    issuer: &'a [u8],
    this_update: i64,
    next_update: Option<i64>,
    /// (serial number contents, revocation time).
    revoked: Vec<(&'a [u8], i64)>,
}
/// A DER CRL (CertificateList).
pub fn parse(crl: &[u8]) -> Option<Crl<'_>> {
    let mut outer = Reader::new(Reader::new(crl).expect(der::SEQUENCE)?.value);
    let tbs = outer.expect(der::SEQUENCE)?;
    let sig_alg = x509::sig_alg(outer.expect(der::SEQUENCE)?)?;
    let signature = der::bits(outer.expect(der::BIT_STRING)?)?;
    let mut t = Reader::new(tbs.value);
    t.optional(der::INTEGER);
    t.expect(der::SEQUENCE)?;
    let issuer = t.expect(der::SEQUENCE)?.value;
    let this_update = der::time(t.read()?)?;
    let next_update = match t.peek_tag() {
        Some(der::UTC_TIME | der::GENERALIZED_TIME) => Some(der::time(t.read()?)?),
        _ => None,
    };
    let mut revoked = Vec::new();
    if let Some(list) = t.optional(der::SEQUENCE) {
        let mut list = Reader::new(list.value);
        while let Some(entry) = list.read() {
            let mut e = Reader::new(entry.value);
            let serial = e.expect(der::INTEGER)?.value;
            let when = der::time(e.read()?)?;
            revoked.push((serial, when));
        }
    }
    Some(Crl {
        tbs: tbs.raw,
        sig_alg,
        signature,
        issuer,
        this_update,
        next_update,
        revoked,
    })
}
impl Crl<'_> {
    /// thisUpdate (Unix seconds).
    pub fn this_update(&self) -> i64 {
        self.this_update
    }
    /// nextUpdate, when the CRL states one.
    pub fn next_update(&self) -> Option<i64> {
        self.next_update
    }
    /// Signed by the holder of `issuer_spki` named `issuer_name`.
    fn issued_by(&self, issuer_name: &[u8], issuer_spki: &[u8]) -> bool {
        self.issuer == issuer_name
            && x509::spki(issuer_spki).is_some_and(|key| {
                door_tls::verify::signature(&key, self.sig_alg, self.tbs, self.signature).is_ok()
            })
    }
    /// When the certificate with serial `serial` was revoked.
    pub fn revoked(&self, serial: &[u8]) -> Option<i64> {
        let strip = |s: &[u8]| s.iter().take_while(|&&b| b == 0).count();
        self.revoked
            .iter()
            .find(|(s, _)| s[strip(s)..] == serial[strip(serial)..])
            .map(|(_, when)| *when)
    }
}
/// The http:// URLs of a certificate's CRL distribution points.
pub fn distribution_points(cert_der: &[u8]) -> Vec<String> {
    let mut urls = Vec::new();
    let Some(extensions) = extensions(cert_der) else {
        return urls;
    };
    let mut list = Reader::new(extensions);
    while let Some(ext) = list.read() {
        let mut e = Reader::new(ext.value);
        if e.expect(der::OID).map(|o| o.value) != Some(OID_CRL_DISTRIBUTION_POINTS) {
            continue;
        }
        e.optional(der::BOOLEAN);
        // CRLDistributionPoints: SEQUENCE OF DistributionPoint {
        // distributionPoint [0] { fullName [0] GeneralNames } ... }.
        let Some(points) = e
            .expect(der::OCTET_STRING)
            .and_then(|v| Reader::new(v.value).expect(der::SEQUENCE))
        else {
            continue;
        };
        let mut points = Reader::new(points.value);
        while let Some(point) = points.read() {
            let Some(full) = Reader::new(point.value)
                .optional(0xA0)
                .and_then(|dp| Reader::new(dp.value).optional(0xA0))
            else {
                continue;
            };
            let mut names = Reader::new(full.value);
            while let Some(name) = names.read() {
                // uniformResourceIdentifier [6] IA5String.
                if name.tag == 0x86
                    && let Ok(url) = core::str::from_utf8(name.value)
                    && url.len() > 7
                    && url[..7].eq_ignore_ascii_case("http://")
                {
                    urls.push(String::from(url));
                }
            }
        }
    }
    urls
}
/// The contents of a certificate's extensions ([3] EXPLICIT SEQUENCE).
fn extensions(cert_der: &[u8]) -> Option<&[u8]> {
    let mut c = Reader::new(Reader::new(cert_der).expect(der::SEQUENCE)?.value);
    let mut tbs = Reader::new(c.expect(der::SEQUENCE)?.value);
    while let Some(field) = tbs.read() {
        if field.tag == 0xA3 {
            return Some(Reader::new(field.value).expect(der::SEQUENCE)?.value);
        }
    }
    None
}
/// Checks every certificate of a validated path (`path`: leaf first,
/// indices into `certs`, ending below `anchor`) at Unix time `now`, for a
/// signature timestamped at `stamp`.
pub(crate) fn check(
    certs: &[&[u8]],
    path: &[usize],
    anchor: &Anchor<'_>,
    now: i64,
    stamp: Option<i64>,
    source: &mut dyn Source,
) -> Result<(), Error> {
    for (depth, &i) in path.iter().enumerate() {
        let (issuer_name, issuer_spki) = match path.get(depth + 1) {
            Some(&parent) => {
                let p = x509::parse(certs[parent]).ok_or(Error::Malformed)?;
                (p.subject, p.spki)
            }
            None => (anchor.subject, anchor.spki),
        };
        let serial = serial_of(certs[i]).ok_or(Error::Malformed)?;
        let mut checked = false;
        for url in distribution_points(certs[i]) {
            let Some(bytes) = source.crl(&url) else {
                continue;
            };
            let Some(crl) = parse(&bytes) else {
                continue;
            };
            // Signed by the issuer and current now; a stale CRL is no answer.
            if !crl.issued_by(issuer_name, issuer_spki)
                || crl.this_update > now
                || crl.next_update.is_some_and(|n| n < now)
            {
                continue;
            }
            if let Some(when) = crl.revoked(serial)
                && stamp.is_none_or(|s| when <= s)
            {
                return Err(Error::Revoked);
            }
            checked = true;
            break;
        }
        if !checked {
            return Err(Error::RevocationOffline);
        }
    }
    Ok(())
}
