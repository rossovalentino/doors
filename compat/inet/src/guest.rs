//! Blocking transport over ntdll's `DoorNet*` sockets and door-tls.
use crate::{Failure, Tolerance, failure_flags, secure};
use alloc::{vec, vec::Vec};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorNetSocket(kind: u32, id: u64) -> u32;
    fn DoorNetClose(id: u32) -> u32;
    fn DoorNetConnect(id: u32, address: u32, port: u32) -> u32;
    fn DoorNetSend(id: u32, source: u64, bytes: u32, sent: u64) -> u32;
    fn DoorNetReceive(id: u32, destination: u64, bytes: u32, received: u64) -> u32;
    fn DoorNetState(id: u32, out: u64) -> u32;
    fn DoorNetShutdown(id: u32) -> u32;
    fn DoorNetResolve(name: u64, length: u32, out: u64, max: u32, count: u64, timeout: u32) -> u32;
    fn DoorGetTickCount64() -> u64;
    fn DoorSleep(millis: u32);
    fn NtQuerySystemTime(time: usize) -> u32;
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
}

const WSAEWOULDBLOCK: u32 = 10035;
const NET_TCP: u32 = 1;
mod state {
    pub const CONNECTING: u8 = 1;
    pub const ESTABLISHED: u8 = 2;
    pub const PEER_CLOSED: u8 = 3;
}

pub fn tick() -> u64 {
    unsafe { DoorGetTickCount64() }
}
/// Unix seconds now (for certificate dates and cookies); 0 if the clock is
/// not set.
pub fn now_unix() -> i64 {
    let mut filetime = 0u64;
    if unsafe { NtQuerySystemTime(crate::narrow(&mut filetime as *mut u64 as u64)) } != 0 {
        return 0;
    }
    (filetime / 10_000_000) as i64 - 11_644_473_600
}
pub fn random(out: &mut [u8]) -> bool {
    unsafe { DoorGenRandom(out.as_mut_ptr() as u64, out.len() as u32) == 0 }
}
/// A deadline `millis` from now; 0 means none (WinHTTP's infinite).
#[derive(Clone, Copy)]
pub struct Deadline(Option<u64>);
impl Deadline {
    pub fn after(millis: u32) -> Self {
        Self((millis != 0 && millis != u32::MAX).then(|| tick() + u64::from(millis)))
    }
    pub fn passed(self) -> bool {
        self.0.is_some_and(|d| tick() >= d)
    }
}

/// The IPv4 address of `host` (a literal, or the network service's
/// resolver).
pub fn resolve(host: &str, deadline: Deadline) -> Result<[u8; 4], Failure> {
    if let Some(address) = crate::ipv4_literal(host) {
        return Ok(address);
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Ok([127, 0, 0, 1]);
    }
    let mut out = [[0u8; 4]; 4];
    let mut count = 0u32;
    let timeout = deadline
        .0
        .map_or(10_000, |d| d.saturating_sub(tick()).clamp(1, 10_000) as u32);
    match unsafe {
        DoorNetResolve(
            host.as_ptr() as u64,
            host.len() as u32,
            out.as_mut_ptr() as u64,
            4,
            &mut count as *mut u32 as u64,
            timeout,
        )
    } {
        0 if count > 0 => Ok(out[0]),
        _ if deadline.passed() => Err(Failure::Timeout),
        _ => Err(Failure::NameNotResolved),
    }
}

/// One connection: TCP, optionally TLS.
pub struct Stream {
    socket: u32,
    tls: Option<door_tls::Client>,
    /// Ciphertext the TLS client produced and the socket did not take yet.
    outbox: Vec<u8>,
    /// The peer closed (orderly).
    closed: bool,
    /// Certificate problems found (tolerated ones included), as
    /// WINHTTP_CALLBACK_STATUS_FLAG_* bits.
    pub certificate_flags: u32,
}
impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { DoorNetClose(self.socket) };
    }
}
fn net_state(socket: u32) -> Option<u8> {
    let mut b = [0u8; 12];
    (unsafe { DoorNetState(socket, b.as_mut_ptr() as u64) } == 0).then_some(b[0])
}
impl Stream {
    pub fn connect(
        address: [u8; 4],
        port: u16,
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Failure> {
        let mut socket = 0u32;
        if unsafe { DoorNetSocket(NET_TCP, &mut socket as *mut u32 as u64) } != 0 {
            return Err(Failure::CannotConnect);
        }
        let stream = Self {
            socket,
            tls: None,
            outbox: Vec::new(),
            closed: false,
            certificate_flags: 0,
        };
        if unsafe { DoorNetConnect(socket, u32::from_ne_bytes(address), u32::from(port)) } != 0 {
            return Err(Failure::CannotConnect);
        }
        // Windows' TCP connect gives up after about 21 s on its own.
        let give_up = tick() + 21_000;
        loop {
            match net_state(socket) {
                Some(state::ESTABLISHED | state::PEER_CLOSED) => return Ok(stream),
                Some(state::CONNECTING) => (),
                _ => return Err(Failure::CannotConnect),
            }
            if cancelled() {
                return Err(Failure::Cancelled);
            }
            if deadline.passed() || tick() >= give_up {
                return Err(Failure::Timeout);
            }
            unsafe { DoorSleep(2) };
        }
    }
    fn send_raw(
        &mut self,
        data: &[u8],
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Failure> {
        let mut done = 0;
        while done < data.len() {
            let mut sent = 0u32;
            match unsafe {
                DoorNetSend(
                    self.socket,
                    data[done..].as_ptr() as u64,
                    (data.len() - done) as u32,
                    &mut sent as *mut u32 as u64,
                )
            } {
                0 => done += sent as usize,
                WSAEWOULDBLOCK => {
                    if cancelled() {
                        return Err(Failure::Cancelled);
                    }
                    if deadline.passed() {
                        return Err(Failure::Timeout);
                    }
                    unsafe { DoorSleep(1) };
                }
                _ => return Err(Failure::ConnectionError),
            }
        }
        Ok(())
    }
    /// Raw bytes from the socket; empty when the peer closed.
    fn receive_raw(
        &mut self,
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, Failure> {
        let mut buffer = vec![0u8; 4096];
        loop {
            let mut received = 0u32;
            match unsafe {
                DoorNetReceive(
                    self.socket,
                    buffer.as_mut_ptr() as u64,
                    buffer.len() as u32,
                    &mut received as *mut u32 as u64,
                )
            } {
                0 if received > 0 => {
                    buffer.truncate(received as usize);
                    return Ok(buffer);
                }
                0 => {
                    self.closed = true;
                    return Ok(Vec::new());
                }
                WSAEWOULDBLOCK => (),
                _ => return Err(Failure::ConnectionError),
            }
            if net_state(self.socket)
                .is_none_or(|s| s != state::ESTABLISHED && s != state::PEER_CLOSED)
            {
                return Err(Failure::ConnectionError);
            }
            if cancelled() {
                return Err(Failure::Cancelled);
            }
            if deadline.passed() {
                return Err(Failure::Timeout);
            }
            unsafe { DoorSleep(1) };
        }
    }
    /// The TLS 1.3 handshake for `host`; certificate problems not in
    /// `tolerance` fail with `Failure::Secure(flags)`.
    pub fn start_tls(
        &mut self,
        host: &str,
        tolerance: Tolerance,
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Failure> {
        let mut seed = [0u8; 96];
        if !random(&mut seed) {
            return Err(Failure::Secure(secure::SECURITY_CHANNEL_ERROR));
        }
        let (client, hello) =
            door_tls::Client::with_policy(host, now_unix(), seed, tolerance.policy());
        self.tls = Some(client);
        self.send_raw(&hello, deadline, cancelled)?;
        while !self.tls.as_ref().is_some_and(door_tls::Client::connected) {
            let bytes = self.receive_raw(deadline, cancelled)?;
            if bytes.is_empty() {
                return Err(Failure::Secure(secure::SECURITY_CHANNEL_ERROR));
            }
            let tls = self.tls.as_mut().expect("tls");
            let reply = tls.receive(&bytes);
            self.certificate_flags = failure_flags(tls.problems());
            match reply {
                Ok(out) => self.send_raw(&out, deadline, cancelled)?,
                Err(door_tls::Error::Certificate(_)) => {
                    // The reasons it was refused, as Windows reports them:
                    // problems the caller chose to ignore are left out.
                    let refused = crate::refused_flags(tls.problems(), tolerance.policy());
                    return Err(Failure::Secure(refused.max(secure::INVALID_CERT)));
                }
                Err(_) => return Err(Failure::Secure(secure::SECURITY_CHANNEL_ERROR)),
            }
        }
        Ok(())
    }
    pub fn secure(&self) -> bool {
        self.tls.is_some()
    }
    pub fn write_all(
        &mut self,
        data: &[u8],
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), Failure> {
        match self.tls.as_mut() {
            Some(tls) => {
                let records = tls.send(data).map_err(|_| Failure::ConnectionError)?;
                self.outbox.extend_from_slice(&records);
                let pending = core::mem::take(&mut self.outbox);
                self.send_raw(&pending, deadline, cancelled)
            }
            None => self.send_raw(data, deadline, cancelled),
        }
    }
    /// Decrypted application bytes; empty when the peer closed.
    pub fn read(
        &mut self,
        deadline: Deadline,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, Failure> {
        loop {
            if let Some(tls) = self.tls.as_mut() {
                let plain = tls.take_plaintext();
                if !plain.is_empty() {
                    return Ok(plain);
                }
                if tls.closed() {
                    return Ok(Vec::new());
                }
            }
            if self.closed {
                return Ok(Vec::new());
            }
            let bytes = self.receive_raw(deadline, cancelled)?;
            match self.tls.as_mut() {
                None => return Ok(bytes),
                Some(_) if bytes.is_empty() => return Ok(Vec::new()),
                Some(tls) => {
                    let out = tls.receive(&bytes).map_err(|_| Failure::ConnectionError)?;
                    if !out.is_empty() {
                        self.send_raw(&out, deadline, cancelled)?;
                    }
                }
            }
        }
    }
    /// Sends close_notify (TLS) and shuts the sending side.
    pub fn shutdown(&mut self) {
        if let Some(tls) = self.tls.as_mut() {
            let alert = tls.close();
            let _ = self.send_raw(&alert, Deadline::after(1000), &|| false);
        }
        unsafe { DoorNetShutdown(self.socket) };
    }
    /// The peer has not closed and the socket is still connected (for
    /// reusing a kept-alive connection).
    pub fn alive(&self) -> bool {
        !self.closed && net_state(self.socket) == Some(state::ESTABLISHED)
    }
}

/// A CONNECT tunnel through an HTTP proxy to `host:port`; the proxy's
/// status code when it refuses.
pub fn tunnel(
    stream: &mut Stream,
    host: &str,
    port: u16,
    authorization: Option<&str>,
    deadline: Deadline,
    cancelled: &dyn Fn() -> bool,
) -> Result<u16, Failure> {
    let mut head = alloc::format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if let Some(a) = authorization {
        head.push_str(&alloc::format!("Proxy-Authorization: {a}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes(), deadline, cancelled)?;
    let mut parser = door_http::Parser::new(true);
    while parser.response().is_none() {
        let bytes = stream.read(deadline, cancelled)?;
        if bytes.is_empty() {
            return Err(Failure::ConnectionError);
        }
        parser.feed(&bytes).map_err(|_| Failure::InvalidResponse)?;
    }
    Ok(parser.response().map_or(0, |r| r.status))
}
