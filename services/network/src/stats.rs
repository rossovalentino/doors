//! Protocol statistics and the neighbour table, read from the frames the
//! adapter carries (the NIC device sees every frame in and out, so these
//! count what the stack received and sent, as Windows' MIB counters do):
//! IPv4 and IPv6 datagrams, ICMP messages by type, TCP segments, opens,
//! failed attempts, resets and retransmissions, UDP datagrams and ports
//! nobody listened on, and the neighbours ARP and IPv6 neighbour discovery
//! show. smoltcp's own neighbour cache is private; this table follows the
//! same traffic.
use alloc::vec::Vec;
use door_service_protocol::net::stats::{Counters, Family, NEIGHBOR_DYNAMIC, Neighbor, icmp_slot};

/// Neighbours kept (Windows keeps far more; a desktop's LAN view needs few).
const MAX_NEIGHBORS: usize = 64;
/// A neighbour unseen for this long is forgotten (ms).
const NEIGHBOR_TTL_MS: u64 = 10 * 60 * 1000;
/// TCP connections followed for the opens/fails/resets/retransmit counters.
const MAX_FLOWS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// We sent a SYN.
    Opening,
    /// A SYN was answered (either direction).
    Established,
}
#[derive(Clone, Copy)]
struct Flow {
    v6: bool,
    local_port: u16,
    remote: [u8; 16],
    remote_port: u16,
    phase: Phase,
    /// The sequence number just past what we have sent.
    next: u32,
    seen: u64,
}

#[derive(Default)]
pub struct Stats {
    pub counters: Counters,
    /// (address, MAC, router, last seen ms).
    neighbors: Vec<([u8; 16], [u8; 6], bool, u64)>,
    flows: Vec<Flow>,
}
fn mapped(a: [u8; 4]) -> [u8; 16] {
    door_service_protocol::net::v6::mapped(a)
}
impl Stats {
    /// One Ethernet frame the adapter received (`outgoing` false) or sent.
    pub fn observe(&mut self, frame: &[u8], outgoing: bool, now_ms: u64) {
        if frame.len() < 14 {
            return;
        }
        let kind = u16::from_be_bytes([frame[12], frame[13]]);
        let payload = &frame[14..];
        match kind {
            0x0806 => self.arp(payload, now_ms),
            0x0800 => self.ipv4(payload, outgoing, now_ms),
            0x86dd => self.ipv6(payload, outgoing, now_ms),
            _ => (),
        }
    }
    fn family(&mut self, v6: bool) -> &mut Family {
        if v6 {
            &mut self.counters.v6
        } else {
            &mut self.counters.v4
        }
    }
    fn arp(&mut self, p: &[u8], now_ms: u64) {
        // Ethernet/IPv4 ARP: sender MAC 8..14, sender IP 14..18.
        if p.len() < 28 || p[0..2] != [0, 1] || p[2..4] != [8, 0] || p[4] != 6 || p[5] != 4 {
            return;
        }
        let mac: [u8; 6] = p[8..14].try_into().unwrap();
        let ip: [u8; 4] = p[14..18].try_into().unwrap();
        if ip != [0; 4] {
            self.learn(mapped(ip), mac, None, now_ms);
        }
    }
    fn learn(&mut self, address: [u8; 16], mac: [u8; 6], router: Option<bool>, now_ms: u64) {
        if mac == [0; 6] || mac[0] & 1 != 0 {
            return;
        }
        self.neighbors
            .retain(|n| now_ms.saturating_sub(n.3) < NEIGHBOR_TTL_MS);
        if let Some(n) = self.neighbors.iter_mut().find(|n| n.0 == address) {
            n.1 = mac;
            n.2 = router.unwrap_or(n.2);
            n.3 = now_ms;
            return;
        }
        if self.neighbors.len() == MAX_NEIGHBORS
            && let Some(oldest) = (0..self.neighbors.len()).min_by_key(|&i| self.neighbors[i].3)
        {
            self.neighbors.remove(oldest);
        }
        self.neighbors
            .push((address, mac, router.unwrap_or(false), now_ms));
    }
    fn ipv4(&mut self, p: &[u8], outgoing: bool, now_ms: u64) {
        if p.len() < 20 || p[0] >> 4 != 4 {
            if !outgoing {
                self.counters.v4.ip_in_header_errors += 1;
            }
            return;
        }
        let header = usize::from(p[0] & 0x0f) * 4;
        let total = usize::from(u16::from_be_bytes([p[2], p[3]]));
        if header < 20 || total < header || total > p.len() {
            if !outgoing {
                self.counters.v4.ip_in_header_errors += 1;
            }
            return;
        }
        let c = &mut self.counters.v4;
        if outgoing {
            c.ip_out_requests += 1;
        } else {
            c.ip_in_receives += 1;
        }
        // Only a datagram's first fragment carries the transport header.
        let fragment_offset = u16::from_be_bytes([p[6], p[7]]) & 0x1fff;
        let more_fragments = p[6] & 0x20 != 0;
        if fragment_offset != 0 || more_fragments {
            if outgoing {
                c.ip_frag_creates += 1;
            } else {
                c.ip_reasm_reqds += 1;
            }
            if fragment_offset != 0 {
                return;
            }
        }
        let source: [u8; 4] = p[12..16].try_into().unwrap();
        let destination: [u8; 4] = p[16..20].try_into().unwrap();
        let remote = mapped(if outgoing { destination } else { source });
        self.transport(false, p[9], &p[header..total], outgoing, remote, now_ms);
    }
    fn ipv6(&mut self, p: &[u8], outgoing: bool, now_ms: u64) {
        if p.len() < 40 || p[0] >> 4 != 6 {
            if !outgoing {
                self.counters.v6.ip_in_header_errors += 1;
            }
            return;
        }
        let length = usize::from(u16::from_be_bytes([p[4], p[5]]));
        if 40 + length > p.len() {
            if !outgoing {
                self.counters.v6.ip_in_header_errors += 1;
            }
            return;
        }
        let c = &mut self.counters.v6;
        if outgoing {
            c.ip_out_requests += 1;
        } else {
            c.ip_in_receives += 1;
        }
        let source: [u8; 16] = p[8..24].try_into().unwrap();
        let destination: [u8; 16] = p[24..40].try_into().unwrap();
        // Skip the extension headers (hop-by-hop, routing, fragment,
        // destination options).
        let mut next = p[6];
        let mut at = 40;
        let end = 40 + length;
        loop {
            match next {
                0 | 43 | 60 => {
                    let Some(h) = p.get(at..at + 2) else { return };
                    next = h[0];
                    at += 8 * (usize::from(h[1]) + 1);
                }
                44 => {
                    let Some(h) = p.get(at..at + 8) else { return };
                    let c = &mut self.counters.v6;
                    if outgoing {
                        c.ip_frag_creates += 1;
                    } else {
                        c.ip_reasm_reqds += 1;
                    }
                    // Not the first fragment: no transport header.
                    if u16::from_be_bytes([h[2], h[3]]) >> 3 != 0 {
                        return;
                    }
                    next = h[0];
                    at += 8;
                }
                _ => break,
            }
            if at > end {
                return;
            }
        }
        let remote = if outgoing { destination } else { source };
        let body = &p[at..end];
        if next == 58 && !outgoing {
            self.discovery(body, source, now_ms);
        }
        self.transport(true, next, body, outgoing, remote, now_ms);
    }
    /// Neighbour discovery: a solicitation's source link-layer option, an
    /// advertisement's target link-layer option.
    fn discovery(&mut self, m: &[u8], source: [u8; 16], now_ms: u64) {
        let (address, options, wanted, router) = match m.first() {
            // Router advertisement: the source is a router.
            Some(134) if m.len() >= 16 => (source, &m[16..], 1, Some(true)),
            Some(135) if m.len() >= 24 => (source, &m[24..], 1, None),
            Some(136) if m.len() >= 24 => {
                let target: [u8; 16] = m[8..24].try_into().unwrap();
                (target, &m[24..], 2, Some(m[4] & 0x80 != 0))
            }
            _ => return,
        };
        if address == [0; 16] {
            return;
        }
        let mut o = options;
        while o.len() >= 8 {
            let len = usize::from(o[1]) * 8;
            if len == 0 || len > o.len() {
                return;
            }
            if o[0] == wanted && len >= 8 {
                let mac: [u8; 6] = o[2..8].try_into().unwrap();
                self.learn(address, mac, router, now_ms);
                return;
            }
            o = &o[len..];
        }
    }
    fn transport(
        &mut self,
        v6: bool,
        protocol: u8,
        body: &[u8],
        outgoing: bool,
        remote: [u8; 16],
        now_ms: u64,
    ) {
        let delivered = matches!((v6, protocol), (false, 1) | (true, 58) | (_, 6) | (_, 17));
        if !outgoing {
            let c = self.family(v6);
            if delivered {
                c.ip_in_delivers += 1;
            } else {
                c.ip_in_unknown_protos += 1;
            }
        }
        match (v6, protocol) {
            (false, 1) | (true, 58) => self.icmp(v6, body, outgoing),
            (_, 6) => self.tcp(v6, body, outgoing, remote, now_ms),
            (_, 17) => {
                let c = self.family(v6);
                if outgoing {
                    c.udp_out_datagrams += 1;
                } else if body.len() < 8 {
                    c.udp_in_errors += 1;
                } else {
                    c.udp_in_datagrams += 1;
                }
            }
            _ => (),
        }
    }
    fn icmp(&mut self, v6: bool, m: &[u8], outgoing: bool) {
        let (Some(&kind), code) = (m.first(), m.get(1).copied().unwrap_or(0)) else {
            return;
        };
        let error = if v6 {
            kind < 128
        } else {
            matches!(kind, 3 | 4 | 5 | 11 | 12)
        };
        let c = self.family(v6);
        if outgoing {
            c.icmp_out_msgs += 1;
            c.icmp_out_errors += u32::from(error);
        } else {
            c.icmp_in_msgs += 1;
            c.icmp_in_errors += u32::from(error);
        }
        if let Some(slot) = icmp_slot(v6, kind) {
            if outgoing {
                c.icmp_out[slot] += 1;
            } else {
                c.icmp_in[slot] += 1;
            }
        }
        // The stack answering a datagram for a port nobody holds: that
        // datagram was not received by anyone (MIB udpNoPorts).
        let port_unreachable = if v6 {
            kind == 1 && code == 4
        } else {
            kind == 3 && code == 3
        };
        if outgoing && port_unreachable {
            c.udp_no_ports += 1;
            c.udp_in_datagrams = c.udp_in_datagrams.saturating_sub(1);
        }
    }
    fn tcp(&mut self, v6: bool, s: &[u8], outgoing: bool, remote: [u8; 16], now_ms: u64) {
        let header = s.get(12).map_or(0, |o| usize::from(o >> 4) * 4);
        if s.len() < 20 || header < 20 || header > s.len() {
            if !outgoing {
                self.family(v6).tcp_in_errs += 1;
            }
            return;
        }
        let source_port = u16::from_be_bytes([s[0], s[1]]);
        let destination_port = u16::from_be_bytes([s[2], s[3]]);
        let sequence = u32::from_be_bytes(s[4..8].try_into().unwrap());
        let flags = s[13];
        let (fin, syn, rst, ack) = (
            flags & 0x01 != 0,
            flags & 0x02 != 0,
            flags & 0x04 != 0,
            flags & 0x10 != 0,
        );
        let (local_port, remote_port) = if outgoing {
            (source_port, destination_port)
        } else {
            (destination_port, source_port)
        };
        let index = self.flows.iter().position(|f| {
            f.v6 == v6
                && f.local_port == local_port
                && f.remote == remote
                && f.remote_port == remote_port
        });
        // Data, SYN and FIN take sequence space.
        let length = (s.len() - header) as u32 + u32::from(syn) + u32::from(fin);
        let mut c = *self.family(v6);
        if outgoing {
            c.tcp_out_segs += 1;
            if rst {
                c.tcp_out_rsts += 1;
            }
            if syn && !ack {
                c.tcp_active_opens += 1;
            }
            if syn && ack {
                c.tcp_passive_opens += 1;
            }
        } else {
            c.tcp_in_segs += 1;
        }
        match index {
            Some(i) => {
                let flow = &mut self.flows[i];
                flow.seen = now_ms;
                if rst {
                    match flow.phase {
                        // Refused: the SYN we sent was answered with a reset.
                        Phase::Opening if !outgoing => c.tcp_attempt_fails += 1,
                        Phase::Established => c.tcp_estab_resets += 1,
                        Phase::Opening => (),
                    }
                    self.flows.remove(i);
                } else {
                    if syn && ack {
                        flow.phase = Phase::Established;
                    }
                    if outgoing && length > 0 {
                        // Sending again below the highest byte already
                        // sent: a retransmission.
                        if (sequence.wrapping_sub(flow.next) as i32) < 0 {
                            c.tcp_retrans_segs += 1;
                        } else {
                            flow.next = sequence.wrapping_add(length);
                        }
                    }
                    if fin && !outgoing && flow.phase == Phase::Opening {
                        self.flows.remove(i);
                    }
                }
            }
            None if syn && !rst => {
                if self.flows.len() == MAX_FLOWS
                    && let Some(oldest) = (0..self.flows.len()).min_by_key(|&i| self.flows[i].seen)
                {
                    self.flows.remove(oldest);
                }
                self.flows.push(Flow {
                    v6,
                    local_port,
                    remote,
                    remote_port,
                    phase: if ack {
                        Phase::Established
                    } else {
                        Phase::Opening
                    },
                    next: if outgoing {
                        sequence.wrapping_add(length)
                    } else {
                        0
                    },
                    seen: now_ms,
                });
            }
            None => (),
        }
        *self.family(v6) = c;
    }
    /// The neighbour table (in no promised order).
    pub fn neighbors(&self, now_ms: u64) -> Vec<Neighbor> {
        self.neighbors
            .iter()
            .filter(|n| now_ms.saturating_sub(n.3) < NEIGHBOR_TTL_MS)
            .map(|n| Neighbor {
                address: n.0,
                mac: n.1,
                kind: NEIGHBOR_DYNAMIC,
                router: n.2,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn ethernet(kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0xff; 6];
        f.extend_from_slice(&[0x52, 0x55, 10, 0, 2, 2]);
        f.extend_from_slice(&kind.to_be_bytes());
        f.extend_from_slice(payload);
        f
    }
    fn ipv4(protocol: u8, body: &[u8], outgoing: bool) -> Vec<u8> {
        let (a, b) = ([10, 0, 2, 15], [10, 0, 2, 2]);
        let (source, destination) = if outgoing { (a, b) } else { (b, a) };
        let mut p = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, protocol, 0, 0];
        p.extend_from_slice(&source);
        p.extend_from_slice(&destination);
        let total = (20 + body.len()) as u16;
        p[2..4].copy_from_slice(&total.to_be_bytes());
        p.extend_from_slice(body);
        ethernet(0x0800, &p)
    }
    fn ipv6(next: u8, body: &[u8], source: [u8; 16]) -> Vec<u8> {
        let mut p = vec![0x60, 0, 0, 0];
        p.extend_from_slice(&(body.len() as u16).to_be_bytes());
        p.extend_from_slice(&[next, 255]);
        p.extend_from_slice(&source);
        p.extend_from_slice(&[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        p.extend_from_slice(body);
        ethernet(0x86dd, &p)
    }
    fn segment(
        local: u16,
        remote: u16,
        sequence: u32,
        flags: u8,
        data: usize,
        outgoing: bool,
    ) -> Vec<u8> {
        let (source, destination) = if outgoing {
            (local, remote)
        } else {
            (remote, local)
        };
        let mut t = vec![0u8; 20 + data];
        t[0..2].copy_from_slice(&source.to_be_bytes());
        t[2..4].copy_from_slice(&destination.to_be_bytes());
        t[4..8].copy_from_slice(&sequence.to_be_bytes());
        t[12] = 0x50;
        t[13] = flags;
        ipv4(6, &t, outgoing)
    }
    #[test]
    fn counts_protocols_and_learns_neighbors() {
        let mut s = Stats::default();
        let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 2];
        arp.extend_from_slice(&[0x52, 0x55, 10, 0, 2, 2, 10, 0, 2, 2]);
        arp.extend_from_slice(&[0x52, 0x54, 0, 0x12, 0x34, 0x56, 10, 0, 2, 15]);
        s.observe(&ethernet(0x0806, &arp), false, 1000);
        // Echo request out, echo reply in.
        s.observe(&ipv4(1, &[8, 0, 0, 0, 0, 1, 0, 1], true), true, 1001);
        s.observe(&ipv4(1, &[0, 0, 0, 0, 0, 1, 0, 1], false), false, 1002);
        // A refused connection: SYN out, RST in.
        s.observe(&segment(50000, 81, 100, 0x02, 0, true), true, 1003);
        s.observe(&segment(50000, 81, 0, 0x14, 0, false), false, 1004);
        // One that opens, sends, retransmits, then is reset by the peer.
        s.observe(&segment(50001, 80, 500, 0x02, 0, true), true, 1005);
        s.observe(&segment(50001, 80, 9, 0x12, 0, false), false, 1006);
        s.observe(&segment(50001, 80, 501, 0x18, 10, true), true, 1007);
        s.observe(&segment(50001, 80, 501, 0x18, 10, true), true, 1008);
        s.observe(&segment(50001, 80, 10, 0x14, 0, false), false, 1009);
        // A datagram for a closed port and the stack's port unreachable.
        s.observe(&ipv4(17, &[0; 8], false), false, 1010);
        s.observe(&ipv4(1, &[3, 3, 0, 0, 0, 0, 0, 0], true), true, 1011);
        s.observe(&ipv4(99, &[0; 8], false), false, 1012);
        let c = s.counters.v4;
        assert_eq!(
            (c.ip_in_receives, c.ip_in_delivers, c.ip_in_unknown_protos),
            (6, 5, 1)
        );
        assert_eq!(c.ip_out_requests, 6);
        assert_eq!(
            (c.icmp_out_msgs, c.icmp_out_errors, c.icmp_in_msgs),
            (2, 1, 1)
        );
        assert_eq!((c.icmp_out[8], c.icmp_in[0], c.icmp_out[3]), (1, 1, 1));
        assert_eq!(
            (c.tcp_active_opens, c.tcp_attempt_fails, c.tcp_estab_resets),
            (2, 1, 1)
        );
        assert_eq!(
            (c.tcp_retrans_segs, c.tcp_in_segs, c.tcp_out_segs),
            (1, 3, 4)
        );
        assert_eq!((c.udp_in_datagrams, c.udp_no_ports), (0, 1));
        let n = s.neighbors(2000);
        assert_eq!(n.len(), 1);
        assert_eq!(
            (n[0].ipv4(), n[0].mac),
            (Some([10, 0, 2, 2]), [0x52, 0x55, 10, 0, 2, 2])
        );
        // Forgotten after ten minutes without traffic.
        assert!(s.neighbors(1000 + NEIGHBOR_TTL_MS).is_empty());
        // A truncated IPv4 header is a header error, not a datagram.
        s.observe(&ethernet(0x0800, &[0x45, 0]), false, 3000);
        assert_eq!(s.counters.v4.ip_in_header_errors, 1);
    }
    #[test]
    fn ipv6_counts_and_neighbor_discovery() {
        let mut s = Stats::default();
        let router = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
        // Router advertisement with a source link-layer option.
        let mut ra = vec![134, 0, 0, 0, 64, 0, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0];
        ra.extend_from_slice(&[1, 1, 0x52, 0x56, 0, 0, 0, 2]);
        s.observe(&ipv6(58, &ra, router), false, 10);
        // UDP over a hop-by-hop header.
        let mut hop = vec![17, 0, 0, 0, 0, 0, 0, 0];
        hop.extend_from_slice(&[0; 8]);
        s.observe(&ipv6(0, &hop, router), false, 11);
        let c = s.counters.v6;
        assert_eq!(
            (c.ip_in_receives, c.ip_in_delivers, c.udp_in_datagrams),
            (2, 2, 1)
        );
        assert_eq!(
            (c.icmp_in_msgs, c.icmp_in_errors, c.icmp(true, false, 134)),
            (1, 0, 1)
        );
        assert_eq!(s.counters.v4, Family::default());
        let n = s.neighbors(20);
        assert_eq!(n.len(), 1);
        assert_eq!(
            (n[0].address, n[0].mac, n[0].router),
            (router, [0x52, 0x56, 0, 0, 0, 2], true)
        );
    }
}
