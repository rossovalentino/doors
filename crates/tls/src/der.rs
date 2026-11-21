//! A strict DER reader for X.509 (definite lengths only, minimal length
//! encodings, no indefinite forms).

/// One TLV: tag, its contents, and the whole encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
    pub raw: &'a [u8],
}
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const OID: u8 = 0x06;
pub const BOOLEAN: u8 = 0x01;
pub const UTC_TIME: u8 = 0x17;
pub const GENERALIZED_TIME: u8 = 0x18;

/// A cursor over consecutive TLVs.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}
impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { rest: bytes }
    }
    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }
    pub fn peek_tag(&self) -> Option<u8> {
        self.rest.first().copied()
    }
    pub fn read(&mut self) -> Option<Tlv<'a>> {
        let b = self.rest;
        let tag = *b.first()?;
        if tag & 0x1F == 0x1F {
            return None; // multi-byte tags never occur in X.509
        }
        let first = *b.get(1)?;
        let (len, header) = if first < 0x80 {
            (usize::from(first), 2)
        } else {
            let n = usize::from(first & 0x7F);
            if n == 0 || n > 4 {
                return None;
            }
            let mut len = 0usize;
            for i in 0..n {
                len = (len << 8) | usize::from(*b.get(2 + i)?);
            }
            if len < 0x80 || (n > 1 && len >> (8 * (n - 1)) == 0) {
                return None; // non-minimal
            }
            (len, 2 + n)
        };
        let end = header.checked_add(len)?;
        if end > b.len() {
            return None;
        }
        self.rest = &b[end..];
        Some(Tlv {
            tag,
            value: &b[header..end],
            raw: &b[..end],
        })
    }
    /// The next TLV, which must have `tag`.
    pub fn expect(&mut self, tag: u8) -> Option<Tlv<'a>> {
        let t = self.read()?;
        (t.tag == tag).then_some(t)
    }
    /// The next TLV if it has `tag`.
    pub fn optional(&mut self, tag: u8) -> Option<Tlv<'a>> {
        if self.peek_tag() == Some(tag) {
            self.read()
        } else {
            None
        }
    }
}
/// The contents of a BIT STRING with no unused bits.
pub fn bits(t: Tlv<'_>) -> Option<&[u8]> {
    if t.tag != BIT_STRING || t.value.first() != Some(&0) {
        return None;
    }
    Some(&t.value[1..])
}
/// Days from 1970-01-01 to `y-m-d` (proleptic Gregorian).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
/// UTCTime / GeneralizedTime (Zulu, seconds) as Unix seconds.
pub fn time(t: Tlv<'_>) -> Option<i64> {
    let s = core::str::from_utf8(t.value).ok()?;
    let (year, rest) = match t.tag {
        UTC_TIME if s.len() == 13 => {
            let yy: i64 = s.get(..2)?.parse().ok()?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, &s[2..])
        }
        GENERALIZED_TIME if s.len() == 15 => (s.get(..4)?.parse().ok()?, &s[4..]),
        _ => return None,
    };
    if !rest.ends_with('Z') {
        return None;
    }
    let n = |a: usize| -> Option<i64> { rest.get(a..a + 2)?.parse().ok() };
    let (mo, d, h, mi, se) = (n(0)?, n(2)?, n(4)?, n(6)?, n(8)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(days(year, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lengths_and_times() {
        let mut r = Reader::new(&[0x30, 0x03, 0x02, 0x01, 0x05, 0x04, 0x00]);
        let s = r.expect(SEQUENCE).unwrap();
        assert_eq!(Reader::new(s.value).expect(INTEGER).unwrap().value, &[5]);
        assert_eq!(r.expect(OCTET_STRING).unwrap().value, &[] as &[u8]);
        // Non-minimal long form is refused.
        assert!(Reader::new(&[0x04, 0x81, 0x01, 0x00]).read().is_none());
        let utc = Tlv {
            tag: UTC_TIME,
            value: b"261102123000Z",
            raw: &[],
        };
        assert_eq!(time(utc), Some(1_793_622_600));
        let generalized = Tlv {
            tag: GENERALIZED_TIME,
            value: b"20500101000000Z",
            raw: &[],
        };
        assert_eq!(time(generalized), Some(2_524_608_000));
    }
}
