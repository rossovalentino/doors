//! Interoperability with an independent implementation: boringtun
//! (Cloudflare) as the peer, door-wireguard as initiator and as responder,
//! with and without a preshared key, then rekeying from both sides.
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use door_wireguard::{Cidr, Io, Now, Random, Tunnel, crypto};
use std::time::{SystemTime, UNIX_EPOCH};

struct Rng(u64);
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
fn packet(src: [u8; 4], dst: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let total = 28 + payload.len();
    let mut p = vec![0u8; total];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&src);
    p[16..20].copy_from_slice(&dst);
    p[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    p[28..].copy_from_slice(payload);
    p
}
fn now() -> Now {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    Now {
        millis: since.as_millis() as u64,
        unix_nanos: since.as_nanos() as u64,
    }
}
const DOORS: [u8; 4] = [10, 8, 0, 2];
const BORING: [u8; 4] = [10, 8, 0, 1];

struct Peers {
    ours: Tunnel,
    theirs: Tunn,
    rng: Rng,
}
impl Peers {
    fn new(psk: Option<[u8; 32]>) -> Self {
        let ours_private = crypto::clamp([0x31; 32]);
        let theirs_private = StaticSecret::from([0x52; 32]);
        let theirs_public = PublicKey::from(&theirs_private);
        Self {
            ours: Tunnel::new(
                ours_private,
                theirs_public.to_bytes(),
                psk,
                vec![Cidr::new(BORING, 24)],
                None,
                1420,
            ),
            theirs: Tunn::new(
                theirs_private,
                PublicKey::from(crypto::public_key(&ours_private)),
                psk,
                None,
                7,
                None,
            ),
            rng: Rng(0xD005_5EED_9E37_79B9),
        }
    }
    /// Datagrams from us to boringtun: what boringtun writes to its tunnel,
    /// and what it sends back (delivered to us until both are quiet).
    fn deliver(&mut self, mut datagrams: Vec<Vec<u8>>) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let (mut their_tunnel, mut our_tunnel) = (Vec::new(), Vec::new());
        let mut buf = vec![0u8; 2048];
        while !datagrams.is_empty() {
            let mut replies = Vec::new();
            for d in datagrams.drain(..) {
                let mut first = true;
                loop {
                    let input: &[u8] = if first { &d } else { &[] };
                    first = false;
                    match self.theirs.decapsulate(None, input, &mut buf) {
                        TunnResult::WriteToNetwork(out) => replies.push(out.to_vec()),
                        TunnResult::WriteToTunnelV4(p, _) => {
                            their_tunnel.push(p.to_vec());
                            break;
                        }
                        TunnResult::Done => break,
                        TunnResult::Err(e) => panic!("boringtun refused our datagram: {e:?}"),
                        TunnResult::WriteToTunnelV6(..) => unreachable!(),
                    }
                }
            }
            for r in replies {
                let mut io = Io::default();
                self.ours
                    .receive(&r, now(), &mut self.rng, &mut io)
                    .expect("we accept boringtun's datagram");
                datagrams.extend(io.network);
                our_tunnel.extend(io.tunnel);
            }
        }
        (their_tunnel, our_tunnel)
    }
    fn theirs_send(&mut self, p: &[u8]) -> Vec<u8> {
        let mut buf = vec![0u8; 2048];
        match self.theirs.encapsulate(p, &mut buf) {
            TunnResult::WriteToNetwork(out) => out.to_vec(),
            other => panic!("boringtun did not encapsulate: {other:?}"),
        }
    }
}

#[test]
fn doors_initiates_to_boringtun() {
    for psk in [None, Some([0x44u8; 32])] {
        let mut p = Peers::new(psk);
        let mut io = Io::default();
        p.ours.send(
            &packet(DOORS, BORING, b"ping from Doors"),
            now(),
            &mut p.rng,
            &mut io,
        );
        assert_eq!(io.network.len(), 1, "an initiation");
        let (their_tunnel, _) = p.deliver(io.network);
        assert_eq!(their_tunnel, [packet(DOORS, BORING, b"ping from Doors")]);
        assert!(p.ours.established(now().millis));
        // boringtun answers on the session.
        let reply = p.theirs_send(&packet(BORING, DOORS, b"pong from boringtun"));
        let mut io = Io::default();
        p.ours.receive(&reply, now(), &mut p.rng, &mut io).unwrap();
        assert_eq!(io.tunnel, [packet(BORING, DOORS, b"pong from boringtun")]);
        // Many packets of every size up to the MTU each way.
        for len in [0usize, 1, 15, 16, 17, 100, 1000, 1392] {
            let payload = vec![len as u8; len];
            let mut io = Io::default();
            p.ours
                .send(&packet(DOORS, BORING, &payload), now(), &mut p.rng, &mut io);
            let (their_tunnel, _) = p.deliver(io.network);
            assert_eq!(their_tunnel, [packet(DOORS, BORING, &payload)]);
            let back = p.theirs_send(&packet(BORING, DOORS, &payload));
            let mut io = Io::default();
            p.ours.receive(&back, now(), &mut p.rng, &mut io).unwrap();
            assert_eq!(io.tunnel, [packet(BORING, DOORS, &payload)]);
        }
    }
}

#[test]
fn boringtun_initiates_to_doors() {
    for psk in [None, Some([0x45u8; 32])] {
        let mut p = Peers::new(psk);
        let init = p.theirs_send(&packet(BORING, DOORS, b"hello Doors"));
        assert_eq!(init[0], 1, "boringtun starts with an initiation");
        let mut io = Io::default();
        p.ours.receive(&init, now(), &mut p.rng, &mut io).unwrap();
        assert_eq!(io.network.len(), 1, "our response");
        // boringtun completes and sends its staged packet; it reaches our
        // tunnel and confirms our keys.
        let (_, our_tunnel) = p.deliver(io.network);
        assert_eq!(our_tunnel, [packet(BORING, DOORS, b"hello Doors")]);
        assert!(p.ours.established(now().millis));
        let mut io = Io::default();
        p.ours.send(
            &packet(DOORS, BORING, b"hi boringtun"),
            now(),
            &mut p.rng,
            &mut io,
        );
        let (their_tunnel, _) = p.deliver(io.network);
        assert_eq!(their_tunnel, [packet(DOORS, BORING, b"hi boringtun")]);
    }
}

#[test]
fn a_new_handshake_mid_session_keeps_data_flowing() {
    let mut p = Peers::new(Some([9; 32]));
    let mut io = Io::default();
    p.ours.connect(now(), &mut p.rng, &mut io);
    p.deliver(io.network);
    assert!(p.ours.established(now().millis));
    // boringtun rekeys on its own (as after REKEY_AFTER_TIME).
    let mut buf = vec![0u8; 2048];
    let init = match p.theirs.format_handshake_initiation(&mut buf, true) {
        TunnResult::WriteToNetwork(out) => out.to_vec(),
        other => panic!("{other:?}"),
    };
    let mut io = Io::default();
    p.ours.receive(&init, now(), &mut p.rng, &mut io).unwrap();
    p.deliver(io.network);
    // Its data now uses the new keys; ours follow once we see them.
    let data = p.theirs_send(&packet(BORING, DOORS, b"new keys"));
    let mut io = Io::default();
    p.ours.receive(&data, now(), &mut p.rng, &mut io).unwrap();
    assert_eq!(io.tunnel, [packet(BORING, DOORS, b"new keys")]);
    assert_eq!(p.ours.stats.handshakes, 2);
    let mut io = Io::default();
    p.ours
        .send(&packet(DOORS, BORING, b"ack"), now(), &mut p.rng, &mut io);
    let (their_tunnel, _) = p.deliver(io.network);
    assert_eq!(their_tunnel, [packet(DOORS, BORING, b"ack")]);
}

#[test]
fn a_wrong_peer_key_never_completes() {
    let mut p = Peers::new(None);
    // boringtun expects another peer: it drops our initiation.
    p.theirs = Tunn::new(
        StaticSecret::from([0x52; 32]),
        PublicKey::from([0x11; 32]),
        None,
        None,
        7,
        None,
    );
    let mut io = Io::default();
    p.ours
        .send(&packet(DOORS, BORING, b"x"), now(), &mut p.rng, &mut io);
    let mut buf = vec![0u8; 2048];
    assert!(matches!(
        p.theirs.decapsulate(None, &io.network[0], &mut buf),
        TunnResult::Err(_)
    ));
    assert!(!p.ours.established(now().millis));
}
