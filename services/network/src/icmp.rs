//! ICMP echo for IcmpSendEcho, ping and tracert: one raw IPv4/ICMP socket
//! sends each request with the caller's TTL, size and Don't Fragment flag,
//! and matches what comes back by identifier and sequence number: an Echo
//! Reply, or a Time Exceeded / Destination Unreachable quoting the request.
//! A client starts a request (`net::op::PING`) and collects its outcome
//! (`PING_RESULT`) when it is ready; unanswered requests time out.
use alloc::{vec, vec::Vec};
use door_abi::Handle;
use door_service_protocol::{
    Code,
    net::{self, ip_status},
};
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    socket::raw,
    time::Instant,
    wire::{Icmpv4Message, Icmpv4Packet, IpProtocol, IpVersion, Ipv4Address, Ipv4Packet},
};

/// Requests a client may have outstanding, and in all.
pub const PER_CLIENT: usize = 8;
const TOTAL: usize = 64;
/// The identifier on every request (Windows uses the process; one stack
/// socket here, so one identifier and distinct sequence numbers).
const IDENT: u16 = 0x0d00;

pub struct Pending {
    pub channel: Handle,
    pub token: u32,
    seq: u16,
    destination: Ipv4Address,
    sent: Instant,
    deadline: Instant,
    pub outcome: Option<net::Echo>,
}
#[derive(Default)]
pub struct Pings {
    socket: Option<SocketHandle>,
    pub pending: Vec<Pending>,
    next_seq: u16,
    next_token: u32,
}
pub fn raw_socket() -> raw::Socket<'static> {
    raw::Socket::new(
        IpVersion::Ipv4,
        IpProtocol::Icmp,
        raw::PacketBuffer::new(vec![raw::PacketMetadata::EMPTY; 16], vec![0; 16 * 1024]),
        raw::PacketBuffer::new(vec![raw::PacketMetadata::EMPTY; 8], vec![0; 16 * 1024]),
    )
}
/// The Internet checksum (RFC 1071) of `data`, complemented.
fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    for pair in data.chunks(2) {
        let word = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
/// The IPv4 packet of an echo request.
pub fn request_packet(
    source: Ipv4Address,
    destination: Ipv4Address,
    request: &net::EchoRequest,
    seq: u16,
) -> Vec<u8> {
    let size = usize::from(request.size);
    let icmp_len = 8 + size;
    let total = 20 + icmp_len;
    let mut b = vec![0u8; total];
    b[0] = 0x45;
    b[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    b[4..6].copy_from_slice(&seq.to_be_bytes());
    if request.dont_fragment {
        b[6] = 0x40;
    }
    b[8] = request.ttl.max(1);
    b[9] = 1;
    b[12..16].copy_from_slice(&source.octets());
    b[16..20].copy_from_slice(&destination.octets());
    let header = checksum(&b[..20]);
    b[10..12].copy_from_slice(&header.to_be_bytes());
    let icmp = &mut b[20..];
    icmp[0] = 8;
    icmp[4..6].copy_from_slice(&IDENT.to_be_bytes());
    icmp[6..8].copy_from_slice(&seq.to_be_bytes());
    for (i, byte) in icmp[8..].iter_mut().enumerate() {
        *byte = request.data_at(i);
    }
    let sum = checksum(icmp);
    icmp[2..4].copy_from_slice(&sum.to_be_bytes());
    b
}
/// What an incoming ICMP packet answers: (sequence, status, the
/// replying address, the reply's TTL, its data length).
pub fn answer(packet: &[u8]) -> Option<(u16, u32, Ipv4Address, u8, u16)> {
    let ip = Ipv4Packet::new_checked(packet).ok()?;
    let ttl = ip.hop_limit();
    let from = ip.src_addr();
    let icmp = Icmpv4Packet::new_checked(ip.payload()).ok()?;
    let quoted_seq = |data: &[u8]| -> Option<u16> {
        // The quoted request: its IPv4 header, then 8 bytes of ICMP.
        let inner = Ipv4Packet::new_unchecked(data);
        let header = usize::from(inner.header_len());
        let echo = data.get(header..header + 8)?;
        (inner.next_header() == IpProtocol::Icmp
            && echo[0] == 8
            && u16::from_be_bytes([echo[4], echo[5]]) == IDENT)
            .then(|| u16::from_be_bytes([echo[6], echo[7]]))
    };
    match icmp.msg_type() {
        Icmpv4Message::EchoReply if icmp.echo_ident() == IDENT => Some((
            icmp.echo_seq_no(),
            ip_status::SUCCESS,
            from,
            ttl,
            icmp.data().len() as u16,
        )),
        Icmpv4Message::TimeExceeded => {
            let seq = quoted_seq(icmp.data())?;
            Some((seq, ip_status::TTL_EXPIRED_TRANSIT, from, ttl, 0))
        }
        Icmpv4Message::DstUnreachable => {
            let seq = quoted_seq(icmp.data())?;
            let status = match icmp.msg_code() {
                0 => ip_status::DEST_NET_UNREACHABLE,
                2 => ip_status::DEST_PROT_UNREACHABLE,
                3 => ip_status::DEST_PORT_UNREACHABLE,
                4 => ip_status::PACKET_TOO_BIG,
                _ => ip_status::DEST_HOST_UNREACHABLE,
            };
            Some((seq, status, from, ttl, 0))
        }
        _ => None,
    }
}
impl Pings {
    /// Start a request for `channel`; its token.
    pub fn start(
        &mut self,
        sockets: &mut SocketSet<'static>,
        channel: Handle,
        source: Option<Ipv4Address>,
        request: &net::EchoRequest,
        now: Instant,
    ) -> Result<u32, Code> {
        if self.pending.iter().filter(|p| p.channel == channel).count() >= PER_CLIENT
            || self.pending.len() >= TOTAL
        {
            return Err(Code::Resources);
        }
        let destination = Ipv4Address::from(request.address);
        let handle = *self.socket.get_or_insert_with(|| sockets.add(raw_socket()));
        self.next_seq = self.next_seq.wrapping_add(1);
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let seq = self.next_seq;
        let token = self.next_token;
        let deadline =
            now + smoltcp::time::Duration::from_millis(u64::from(request.timeout.max(1)));
        let outcome = match source {
            // No address yet: Windows reports a general failure.
            None => Some(net::Echo {
                status: ip_status::GENERAL_FAILURE,
                ..net::Echo::default()
            }),
            Some(source) => {
                let packet = request_packet(source, destination, request, seq);
                let socket = sockets.get_mut::<raw::Socket>(handle);
                match socket.send_slice(&packet) {
                    Ok(()) => None,
                    Err(_) => Some(net::Echo {
                        status: ip_status::NO_RESOURCES,
                        ..net::Echo::default()
                    }),
                }
            }
        };
        self.pending.push(Pending {
            channel,
            token,
            seq,
            destination,
            sent: now,
            deadline,
            outcome,
        });
        Ok(token)
    }
    /// Match what arrived and time out the rest. True when anything ended.
    pub fn poll(&mut self, sockets: &mut SocketSet<'static>, now: Instant) -> bool {
        let mut changed = false;
        if let Some(handle) = self.socket {
            let socket = sockets.get_mut::<raw::Socket>(handle);
            while let Ok(packet) = socket.recv() {
                let Some((seq, status, from, ttl, size)) = answer(packet) else {
                    continue;
                };
                if let Some(p) = self
                    .pending
                    .iter_mut()
                    .find(|p| p.seq == seq && p.outcome.is_none())
                {
                    // An echo reply must come from the address asked.
                    if status == ip_status::SUCCESS && from != p.destination {
                        continue;
                    }
                    p.outcome = Some(net::Echo {
                        status,
                        rtt: (now - p.sent).total_millis() as u32,
                        ttl,
                        from: from.octets(),
                        size: if status == ip_status::SUCCESS {
                            size
                        } else {
                            0
                        },
                    });
                    changed = true;
                }
            }
        }
        for p in self.pending.iter_mut().filter(|p| p.outcome.is_none()) {
            if now >= p.deadline {
                p.outcome = Some(net::Echo {
                    status: ip_status::REQ_TIMED_OUT,
                    ..net::Echo::default()
                });
                changed = true;
            }
        }
        changed
    }
    /// The outcome of `token` for `channel`, once (WouldBlock while open).
    pub fn take(&mut self, channel: Handle, token: u32) -> Result<net::Echo, Code> {
        let at = self
            .pending
            .iter()
            .position(|p| p.channel == channel && p.token == token)
            .ok_or(Code::NotFound)?;
        if self.pending[at].outcome.is_none() {
            return Err(Code::WouldBlock);
        }
        Ok(self.pending.remove(at).outcome.unwrap_or_default())
    }
    /// A client went away: its requests go.
    pub fn forget(&mut self, channel: Handle) {
        self.pending.retain(|p| p.channel != channel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_are_well_formed_and_answers_match_them() {
        let request = net::EchoRequest {
            address: [10, 0, 2, 2],
            ttl: 7,
            dont_fragment: true,
            size: 32,
            timeout: 1000,
            pattern: [0; net::PATTERN_BYTES],
            pattern_len: 0,
        };
        let src = Ipv4Address::new(10, 0, 2, 15);
        let dst = Ipv4Address::new(10, 0, 2, 2);
        let p = request_packet(src, dst, &request, 42);
        let ip = Ipv4Packet::new_checked(&p[..]).unwrap();
        assert!(ip.verify_checksum());
        assert_eq!((ip.hop_limit(), ip.dont_frag()), (7, true));
        let icmp = Icmpv4Packet::new_checked(ip.payload()).unwrap();
        assert!(icmp.verify_checksum());
        assert_eq!(icmp.data(), &b"abcdefghijklmnopqrstuvwabcdefghi"[..]);
        // The echo reply the peer sends back.
        let mut reply = p.clone();
        reply[12..16].copy_from_slice(&dst.octets());
        reply[16..20].copy_from_slice(&src.octets());
        reply[8] = 255;
        reply[20] = 0;
        reply[22..24].copy_from_slice(&[0, 0]);
        let sum = checksum(&reply[20..]);
        reply[22..24].copy_from_slice(&sum.to_be_bytes());
        reply[10..12].copy_from_slice(&[0, 0]);
        let h = checksum(&reply[..20]);
        reply[10..12].copy_from_slice(&h.to_be_bytes());
        assert_eq!(answer(&reply), Some((42, ip_status::SUCCESS, dst, 255, 32)));
        // A router's Time Exceeded quoting the request.
        let router = Ipv4Address::new(10, 0, 2, 1);
        let mut te = vec![0u8; 20 + 8 + 28];
        te[0] = 0x45;
        let length = te.len() as u16;
        te[2..4].copy_from_slice(&length.to_be_bytes());
        te[8] = 64;
        te[9] = 1;
        te[12..16].copy_from_slice(&router.octets());
        te[16..20].copy_from_slice(&src.octets());
        te[20] = 11;
        te[28..56].copy_from_slice(&p[..28]);
        let sum = checksum(&te[20..]);
        te[22..24].copy_from_slice(&sum.to_be_bytes());
        let h = checksum(&te[..20]);
        te[10..12].copy_from_slice(&h.to_be_bytes());
        assert_eq!(
            answer(&te),
            Some((42, ip_status::TTL_EXPIRED_TRANSIT, router, 64, 0))
        );
    }
}
