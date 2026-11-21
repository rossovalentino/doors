#![no_std]
//! Original DNS client API (dnsapi.dll): `DnsQuery_W/A/UTF8`, `DnsQueryEx`,
//! record lists, name validation and comparison, and resolver configuration.
//! Queries go on the wire to the adapter's DNS servers (UDP, falling back to
//! TCP when a reply is truncated), so every record type is answered, not
//! only addresses.
//!
//! This module is the host-testable part: the DNS message codec (RFC 1035,
//! with RFC 3596 AAAA and RFC 2782 SRV), name rules and the status mapping.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// DNS_STATUS values.
pub mod status {
    pub const SUCCESS: u32 = 0;
    pub const ERROR_INVALID_PARAMETER: u32 = 87;
    pub const ERROR_INVALID_NAME: u32 = 123;
    pub const ERROR_NOT_SUPPORTED: u32 = 50;
    pub const ERROR_OUTOFMEMORY: u32 = 14;
    pub const ERROR_MORE_DATA: u32 = 234;
    pub const ERROR_TIMEOUT: u32 = 1460;
    pub const RCODE_FORMAT_ERROR: u32 = 9001;
    pub const RCODE_SERVER_FAILURE: u32 = 9002;
    pub const RCODE_NAME_ERROR: u32 = 9003;
    pub const RCODE_NOT_IMPLEMENTED: u32 = 9004;
    pub const RCODE_REFUSED: u32 = 9005;
    pub const INFO_NO_RECORDS: u32 = 9501;
    pub const BAD_PACKET: u32 = 9502;
    pub const NO_PACKET: u32 = 9503;
    pub const REQUEST_PENDING: u32 = 9506;
    pub const NON_RFC_NAME: u32 = 9556;
    pub const INVALID_NAME_CHAR: u32 = 9560;
    pub const NUMERIC_NAME: u32 = 9561;
    pub const RECORD_DOES_NOT_EXIST: u32 = 9701;
    pub const NO_DNS_SERVERS: u32 = 9852;
}

/// Record types the codec decodes into structured data.
pub mod kind {
    pub const A: u16 = 1;
    pub const NS: u16 = 2;
    pub const CNAME: u16 = 5;
    pub const SOA: u16 = 6;
    pub const PTR: u16 = 12;
    pub const MX: u16 = 15;
    pub const TXT: u16 = 16;
    pub const AAAA: u16 = 28;
    pub const SRV: u16 = 33;
    pub const ANY: u16 = 255;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Data {
    A([u8; 4]),
    Aaaa([u8; 16]),
    /// NS, CNAME, PTR: one domain name.
    Name(String),
    Mx {
        preference: u16,
        exchange: String,
    },
    Srv {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    Txt(Vec<String>),
    Soa {
        primary: String,
        admin: String,
        serial: u32,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    },
    /// A type this codec does not decode: its RDATA.
    Other(Vec<u8>),
}

/// DNS_SECTION: where a record came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Answer = 1,
    Authority = 2,
    Additional = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub kind: u16,
    pub ttl: u32,
    pub section: Section,
    pub data: Data,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub id: u16,
    pub rcode: u8,
    pub truncated: bool,
    pub records: Vec<Record>,
}

/// The textual name without one trailing dot, checked for wire limits:
/// labels of 1..=63 bytes, at most 255 bytes encoded.
fn labels(name: &str) -> Result<Vec<&str>, u32> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<&str> = name.split('.').collect();
    let encoded: usize = parts.iter().map(|l| l.len() + 1).sum::<usize>() + 1;
    if encoded > 255 || parts.iter().any(|l| l.is_empty() || l.len() > 63) {
        return Err(status::ERROR_INVALID_NAME);
    }
    Ok(parts)
}

/// A standard recursive query for `name`/`kind` (class IN).
pub fn encode_query(id: u16, name: &str, kind: u16) -> Result<Vec<u8>, u32> {
    let mut m = Vec::with_capacity(32 + name.len());
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    m.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    for label in labels(name)? {
        m.push(label.len() as u8);
        m.extend_from_slice(label.as_bytes());
    }
    m.push(0);
    m.extend_from_slice(&kind.to_be_bytes());
    m.extend_from_slice(&1u16.to_be_bytes());
    Ok(m)
}

struct Reader<'a> {
    m: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn u8(&mut self) -> Result<u8, u32> {
        let v = *self.m.get(self.at).ok_or(status::BAD_PACKET)?;
        self.at += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, u32> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn u32(&mut self) -> Result<u32, u32> {
        Ok(u32::from(self.u16()?) << 16 | u32::from(self.u16()?))
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8], u32> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.m.len())
            .ok_or(status::BAD_PACKET)?;
        let b = &self.m[self.at..end];
        self.at = end;
        Ok(b)
    }
    /// A possibly compressed name; follows at most 64 pointers, each to an
    /// earlier offset.
    fn name(&mut self) -> Result<String, u32> {
        let mut out = String::new();
        let mut at = self.at;
        let mut jumped = false;
        let mut hops = 0;
        loop {
            let length = *self.m.get(at).ok_or(status::BAD_PACKET)?;
            match length & 0xc0 {
                0xc0 => {
                    let target = usize::from(u16::from_be_bytes([
                        length & 0x3f,
                        *self.m.get(at + 1).ok_or(status::BAD_PACKET)?,
                    ]));
                    if !jumped {
                        self.at = at + 2;
                        jumped = true;
                    }
                    hops += 1;
                    if hops > 64 || target >= at {
                        return Err(status::BAD_PACKET);
                    }
                    at = target;
                }
                0x00 => {
                    if length == 0 {
                        if !jumped {
                            self.at = at + 1;
                        }
                        return Ok(out);
                    }
                    let label = self
                        .m
                        .get(at + 1..at + 1 + usize::from(length))
                        .ok_or(status::BAD_PACKET)?;
                    if !out.is_empty() {
                        out.push('.');
                    }
                    // Non-ASCII bytes stay as Latin-1 code points (Windows
                    // reports them unchanged in the record name).
                    out.extend(label.iter().map(|b| char::from(*b)));
                    if out.len() > 255 {
                        return Err(status::BAD_PACKET);
                    }
                    at += 1 + usize::from(length);
                }
                _ => return Err(status::BAD_PACKET),
            }
        }
    }
}

/// Whether `reply` answers `query`: the same ID and one question, the
/// query's name (ASCII case aside), type and class (RFC 5452 section 9.1).
pub fn answers(query: &[u8], reply: &[u8]) -> bool {
    if query.len() < 12 || reply.len() < query.len() || reply[..2] != query[..2] {
        return false;
    }
    // The name (its length octets are at most 63, never a letter) folds
    // ASCII case; QTYPE and QCLASS, the last four octets, match exactly.
    if query.len() < 16 {
        return false;
    }
    let name = 12..query.len() - 4;
    reply[4..6] == [0, 1]
        && reply[name.clone()].eq_ignore_ascii_case(&query[name])
        && reply[query.len() - 4..query.len()] == query[query.len() - 4..]
}
/// Parses a response; records of every section, class IN only.
pub fn parse_response(m: &[u8]) -> Result<Response, u32> {
    let mut r = Reader { m, at: 0 };
    let id = r.u16()?;
    let flags = r.u16()?;
    if flags & 0x8000 == 0 {
        return Err(status::BAD_PACKET);
    }
    let counts = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
    for _ in 0..counts[0] {
        r.name()?;
        r.bytes(4)?;
    }
    let mut records = Vec::new();
    for (section, count) in [
        (Section::Answer, counts[1]),
        (Section::Authority, counts[2]),
        (Section::Additional, counts[3]),
    ] {
        for _ in 0..count {
            let name = r.name()?;
            let kind = r.u16()?;
            let class = r.u16()?;
            let ttl = r.u32()?;
            let length = usize::from(r.u16()?);
            let start = r.at;
            r.bytes(length)?;
            // OPT pseudo-records and other classes are not records of the
            // answer (Windows drops them too).
            if class != 1 || kind == 41 {
                continue;
            }
            let mut d = Reader { m, at: start };
            let data = match kind {
                kind::A if length == 4 => Data::A(d.bytes(4)?.try_into().unwrap()),
                kind::AAAA if length == 16 => Data::Aaaa(d.bytes(16)?.try_into().unwrap()),
                kind::NS | kind::CNAME | kind::PTR => Data::Name(d.name()?),
                kind::MX => Data::Mx {
                    preference: d.u16()?,
                    exchange: d.name()?,
                },
                kind::SRV => Data::Srv {
                    priority: d.u16()?,
                    weight: d.u16()?,
                    port: d.u16()?,
                    target: d.name()?,
                },
                kind::TXT => {
                    let mut strings = Vec::new();
                    while d.at < start + length {
                        let n = usize::from(d.u8()?);
                        strings.push(d.bytes(n)?.iter().map(|b| char::from(*b)).collect());
                    }
                    Data::Txt(strings)
                }
                kind::SOA => Data::Soa {
                    primary: d.name()?,
                    admin: d.name()?,
                    serial: d.u32()?,
                    refresh: d.u32()?,
                    retry: d.u32()?,
                    expire: d.u32()?,
                    minimum: d.u32()?,
                },
                _ => Data::Other(m[start..start + length].to_vec()),
            };
            if d.at > start + length {
                return Err(status::BAD_PACKET);
            }
            records.push(Record {
                name,
                kind,
                ttl,
                section,
                data,
            });
        }
    }
    Ok(Response {
        id,
        rcode: (flags & 0xf) as u8,
        truncated: flags & 0x0200 != 0,
        records,
    })
}

/// The DNS_STATUS of a parsed response to a `kind` query.
pub fn outcome(response: &Response, kind: u16) -> u32 {
    match response.rcode {
        0 => {
            let answered = response
                .records
                .iter()
                .any(|r| r.section == Section::Answer && (r.kind == kind || kind == kind::ANY));
            if answered {
                status::SUCCESS
            } else {
                status::INFO_NO_RECORDS
            }
        }
        1 => status::RCODE_FORMAT_ERROR,
        2 => status::RCODE_SERVER_FAILURE,
        3 => status::RCODE_NAME_ERROR,
        4 => status::RCODE_NOT_IMPLEMENTED,
        _ => status::RCODE_REFUSED,
    }
}

/// `in-addr.arpa` name of an IPv4 address (for PTR queries).
pub fn reverse_name(address: [u8; 4]) -> String {
    alloc::format!(
        "{}.{}.{}.{}.in-addr.arpa",
        address[3],
        address[2],
        address[1],
        address[0]
    )
}

/// Strict dotted-quad IPv4 text.
pub fn ipv4_literal(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// DNS_NAME_FORMAT values of `DnsValidateName`.
pub mod format {
    pub const DOMAIN: u32 = 0;
    pub const DOMAIN_LABEL: u32 = 1;
    pub const HOSTNAME_FULL: u32 = 2;
    pub const HOSTNAME_LABEL: u32 = 3;
    pub const WILDCARD: u32 = 4;
    pub const SRV_RECORD: u32 = 5;
    pub const VALIDATE_TLD: u32 = 6;
}

/// `DnsValidateName`, as measured on Windows 11 26H2: structure first
/// (labels of 1..=63 bytes, 255 in all; one label for the *_LABEL formats,
/// a leading `*` label for WILDCARD, a leading `_` label for
/// SRV_RECORD) is ERROR_INVALID_NAME; then any ASCII character outside
/// letters, digits, `-` and `_` is DNS_ERROR_INVALID_NAME_CHAR; an
/// underscore outside SRV names is DNS_ERROR_NON_RFC_NAME. Non-ASCII
/// (UTF-8) characters are accepted. VALIDATE_TLD only rejects an
/// all-numeric top-level label (DNS_ERROR_NUMERIC_NAME).
pub fn validate_name(name: &str, format: u32) -> u32 {
    if format > format::VALIDATE_TLD {
        return status::ERROR_INVALID_PARAMETER;
    }
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    let parts: Vec<&str> = trimmed.split('.').collect();
    if format == format::VALIDATE_TLD {
        let numeric = parts.len() > 1
            && parts[parts.len() - 1].bytes().all(|b| b.is_ascii_digit())
            && !parts[parts.len() - 1].is_empty();
        return if numeric {
            status::NUMERIC_NAME
        } else {
            status::SUCCESS
        };
    }
    if trimmed.is_empty()
        || trimmed.len() > 255
        || parts.iter().any(|l| l.is_empty() || l.len() > 63)
    {
        return status::ERROR_INVALID_NAME;
    }
    let checked = match format {
        format::DOMAIN_LABEL | format::HOSTNAME_LABEL if parts.len() != 1 => {
            return status::ERROR_INVALID_NAME;
        }
        format::WILDCARD if parts[0] != "*" => return status::ERROR_INVALID_NAME,
        format::WILDCARD => &parts[1..],
        format::SRV_RECORD if !parts[0].starts_with('_') => return status::ERROR_INVALID_NAME,
        _ => &parts[..],
    };
    let mut non_rfc = false;
    for label in checked {
        for c in label.chars() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' => (),
                '_' => non_rfc |= format != format::SRV_RECORD,
                c if c.is_ascii() => return status::INVALID_NAME_CHAR,
                _ => (),
            }
        }
    }
    if non_rfc {
        status::NON_RFC_NAME
    } else {
        status::SUCCESS
    }
}

/// `DnsNameCompare`: case-insensitive (ASCII), one trailing dot ignored.
pub fn names_equal(a: &str, b: &str) -> bool {
    let a = a.strip_suffix('.').unwrap_or(a);
    let b = b.strip_suffix('.').unwrap_or(b);
    a.len() == b.len()
        && a.chars()
            .zip(b.chars())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

#[cfg(test)]
mod tests;
