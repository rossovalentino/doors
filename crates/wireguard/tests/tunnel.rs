//! Two door-wireguard tunnels talking to each other: handshakes, data both
//! ways, rekeying, retransmission, give-up, keepalives, replay and routing.
use door_wireguard::{
    Cidr, Drop, Io, Now, Random, Tunnel, crypto,
    tunnel::{
        KEEPALIVE_TIMEOUT, REJECT_AFTER_TIME, REKEY_AFTER_TIME, REKEY_ATTEMPT_TIME, REKEY_TIMEOUT,
    },
};

pub struct Rng(u64);
impl Random for Rng {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            *b = self.0 as u8;
        }
    }
}
pub fn packet(src: [u8; 4], dst: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let total = 28 + payload.len();
    let mut p = vec![0u8; total];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&src);
    p[16..20].copy_from_slice(&dst);
    p[20..22].copy_from_slice(&1000u16.to_be_bytes());
    p[22..24].copy_from_slice(&53u16.to_be_bytes());
    p[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    p[28..].copy_from_slice(payload);
    p
}
fn at(millis: u64) -> Now {
    Now {
        millis,
        unix_nanos: 1_790_000_000_000_000_000 + millis * 1_000_000,
    }
}
const CLIENT: [u8; 4] = [10, 8, 0, 2];
const SERVER: [u8; 4] = [10, 8, 0, 1];

struct Pair {
    a: Tunnel,
    b: Tunnel,
    rng: Rng,
}
impl Pair {
    fn new(psk: Option<[u8; 32]>, keepalive: Option<u16>) -> Self {
        let (ka, kb) = (crypto::clamp([1; 32]), crypto::clamp([2; 32]));
        Self {
            a: Tunnel::new(
                ka,
                crypto::public_key(&kb),
                psk,
                vec![Cidr::new([0; 4], 0)],
                keepalive,
                1420,
            ),
            b: Tunnel::new(
                kb,
                crypto::public_key(&ka),
                psk,
                vec![Cidr::new(CLIENT, 32)],
                None,
                1420,
            ),
            rng: Rng(0x1234_5678_9abc_def1),
        }
    }
    /// Deliver datagrams back and forth until both are quiet; the packets
    /// each side's tunnel interface got.
    fn pump(&mut self, to_b: Vec<Vec<u8>>, now: Now) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        self.exchange(Vec::new(), to_b, now)
    }
    fn exchange(
        &mut self,
        mut to_a: Vec<Vec<u8>>,
        mut to_b: Vec<Vec<u8>>,
        now: Now,
    ) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let (mut got_a, mut got_b) = (Vec::new(), Vec::new());
        while !to_a.is_empty() || !to_b.is_empty() {
            for d in std::mem::take(&mut to_b) {
                let mut io = Io::default();
                let _ = self.b.receive(&d, now, &mut self.rng, &mut io);
                to_a.extend(io.network);
                got_b.extend(io.tunnel);
            }
            for d in std::mem::take(&mut to_a) {
                let mut io = Io::default();
                let _ = self.a.receive(&d, now, &mut self.rng, &mut io);
                to_b.extend(io.network);
                got_a.extend(io.tunnel);
            }
        }
        (got_a, got_b)
    }
    fn a_sends(&mut self, payload: &[u8], now: Now) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut io = Io::default();
        self.a.send(
            &packet(CLIENT, SERVER, payload),
            now,
            &mut self.rng,
            &mut io,
        );
        self.pump(io.network, now)
    }
}

#[test]
fn first_packet_waits_for_the_handshake_then_flows_both_ways() {
    for psk in [None, Some([7u8; 32])] {
        let mut p = Pair::new(psk, None);
        let t = at(1000);
        let (_, got_b) = p.a_sends(b"hello", t);
        assert_eq!(got_b, [packet(CLIENT, SERVER, b"hello")]);
        assert!(p.a.established(t.millis) && p.b.established(t.millis));
        assert_eq!((p.a.stats.handshakes, p.b.stats.handshakes), (1, 1));
        // The responder answers on the confirmed session.
        let mut io = Io::default();
        p.b.send(&packet(SERVER, CLIENT, b"world"), t, &mut p.rng, &mut io);
        assert_eq!(io.network.len(), 1);
        let (got_a, _) = p.exchange(io.network, Vec::new(), t);
        assert_eq!(got_a, [packet(SERVER, CLIENT, b"world")]);
        assert_eq!(p.a.stats.rx_bytes, 33);
    }
}

/// A UDP packet over IPv6.
fn packet6(src: [u8; 16], dst: [u8; 16], payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 48 + payload.len()];
    p[0] = 0x60;
    p[4..6].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    p[6] = 17;
    p[7] = 64;
    p[8..24].copy_from_slice(&src);
    p[24..40].copy_from_slice(&dst);
    p[48..].copy_from_slice(payload);
    p
}

#[test]
fn ipv6_packets_follow_the_ipv6_allowed_ips() {
    use door_wireguard::ip::Cidr6;
    let mut p = Pair::new(None, None);
    let t = at(500);
    p.a_sends(b"up", t);
    let client6 = Cidr6::parse("fd00::2").unwrap().address;
    let server6 = Cidr6::parse("fd00::1").unwrap().address;
    // B has no IPv6 AllowedIPs yet: dropped.
    let mut io = Io::default();
    p.a.send(&packet6(client6, server6, b"six"), t, &mut p.rng, &mut io);
    let mut out = Io::default();
    assert_eq!(
        p.b.receive(&io.network[0], t, &mut p.rng, &mut out),
        Err(Drop::NotAllowed)
    );
    p.b.allow6(vec![Cidr6::parse("fd00::2/128").unwrap()]);
    let mut io = Io::default();
    p.a.send(&packet6(client6, server6, b"six"), t, &mut p.rng, &mut io);
    let mut out = Io::default();
    assert_eq!(p.b.receive(&io.network[0], t, &mut p.rng, &mut out), Ok(()));
    // The padding is cut off: the packet as sent.
    assert_eq!(out.tunnel, [packet6(client6, server6, b"six")]);
}

#[test]
fn transport_replays_and_foreign_sources_are_dropped() {
    let mut p = Pair::new(None, None);
    let t = at(0);
    p.a_sends(b"x", t);
    let mut io = Io::default();
    p.a.send(&packet(CLIENT, SERVER, b"again"), t, &mut p.rng, &mut io);
    let datagram = io.network.pop().unwrap();
    let mut out = Io::default();
    assert_eq!(p.b.receive(&datagram, t, &mut p.rng, &mut out), Ok(()));
    assert_eq!(
        p.b.receive(&datagram, t, &mut p.rng, &mut out),
        Err(Drop::Replay)
    );
    let mut tampered = datagram.clone();
    tampered[20] ^= 1;
    // A higher counter would pass the window, but the tag fails.
    tampered[8] = 200;
    assert_eq!(
        p.b.receive(&tampered, t, &mut p.rng, &mut out),
        Err(Drop::Unauthenticated)
    );
    // B only accepts CLIENT/32 as a source.
    let mut io = Io::default();
    p.a.send(
        &packet([10, 8, 0, 9], SERVER, b"spoof"),
        t,
        &mut p.rng,
        &mut io,
    );
    assert_eq!(
        p.b.receive(&io.network[0], t, &mut p.rng, &mut out),
        Err(Drop::NotAllowed)
    );
    // An unknown index.
    let mut unknown = datagram;
    unknown[4] ^= 0xFF;
    assert_eq!(
        p.b.receive(&unknown, t, &mut p.rng, &mut out),
        Err(Drop::UnknownIndex)
    );
}

#[test]
fn a_replayed_initiation_is_refused() {
    let mut p = Pair::new(None, None);
    let mut io = Io::default();
    p.a.connect(at(0), &mut p.rng, &mut io);
    let init = io.network[0].clone();
    let mut out = Io::default();
    assert_eq!(p.b.receive(&init, at(0), &mut p.rng, &mut out), Ok(()));
    assert_eq!(
        p.b.receive(&init, at(100), &mut p.rng, &mut out),
        Err(Drop::Replay)
    );
    let mut forged = init;
    forged[120] ^= 1;
    assert_eq!(
        p.b.receive(&forged, at(200), &mut p.rng, &mut out),
        Err(Drop::Unauthenticated)
    );
}

#[test]
fn unanswered_initiations_retransmit_then_give_up() {
    let mut p = Pair::new(None, None);
    let mut io = Io::default();
    p.a.send(&packet(CLIENT, SERVER, b"lost"), at(0), &mut p.rng, &mut io);
    assert_eq!(io.network.len(), 1);
    let mut sent = 1;
    let mut t = 0;
    while t <= REKEY_ATTEMPT_TIME + REKEY_TIMEOUT * 2 {
        t += 100;
        let mut io = Io::default();
        p.a.tick(at(t), &mut p.rng, &mut io);
        sent += io.network.len();
    }
    // About one per 5 s for 90 s, then nothing.
    assert!((18..=19).contains(&sent), "{sent} initiations");
    assert!(p.a.gave_up());
    // New data starts over: the staged "lost" packet was dropped, the new
    // one arrives once the peer answers.
    let (_, got_b) = p.a_sends(b"later", at(t));
    assert_eq!(got_b, [packet(CLIENT, SERVER, b"later")]);
}

#[test]
fn keepalives_and_rekeying_follow_the_timers() {
    let mut p = Pair::new(None, None);
    p.a_sends(b"one", at(0));
    // B received data and sent nothing: a keepalive 10 s later.
    let mut io = Io::default();
    p.b.tick(at(KEEPALIVE_TIMEOUT - 1), &mut p.rng, &mut io);
    assert!(io.network.is_empty());
    p.b.tick(at(KEEPALIVE_TIMEOUT), &mut p.rng, &mut io);
    assert_eq!(io.network.len(), 1);
    assert_eq!(io.network[0].len(), 32, "an empty transport message");
    let (got_a, _) = p.exchange(
        std::mem::take(&mut io.network),
        Vec::new(),
        at(KEEPALIVE_TIMEOUT),
    );
    assert!(got_a.is_empty());
    // After REKEY_AFTER_TIME the initiator rekeys on its next send.
    let t = at(REKEY_AFTER_TIME + 1);
    let (_, got_b) = p.a_sends(b"two", t);
    assert_eq!(got_b, [packet(CLIENT, SERVER, b"two")]);
    assert_eq!(p.a.stats.handshakes, 2);
    // The old keys still decrypt in flight, the new ones are current.
    let (_, got_b) = p.a_sends(b"three", t);
    assert_eq!(got_b.len(), 1);
    // Past REJECT_AFTER_TIME without traffic nothing is usable.
    assert!(!p.a.established(REKEY_AFTER_TIME + 1 + REJECT_AFTER_TIME));
}

#[test]
fn data_without_an_answer_starts_a_new_handshake() {
    let mut p = Pair::new(None, None);
    p.a_sends(b"one", at(0));
    // B's keepalive answers "one"; then B stops answering: A sends data at
    // 20 s and hears nothing for 15 s.
    let mut io = Io::default();
    p.b.tick(at(KEEPALIVE_TIMEOUT), &mut p.rng, &mut io);
    p.exchange(
        std::mem::take(&mut io.network),
        Vec::new(),
        at(KEEPALIVE_TIMEOUT),
    );
    let mut io = Io::default();
    p.a.send(
        &packet(CLIENT, SERVER, b"two"),
        at(20_000),
        &mut p.rng,
        &mut io,
    );
    let mut io = Io::default();
    p.a.tick(
        at(20_000 + KEEPALIVE_TIMEOUT + REKEY_TIMEOUT - 1),
        &mut p.rng,
        &mut io,
    );
    assert!(io.network.is_empty());
    p.a.tick(
        at(20_000 + KEEPALIVE_TIMEOUT + REKEY_TIMEOUT),
        &mut p.rng,
        &mut io,
    );
    assert_eq!(io.network.len(), 1);
    assert_eq!(io.network[0][0], 1, "an initiation");
}

#[test]
fn persistent_keepalive_brings_the_tunnel_up_and_keeps_it_warm() {
    let mut p = Pair::new(None, Some(25));
    let mut io = Io::default();
    p.a.tick(at(0), &mut p.rng, &mut io);
    assert_eq!(io.network.len(), 1);
    p.pump(std::mem::take(&mut io.network), at(0));
    assert!(p.a.established(0));
    let mut io = Io::default();
    p.a.tick(at(24_999), &mut p.rng, &mut io);
    assert!(io.network.is_empty());
    p.a.tick(at(25_000), &mut p.rng, &mut io);
    assert_eq!(io.network.len(), 1);
    assert_eq!(io.network[0].len(), 32);
}
