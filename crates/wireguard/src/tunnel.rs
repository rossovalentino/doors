//! One WireGuard tunnel to one peer: handshakes in both roles, the three
//! keypairs (previous, current, next), transport data with cryptokey
//! routing, packets staged while no session exists, and the timers of
//! whitepaper section 6 (as wireguard-go schedules them). Sans-IO: callers
//! pass datagrams, packets, the time and an entropy source, and send what
//! `Io` collects.
use crate::{
    crypto,
    handshake::{self, Pending, Statics},
    ip::{self, Cidr},
    replay::Window,
};
use alloc::{collections::VecDeque, vec, vec::Vec};

pub const REKEY_AFTER_MESSAGES: u64 = 1 << 60;
pub const REJECT_AFTER_MESSAGES: u64 = u64::MAX - (1 << 13);
pub const REKEY_AFTER_TIME: u64 = 120_000;
pub const REJECT_AFTER_TIME: u64 = 180_000;
pub const REKEY_ATTEMPT_TIME: u64 = 90_000;
pub const REKEY_TIMEOUT: u64 = 5_000;
pub const KEEPALIVE_TIMEOUT: u64 = 10_000;
const COOKIE_LIFETIME: u64 = 120_000;
/// Packets waiting for a session.
const MAX_STAGED: usize = 128;
/// Initiations accepted at most every 20 ms (wireguard-go's 50 per second).
const INITIATION_GAP: u64 = 20;

/// Entropy for ephemeral keys, indices and jitter.
pub trait Random {
    fn fill(&mut self, out: &mut [u8]);
}
/// The time: a monotonic clock for timers, UTC for handshake timestamps.
#[derive(Clone, Copy, Debug, Default)]
pub struct Now {
    pub millis: u64,
    /// Nanoseconds since 1970-01-01 UTC (0 when no clock: timestamps then
    /// still grow within this tunnel's life).
    pub unix_nanos: u64,
}
/// What a call produced: UDP payloads for the endpoint, IPv4 packets for
/// the tunnel interface.
#[derive(Default, Debug)]
pub struct Io {
    pub network: Vec<Vec<u8>>,
    pub tunnel: Vec<Vec<u8>>,
}
/// Why a datagram was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drop {
    Malformed,
    /// mac1 or a handshake did not verify, or the peer is not ours.
    Unauthenticated,
    /// A replayed initiation timestamp or transport counter.
    Replay,
    /// Initiations faster than 50 per second.
    Flood,
    /// No session holds that index, or it expired.
    UnknownIndex,
    /// The packet's source is outside the peer's AllowedIPs.
    NotAllowed,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Plaintext bytes sent and received through the tunnel.
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub handshakes: u32,
    /// When the latest handshake completed (monotonic milliseconds).
    pub last_handshake: Option<u64>,
}

struct Keypair {
    send: [u8; 32],
    receive: [u8; 32],
    local: u32,
    remote: u32,
    sent: u64,
    window: Window,
    created: u64,
    initiator: bool,
}
impl Keypair {
    fn usable(&self, now: u64) -> bool {
        now.saturating_sub(self.created) < REJECT_AFTER_TIME && self.sent < REJECT_AFTER_MESSAGES
    }
}

pub struct Tunnel {
    st: Statics,
    allowed: Vec<Cidr>,
    /// AllowedIPs' IPv6 prefixes (`allow6`).
    allowed6: Vec<ip::Cidr6>,
    keepalive: Option<u16>,
    mtu: usize,
    previous: Option<Keypair>,
    current: Option<Keypair>,
    next: Option<Keypair>,
    pending: Option<Pending>,
    /// The first initiation of the current attempt (REKEY_ATTEMPT_TIME).
    attempt_started: Option<u64>,
    last_initiation: Option<u64>,
    retransmit_at: Option<u64>,
    /// mac1 of our latest handshake message (a cookie reply's AAD).
    last_mac1: Option<[u8; 16]>,
    cookie: Option<([u8; 16], u64)>,
    timestamp_sent: [u8; 12],
    timestamp_received: [u8; 12],
    initiation_received: Option<u64>,
    staged: VecDeque<Vec<u8>>,
    last_sent: Option<u64>,
    last_received: Option<u64>,
    /// Data went out at this time with nothing heard since: a new handshake
    /// is due 15 s later.
    unanswered_since: Option<u64>,
    keepalive_due: Option<u64>,
    /// The last-minute rekey on receive was already sent for `current`.
    last_minute: bool,
    gave_up: bool,
    pub stats: Stats,
}

impl Tunnel {
    pub fn new(
        private: [u8; 32],
        peer: [u8; 32],
        psk: Option<[u8; 32]>,
        allowed: Vec<Cidr>,
        keepalive: Option<u16>,
        mtu: u16,
    ) -> Self {
        Self {
            st: Statics::new(private, peer, psk),
            allowed,
            allowed6: Vec::new(),
            keepalive: keepalive.filter(|k| *k != 0),
            mtu: usize::from(mtu),
            previous: None,
            current: None,
            next: None,
            pending: None,
            attempt_started: None,
            last_initiation: None,
            retransmit_at: None,
            last_mac1: None,
            cookie: None,
            timestamp_sent: [0; 12],
            timestamp_received: [0; 12],
            initiation_received: None,
            staged: VecDeque::new(),
            last_sent: None,
            last_received: None,
            unanswered_since: None,
            keepalive_due: None,
            last_minute: false,
            gave_up: false,
            stats: Stats::default(),
        }
    }
    /// The peer's IPv6 AllowedIPs (the IPv6 sources it may send from).
    pub fn allow6(&mut self, list: Vec<ip::Cidr6>) {
        self.allowed6 = list;
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.st.public
    }
    /// A session can carry data now.
    pub fn established(&self, now: u64) -> bool {
        self.current.as_ref().is_some_and(|k| k.usable(now))
    }
    /// The handshake gave up after REKEY_ATTEMPT_TIME without an answer
    /// (until data or `connect` starts a new attempt).
    pub fn gave_up(&self) -> bool {
        self.gave_up
    }
    /// Start a handshake now (bringing the tunnel up).
    pub fn connect(&mut self, now: Now, rng: &mut impl Random, io: &mut Io) {
        self.gave_up = false;
        if !self.established(now.millis) {
            self.initiate(now, rng, io, false);
        }
    }

    /// Encrypt one IPv4 packet for the peer (staged and a handshake started
    /// when no session is usable).
    pub fn send(&mut self, packet: &[u8], now: Now, rng: &mut impl Random, io: &mut Io) {
        if self.established(now.millis) {
            self.transmit(packet, now.millis, io);
            let rekey = self.current.as_ref().is_some_and(|k| {
                k.initiator
                    && (now.millis - k.created >= REKEY_AFTER_TIME
                        || k.sent >= REKEY_AFTER_MESSAGES)
            });
            if rekey {
                self.initiate(now, rng, io, false);
            }
            return;
        }
        if self.staged.len() == MAX_STAGED {
            self.staged.pop_front();
        }
        self.staged.push_back(packet.to_vec());
        self.gave_up = false;
        self.initiate(now, rng, io, false);
    }

    fn transmit(&mut self, packet: &[u8], now: u64, io: &mut Io) {
        let Some(k) = self.current.as_mut() else {
            return;
        };
        let padded = if packet.is_empty() {
            0
        } else {
            packet
                .len()
                .next_multiple_of(16)
                .min(self.mtu.max(packet.len()))
        };
        let mut plain = vec![0u8; padded];
        plain[..packet.len()].copy_from_slice(packet);
        let counter = k.sent;
        k.sent += 1;
        let mut msg = Vec::with_capacity(handshake::TRANSPORT_HEADER + padded + crypto::TAG);
        msg.extend_from_slice(&[handshake::TRANSPORT, 0, 0, 0]);
        msg.extend_from_slice(&k.remote.to_le_bytes());
        msg.extend_from_slice(&counter.to_le_bytes());
        msg.extend_from_slice(&crypto::seal(&k.send, counter, &plain, &[]));
        io.network.push(msg);
        self.last_sent = Some(now);
        self.keepalive_due = None;
        if !packet.is_empty() {
            self.stats.tx_bytes += packet.len() as u64;
            self.unanswered_since.get_or_insert(now);
        }
    }

    fn fresh_index(&self, rng: &mut impl Random) -> u32 {
        loop {
            let mut b = [0u8; 4];
            rng.fill(&mut b);
            let index = u32::from_le_bytes(b);
            let taken = [&self.previous, &self.current, &self.next]
                .iter()
                .any(|k| k.as_ref().is_some_and(|k| k.local == index))
                || self.pending.as_ref().is_some_and(|p| p.sender == index);
            if !taken {
                return index;
            }
        }
    }
    fn cookie_now(&self, now: u64) -> Option<[u8; 16]> {
        self.cookie
            .filter(|(_, at)| now.saturating_sub(*at) < COOKIE_LIFETIME)
            .map(|(c, _)| c)
    }
    /// Send an initiation; `retry` is the retransmit timer (otherwise at
    /// most one per REKEY_TIMEOUT).
    fn initiate(&mut self, now: Now, rng: &mut impl Random, io: &mut Io, retry: bool) {
        let t = now.millis;
        if !retry && self.pending.is_some() {
            return;
        }
        if !retry
            && self
                .last_initiation
                .is_some_and(|at| t.saturating_sub(at) < REKEY_TIMEOUT)
        {
            return;
        }
        let sender = self.fresh_index(rng);
        let mut ephemeral = [0u8; 32];
        rng.fill(&mut ephemeral);
        let mut stamp = handshake::tai64n(now.unix_nanos);
        if stamp <= self.timestamp_sent {
            stamp = self.timestamp_sent;
            for b in stamp.iter_mut().rev() {
                *b = b.wrapping_add(1);
                if *b != 0 {
                    break;
                }
            }
        }
        let Some((mut msg, pending)) = handshake::initiate(&self.st, sender, ephemeral, stamp)
        else {
            // The peer's key is a low-order point: nothing can complete.
            self.gave_up = true;
            return;
        };
        self.timestamp_sent = stamp;
        self.last_mac1 = Some(handshake::seal_macs(
            &mut msg,
            &self.st.mac1_send,
            self.cookie_now(t).as_ref(),
        ));
        io.network.push(msg.to_vec());
        self.pending = Some(pending);
        self.attempt_started.get_or_insert(t);
        self.last_initiation = Some(t);
        let mut jitter = [0u8; 2];
        rng.fill(&mut jitter);
        self.retransmit_at = Some(t + REKEY_TIMEOUT + u64::from(u16::from_le_bytes(jitter) % 334));
        self.last_sent = Some(t);
        self.keepalive_due = None;
    }

    /// A UDP datagram from the peer.
    pub fn receive(
        &mut self,
        datagram: &[u8],
        now: Now,
        rng: &mut impl Random,
        io: &mut Io,
    ) -> Result<(), Drop> {
        if datagram.len() < 4 || datagram[1..4] != [0; 3] {
            return Err(Drop::Malformed);
        }
        match datagram[0] {
            handshake::INITIATION => self.on_initiation(datagram, now, rng, io),
            handshake::RESPONSE => self.on_response(datagram, now, rng, io),
            handshake::COOKIE_REPLY => self.on_cookie(datagram, now),
            handshake::TRANSPORT => self.on_transport(datagram, now, rng, io),
            _ => Err(Drop::Malformed),
        }
    }

    fn on_initiation(
        &mut self,
        msg: &[u8],
        now: Now,
        rng: &mut impl Random,
        io: &mut Io,
    ) -> Result<(), Drop> {
        let t = now.millis;
        if msg.len() != handshake::INITIATION_BYTES {
            return Err(Drop::Malformed);
        }
        if !handshake::mac1_valid(msg, &self.st.mac1_receive) {
            return Err(Drop::Unauthenticated);
        }
        let accepted = handshake::accept(&self.st, msg).ok_or(Drop::Unauthenticated)?;
        if accepted.timestamp <= self.timestamp_received {
            return Err(Drop::Replay);
        }
        if self
            .initiation_received
            .is_some_and(|at| t.saturating_sub(at) < INITIATION_GAP)
        {
            return Err(Drop::Flood);
        }
        let sender = self.fresh_index(rng);
        let mut ephemeral = [0u8; 32];
        rng.fill(&mut ephemeral);
        let (mut reply, keys) = handshake::respond(&self.st, &accepted, sender, ephemeral)
            .ok_or(Drop::Unauthenticated)?;
        self.timestamp_received = accepted.timestamp;
        self.initiation_received = Some(t);
        self.last_mac1 = Some(handshake::seal_macs(
            &mut reply,
            &self.st.mac1_send,
            self.cookie_now(t).as_ref(),
        ));
        io.network.push(reply.to_vec());
        // The responder's keys wait in `next` until the initiator's first
        // data confirms them.
        self.next = Some(Keypair {
            send: keys.send,
            receive: keys.receive,
            local: sender,
            remote: accepted.sender,
            sent: 0,
            window: Window::default(),
            created: t,
            initiator: false,
        });
        self.previous = None;
        self.last_received = Some(t);
        self.last_sent = Some(t);
        self.keepalive_due = None;
        self.unanswered_since = None;
        Ok(())
    }

    fn on_response(
        &mut self,
        msg: &[u8],
        now: Now,
        rng: &mut impl Random,
        io: &mut Io,
    ) -> Result<(), Drop> {
        let t = now.millis;
        if msg.len() != handshake::RESPONSE_BYTES {
            return Err(Drop::Malformed);
        }
        if !handshake::mac1_valid(msg, &self.st.mac1_receive) {
            return Err(Drop::Unauthenticated);
        }
        let pending = self.pending.as_ref().ok_or(Drop::UnknownIndex)?;
        let (remote, keys) =
            handshake::complete(&self.st, pending, msg).ok_or(Drop::Unauthenticated)?;
        let local = pending.sender;
        self.pending = None;
        let fresh = Keypair {
            send: keys.send,
            receive: keys.receive,
            local,
            remote,
            sent: 0,
            window: Window::default(),
            created: t,
            initiator: true,
        };
        // As wireguard-go rotates for the initiator.
        if let Some(next) = self.next.take() {
            self.previous = Some(next);
        } else {
            self.previous = self.current.take();
        }
        self.current = Some(fresh);
        self.handshake_done(t);
        self.last_received = Some(t);
        self.unanswered_since = None;
        // Staged packets go now; without any, a keepalive confirms the
        // session to the responder.
        if self.staged.is_empty() {
            self.transmit(&[], t, io);
        }
        while let Some(packet) = self.staged.pop_front() {
            self.send(&packet, now, rng, io);
        }
        Ok(())
    }
    fn handshake_done(&mut self, t: u64) {
        self.attempt_started = None;
        self.retransmit_at = None;
        self.last_minute = false;
        self.gave_up = false;
        self.stats.handshakes += 1;
        self.stats.last_handshake = Some(t);
    }

    fn on_cookie(&mut self, msg: &[u8], now: Now) -> Result<(), Drop> {
        let mac1 = self.last_mac1.ok_or(Drop::UnknownIndex)?;
        let (index, cookie) =
            handshake::open_cookie(&self.st, msg, &mac1).ok_or(Drop::Unauthenticated)?;
        let ours = self.pending.as_ref().is_some_and(|p| p.sender == index)
            || self.next.as_ref().is_some_and(|k| k.local == index);
        if !ours {
            return Err(Drop::UnknownIndex);
        }
        // The retransmit timer resends with mac2.
        self.cookie = Some((cookie, now.millis));
        Ok(())
    }

    fn on_transport(
        &mut self,
        msg: &[u8],
        now: Now,
        rng: &mut impl Random,
        io: &mut Io,
    ) -> Result<(), Drop> {
        let t = now.millis;
        if msg.len() < handshake::TRANSPORT_HEADER + crypto::TAG {
            return Err(Drop::Malformed);
        }
        let receiver = u32::from_le_bytes(msg[4..8].try_into().unwrap());
        let counter = u64::from_le_bytes(msg[8..16].try_into().unwrap());
        let slot = [&self.current, &self.next, &self.previous]
            .iter()
            .position(|k| k.as_ref().is_some_and(|k| k.local == receiver))
            .ok_or(Drop::UnknownIndex)?;
        let k = match slot {
            0 => self.current.as_mut(),
            1 => self.next.as_mut(),
            _ => self.previous.as_mut(),
        }
        .unwrap();
        if t.saturating_sub(k.created) >= REJECT_AFTER_TIME {
            return Err(Drop::UnknownIndex);
        }
        if !k.window.fresh(counter, REJECT_AFTER_MESSAGES) {
            return Err(Drop::Replay);
        }
        let plain = crypto::open(
            &k.receive,
            counter,
            &msg[handshake::TRANSPORT_HEADER..],
            &[],
        )
        .ok_or(Drop::Unauthenticated)?;
        if !k.window.accept(counter, REJECT_AFTER_MESSAGES) {
            return Err(Drop::Replay);
        }
        if slot == 1 {
            // The initiator used the new keys: they become current.
            self.previous = self.current.take();
            self.current = self.next.take();
            self.handshake_done(t);
            // Anything staged while the session was missing goes now.
            while let Some(packet) = self.staged.pop_front() {
                self.send(&packet, now, rng, io);
            }
        }
        self.last_received = Some(t);
        self.unanswered_since = None;
        // Our keys are getting old and we initiated them: rekey before the
        // peer has to drop them.
        let stale = self.current.as_ref().is_some_and(|k| {
            k.initiator
                && t.saturating_sub(k.created)
                    >= REJECT_AFTER_TIME - KEEPALIVE_TIMEOUT - REKEY_TIMEOUT
        });
        if stale && !self.last_minute {
            self.last_minute = true;
            self.initiate(now, rng, io, false);
        }
        if plain.is_empty() {
            return Ok(());
        }
        self.keepalive_due.get_or_insert(t + KEEPALIVE_TIMEOUT);
        // Cryptokey routing: the source must be one the peer may use.
        let total = match plain[0] >> 4 {
            6 => {
                let h = ip::header6(&plain).ok_or(Drop::Malformed)?;
                if !self.allowed6.iter().any(|c| c.contains(h.source)) {
                    return Err(Drop::NotAllowed);
                }
                h.total
            }
            _ => {
                let h = ip::header(&plain).ok_or(Drop::Malformed)?;
                if !self.allowed.iter().any(|c| c.contains(h.source)) {
                    return Err(Drop::NotAllowed);
                }
                h.total
            }
        };
        self.stats.rx_bytes += total as u64;
        let mut packet = plain;
        packet.truncate(total);
        io.tunnel.push(packet);
        Ok(())
    }

    /// Timers: retransmission, keepalives, new handshakes, key expiry. Call
    /// at least every few hundred milliseconds.
    pub fn tick(&mut self, now: Now, rng: &mut impl Random, io: &mut Io) {
        let t = now.millis;
        if self.pending.is_some() && self.retransmit_at.is_some_and(|at| t >= at) {
            if self
                .attempt_started
                .is_some_and(|s| t.saturating_sub(s) >= REKEY_ATTEMPT_TIME)
            {
                // Give up: drop what waited (wireguard-go's "Handshake did
                // not complete after 20 attempts").
                self.pending = None;
                self.retransmit_at = None;
                self.attempt_started = None;
                self.staged.clear();
                self.gave_up = true;
            } else {
                self.initiate(now, rng, io, true);
            }
        }
        if self.keepalive_due.is_some_and(|at| t >= at) {
            self.keepalive_due = None;
            if self.established(t) {
                self.transmit(&[], t, io);
            }
        }
        if self
            .unanswered_since
            .is_some_and(|at| t >= at + KEEPALIVE_TIMEOUT + REKEY_TIMEOUT)
        {
            self.unanswered_since = None;
            self.initiate(now, rng, io, false);
        }
        if let Some(k) = self.keepalive {
            let last = self.last_sent.max(self.last_received);
            if last.is_none_or(|at| t >= at + u64::from(k) * 1000) && !self.gave_up {
                if self.established(t) {
                    self.transmit(&[], t, io);
                } else {
                    self.initiate(now, rng, io, false);
                }
            }
        }
        // Three REJECT_AFTER_TIMEs after the last handshake, all keys go.
        if self
            .stats
            .last_handshake
            .is_some_and(|at| t >= at + 3 * REJECT_AFTER_TIME)
        {
            self.previous = None;
            self.current = None;
            self.next = None;
            self.pending = None;
        }
        for slot in [&mut self.previous, &mut self.next] {
            if slot
                .as_ref()
                .is_some_and(|k| t.saturating_sub(k.created) >= REJECT_AFTER_TIME)
            {
                *slot = None;
            }
        }
    }
}
