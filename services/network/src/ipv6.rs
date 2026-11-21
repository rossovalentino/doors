//! IPv6 for door-network: a link-local address from the MAC (modified
//! EUI-64, RFC 4291 appendix A), router discovery (RFC 4861: Router
//! Solicitations at start, Router Advertisements' router, prefixes and
//! RDNSS servers, RFC 8106) and stateless autoconfiguration of a global
//! address on each advertised /64 prefix with the autonomous flag (RFC
//! 4862). smoltcp answers neighbor solicitations and resolves neighbors but
//! does not act on Router Advertisements, so a raw ICMPv6 socket sees them
//! here (smoltcp still processes every packet it hands to a raw socket).
use alloc::vec;
use smoltcp::{
    iface::{Interface, SocketHandle, SocketSet},
    socket::raw,
    time::{Duration, Instant},
    wire::{IpAddress, IpCidr, IpProtocol, IpVersion, Ipv6Address, Ipv6Cidr},
};

const ICMPV6: u8 = 58;
const ROUTER_SOLICIT: u8 = 133;
const ROUTER_ADVERT: u8 = 134;
/// RFC 4861 §10: MAX_RTR_SOLICITATIONS and RTR_SOLICITATION_INTERVAL.
const SOLICITATIONS: u8 = 3;
const SOLICIT_INTERVAL: Duration = Duration::from_secs(4);
/// RFC 4861 §6.3.7: wait up to a second before the first solicitation.
const FIRST_SOLICIT: Duration = Duration::from_millis(500);

/// The interface index Windows reports for the Ethernet adapter (the
/// `%2` of `fe80::...%2`, GetAdaptersAddresses' Ipv6IfIndex).
pub const SCOPE: u32 = 2; // iphlpapi ETHERNET_INDEX

pub fn link_local(mac: [u8; 6]) -> Ipv6Address {
    let mut a = [0u8; 16];
    a[0] = 0xfe;
    a[1] = 0x80;
    a[8..].copy_from_slice(&interface_id(mac));
    Ipv6Address::from(a)
}
/// Modified EUI-64: the universal/local bit flipped, ff:fe in the middle.
pub fn interface_id(mac: [u8; 6]) -> [u8; 8] {
    [
        mac[0] ^ 0x02,
        mac[1],
        mac[2],
        0xff,
        0xfe,
        mac[3],
        mac[4],
        mac[5],
    ]
}

/// The one's-complement checksum of an ICMPv6 message with its pseudo
/// header (RFC 8200 §8.1).
pub fn checksum(source: &[u8; 16], destination: &[u8; 16], message: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut add = |b: &[u8]| {
        for pair in b.chunks(2) {
            let word = (u32::from(pair[0]) << 8) | u32::from(*pair.get(1).unwrap_or(&0));
            sum = sum.wrapping_add(word);
        }
    };
    add(source);
    add(destination);
    add(&(message.len() as u32).to_be_bytes());
    add(&[0, 0, 0, ICMPV6]);
    add(message);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// A Router Solicitation from `source` to all-routers, with our link-layer
/// address: the whole IPv6 packet.
pub fn solicitation(source: Ipv6Address, mac: [u8; 6]) -> [u8; 56] {
    let all_routers = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let mut p = [0u8; 56];
    p[0] = 0x60;
    p[4..6].copy_from_slice(&16u16.to_be_bytes());
    p[6] = ICMPV6;
    p[7] = 255;
    p[8..24].copy_from_slice(&source.octets());
    p[24..40].copy_from_slice(&all_routers);
    let m = &mut p[40..];
    m[0] = ROUTER_SOLICIT;
    // Source link-layer address option (type 1, 8 bytes).
    m[8] = 1;
    m[9] = 1;
    m[10..16].copy_from_slice(&mac);
    let sum = checksum(&source.octets(), &all_routers, m);
    m[2..4].copy_from_slice(&sum.to_be_bytes());
    p
}

/// What one Router Advertisement says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advert {
    pub router: [u8; 16],
    /// Seconds this router is a default router (0: it is not).
    pub lifetime: u16,
    /// An autonomous /64 prefix and its valid lifetime in seconds.
    pub prefix: Option<([u8; 8], u32)>,
    /// RDNSS servers and their lifetime in seconds.
    pub dns: [Option<[u8; 16]>; 2],
    pub dns_lifetime: u32,
}
/// Parse a whole IPv6 packet as a Router Advertisement (RFC 4861 §6.1.2
/// validity checks: hop limit 255, link-local source, checksum, code 0).
pub fn advert(packet: &[u8]) -> Option<Advert> {
    if packet.len() < 40 + 16 || packet[0] >> 4 != 6 || packet[6] != ICMPV6 || packet[7] != 255 {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([packet[4], packet[5]]));
    let m = packet.get(40..40 + length)?;
    let source: [u8; 16] = packet[8..24].try_into().ok()?;
    let destination: [u8; 16] = packet[24..40].try_into().ok()?;
    if source[0] != 0xfe || source[1] & 0xc0 != 0x80 {
        return None;
    }
    if m[0] != ROUTER_ADVERT || m[1] != 0 || checksum(&source, &destination, m) != 0 {
        return None;
    }
    let mut a = Advert {
        router: source,
        lifetime: u16::from_be_bytes([m[6], m[7]]),
        ..Advert::default()
    };
    let mut at = 16;
    while at + 2 <= m.len() {
        let kind = m[at];
        let len = usize::from(m[at + 1]) * 8;
        if len == 0 {
            return None;
        }
        let option = m.get(at..at + len)?;
        match kind {
            // Prefix information: on-link and autonomous flags, lifetimes.
            3 if len == 32 => {
                let prefix_len = option[2];
                let autonomous = option[3] & 0x40 != 0;
                let valid = u32::from_be_bytes(option[4..8].try_into().ok()?);
                let preferred = u32::from_be_bytes(option[8..12].try_into().ok()?);
                // RFC 4862 §5.5.3: link-local prefixes and preferred >
                // valid are ignored; autoconfiguration needs a /64.
                let link_local = option[16] == 0xfe && option[17] & 0xc0 == 0x80;
                if autonomous && prefix_len == 64 && !link_local && preferred <= valid && valid > 0
                {
                    a.prefix = Some((option[16..24].try_into().ok()?, valid));
                }
            }
            // Recursive DNS servers (RFC 8106 §5.1).
            25 if len >= 24 => {
                a.dns_lifetime = u32::from_be_bytes(option[4..8].try_into().ok()?);
                for (slot, server) in a.dns.iter_mut().zip(option[8..].chunks_exact(16)) {
                    *slot = Some(server.try_into().ok()?);
                }
            }
            _ => (),
        }
        at += len;
    }
    Some(a)
}

fn deadline(now: Instant, seconds: u32) -> Instant {
    if seconds == u32::MAX {
        Instant::from_millis(i64::MAX / 2)
    } else {
        now + Duration::from_secs(u64::from(seconds))
    }
}

/// The adapter's IPv6 configuration and router discovery.
pub struct Slaac {
    mac: [u8; 6],
    raw: SocketHandle,
    pub link_local: Ipv6Address,
    /// The autoconfigured address and when it expires.
    pub global: Option<(Ipv6Cidr, Instant)>,
    pub router: Option<(Ipv6Address, Instant)>,
    pub dns: [Option<Ipv6Address>; 2],
    dns_until: Instant,
    solicits: u8,
    next_solicit: Instant,
}
impl Slaac {
    /// Router discovery on a new interface: its link-local address now, a
    /// solicitation shortly.
    pub fn new(
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        mac: [u8; 6],
        now: Instant,
    ) -> Self {
        let link_local = link_local(mac);
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::Ipv6(Ipv6Cidr::new(link_local, 64)));
        });
        let socket = raw::Socket::new(
            IpVersion::Ipv6,
            IpProtocol::Icmpv6,
            raw::PacketBuffer::new(vec![raw::PacketMetadata::EMPTY; 4], vec![0; 2048]),
            raw::PacketBuffer::new(vec![raw::PacketMetadata::EMPTY; 2], vec![0; 256]),
        );
        Self {
            mac,
            raw: sockets.add(socket),
            link_local,
            global: None,
            router: None,
            dns: [None; 2],
            dns_until: now,
            solicits: 0,
            next_solicit: now + FIRST_SOLICIT,
        }
    }
    /// The interface goes away (reconfiguration): the raw socket too.
    pub fn remove(self, sockets: &mut SocketSet<'static>) {
        sockets.remove(self.raw);
    }
    /// Solicit, take advertisements, expire what timed out. True when the
    /// configuration changed.
    pub fn poll(
        &mut self,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        now: Instant,
    ) -> bool {
        let socket = sockets.get_mut::<raw::Socket>(self.raw);
        if self.router.is_none() && self.solicits < SOLICITATIONS && now >= self.next_solicit {
            let packet = solicitation(self.link_local, self.mac);
            if socket.send_slice(&packet).is_ok() {
                self.solicits += 1;
                self.next_solicit = now + SOLICIT_INTERVAL;
            }
        }
        let mut changed = false;
        let mut buffer = [0u8; 1500];
        while let Ok(n) = socket.recv_slice(&mut buffer) {
            if let Some(a) = advert(&buffer[..n]) {
                changed |= self.apply(iface, &a, now);
            }
        }
        if self.global.is_some_and(|(_, until)| now >= until) {
            self.set_global(iface, None);
            changed = true;
        }
        if self.router.is_some_and(|(_, until)| now >= until) {
            self.router = None;
            iface.routes_mut().remove_default_ipv6_route();
            changed = true;
        }
        if self.dns.iter().any(Option::is_some) && now >= self.dns_until {
            self.dns = [None; 2];
            changed = true;
        }
        changed
    }
    fn set_global(&mut self, iface: &mut Interface, new: Option<(Ipv6Cidr, Instant)>) {
        let old = self.global.map(|(c, _)| c);
        if old != new.map(|(c, _)| c) {
            iface.update_ip_addrs(|addrs| {
                if let Some(old) = old {
                    addrs.retain(|c| *c != IpCidr::Ipv6(old));
                }
                if let Some((cidr, _)) = new {
                    let _ = addrs.push(IpCidr::Ipv6(cidr));
                }
            });
        }
        self.global = new;
    }
    fn apply(&mut self, iface: &mut Interface, a: &Advert, now: Instant) -> bool {
        let before = (
            self.global.map(|(c, _)| c),
            self.router.map(|(r, _)| r),
            self.dns,
        );
        let router = Ipv6Address::from(a.router);
        if a.lifetime == 0 {
            if self.router.is_some_and(|(r, _)| r == router) {
                self.router = None;
                iface.routes_mut().remove_default_ipv6_route();
            }
        } else {
            let until = now + Duration::from_secs(u64::from(a.lifetime));
            if iface.routes_mut().add_default_ipv6_route(router).is_ok() {
                self.router = Some((router, until));
            }
        }
        if let Some((prefix, valid)) = a.prefix {
            let mut address = [0u8; 16];
            address[..8].copy_from_slice(&prefix);
            address[8..].copy_from_slice(&interface_id(self.mac));
            let cidr = Ipv6Cidr::new(Ipv6Address::from(address), 64);
            self.set_global(iface, Some((cidr, deadline(now, valid))));
        }
        if a.dns.iter().any(Option::is_some) {
            if a.dns_lifetime == 0 {
                self.dns = [None; 2];
            } else {
                self.dns = a.dns.map(|d| d.map(Ipv6Address::from));
                self.dns_until = deadline(now, a.dns_lifetime);
            }
        }
        let after = (
            self.global.map(|(c, _)| c),
            self.router.map(|(r, _)| r),
            self.dns,
        );
        if before != after {
            door_rt::diagnostic(format_args!(
                "DOOR_P15 NET IPV6 address={} router={} dns={}\n",
                self.global
                    .map_or(IpAddress::Ipv6(Ipv6Address::UNSPECIFIED), |(c, _)| {
                        IpAddress::Ipv6(c.address())
                    }),
                self.router.map_or(Ipv6Address::UNSPECIFIED, |(r, _)| r),
                self.dns[0].unwrap_or(Ipv6Address::UNSPECIFIED)
            ));
            return true;
        }
        false
    }
    pub fn status(&self) -> door_service_protocol::net::v6::Status6 {
        door_service_protocol::net::v6::Status6 {
            link_local: self.link_local.octets(),
            global: self.global.map_or([0; 16], |(c, _)| c.address().octets()),
            prefix: self.global.map_or(0, |(c, _)| c.prefix_len()),
            router: self.router.map_or([0; 16], |(r, _)| r.octets()),
            dns: self.dns.map(|d| d.map_or([0; 16], |a| a.octets())),
            scope: SCOPE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_local_and_solicitation_follow_the_rfcs() {
        let mac = [0x52, 0x54, 0x00, 0xd0, 0x05, 0x01];
        // RFC 4291 appendix A: 52:54:00:... becomes 5054:00ff:fe..
        assert_eq!(
            link_local(mac).octets(),
            [
                0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x50, 0x54, 0x00, 0xff, 0xfe, 0xd0, 0x05, 0x01
            ]
        );
        let p = solicitation(link_local(mac), mac);
        let src: [u8; 16] = p[8..24].try_into().unwrap();
        let dst: [u8; 16] = p[24..40].try_into().unwrap();
        // A correct checksum sums to zero over the message.
        assert_eq!(checksum(&src, &dst, &p[40..]), 0);
        assert_eq!((p[40], p[7]), (133, 255));
    }
    #[test]
    fn router_advertisements_give_prefix_router_and_dns() {
        // QEMU's slirp: fec0::/64, RDNSS fec0::3, from fe80::2.
        let router = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        let all_nodes = [0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let mut m = vec![134u8, 0, 0, 0, 64, 0, 0x07, 0x08, 0, 0, 0, 0, 0, 0, 0, 0];
        // Prefix information.
        let mut prefix = vec![3u8, 4, 64, 0xc0];
        prefix.extend_from_slice(&86400u32.to_be_bytes());
        prefix.extend_from_slice(&14400u32.to_be_bytes());
        prefix.extend_from_slice(&[0; 4]);
        prefix.extend_from_slice(&[0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        m.extend_from_slice(&prefix);
        // RDNSS.
        let mut rdnss = vec![25u8, 3, 0, 0];
        rdnss.extend_from_slice(&1800u32.to_be_bytes());
        rdnss.extend_from_slice(&[0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]);
        m.extend_from_slice(&rdnss);
        let sum = checksum(&router, &all_nodes, &m);
        m[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut packet = vec![0x60u8, 0, 0, 0];
        packet.extend_from_slice(&(m.len() as u16).to_be_bytes());
        packet.extend_from_slice(&[58, 255]);
        packet.extend_from_slice(&router);
        packet.extend_from_slice(&all_nodes);
        packet.extend_from_slice(&m);
        let a = advert(&packet).unwrap();
        assert_eq!(a.lifetime, 0x0708);
        assert_eq!(a.prefix, Some(([0xfe, 0xc0, 0, 0, 0, 0, 0, 0], 86400)));
        assert_eq!(a.dns[0].unwrap()[15], 3);
        // A forwarded advertisement (hop limit below 255) is not believed.
        packet[7] = 254;
        assert_eq!(advert(&packet), None);
        packet[7] = 255;
        packet[40 + 4] ^= 1;
        assert_eq!(advert(&packet), None);
    }
}
