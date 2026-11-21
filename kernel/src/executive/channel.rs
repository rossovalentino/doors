use super::*;
use door_abi::{
    CANCEL_HISTORY_DEPTH, CancelOutcome, Header, MAX_PACKET, MAX_TRANSFERS, Metadata, QUEUE_DEPTH,
    Transfer,
};

#[derive(Clone, Copy)]
struct QueuedGrant {
    object: ObjectId,
    rights: u64,
}
#[derive(Clone, Copy)]
pub struct Message {
    pub packet: [u8; MAX_PACKET],
    pub length: usize,
    pub transfers: [Transfer; MAX_TRANSFERS],
    pub metadata: Metadata,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiveInfo {
    pub packet_bytes: usize,
    pub transfers: usize,
}
#[derive(Clone, Copy)]
struct Queued {
    packet: [u8; MAX_PACKET],
    length: usize,
    grants: [Option<QueuedGrant>; MAX_TRANSFERS],
    metadata: Metadata,
    deadline: Option<u64>,
}

// Terminal transport history is bounded independently of queued messages.
// Eviction forgets the outcome, never the committed request-ID high-water mark.
#[derive(Clone, Copy)]
enum Terminal {
    Dispatched,
    Canceled,
    Expired,
}
impl Terminal {
    fn cancel_result(self) -> u64 {
        match self {
            Self::Dispatched => CancelOutcome::Dispatched as u64,
            Self::Canceled => CancelOutcome::AlreadyCanceled as u64,
            Self::Expired => CancelOutcome::Expired as u64,
        }
    }
}
#[derive(Clone, Copy)]
struct Completion {
    request: u64,
    terminal: Terminal,
}
#[derive(Clone, Copy)]
pub(super) struct Channel {
    owners: [ProcessId; 2],
    identities: [Authority; 2],
    open: [bool; 2],
    last_request: [u64; 2],
    queue: [[Option<Queued>; QUEUE_DEPTH]; 2],
    history: [[Option<Completion>; CANCEL_HISTORY_DEPTH]; 2],
}
impl Channel {
    fn complete(&mut self, sender: usize, request: u64, terminal: Terminal) {
        let history = &mut self.history[sender];
        let slot = if let Some(slot) = history.iter().position(Option::is_none) {
            slot
        } else {
            history.rotate_left(1);
            CANCEL_HISTORY_DEPTH - 1
        };
        history[slot] = Some(Completion { request, terminal });
    }
    fn remove(&mut self, receiver: usize, index: usize, terminal: Terminal) -> Queued {
        let queue = &mut self.queue[receiver];
        let message = queue[index].take().expect("live queued request");
        queue[index..].rotate_left(1);
        self.complete(1 - receiver, message.metadata.request_id, terminal);
        message
    }
}
impl Executive {
    pub fn channel_pair(&mut self, a: ProcessId, b: ProcessId) -> Result<[u64; 2]> {
        // Two global incarnations are consumed even when the destination grant
        // tables differ. Per-table checks alone each reserve only one in that
        // case, allowing the second install to wrap after partial publication.
        self.next_handle
            .checked_add(2 * HANDLE_STRIDE)
            .ok_or(Status::ResourceExhausted)?;
        self.handle_capacity(a, if a == b { 2 } else { 1 })?;
        self.handle_capacity(b, 1)?;
        self.object_capacity(2)?;
        let pair = self
            .channels
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        self.channels[pair] = Some(Channel {
            owners: [a, b],
            identities: [self.authority(a)?, self.authority(b)?],
            open: [true; 2],
            last_request: [0; 2],
            queue: [[None; QUEUE_DEPTH]; 2],
            history: [[None; CANCEL_HISTORY_DEPTH]; 2],
        });
        let oa = self.allocate_object(Kind::Channel {
            pair,
            side: 0,
            owner: a,
        });
        let ob = self.allocate_object(Kind::Channel {
            pair,
            side: 1,
            owner: b,
        });
        let rights = rights::QUERY
            | rights::DUPLICATE
            | rights::TRANSFER
            | rights::READ
            | rights::WRITE
            | rights::WAIT;
        Ok([self.install(a, oa, rights), self.install(b, ob, rights)])
    }
    fn endpoint(&self, pid: ProcessId, handle: u64, rights: u64) -> Result<(usize, usize)> {
        let g = self.grant(pid, handle, rights)?;
        if let Kind::Channel { pair, side, owner } = self.object(g.object).kind {
            if owner != pid {
                return Err(Status::AccessDenied);
            }
            Ok((pair, side))
        } else {
            Err(Status::InvalidArgument)
        }
    }
    pub fn channel_peer(&self, pid: ProcessId, handle: u64) -> Result<u64> {
        let (pair, side) = self.endpoint(pid, handle, rights::QUERY)?;
        Ok(self.channels[pair].as_ref().unwrap().owners[1 - side].raw())
    }
    pub fn channel_identity(&self, pid: ProcessId, handle: u64) -> Result<(u64, Authority)> {
        self.authority(pid)?;
        let (pair, side) = self.endpoint(pid, handle, rights::QUERY)?;
        let channel = self.channels[pair].as_ref().unwrap();
        let authority = channel.identities[1 - side];
        self.validate_authority(authority)?;
        Ok((channel.owners[1 - side].raw(), authority))
    }
    pub fn send(
        &mut self,
        pid: ProcessId,
        handle: u64,
        packet: &[u8],
        transfers: &[Transfer],
    ) -> Result<()> {
        self.send_at(pid, handle, packet, transfers, 0, None)
    }
    /// Deadline is an absolute trusted monotonic tick. Ownership transfers only
    /// at enqueue commit. An already elapsed deadline does not consume the ID.
    pub fn send_at(
        &mut self,
        pid: ProcessId,
        handle: u64,
        packet: &[u8],
        transfers: &[Transfer],
        now: u64,
        deadline: Option<u64>,
    ) -> Result<()> {
        let (header, _) = Header::packet(packet)?;
        if transfers.len() != header.handle_count as usize {
            return Err(Status::InvalidArgument);
        }
        let (pair, side) = self.endpoint(pid, handle, rights::WRITE)?;
        if deadline.is_some_and(|deadline| deadline <= now) {
            return Err(Status::Timeout);
        }
        self.expire(pair, now);
        let channel = self.channels[pair].as_ref().unwrap();
        if !channel.open[1 - side] {
            return Err(Status::PeerClosed);
        }
        if header.request_id <= channel.last_request[side] {
            return Err(Status::InvalidArgument);
        }
        let qslot = channel.queue[1 - side]
            .iter()
            .position(Option::is_none)
            .ok_or(Status::WouldBlock)?;
        let mut grants = [None; MAX_TRANSFERS];
        for (i, t) in transfers.iter().enumerate() {
            if t.mode > 1 || transfers[..i].iter().any(|x| x.handle == t.handle) {
                return Err(Status::InvalidArgument);
            }
            let g = self.grant(pid, t.handle, rights::TRANSFER)?;
            if t.rights & !g.rights != 0 {
                return Err(Status::AccessDenied);
            }
            // Endpoints are bound to their original process. Queuing an endpoint
            // into its own channel would also create an uncollectable ref cycle.
            if matches!(
                self.object(g.object).kind,
                Kind::Channel { .. } | Kind::Platform(platform::Resource::Authority { .. })
            ) {
                return Err(Status::AccessDenied);
            }
            grants[i] = Some(QueuedGrant {
                object: g.object,
                rights: t.rights,
            });
        }
        let mut q = Queued {
            packet: [0; MAX_PACKET],
            length: packet.len(),
            grants,
            metadata: Metadata {
                sender_pid: pid.raw(),
                packet_bytes: packet.len() as u64,
                handle_count: transfers.len() as u64,
                request_id: header.request_id,
            },
            deadline,
        };
        q.packet[..packet.len()].copy_from_slice(packet);
        for grant in grants.iter().flatten() {
            self.retain(grant.object);
        }
        let channel = self.channels[pair].as_mut().unwrap();
        channel.queue[1 - side][qslot] = Some(q);
        channel.last_request[side] = header.request_id;
        for transfer in transfers {
            if transfer.mode == 1 {
                self.close_owned(pid, transfer.handle)
                    .expect("preflight move");
            }
        }
        Ok(())
    }
    pub fn receive_info(&self, pid: ProcessId, handle: u64) -> Result<ReceiveInfo> {
        let (pair, side) = self.endpoint(pid, handle, rights::READ)?;
        let c = self.channels[pair].as_ref().unwrap();
        if let Some(q) = c.queue[side][0] {
            Ok(ReceiveInfo {
                packet_bytes: q.length,
                transfers: q.metadata.handle_count as usize,
            })
        } else if c.open[1 - side] {
            Err(Status::WouldBlock)
        } else {
            Err(Status::PeerClosed)
        }
    }
    pub fn receive_info_at(
        &mut self,
        pid: ProcessId,
        handle: u64,
        now: u64,
    ) -> Result<ReceiveInfo> {
        let (pair, _) = self.endpoint(pid, handle, rights::READ)?;
        self.expire(pair, now);
        self.receive_info(pid, handle)
    }
    pub fn receive(
        &mut self,
        pid: ProcessId,
        handle: u64,
        packet_capacity: usize,
        transfer_capacity: usize,
    ) -> Result<Message> {
        self.receive_at(pid, handle, packet_capacity, transfer_capacity, 0)
    }
    /// Dispatch is the transport commit point: subsequent cancellation reports
    /// Dispatched and cannot undo receiver-visible grants or service effects.
    pub fn receive_at(
        &mut self,
        pid: ProcessId,
        handle: u64,
        packet_capacity: usize,
        transfer_capacity: usize,
        now: u64,
    ) -> Result<Message> {
        let info = self.receive_info_at(pid, handle, now)?;
        if packet_capacity < info.packet_bytes || transfer_capacity < info.transfers {
            return Err(Status::InvalidArgument);
        }
        self.handle_capacity(pid, info.transfers)?;
        let (pair, side) = self.endpoint(pid, handle, rights::READ)?;
        let c = self.channels[pair].as_mut().unwrap();
        let q = c.remove(side, 0, Terminal::Dispatched);
        let mut result = Message {
            packet: q.packet,
            length: q.length,
            transfers: [Transfer::default(); MAX_TRANSFERS],
            metadata: q.metadata,
        };
        for (i, g) in q.grants.iter().flatten().enumerate() {
            let handle = self.install(pid, g.object, g.rights);
            self.drop_ref(g.object);
            result.transfers[i] = Transfer {
                handle,
                rights: g.rights,
                mode: 0,
            };
        }
        Ok(result)
    }
    ///0 newly canceled,1 dispatched,2 already canceled,3 expired. Unknown or
    /// evicted IDs return NotFound; request zero is never a valid identity.
    pub fn cancel(&mut self, pid: ProcessId, handle: u64, request: u64, now: u64) -> Result<u64> {
        if request == 0 {
            return Err(Status::InvalidArgument);
        }
        let (pair, side) = self.endpoint(pid, handle, rights::WRITE)?;
        self.expire(pair, now);
        let channel = self.channels[pair].as_mut().unwrap();
        if let Some(index) = channel.queue[1 - side].iter().position(|message| {
            message.is_some_and(|message| message.metadata.request_id == request)
        }) {
            let removed = channel.remove(1 - side, index, Terminal::Canceled);
            self.release_message(removed);
            return Ok(CancelOutcome::Canceled as u64);
        }
        channel.history[side]
            .iter()
            .flatten()
            .find(|completion| completion.request == request)
            .map(|completion| completion.terminal.cancel_result())
            .ok_or(Status::NotFound)
    }
    fn release_message(&mut self, message: Queued) {
        for grant in message.grants.iter().flatten() {
            self.drop_ref(grant.object);
        }
    }
    fn expire(&mut self, pair: usize, now: u64) {
        for receiver in 0..2 {
            let mut index = 0;
            while index < QUEUE_DEPTH {
                let channel = self.channels[pair].as_mut().unwrap();
                if channel.queue[receiver][index]
                    .is_some_and(|message| message.deadline.is_some_and(|deadline| deadline <= now))
                {
                    let removed = channel.remove(receiver, index, Terminal::Expired);
                    self.release_message(removed);
                    // Compaction moved another request into this same slot.
                } else {
                    index += 1;
                }
            }
        }
    }
    pub(super) fn close_endpoint(&mut self, pair: usize, side: usize) {
        let c = self.channels[pair].as_mut().expect("live pair");
        c.open[side] = false;
        let queued = core::mem::replace(&mut c.queue[side], [None; QUEUE_DEPTH]);
        for message in queued.iter().flatten() {
            c.complete(1 - side, message.metadata.request_id, Terminal::Canceled);
        }
        for q in queued.iter().flatten() {
            for g in q.grants.iter().flatten() {
                self.drop_ref(g.object);
            }
        }
        if self.channels[pair]
            .as_ref()
            .is_some_and(|c| !c.open[0] && !c.open[1])
        {
            self.channels[pair] = None;
        }
    }
}
