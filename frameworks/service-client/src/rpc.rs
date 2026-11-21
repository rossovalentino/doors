use door_abi::{Handle, Status, Transfer};
use door_service_protocol::{BODY_BYTES, Code, Frame, Message};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Transport(Status),
    Service(Code),
    Malformed,
}
impl From<Status> for Error {
    fn from(status: Status) -> Self {
        Self::Transport(status)
    }
}
pub type Result<T> = core::result::Result<T, Error>;
pub(crate) struct Connection {
    channel: Handle,
    instance: u64,
    next: u64,
    poisoned: bool,
    pending: Option<(u32, u64, u64)>,
}
impl Connection {
    pub fn needs_reconnect(&self) -> bool {
        self.poisoned
    }
    /// Retire before reconnecting so the broker can reclaim its bounded slot.
    /// Taking the handle first makes explicit retirement and Drop idempotent.
    pub fn retire(&mut self) {
        self.retire_with(|channel| {
            let _ = door_sys::close(channel);
        });
    }
    fn retire_with(&mut self, close: impl FnOnce(Handle)) {
        self.poisoned = true;
        self.pending = None;
        let channel = core::mem::replace(&mut self.channel, 0);
        if channel != 0 {
            close(channel);
        }
    }
    fn sent(
        &mut self,
        result: core::result::Result<(), Status>,
        retry_would_block: bool,
    ) -> Result<()> {
        if let Err(error) = result {
            if !(retry_would_block && error == Status::WouldBlock) {
                self.retire();
            }
            return Err(error.into());
        }
        Ok(())
    }
    pub fn key(&self) -> (Handle, u64) {
        (self.channel, self.instance)
    }
    pub fn connect(port: Handle, manifest: u64) -> Result<Self> {
        let channel = match door_sys::port_connect(port) {
            // The kernel queues four unaccepted connections per port; a busy
            // moment (session start, when every service and app connects to
            // Init) waits for a slot within a call's deadline instead of
            // failing. Still full at the deadline: WouldBlock, as before.
            Err(Status::WouldBlock) => {
                let deadline = door_sys::time()?
                    .checked_add(500)
                    .ok_or(Error::Service(Code::Resources))?;
                receive_reply(
                    deadline,
                    || match door_sys::port_connect(port) {
                        Ok(channel) => Ok(Some(channel)),
                        Err(Status::WouldBlock) => Ok(None),
                        Err(error) => Err(error.into()),
                    },
                    || Ok(door_sys::time()?),
                    || Ok(door_sys::yield_now()?),
                    |wake| Ok(door_sys::sleep(wake)?),
                )
                .map_err(|error| {
                    if error == Error::Transport(Status::Timeout) {
                        Error::Transport(Status::WouldBlock)
                    } else {
                        error
                    }
                })?
            }
            result => result?,
        };
        Self::authenticate(channel, manifest)
    }
    /// `connect` without waiting: a port whose accept queue is full answers
    /// WouldBlock at once. For callers that must never stall (the display
    /// service's background lookups through Init, which Init may itself be
    /// calling at that moment) and retry later instead.
    pub fn connect_now(port: Handle, manifest: u64) -> Result<Self> {
        let channel = door_sys::port_connect(port)?;
        Self::authenticate(channel, manifest)
    }
    fn authenticate(channel: Handle, manifest: u64) -> Result<Self> {
        let identity = match door_sys::identity(channel) {
            Ok(identity) if identity.manifest == manifest && identity.instance != 0 => identity,
            Ok(_) => {
                let _ = door_sys::close(channel);
                return Err(Error::Malformed);
            }
            Err(error) => {
                let _ = door_sys::close(channel);
                return Err(error.into());
            }
        };
        Ok(Self {
            channel,
            instance: identity.instance,
            next: 1,
            poisoned: false,
            pending: None,
        })
    }
    /// Send once without sleeping. The caller services input while awaiting the
    /// authenticated reply; only one request may be outstanding per channel.
    pub fn begin(&mut self, opcode: u32, body: &[u8], now: u64) -> Result<()> {
        self.begin_transfer(opcode, body, &[], now)
    }
    /// `begin` with handles transferred alongside the request.
    pub fn begin_transfer(
        &mut self,
        opcode: u32,
        body: &[u8],
        transfers: &[Transfer],
        now: u64,
    ) -> Result<()> {
        if self.poisoned {
            return Err(Error::Service(Code::Closing));
        }
        if self.pending.is_some() {
            return Err(Status::WouldBlock.into());
        }
        let request = self.next;
        let next = request
            .checked_add(1)
            .ok_or(Error::Service(Code::Resources))?;
        let deadline = now
            .checked_add(500)
            .ok_or(Error::Service(Code::Resources))?;
        let packet = Message::new(
            opcode,
            request,
            self.instance,
            Code::Success,
            body,
            transfers.len(),
        )?;
        // WouldBlock did not publish a request; a later begin is safe.
        self.sent(
            door_sys::channel_send(self.channel, packet.bytes(), transfers),
            true,
        )?;
        self.next = next;
        self.pending = Some((opcode, request, deadline));
        Ok(())
    }
    pub fn poll(&mut self, now: u64) -> Result<Option<Reply>> {
        let (opcode, request, deadline) = self.pending.ok_or(Error::Malformed)?;
        let response = match self.try_receive(opcode, request) {
            Ok(None) if now >= deadline => Err(Status::Timeout.into()),
            response => response,
        };
        if !matches!(response, Ok(None)) {
            self.pending = None;
        }
        if matches!(response, Err(Error::Transport(_)) | Err(Error::Malformed)) {
            self.retire();
        }
        response
    }
    pub fn call(&mut self, opcode: u32, body: &[u8], transfers: &[Transfer]) -> Result<Reply> {
        if self.poisoned {
            return Err(Error::Service(Code::Closing));
        }
        if self.pending.is_some() {
            return Err(Status::WouldBlock.into());
        }
        let request = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or(Error::Service(Code::Resources))?;
        let packet = Message::new(
            opcode,
            request,
            self.instance,
            Code::Success,
            body,
            transfers.len(),
        )?;
        let deadline = door_sys::time()?
            .checked_add(500)
            .ok_or(Error::Service(Code::Resources))?;
        self.sent(
            door_sys::channel_send_deadline(self.channel, packet.bytes(), transfers, deadline),
            false,
        )?;
        let response = self.receive(opcode, request, deadline);
        if matches!(response, Err(Error::Transport(_)) | Err(Error::Malformed)) {
            self.retire();
        }
        response
    }
    fn receive(&self, opcode: u32, request: u64, deadline: u64) -> Result<Reply> {
        receive_reply(
            deadline,
            || self.try_receive(opcode, request),
            || Ok(door_sys::time()?),
            || Ok(door_sys::yield_now()?),
            |wake| Ok(door_sys::sleep(wake)?),
        )
    }
    fn try_receive(&self, opcode: u32, request: u64) -> Result<Option<Reply>> {
        let mut bytes = [0; door_abi::MAX_PACKET];
        let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
        match door_sys::channel_receive(self.channel, &mut bytes, &mut transfers) {
            Ok(metadata) => Reply::decode(
                &bytes[..metadata.packet_bytes as usize],
                transfers,
                metadata.handle_count as usize,
                opcode,
                request,
                self.instance,
            )
            .map(Some),
            Err(Status::WouldBlock) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}
/// Give a runnable server bounded cooperative turns before timer backoff. These
/// callbacks inline to native syscalls; tests exercise the same loop without a
/// host clock/IPC fallback. The request's original absolute deadline never moves.
fn receive_reply<T>(
    deadline: u64,
    mut receive: impl FnMut() -> Result<Option<T>>,
    mut time: impl FnMut() -> Result<u64>,
    mut yield_now: impl FnMut() -> Result<()>,
    mut sleep: impl FnMut(u64) -> Result<()>,
) -> Result<T> {
    let mut cooperative_turns = 8;
    loop {
        if let Some(reply) = receive()? {
            return Ok(reply);
        }
        let now = time()?;
        if now >= deadline {
            return Err(Status::Timeout.into());
        }
        if cooperative_turns != 0 {
            cooperative_turns -= 1;
            yield_now()?;
        } else {
            // now < deadline guarantees that this addition cannot overflow.
            sleep(now + 1)?;
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.retire();
    }
}
pub(crate) struct Reply {
    body: [u8; BODY_BYTES],
    len: usize,
    transfers: [Transfer; door_abi::MAX_TRANSFERS],
    handles: usize,
}
impl Reply {
    fn decode(
        bytes: &[u8],
        transfers: [Transfer; door_abi::MAX_TRANSFERS],
        handles: usize,
        opcode: u32,
        request: u64,
        instance: u64,
    ) -> Result<Self> {
        let mut reply = Self {
            body: [0; BODY_BYTES],
            len: 0,
            transfers,
            handles,
        };
        let frame = Frame::decode(bytes).map_err(|_| Error::Malformed)?;
        if frame.header.opcode != opcode
            || frame.header.request_id != request
            || frame.instance != instance
            || frame.header.handle_count as usize != handles
        {
            return Err(Error::Malformed);
        }
        if frame.code != Code::Success {
            if !frame.body.is_empty() || handles != 0 {
                return Err(Error::Malformed);
            }
            return Err(Error::Service(frame.code));
        }
        reply.body[..frame.body.len()].copy_from_slice(frame.body);
        reply.len = frame.body.len();
        Ok(reply)
    }
    pub fn data(&self) -> Result<&[u8]> {
        if self.handles != 0 {
            return Err(Error::Malformed);
        }
        Ok(&self.body[..self.len])
    }
    pub fn empty(self) -> Result<()> {
        if self.data()?.is_empty() {
            Ok(())
        } else {
            Err(Error::Malformed)
        }
    }
    pub fn scalar(self) -> Result<u64> {
        let data = self.data()?;
        if data.len() != 8 {
            return Err(Error::Malformed);
        }
        Ok(u64::from_le_bytes(data.try_into().unwrap()))
    }
    pub fn token(self) -> Result<u64> {
        let token = self.scalar()?;
        if token == 0 {
            Err(Error::Malformed)
        } else {
            Ok(token)
        }
    }
    /// Handles the reply carries.
    pub fn handle_count(&self) -> usize {
        self.handles
    }
    pub fn one_handle(mut self) -> Result<Handle> {
        if self.len != 0 || self.handles != 1 {
            return Err(Error::Malformed);
        }
        self.handles = 0;
        Ok(self.transfers[0].handle)
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        for transfer in &self.transfers[..self.handles] {
            let _ = door_sys::close(transfer.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    fn connection() -> Connection {
        Connection {
            channel: 37,
            instance: 9,
            next: 1,
            poisoned: false,
            pending: None,
        }
    }
    #[test]
    fn retirement_takes_authority_and_pending_request_before_closing_once() {
        let mut c = connection();
        c.pending = Some((2, 3, 500));
        let closes = Cell::new(0);
        c.retire_with(|h| {
            assert_eq!(h, 37);
            closes.set(closes.get() + 1);
        });
        assert!(c.needs_reconnect());
        assert_eq!(c.key(), (0, 9));
        assert!(c.pending.is_none());
        c.retire_with(|_| panic!("retirement closed twice"));
        assert_eq!(closes.get(), 1);
        assert_eq!(c.begin(1, &[], 0), Err(Error::Service(Code::Closing)));
        assert!(matches!(
            c.call(1, &[], &[]),
            Err(Error::Service(Code::Closing))
        ));
    }
    #[test]
    fn fatal_send_retires_but_nonblocking_unpublished_send_can_retry() {
        for error in [
            Status::PeerClosed,
            Status::Timeout,
            Status::AccessDenied,
            Status::BadHandle,
        ] {
            let mut c = connection();
            assert_eq!(c.sent(Err(error), true), Err(Error::Transport(error)));
            assert!(c.needs_reconnect());
            assert_eq!(c.key().0, 0);
        }
        let mut c = connection();
        assert_eq!(
            c.sent(Err(Status::WouldBlock), true),
            Err(Error::Transport(Status::WouldBlock))
        );
        assert!(!c.needs_reconnect());
        assert!(c.pending.is_none());
        assert_eq!(
            c.sent(Err(Status::WouldBlock), false),
            Err(Error::Transport(Status::WouldBlock))
        );
        assert!(c.needs_reconnect());
    }
    #[test]
    fn malformed_success_bodies_retire_but_local_preconditions_preserve_channel() {
        for decoder in 0..4 {
            let mut storage = crate::Storage {
                connection: connection(),
            };
            let result: Result<()> = storage.operation(|_| {
                let reply = Reply {
                    body: [0; BODY_BYTES],
                    len: 1,
                    transfers: [Transfer::default(); door_abi::MAX_TRANSFERS],
                    handles: 0,
                };
                match decoder {
                    0 => reply.empty(),
                    1 => reply.scalar().map(|_| ()),
                    2 => door_service_protocol::files::FileInfo::decode(reply.data()?)
                        .map(|_| ())
                        .map_err(|_| Error::Malformed),
                    _ => reply.one_handle().map(|_| ()),
                }
            });
            assert_eq!(result, Err(Error::Malformed));
            assert!(storage.needs_reconnect());
            assert_eq!(storage.connection.key().0, 0);
        }
        let mut storage = crate::Storage {
            connection: connection(),
        };
        assert!(matches!(
            storage.open(
                "",
                crate::Disposition::OpenExisting,
                crate::READ,
                crate::SHARE_ALL
            ),
            Err(Error::Service(Code::InvalidRequest))
        ));
        assert!(!storage.needs_reconnect());
        assert_eq!(storage.connection.key().0, 37);
        let _: Result<()> = storage.operation(|_| Err(Error::Service(Code::Denied)));
        assert!(!storage.needs_reconnect());
    }
    #[test]
    fn hosted_clients_cannot_claim_service_execution() {
        assert!(matches!(
            Connection::connect(1, 1),
            Err(Error::Transport(Status::Unsupported))
        ));
    }
    #[test]
    fn error_envelopes_cannot_masquerade_as_definite_service_rejections() {
        for code in [Code::Denied, Code::Changed, Code::Io] {
            for (body, handles) in [(&[][..], 0), (&[1][..], 0), (&[][..], 1)] {
                let packet = Message::new(7, 1, 9, code, body, handles).unwrap();
                let reply = Reply::decode(
                    packet.bytes(),
                    [Transfer::default(); door_abi::MAX_TRANSFERS],
                    handles,
                    7,
                    1,
                    9,
                );
                assert!(
                    matches!(reply, Err(error) if error == if body.is_empty() && handles == 0 { Error::Service(code) } else { Error::Malformed })
                );
            }
        }
    }
    #[test]
    fn zero_remote_tokens_and_invalid_metadata_retire_the_storage_connection() {
        for object in [0, 1] {
            for version in [0, 1] {
                for directory in [false, true] {
                    let mut storage = crate::Storage {
                        connection: connection(),
                    };
                    let info = door_service_protocol::files::FileInfo {
                        attributes: 0,
                        bytes: 1,
                        object,
                        version,
                        directory,
                    };
                    let result = storage.operation(|_| {
                        door_service_protocol::files::FileInfo::decode(&info.encode())
                            .map_err(|_| Error::Malformed)
                    });
                    let valid = object != 0 && version != 0 && !directory;
                    assert_eq!(result.is_ok(), valid);
                    assert_eq!(storage.needs_reconnect(), !valid);
                }
            }
        }
        let packet = Message::new(7, 1, 9, Code::Success, &0u64.to_le_bytes(), 0).unwrap();
        let mut storage = crate::Storage {
            connection: connection(),
        };
        let result = storage.operation(|_| {
            Reply::decode(
                packet.bytes(),
                [Transfer::default(); door_abi::MAX_TRANSFERS],
                0,
                7,
                1,
                9,
            )?
            .token()
        });
        assert_eq!(result, Err(Error::Malformed));
        assert!(storage.needs_reconnect());
    }
    #[test]
    fn ready_reply_needs_no_clock_or_scheduler_call() {
        assert_eq!(
            receive_reply(0, || Ok(Some(42)), || panic!(), || panic!(), |_| panic!()),
            Ok(42)
        );
    }
    #[test]
    fn runnable_server_can_reply_without_waiting_for_a_timer_tick() {
        let turns = Cell::new(0);
        assert_eq!(
            receive_reply(
                500,
                || Ok((turns.get() == 3).then_some(42)),
                || Ok(0),
                || {
                    turns.set(turns.get() + 1);
                    Ok(())
                },
                |_| panic!("short reply slept"),
            ),
            Ok(42)
        );
        assert_eq!(turns.get(), 3);
    }
    #[test]
    fn stalled_server_uses_bounded_yields_then_sleeps_until_original_deadline() {
        let turns = Cell::new(0);
        let clock = Cell::new(0);
        let sleeps = Cell::new(0);
        assert_eq!(
            receive_reply::<()>(
                500,
                || Ok(None),
                || Ok(clock.get()),
                || {
                    turns.set(turns.get() + 1);
                    Ok(())
                },
                |wake| {
                    assert_eq!(turns.get(), 8);
                    assert_eq!(wake, clock.get() + 1);
                    clock.set(wake);
                    sleeps.set(sleeps.get() + 1);
                    Ok(())
                },
            ),
            Err(Error::Transport(Status::Timeout))
        );
        assert_eq!((turns.get(), sleeps.get(), clock.get()), (8, 500, 500));
    }
    #[test]
    fn time_advancing_during_yields_cannot_extend_deadline_or_overflow() {
        let clock = Cell::new(u64::MAX - 1);
        assert_eq!(
            receive_reply::<()>(
                u64::MAX,
                || Ok(None),
                || Ok(clock.get()),
                || {
                    clock.set(u64::MAX);
                    Ok(())
                },
                |_| panic!("expired deadline slept"),
            ),
            Err(Error::Transport(Status::Timeout))
        );
    }
    #[test]
    fn reply_and_scheduler_errors_propagate_without_retry() {
        for failure in 0..4 {
            let error = Error::Transport(Status::PeerClosed);
            let turns = Cell::new(0);
            let observed = receive_reply::<()>(
                500,
                || {
                    if failure == 0 {
                        Err(Error::Malformed)
                    } else {
                        Ok(None)
                    }
                },
                || if failure == 1 { Err(error) } else { Ok(0) },
                || {
                    turns.set(turns.get() + 1);
                    if failure == 2 { Err(error) } else { Ok(()) }
                },
                |_| {
                    assert_eq!(turns.get(), 8);
                    Err(error)
                },
            );
            assert_eq!(
                observed,
                Err(if failure == 0 {
                    Error::Malformed
                } else {
                    error
                })
            );
        }
    }
}
