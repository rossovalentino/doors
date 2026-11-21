//! Address and service text, as Winsock parses and formats it. Pure
//! functions; host tested.

/// `inet_addr`'s classic BSD forms: `a.b.c.d`, `a.b.c` (c is 16 bits),
/// `a.b` (b is 24 bits) and `a` (32 bits), each part decimal, octal
/// (leading 0) or hexadecimal (0x). Returns the address in network order
/// as bytes; None for anything else (`INADDR_NONE`).
pub fn inet_addr(text: &[u8]) -> Option<[u8; 4]> {
    let text = trim(text);
    if text.is_empty() {
        return None;
    }
    let mut parts = [0u64; 4];
    let mut count = 0;
    for part in text.split(|b| *b == b'.') {
        if count == 4 {
            return None;
        }
        parts[count] = number(part)?;
        count += 1;
    }
    let value: u64 = match count {
        1 => parts[0],
        2 if parts[0] <= 0xFF && parts[1] <= 0xFF_FFFF => parts[0] << 24 | parts[1],
        3 if parts[0] <= 0xFF && parts[1] <= 0xFF && parts[2] <= 0xFFFF => {
            parts[0] << 24 | parts[1] << 16 | parts[2]
        }
        4 if parts.iter().all(|p| *p <= 0xFF) => {
            parts[0] << 24 | parts[1] << 16 | parts[2] << 8 | parts[3]
        }
        _ => return None,
    };
    u32::try_from(value).ok().map(u32::to_be_bytes)
}
/// One numeric part: decimal, octal (leading 0) or hexadecimal (0x).
fn number(part: &[u8]) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (digits, radix) = match part {
        [b'0', b'x' | b'X', rest @ ..] => (rest, 16),
        [b'0', rest @ ..] if !rest.is_empty() => (rest, 8),
        _ => (part, 10),
    };
    if digits.is_empty() || digits.len() > 11 {
        return None;
    }
    let mut value = 0u64;
    for d in digits {
        let v = (*d as char).to_digit(radix)?;
        value = value * u64::from(radix) + u64::from(v);
    }
    Some(value)
}
fn trim(text: &[u8]) -> &[u8] {
    // inet_addr stops at the first space, as Windows does.
    let end = text
        .iter()
        .position(|b| *b == b' ' || *b == 0)
        .unwrap_or(text.len());
    &text[..end]
}
/// `inet_pton(AF_INET, ...)`: exactly four decimal parts 0-255, no leading
/// zeros beyond a single 0.
pub fn pton4(text: &[u8]) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split(|b| *b == b'.');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty()
            || part.len() > 3
            || (part.len() > 1 && part[0] == b'0')
            || !part.iter().all(u8::is_ascii_digit)
        {
            return None;
        }
        let v: u32 = part.iter().fold(0, |acc, d| acc * 10 + u32::from(d - b'0'));
        *slot = u8::try_from(v).ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// Dotted-decimal text of an address (`inet_ntoa`, `inet_ntop`): the bytes
/// written into `out` (at most 15).
pub fn ntop4(address: [u8; 4], out: &mut [u8; 16]) -> usize {
    let mut at = 0;
    for (i, byte) in address.iter().enumerate() {
        if i > 0 {
            out[at] = b'.';
            at += 1;
        }
        let (h, t, o) = (byte / 100, byte / 10 % 10, byte % 10);
        if h > 0 {
            out[at] = b'0' + h;
            at += 1;
        }
        if h > 0 || t > 0 {
            out[at] = b'0' + t;
            at += 1;
        }
        out[at] = b'0' + o;
        at += 1;
    }
    at
}
/// `inet_pton(AF_INET6, ...)` (RFC 4291 §2.2): eight groups of up to four
/// hexadecimal digits, at most one `::`, an optional trailing dotted IPv4
/// address. A `%scope` suffix is not part of the address (callers strip it).
pub fn pton6(text: &[u8]) -> Option<[u8; 16]> {
    if text.is_empty() || text.len() > 45 {
        return None;
    }
    // Split at "::" (at most once).
    let (head, tail, compressed) = match text.windows(2).position(|w| w == b"::") {
        Some(at) => {
            let (h, t) = (&text[..at], &text[at + 2..]);
            if t.windows(2).any(|w| w == b"::") {
                return None;
            }
            (h, t, true)
        }
        None => (text, &[][..], false),
    };
    let groups = |part: &[u8], last_part: bool| -> Option<([u16; 8], usize)> {
        let mut out = [0u16; 8];
        let mut n = 0;
        if part.is_empty() {
            return Some((out, 0));
        }
        let pieces: alloc::vec::Vec<&[u8]> = part.split(|b| *b == b':').collect();
        for (i, piece) in pieces.iter().enumerate() {
            let last = i + 1 == pieces.len();
            if last && last_part && piece.contains(&b'.') {
                let v4 = pton4(piece)?;
                if n + 2 > 8 {
                    return None;
                }
                out[n] = u16::from_be_bytes([v4[0], v4[1]]);
                out[n + 1] = u16::from_be_bytes([v4[2], v4[3]]);
                n += 2;
                continue;
            }
            if piece.is_empty() || piece.len() > 4 || n == 8 {
                return None;
            }
            let mut v = 0u16;
            for d in *piece {
                v = v << 4 | (*d as char).to_digit(16)? as u16;
            }
            out[n] = v;
            n += 1;
        }
        Some((out, n))
    };
    let (front, nf) = groups(head, !compressed)?;
    let (back, nb) = groups(tail, true)?;
    if compressed {
        if nf + nb > 7 {
            return None;
        }
    } else if nf != 8 {
        return None;
    }
    let mut words = [0u16; 8];
    words[..nf].copy_from_slice(&front[..nf]);
    words[8 - nb..].copy_from_slice(&back[..nb]);
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes());
    }
    Some(out)
}
/// IPv6 text as Windows writes it (RFC 5952: lower case, no leading zeros,
/// the longest run of two or more zero groups as `::`, an IPv4-mapped
/// address as `::ffff:a.b.c.d`). Bytes written into `out` (at most 45).
pub fn ntop6(address: [u8; 16], out: &mut [u8; 46]) -> usize {
    let mut at = 0;
    let mut push = |b: u8, at: &mut usize| {
        out[*at] = b;
        *at += 1;
    };
    if address[..10] == [0; 10] && address[10..12] == [0xff, 0xff] {
        for b in b"::ffff:" {
            push(*b, &mut at);
        }
        let mut v4 = [0u8; 16];
        let n = ntop4(address[12..16].try_into().unwrap(), &mut v4);
        for b in &v4[..n] {
            push(*b, &mut at);
        }
        return at;
    }
    let words: [u16; 8] =
        core::array::from_fn(|i| u16::from_be_bytes([address[2 * i], address[2 * i + 1]]));
    // The longest run of zero words (two or more), first one on ties.
    let (mut best, mut best_len, mut run, mut run_len) = (8, 0, 0, 0);
    for (i, w) in words.iter().enumerate() {
        if *w == 0 {
            if run_len == 0 {
                run = i;
            }
            run_len += 1;
            if run_len > best_len {
                best = run;
                best_len = run_len;
            }
        } else {
            run_len = 0;
        }
    }
    if best_len < 2 {
        best = 8;
    }
    let mut i = 0;
    while i < 8 {
        if i == best {
            push(b':', &mut at);
            if i == 0 {
                push(b':', &mut at);
            }
            i += best_len;
            continue;
        }
        let w = words[i];
        let mut started = false;
        for shift in [12, 8, 4, 0] {
            let d = (w >> shift) & 0xf;
            if d != 0 || started || shift == 0 {
                started = true;
                push(b"0123456789abcdef"[d as usize], &mut at);
            }
        }
        i += 1;
        if i < 8 {
            push(b':', &mut at);
        }
    }
    at
}
/// Well-known services (the `services` file Windows ships, common entries):
/// (name, port, protocols: 1 TCP, 2 UDP, 3 both).
pub const SERVICES: [(&str, u16, u8); 22] = [
    ("echo", 7, 3),
    ("discard", 9, 3),
    ("daytime", 13, 3),
    ("ftp-data", 20, 1),
    ("ftp", 21, 1),
    ("ssh", 22, 1),
    ("telnet", 23, 1),
    ("smtp", 25, 1),
    ("domain", 53, 3),
    ("http", 80, 1),
    ("www", 80, 1),
    ("www-http", 80, 1),
    ("kerberos", 88, 3),
    ("pop3", 110, 1),
    ("ntp", 123, 2),
    ("imap", 143, 1),
    ("snmp", 161, 2),
    ("ldap", 389, 1),
    ("https", 443, 3),
    ("microsoft-ds", 445, 1),
    ("submission", 587, 1),
    ("imaps", 993, 1),
];
/// A service name or decimal port for `protocol` (1 TCP, 2 UDP, 0 any).
pub fn service_port(text: &[u8], protocol: u8) -> Option<u16> {
    if !text.is_empty() && text.iter().all(u8::is_ascii_digit) {
        let v: u32 = text.iter().try_fold(0u32, |acc, d| {
            acc.checked_mul(10)?.checked_add(u32::from(d - b'0'))
        })?;
        return u16::try_from(v).ok();
    }
    SERVICES
        .iter()
        .find(|(name, _, protocols)| {
            name.as_bytes().eq_ignore_ascii_case(text)
                && (protocol == 0 || protocols & protocol != 0)
        })
        .map(|(_, port, _)| *port)
}
/// The service name of `port` for `protocol` (`getservbyport`, NI_NAMEREQD-free `getnameinfo`).
pub fn service_name(port: u16, protocol: u8) -> Option<&'static str> {
    SERVICES
        .iter()
        .find(|(_, p, protocols)| *p == port && (protocol == 0 || protocols & protocol != 0))
        .map(|(name, _, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inet_addr_accepts_the_bsd_forms_windows_accepts() {
        assert_eq!(inet_addr(b"10.0.2.15"), Some([10, 0, 2, 15]));
        assert_eq!(inet_addr(b"127.1"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr(b"0x7f.0.0.1"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr(b"0177.0.0.01"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr(b"192.168.257"), Some([192, 168, 1, 1]));
        assert_eq!(inet_addr(b"3232235777"), Some([192, 168, 1, 1]));
        assert_eq!(inet_addr(b"1.2.3.4 trailing"), Some([1, 2, 3, 4]));
        for bad in [
            &b""[..],
            b"256.1.1.1",
            b"1.2.3.4.5",
            b"1..2.3",
            b"a.b.c.d",
            b"09.1.1.1",
        ] {
            assert_eq!(inet_addr(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }
    #[test]
    fn inet_pton_is_strict_and_ntop_round_trips() {
        assert_eq!(pton4(b"192.168.0.1"), Some([192, 168, 0, 1]));
        for bad in [&b"127.1"[..], b"01.2.3.4", b"1.2.3.256", b"1.2.3.4 "] {
            assert_eq!(pton4(bad), None);
        }
        let mut out = [0u8; 16];
        for address in [
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            [10, 0, 2, 15],
            [100, 20, 3, 0],
        ] {
            let n = ntop4(address, &mut out);
            assert_eq!(pton4(&out[..n]), Some(address));
        }
    }
    #[test]
    fn ipv6_text_parses_and_formats_as_windows_does() {
        let mut out = [0u8; 46];
        for (text, canonical) in [
            ("::", "::"),
            ("::1", "::1"),
            ("fe80::5054:ff:fed0:501", "fe80::5054:ff:fed0:501"),
            ("FEC0:0:0:0:0:0:0:2", "fec0::2"),
            ("2001:db8:0:0:1:0:0:1", "2001:db8::1:0:0:1"),
            ("2001:db8::", "2001:db8::"),
            ("1:2:3:4:5:6:7:8", "1:2:3:4:5:6:7:8"),
            ("1:0:3:4:5:6:7:8", "1:0:3:4:5:6:7:8"),
            ("::ffff:10.0.2.2", "::ffff:10.0.2.2"),
            ("::ffff:a00:202", "::ffff:10.0.2.2"),
        ] {
            let a = pton6(text.as_bytes()).unwrap_or_else(|| panic!("{text}"));
            let n = ntop6(a, &mut out);
            assert_eq!(core::str::from_utf8(&out[..n]).unwrap(), canonical);
            assert_eq!(pton6(&out[..n]), Some(a));
        }
        for bad in [
            &b""[..],
            b":",
            b":::",
            b"1::2::3",
            b"1:2:3:4:5:6:7:8:9",
            b"12345::",
            b"1:2:3:4:5:6:7",
            b"g::1",
            b"::1.2.3",
            b"1:2:3:4:5:6:7:1.2.3.4",
        ] {
            assert_eq!(pton6(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }
    #[test]
    fn services_resolve_by_name_number_and_protocol() {
        assert_eq!(service_port(b"https", 1), Some(443));
        assert_eq!(service_port(b"HTTP", 0), Some(80));
        assert_eq!(service_port(b"ntp", 1), None);
        assert_eq!(service_port(b"ntp", 2), Some(123));
        assert_eq!(service_port(b"8080", 1), Some(8080));
        assert_eq!(service_port(b"70000", 1), None);
        assert_eq!(service_name(443, 1), Some("https"));
        assert_eq!(service_name(8080, 1), None);
    }
}
