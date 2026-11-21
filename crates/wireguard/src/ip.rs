//! IPv4 pieces the tunnel needs: prefixes (AllowedIPs, cryptokey routing),
//! packet addresses and lengths, and TCP MSS clamping so segments fit the
//! tunnel's MTU.
use core::fmt;

/// An IPv4 prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub address: [u8; 4],
    pub prefix: u8,
}
impl Cidr {
    pub const DEFAULT: Self = Self {
        address: [0; 4],
        prefix: 0,
    };
    pub const fn new(address: [u8; 4], prefix: u8) -> Self {
        Self { address, prefix }
    }
    fn mask(&self) -> u32 {
        if self.prefix == 0 {
            0
        } else {
            u32::MAX << (32 - u32::from(self.prefix.min(32)))
        }
    }
    pub fn contains(&self, address: [u8; 4]) -> bool {
        (u32::from_be_bytes(address) ^ u32::from_be_bytes(self.address)) & self.mask() == 0
    }
    /// The network address (host bits cleared).
    pub fn network(&self) -> [u8; 4] {
        (u32::from_be_bytes(self.address) & self.mask()).to_be_bytes()
    }
    /// `a.b.c.d/n`; a bare address is /32.
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, p.parse::<u8>().ok().filter(|p| *p <= 32)?),
            None => (text, 32),
        };
        Some(Self {
            address: parse_ipv4(address)?,
            prefix,
        })
    }
}
impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let a = self.address;
        write!(f, "{}.{}.{}.{}/{}", a[0], a[1], a[2], a[3], self.prefix)
    }
}
/// Dotted decimal, four parts, no leading zeros beyond "0".
pub fn parse_ipv4(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.split('.');
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// The longest prefix in `list` holding `address`.
pub fn longest(list: &[Cidr], address: [u8; 4]) -> Option<u8> {
    list.iter()
        .filter(|c| c.contains(address))
        .map(|c| c.prefix)
        .max()
}

/// Version, header length and total length of an IPv4 packet whose header
/// is well formed (`len` within the buffer).
pub struct Header {
    pub header: usize,
    pub total: usize,
    pub protocol: u8,
    pub source: [u8; 4],
    pub destination: [u8; 4],
}
pub fn header(packet: &[u8]) -> Option<Header> {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return None;
    }
    let header = usize::from(packet[0] & 0x0F) * 4;
    let total = usize::from(u16::from_be_bytes([packet[2], packet[3]]));
    if header < 20 || total < header || total > packet.len() {
        return None;
    }
    Some(Header {
        header,
        total,
        protocol: packet[9],
        source: packet[12..16].try_into().unwrap(),
        destination: packet[16..20].try_into().unwrap(),
    })
}
fn fold(mut sum: u32) -> u16 {
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    sum as u16
}
/// Lower the MSS option of a TCP SYN (either direction) to `max`, fixing
/// the checksum incrementally (RFC 1624). Answers whether it changed.
pub fn clamp_mss(packet: &mut [u8], max: u16) -> bool {
    let Some(h) = header(packet) else {
        return false;
    };
    // TCP, not a fragment.
    if h.protocol != 6 || u16::from_be_bytes([packet[6], packet[7]]) & 0x3FFF != 0 {
        return false;
    }
    let tcp = &mut packet[h.header..h.total];
    if tcp.len() < 20 || tcp[13] & 0x02 == 0 {
        return false;
    }
    let offset = usize::from(tcp[12] >> 4) * 4;
    if offset < 20 || offset > tcp.len() {
        return false;
    }
    let mut at = 20;
    while at < offset {
        match tcp[at] {
            0 => break,
            1 => at += 1,
            kind => {
                let Some(&len) = tcp.get(at + 1) else { break };
                let len = usize::from(len);
                if len < 2 || at + len > offset {
                    break;
                }
                if kind == 2 && len == 4 {
                    let old = u16::from_be_bytes([tcp[at + 2], tcp[at + 3]]);
                    if old <= max {
                        return false;
                    }
                    tcp[at + 2..at + 4].copy_from_slice(&max.to_be_bytes());
                    // The option starts at an even offset or not: adjust the
                    // 16-bit word it lies in.
                    let (word_old, word_new) = if at % 2 == 0 {
                        (old, max)
                    } else {
                        // Bytes at+2,at+3 straddle two words: recompute.
                        return fix_checksum_full(tcp, &h);
                    };
                    let check = u16::from_be_bytes([tcp[16], tcp[17]]);
                    // HC' = ~(~HC + ~m + m')
                    let sum = u32::from(!check) + u32::from(!word_old) + u32::from(word_new);
                    tcp[16..18].copy_from_slice(&(!fold(sum)).to_be_bytes());
                    return true;
                }
                at += len;
            }
        }
    }
    false
}
/// An IPv6 prefix (an interface address with its prefix length, or an
/// AllowedIPs entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr6 {
    pub address: [u8; 16],
    pub prefix: u8,
}
impl Cidr6 {
    pub const fn new(address: [u8; 16], prefix: u8) -> Self {
        Self { address, prefix }
    }
    pub fn contains(&self, address: [u8; 16]) -> bool {
        let p = u128::from_be_bytes(self.address);
        let a = u128::from_be_bytes(address);
        let mask = if self.prefix == 0 {
            0
        } else {
            u128::MAX << (128 - u32::from(self.prefix.min(128)))
        };
        (p ^ a) & mask == 0
    }
    /// `fd00::2/64`; a bare address is /128.
    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, p.parse::<u8>().ok().filter(|p| *p <= 128)?),
            None => (text, 128),
        };
        let address: core::net::Ipv6Addr = address.parse().ok()?;
        Some(Self {
            address: address.octets(),
            prefix,
        })
    }
}
impl fmt::Display for Cidr6 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}",
            core::net::Ipv6Addr::from(self.address),
            self.prefix
        )
    }
}
/// The longest prefix in `list` holding `address`.
pub fn longest6(list: &[Cidr6], address: [u8; 16]) -> Option<u8> {
    list.iter()
        .filter(|c| c.contains(address))
        .map(|c| c.prefix)
        .max()
}
/// An IPv6 packet's fixed header: its whole length, next header and
/// addresses.
pub struct Header6 {
    pub total: usize,
    pub next: u8,
    pub source: [u8; 16],
    pub destination: [u8; 16],
}
pub fn header6(packet: &[u8]) -> Option<Header6> {
    if packet.len() < 40 || packet[0] >> 4 != 6 {
        return None;
    }
    let total = 40 + usize::from(u16::from_be_bytes([packet[4], packet[5]]));
    if total > packet.len() {
        return None;
    }
    Some(Header6 {
        total,
        next: packet[6],
        source: packet[8..24].try_into().unwrap(),
        destination: packet[24..40].try_into().unwrap(),
    })
}
/// The IPv6 form of `clamp_mss` (a TCP SYN directly after the fixed
/// header), the checksum recomputed over the pseudo-header.
pub fn clamp_mss6(packet: &mut [u8], max: u16) -> bool {
    let Some(h) = header6(packet) else {
        return false;
    };
    if h.next != 6 {
        return false;
    }
    let tcp = &mut packet[40..h.total];
    if tcp.len() < 20 || tcp[13] & 0x02 == 0 {
        return false;
    }
    let offset = usize::from(tcp[12] >> 4) * 4;
    if offset < 20 || offset > tcp.len() {
        return false;
    }
    let mut at = 20;
    let mut changed = false;
    while at < offset {
        match tcp[at] {
            0 => break,
            1 => at += 1,
            kind => {
                let Some(&len) = tcp.get(at + 1) else { break };
                let len = usize::from(len);
                if len < 2 || at + len > offset {
                    break;
                }
                if kind == 2 && len == 4 {
                    let old = u16::from_be_bytes([tcp[at + 2], tcp[at + 3]]);
                    if old > max {
                        tcp[at + 2..at + 4].copy_from_slice(&max.to_be_bytes());
                        changed = true;
                    }
                    break;
                }
                at += len;
            }
        }
    }
    if changed {
        tcp[16] = 0;
        tcp[17] = 0;
        let mut sum = 0u32;
        for address in [h.source, h.destination] {
            for pair in address.chunks(2) {
                sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
            }
        }
        sum += tcp.len() as u32 + 6;
        for chunk in tcp.chunks(2) {
            sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
        }
        tcp[16..18].copy_from_slice(&(!fold(sum)).to_be_bytes());
    }
    changed
}
fn fix_checksum_full(tcp: &mut [u8], h: &Header) -> bool {
    tcp[16] = 0;
    tcp[17] = 0;
    let mut sum = 0u32;
    for pair in [h.source, h.destination] {
        sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]))
            + u32::from(u16::from_be_bytes([pair[2], pair[3]]));
    }
    sum += 6 + tcp.len() as u32;
    for chunk in tcp.chunks(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
    }
    tcp[16..18].copy_from_slice(&(!fold(sum)).to_be_bytes());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn prefixes_parse_and_match() {
        let c = Cidr::parse("10.8.0.2/24").unwrap();
        assert!(c.contains([10, 8, 0, 200]) && !c.contains([10, 8, 1, 1]));
        assert_eq!(c.network(), [10, 8, 0, 0]);
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains([1, 2, 3, 4]));
        assert_eq!(Cidr::parse("1.2.3.4").unwrap().prefix, 32);
        for bad in [
            "1.2.3",
            "1.2.3.4/33",
            "01.2.3.4",
            "1.2.3.256",
            "1.2.3.4.5",
            "::/0",
        ] {
            assert!(Cidr::parse(bad).is_none(), "{bad}");
        }
        let list = [
            Cidr::parse("0.0.0.0/1").unwrap(),
            Cidr::parse("10.0.0.0/8").unwrap(),
        ];
        assert_eq!(longest(&list, [10, 1, 1, 1]), Some(8));
        assert_eq!(longest(&list, [200, 1, 1, 1]), None);
        assert_eq!(alloc::format!("{c}"), "10.8.0.2/24");
    }

    fn checksum_ok(packet: &[u8]) -> bool {
        let h = header(packet).unwrap();
        let tcp = &packet[h.header..h.total];
        let mut sum = 0u32;
        for pair in [h.source, h.destination] {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]))
                + u32::from(u16::from_be_bytes([pair[2], pair[3]]));
        }
        sum += 6 + tcp.len() as u32;
        for chunk in tcp.chunks(2) {
            sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
        }
        fold(sum) == 0xFFFF
    }
    fn syn(options: &[u8]) -> Vec<u8> {
        let tcp_len = 20 + options.len();
        let total = 20 + tcp_len;
        let mut p = alloc::vec![0u8; total];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        p[8] = 64;
        p[9] = 6;
        p[12..16].copy_from_slice(&[10, 8, 0, 2]);
        p[16..20].copy_from_slice(&[93, 184, 216, 34]);
        let t = &mut p[20..];
        t[0..2].copy_from_slice(&49152u16.to_be_bytes());
        t[2..4].copy_from_slice(&443u16.to_be_bytes());
        t[12] = ((tcp_len / 4) as u8) << 4;
        t[13] = 0x02;
        t[14..16].copy_from_slice(&64240u16.to_be_bytes());
        t[20..].copy_from_slice(options);
        let h = header(&p).unwrap();
        let _ = fix_checksum_full(&mut p[20..], &h);
        p
    }

    #[test]
    fn mss_is_clamped_with_a_valid_checksum() {
        // MSS 1460, then SACK permitted, NOP padding.
        let mut p = syn(&[2, 4, 0x05, 0xB4, 4, 2, 1, 1]);
        assert!(checksum_ok(&p));
        assert!(clamp_mss(&mut p, 1380));
        assert_eq!(&p[42..44], &1380u16.to_be_bytes());
        assert!(checksum_ok(&p));
        // Already small enough: untouched.
        assert!(!clamp_mss(&mut p, 1400));
        // An MSS at an odd offset (after one NOP).
        let mut q = syn(&[1, 2, 4, 0x05, 0xB4, 1, 1, 1]);
        assert!(clamp_mss(&mut q, 1380));
        assert_eq!(&q[43..45], &1380u16.to_be_bytes());
        assert!(checksum_ok(&q));
        // Not a SYN.
        let mut r = syn(&[2, 4, 0x05, 0xB4]);
        r[33] = 0x10;
        assert!(!clamp_mss(&mut r, 1000));
    }

    #[test]
    fn ipv6_prefixes_and_mss() {
        let c = Cidr6::parse("fd00::2/64").unwrap();
        assert!(c.contains(Cidr6::parse("fd00::99").unwrap().address));
        assert!(!c.contains(Cidr6::parse("fd01::2").unwrap().address));
        assert!(Cidr6::parse("::/0").unwrap().contains([7; 16]));
        assert_eq!(Cidr6::parse("::1").unwrap().prefix, 128);
        assert!(Cidr6::parse("fd00::/129").is_none() && Cidr6::parse("1.2.3.4").is_none());
        assert_eq!(alloc::format!("{c}"), "fd00::2/64");
        assert_eq!(longest6(&[c, Cidr6::new([0; 16], 0)], c.address), Some(64));
        // A SYN with MSS 1440 over IPv6: clamped, checksum valid.
        let options = [2u8, 4, 0x05, 0xA0];
        let mut p = alloc::vec![0u8; 40 + 24];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&24u16.to_be_bytes());
        p[6] = 6;
        p[8..24].copy_from_slice(&c.address);
        p[24..40].copy_from_slice(&Cidr6::parse("2001:db8::1").unwrap().address);
        p[40 + 12] = 6 << 4;
        p[40 + 13] = 0x02;
        p[60..64].copy_from_slice(&options);
        assert!(clamp_mss6(&mut p, 1360));
        assert_eq!(&p[62..64], &1360u16.to_be_bytes());
        let mut sum = 0u32;
        for pair in p[8..40].chunks(2) {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        }
        sum += 24 + 6;
        for pair in p[40..].chunks(2) {
            sum += u32::from(u16::from_be_bytes([pair[0], pair[1]]));
        }
        assert_eq!(fold(sum), 0xFFFF);
        assert!(!clamp_mss6(&mut p, 1400));
    }

    #[test]
    fn malformed_headers_are_refused() {
        assert!(header(&[0x45; 10]).is_none());
        let mut p = syn(&[]);
        p[0] = 0x65;
        assert!(header(&p).is_none());
        let mut p = syn(&[]);
        p[3] = 200;
        assert!(header(&p).is_none());
    }
}
