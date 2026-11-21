//! Private, allocation-free candidate bytes. No append/abort touches the volume.
//! Staged whole-file replacements (STAGE_*) keep only the volume's owned
//! `Staged` capability here; their bytes live in a pending catalog node whose
//! blocks the volume reserves until they are written, committed or aborted.
//!
//! Both kinds of slot are bounded per client process (kernel-authenticated
//! pid/session/epoch, across all its channels) so no client can hold every
//! slot, and a candidate untouched for `IDLE_TICKS` is aborted on the next
//! request. Disconnect (including a revoked session, which the server now
//! disconnects) aborts a channel's candidates at once.
use super::*;
use door_service_protocol::files::{BeginReplace, REPLACE_BYTES, STAGE_CHUNK_BYTES, StageSection};
use door_storage_core::persistence::Staged;
const SLOTS: usize = 4;
/// Small candidates one client process may hold.
const CLIENT_SLOTS: usize = 2;
/// Concurrent staged whole-file replacements service-wide (each holds one
/// catalog node and its reserved blocks until commit or abort).
const LARGE_SLOTS: usize = 4;
/// Staged replacements one client process may hold.
const CLIENT_LARGE_SLOTS: usize = 2;
/// Monotonic ticks (`door_sys::time`) after which an untouched candidate is
/// aborted: 30 s, far beyond the 5 s per-request client deadline.
pub(super) const IDLE_TICKS: u64 = match door_abi::platform::ticks_from_millis(30_000) {
    Some(ticks) => ticks,
    None => u64::MAX,
};
const _: () = assert!(CLIENT_SLOTS < SLOTS && CLIENT_LARGE_SLOTS < LARGE_SLOTS);
struct LargeSlot {
    owner: Owner,
    token: u64,
    touched: u64,
    staged: Option<Staged>,
}
impl LargeSlot {
    const EMPTY: Self = Self {
        owner: Owner::EMPTY,
        token: 0,
        touched: 0,
        staged: None,
    };
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Owner {
    channel: Handle,
    file: u64,
    pid: u64,
    session: u64,
    epoch: u64,
}
impl Owner {
    const EMPTY: Self = Self {
        channel: 0,
        file: 0,
        pid: 0,
        session: 0,
        epoch: 0,
    };
    fn new(channel: Handle, peer: Identity, file: u64) -> Self {
        Self {
            channel,
            file,
            pid: peer.pid,
            session: peer.session,
            epoch: peer.epoch,
        }
    }
    /// The client process, whatever channel it uses.
    fn same_client(&self, other: &Owner) -> bool {
        (self.pid, self.session, self.epoch) == (other.pid, other.session, other.epoch)
    }
}
fn idle(touched: u64, now: u64) -> bool {
    now.saturating_sub(touched) >= IDLE_TICKS
}
struct Candidate {
    owner: Owner,
    token: u64,
    touched: u64,
    request: Option<BeginReplace>,
    received: usize,
    bytes: [u8; REPLACE_BYTES],
}
impl Candidate {
    const EMPTY: Self = Self {
        owner: Owner::EMPTY,
        token: 0,
        touched: 0,
        request: None,
        received: 0,
        bytes: [0; REPLACE_BYTES],
    };
    fn clear(&mut self) {
        self.token = 0;
        self.touched = 0;
        self.request = None;
        self.received = 0;
        self.owner = Owner::EMPTY;
        self.bytes.fill(0);
    }
}
pub(super) struct Staging {
    slots: [Candidate; SLOTS],
    next: u64,
    large: [LargeSlot; LARGE_SLOTS],
}
impl Staging {
    pub(super) const fn new() -> Self {
        Self {
            slots: [const { Candidate::EMPTY }; SLOTS],
            next: 1,
            large: [const { LargeSlot::EMPTY }; LARGE_SLOTS],
        }
    }
    /// Preflight a staged replacement: one per owner/file, at most
    /// `CLIENT_LARGE_SLOTS` per client, bounded slots, a nonwrapping token.
    /// The volume reserves the candidate's blocks in `stage_replace`.
    fn large_reserve(&self, owner: Owner) -> Result<usize, Code> {
        let live = || self.large.iter().filter(|s| s.token != 0);
        if live().any(|s| s.owner == owner) {
            return Err(Code::WouldBlock);
        }
        if live().filter(|s| s.owner.same_client(&owner)).count() >= CLIENT_LARGE_SLOTS {
            return Err(Code::Resources);
        }
        self.next.checked_add(1).ok_or(Code::Resources)?;
        self.large
            .iter()
            .position(|s| s.token == 0)
            .ok_or(Code::Resources)
    }
    fn large_fill(&mut self, slot: usize, owner: Owner, staged: Staged, now: u64) -> u64 {
        let token = self.next;
        self.next += 1; // preflighted by large_reserve
        self.large[slot] = LargeSlot {
            owner,
            token,
            touched: now,
            staged: Some(staged),
        };
        token
    }
    /// The owner's live slot for `token`, marked as used at `now`.
    fn large_find(&mut self, owner: Owner, token: u64, now: u64) -> Result<usize, Code> {
        let slot = self
            .large
            .iter()
            .position(|s| token != 0 && s.token == token && s.owner == owner)
            .ok_or(Code::InvalidHandle)?;
        self.large[slot].touched = now;
        Ok(slot)
    }
    fn large_staged(&mut self, slot: usize) -> &mut Staged {
        self.large[slot].staged.as_mut().unwrap()
    }
    fn large_take(&mut self, slot: usize) -> Staged {
        let staged = self.large[slot].staged.take().unwrap();
        self.large[slot] = LargeSlot::EMPTY;
        staged
    }
    /// Remove and return slot `slot`'s candidate when `owned` matches it
    /// (file close, client exit or idle expiry); the caller aborts it on the
    /// volume.
    fn take_large_if(&mut self, slot: usize, owned: impl Fn(&LargeSlot) -> bool) -> Option<Staged> {
        let s = self.large.get(slot)?;
        if s.token == 0 || s.staged.is_none() || !owned(s) {
            return None;
        }
        Some(self.large_take(slot))
    }
    const LARGE_SLOTS: usize = LARGE_SLOTS;
    fn begin(&mut self, owner: Owner, request: BeginReplace, now: u64) -> Result<u64, Code> {
        if owner.file != request.file() {
            return Err(Code::InvalidRequest);
        }
        let live = || self.slots.iter().filter(|s| s.token != 0);
        if live().any(|s| s.owner == owner) {
            return Err(Code::WouldBlock);
        }
        if live().filter(|s| s.owner.same_client(&owner)).count() >= CLIENT_SLOTS {
            return Err(Code::Resources);
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|s| s.token == 0)
            .ok_or(Code::Resources)?;
        let token = self.next;
        let next = token.checked_add(1).ok_or(Code::Resources)?;
        slot.owner = owner;
        slot.request = Some(request);
        slot.token = token;
        slot.touched = now;
        slot.received = 0;
        self.next = next;
        Ok(token)
    }
    fn find(&self, owner: Owner, token: u64) -> Result<usize, Code> {
        self.slots
            .iter()
            .position(|s| token != 0 && s.token == token && s.owner == owner)
            .ok_or(Code::InvalidHandle)
    }
    fn append(
        &mut self,
        owner: Owner,
        token: u64,
        offset: u64,
        input: &[u8],
        now: u64,
    ) -> Result<(), Code> {
        let index = self.find(owner, token)?;
        let slot = &mut self.slots[index];
        slot.touched = now;
        if input.is_empty() || input.len() > CHUNK_BYTES || offset != slot.received as u64 {
            return Err(Code::InvalidRequest);
        }
        let end = slot
            .received
            .checked_add(input.len())
            .ok_or(Code::InvalidRequest)?;
        if end > slot.request.unwrap().bytes() {
            return Err(Code::InvalidRequest);
        }
        slot.bytes[slot.received..end].copy_from_slice(input);
        slot.received = end;
        Ok(())
    }
    fn complete(&self, owner: Owner, token: u64) -> Result<(BeginReplace, &[u8]), Code> {
        let slot = &self.slots[self.find(owner, token)?];
        let request = slot.request.unwrap();
        if slot.received != request.bytes() {
            return Err(Code::InvalidRequest);
        }
        Ok((request, &slot.bytes[..slot.received]))
    }
    fn abort(&mut self, owner: Owner, token: u64) -> Result<(), Code> {
        let i = self.find(owner, token)?;
        self.slots[i].clear();
        Ok(())
    }
    pub(super) fn close(&mut self, channel: Handle, peer: Identity, file: u64) {
        let owner = Owner::new(channel, peer, file);
        for slot in &mut self.slots {
            if slot.token != 0 && slot.owner == owner {
                slot.clear();
            }
        }
    }
    pub(super) fn disconnected(&mut self, channel: Handle) {
        for slot in &mut self.slots {
            if slot.token != 0 && slot.owner.channel == channel {
                slot.clear();
            }
        }
    }
    /// Drop small candidates untouched since `now - IDLE_TICKS`.
    fn expire(&mut self, now: u64) {
        for slot in &mut self.slots {
            if slot.token != 0 && idle(slot.touched, now) {
                slot.clear();
            }
        }
    }
}
/// Copy `request.bytes` candidate bytes from a section into the candidate,
/// one journal-sized (4 KiB) write at a time.
fn copy_section(
    request: StageSection,
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), Code>,
    mut write: impl FnMut(u64, &[u8]) -> Result<(), Code>,
) -> Result<(), Code> {
    let mut buffer = [0u8; 4096];
    let total = request.bytes as usize;
    let mut done = 0;
    while done < total {
        let n = (total - done).min(buffer.len());
        read(request.section_offset + done as u64, &mut buffer[..n])?;
        write(request.offset + done as u64, &buffer[..n])?;
        done += n;
    }
    Ok(())
}
impl Broker<'_> {
    /// Abort every staged replacement matching `owned` (close/disconnect/idle).
    fn abort_large(&mut self, owned: impl Fn(&LargeSlot) -> bool + Copy) {
        for slot in 0..Staging::LARGE_SLOTS {
            if let Some(staged) = self.staging.take_large_if(slot, owned) {
                // A stale mount or poisoned owner already lost the candidate;
                // validated remount reclaims any persisted staging node.
                let _ = self.volume.abort_staged(staged);
            }
        }
    }
    pub(super) fn abort_large_file(&mut self, channel: Handle, peer: Identity, file: u64) {
        let owner = Owner::new(channel, peer, file);
        self.abort_large(move |s| s.owner == owner);
    }
    pub(super) fn abort_large_channel(&mut self, channel: Handle) {
        self.abort_large(move |s| s.owner.channel == channel);
    }
    /// Abort candidates of either kind untouched for `IDLE_TICKS`.
    pub(super) fn expire_staging(&mut self, now: u64) {
        self.staging.expire(now);
        self.abort_large(move |s| idle(s.touched, now));
    }
    /// STAGE_WRITE_SECTION: up to `STAGE_SECTION_BYTES` candidate bytes read
    /// from a transferred section (a READ snapshot; no mapping is made).
    pub(super) fn stage_section(
        &mut self,
        channel: Handle,
        peer: Identity,
        body: &[u8],
        section: Handle,
        now: u64,
    ) -> Result<Response, Code> {
        let request = StageSection::decode(body)?;
        self.file(channel, peer, request.file)?;
        server::transferred_capability(section, door_abi::HandleKind::Section)
            .map_err(server::status)?;
        let slot =
            self.staging
                .large_find(Owner::new(channel, peer, request.file), request.token, now)?;
        let volume = &mut self.volume;
        let staged = self.staging.large_staged(slot);
        copy_section(
            request,
            |offset, out| match door_sys::section_snapshot(section, offset, out) {
                Ok(n) if n == out.len() => Ok(()),
                Ok(_) => Err(Code::Io),
                Err(error) => Err(server::status(error)),
            },
            |offset, input| {
                volume
                    .stage_write(staged, offset, input)
                    .map_err(persistence_error)
            },
        )?;
        Ok(Response::empty())
    }
    pub(super) fn replacement(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        now: u64,
    ) -> Result<Response, Code> {
        if operation == opcode::STAGE_BEGIN {
            let request = BeginReplace::decode_large(body)?;
            let i = self.file(channel, peer, request.file())?;
            let owner = Owner::new(channel, peer, request.file());
            let slot = self.staging.large_reserve(owner)?;
            let staged = self
                .volume
                .stage_replace(
                    &self.files[i].as_ref().unwrap().file,
                    request.object(),
                    request.version(),
                    request.bytes() as u64,
                )
                .map_err(persistence_error)?;
            return Ok(Response::scalar(
                self.staging.large_fill(slot, owner, staged, now),
            ));
        }
        if operation == opcode::REPLACE_BEGIN {
            let request = BeginReplace::decode(body)?;
            let i = self.file(channel, peer, request.file())?;
            self.volume
                .check_replace(
                    &self.files[i].as_ref().unwrap().file,
                    request.object(),
                    request.version(),
                    request.bytes(),
                )
                .map_err(persistence_error)?;
            let token =
                self.staging
                    .begin(Owner::new(channel, peer, request.file()), request, now)?;
            return Ok(Response::scalar(token));
        }
        let valid = match operation {
            opcode::REPLACE_CHUNK => (25..=24 + CHUNK_BYTES).contains(&body.len()),
            opcode::STAGE_WRITE => (25..=24 + STAGE_CHUNK_BYTES).contains(&body.len()),
            _ => body.len() == 16,
        };
        if !valid {
            return Err(Code::InvalidRequest);
        }
        let file = u64at(body, 0);
        let token = u64at(body, 8);
        let i = self.file(channel, peer, file)?;
        let owner = Owner::new(channel, peer, file);
        match operation {
            opcode::REPLACE_CHUNK => {
                self.staging
                    .append(owner, token, u64at(body, 16), &body[24..], now)?;
                Ok(Response::empty())
            }
            opcode::REPLACE_ABORT => {
                self.staging.abort(owner, token)?;
                Ok(Response::empty())
            }
            opcode::STAGE_WRITE => {
                let slot = self.staging.large_find(owner, token, now)?;
                self.volume
                    .stage_write(
                        self.staging.large_staged(slot),
                        u64at(body, 16),
                        &body[24..],
                    )
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::STAGE_ABORT => {
                let slot = self.staging.large_find(owner, token, now)?;
                let staged = self.staging.large_take(slot);
                self.volume
                    .abort_staged(staged)
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::STAGE_COMMIT => {
                // Every commit attempt consumes the candidate: a refused one
                // (incomplete, changed target) is reclaimed here; an uncertain
                // one is resolved only by remount.
                let slot = self.staging.large_find(owner, token, now)?;
                let staged = self.staging.large_take(slot);
                let result = self
                    .volume
                    .commit_staged(&self.files[i].as_ref().unwrap().file, &staged);
                if self.volume.staged_live(&staged) {
                    let _ = self.volume.abort_staged(staged);
                }
                Ok(Response::bytes(&super::files::info(
                    result.map_err(persistence_error)?,
                )))
            }
            opcode::REPLACE_COMMIT => {
                let (request, bytes) = self.staging.complete(owner, token)?;
                let result = self.volume.replace_checked(
                    &self.files[i].as_ref().unwrap().file,
                    request.object(),
                    request.version(),
                    bytes,
                );
                // A complete commit attempt consumes the candidate on either
                // result. Uncertain durability is resolved only by remount.
                self.staging.abort(owner, token)?;
                Ok(Response::bytes(&super::files::info(
                    result.map_err(persistence_error)?,
                )))
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use door_service_protocol::files::STAGE_SECTION_BYTES;
    use std::{boxed::Box, vec::Vec};
    fn owner(file: u64) -> Owner {
        Owner {
            channel: 7,
            file,
            pid: 11,
            session: 17,
            epoch: 23,
        }
    }
    fn client(pid: u64, file: u64) -> Owner {
        Owner {
            channel: 100 + pid,
            pid,
            ..owner(file)
        }
    }
    fn request(file: u64, n: usize) -> BeginReplace {
        BeginReplace::new(file, 27, 3, n).unwrap()
    }
    /// Occupy a large slot without a volume (tests never take it).
    fn hold(s: &mut Staging, owner: Owner, now: u64) -> Result<usize, Code> {
        let slot = s.large_reserve(owner)?;
        s.large[slot] = LargeSlot {
            owner,
            token: s.next,
            touched: now,
            staged: None,
        };
        s.next += 1;
        Ok(slot)
    }
    #[test]
    fn ordered_candidates_are_private_complete_bounded_and_consumed() {
        let mut s = Box::new(Staging::new());
        let a = owner(1);
        let token = s.begin(a, request(1, 3), 0).unwrap();
        assert_eq!(s.complete(a, token), Err(Code::InvalidRequest));
        for (offset, input) in [(1, &b"x"[..]), (0, &b""[..]), (0, &b"toolong"[..])] {
            assert_eq!(
                s.append(a, token, offset, input, 0),
                Err(Code::InvalidRequest)
            );
        }
        s.append(a, token, 0, b"a", 0).unwrap();
        assert_eq!(s.append(a, token, 0, b"b", 0), Err(Code::InvalidRequest));
        s.append(a, token, 1, b"\0\xff", 0).unwrap();
        assert_eq!(s.complete(a, token).unwrap().1, b"a\0\xff");
        assert_eq!(s.append(a, token, 3, b"x", 0), Err(Code::InvalidRequest));
        s.abort(a, token).unwrap();
        assert_eq!(s.complete(a, token), Err(Code::InvalidHandle));
        assert!(s.slots.iter().all(|s| s.bytes.iter().all(|b| *b == 0)));
        let empty = s.begin(a, request(1, 0), 0).unwrap();
        assert!(s.complete(a, empty).unwrap().1.is_empty());
        s.abort(a, empty).unwrap();
        let full = s.begin(a, request(1, REPLACE_BYTES), 0).unwrap();
        for offset in (0..REPLACE_BYTES).step_by(CHUNK_BYTES) {
            s.append(
                a,
                full,
                offset as u64,
                &[0x7f; CHUNK_BYTES][..(REPLACE_BYTES - offset).min(CHUNK_BYTES)],
                0,
            )
            .unwrap();
        }
        assert_eq!(s.complete(a, full).unwrap().1, &[0x7f; REPLACE_BYTES]);
    }
    #[test]
    fn owner_fields_and_open_file_are_all_required_and_disconnect_reclaims() {
        let mut s = Box::new(Staging::new());
        let a = owner(1);
        let token = s.begin(a, request(1, 1), 0).unwrap();
        for foreign in [
            Owner { channel: 8, ..a },
            Owner { file: 2, ..a },
            Owner { pid: 12, ..a },
            Owner { session: 18, ..a },
            Owner { epoch: 24, ..a },
        ] {
            assert_eq!(
                s.append(foreign, token, 0, b"x", 0),
                Err(Code::InvalidHandle)
            );
            assert_eq!(s.abort(foreign, token), Err(Code::InvalidHandle));
            assert_eq!(s.complete(foreign, token), Err(Code::InvalidHandle));
        }
        assert_eq!(s.begin(a, request(1, 1), 0), Err(Code::WouldBlock));
        s.disconnected(8);
        assert!(s.find(a, token).is_ok());
        s.disconnected(7);
        assert_eq!(s.find(a, token), Err(Code::InvalidHandle));
        let next = s.begin(a, request(1, 1), 0).unwrap();
        assert!(next > token);
        s.close(
            7,
            Identity {
                pid: 11,
                session: 17,
                epoch: 23,
                ..Identity::default()
            },
            1,
        );
        assert_eq!(s.find(a, next), Err(Code::InvalidHandle));
    }
    #[test]
    fn staged_slots_are_bounded_per_owner_and_client_and_preflight_tokens() {
        let mut s = Box::new(Staging::new());
        // One client (any channel) gets CLIENT_LARGE_SLOTS, never all.
        for file in 1..=CLIENT_LARGE_SLOTS as u64 {
            hold(&mut s, owner(file), 0).unwrap();
        }
        assert_eq!(s.large_reserve(owner(1)), Err(Code::WouldBlock));
        assert_eq!(s.large_reserve(owner(9)), Err(Code::Resources));
        assert_eq!(
            s.large_reserve(Owner {
                channel: 99,
                ..owner(9)
            }),
            Err(Code::Resources),
            "another channel of the same process"
        );
        // Other clients still find slots, up to the service-wide bound.
        for pid in 0..(LARGE_SLOTS - CLIENT_LARGE_SLOTS) as u64 {
            hold(&mut s, client(50 + pid, 1), 0).unwrap();
        }
        assert_eq!(s.large_reserve(client(70, 1)), Err(Code::Resources));
        let token = s.large[1].token;
        assert_eq!(s.large_find(owner(2), token, 5), Ok(1));
        assert_eq!(s.large[1].touched, 5);
        assert_eq!(
            s.large_find(
                Owner {
                    channel: 8,
                    ..owner(2)
                },
                token,
                6
            ),
            Err(Code::InvalidHandle)
        );
        assert_eq!(s.large_find(owner(2), 0, 6), Err(Code::InvalidHandle));
        // A capability-less slot is never taken.
        assert!(s.take_large_if(0, |_| true).is_none());
        s.large = [const { LargeSlot::EMPTY }; LARGE_SLOTS];
        s.next = u64::MAX;
        assert_eq!(s.large_reserve(owner(1)), Err(Code::Resources));
    }
    #[test]
    fn idle_candidates_expire_and_activity_keeps_them() {
        let mut s = Box::new(Staging::new());
        let a = owner(1);
        let token = s.begin(a, request(1, 2), 100).unwrap();
        s.expire(100 + IDLE_TICKS - 1);
        s.append(a, token, 0, b"x", 100 + IDLE_TICKS - 1).unwrap();
        s.expire(100 + IDLE_TICKS);
        assert!(s.find(a, token).is_ok(), "the append renewed it");
        s.expire(100 + 2 * IDLE_TICKS - 1);
        assert_eq!(s.find(a, token), Err(Code::InvalidHandle));
        // Large slots: the selector the broker uses to abort them.
        let slot = hold(&mut s, owner(2), 10).unwrap();
        assert!(!idle(s.large[slot].touched, 10 + IDLE_TICKS - 1));
        assert!(idle(s.large[slot].touched, 10 + IDLE_TICKS));
        let token = s.large[slot].token;
        s.large_find(owner(2), token, 10 + IDLE_TICKS).unwrap();
        assert!(!idle(s.large[slot].touched, 10 + IDLE_TICKS));
        // A clock that has not advanced (or failed: 0) never expires.
        assert!(!idle(10, 0));
        assert_eq!(IDLE_TICKS, 3000);
    }
    #[test]
    fn slot_and_token_exhaustion_preflight_without_alias_or_candidate_loss() {
        let mut s = Box::new(Staging::new());
        // A client holds at most CLIENT_SLOTS small candidates.
        for file in 1..=CLIENT_SLOTS as u64 {
            s.begin(owner(file), request(file, 0), 0).unwrap();
        }
        assert_eq!(s.begin(owner(9), request(9, 0), 0), Err(Code::Resources));
        for pid in 0..(SLOTS - CLIENT_SLOTS) as u64 {
            s.begin(client(30 + pid, 5), request(5, 0), 0).unwrap();
        }
        assert_eq!(
            s.begin(client(40, 5), request(5, 0), 0),
            Err(Code::Resources)
        );
        let first = s.slots[0].token;
        s.abort(owner(1), first).unwrap();
        s.next = u64::MAX;
        assert_eq!(s.begin(owner(1), request(1, 0), 0), Err(Code::Resources));
        assert_eq!(s.slots.iter().filter(|s| s.token != 0).count(), SLOTS - 1);
        assert!(s.complete(owner(2), s.slots[1].token).is_ok());
    }
    #[test]
    fn section_copies_are_ordered_journal_sized_and_stop_at_the_first_error() {
        let request = StageSection::new(1, 2, 8192, 100, 10_000).unwrap();
        let mut reads = Vec::new();
        let mut writes = Vec::new();
        copy_section(
            request,
            |at, out| {
                out.fill(at as u8);
                reads.push((at, out.len()));
                Ok(())
            },
            |at, input| {
                writes.push((at, input.len(), input[0]));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(reads, [(100, 4096), (4196, 4096), (8292, 1808)]);
        assert_eq!(
            writes,
            [
                (8192, 4096, 100),
                (12288, 4096, 4196u64 as u8),
                (16384, 1808, 8292u64 as u8)
            ]
        );
        let mut written = 0;
        let failed = copy_section(
            StageSection::new(1, 2, 0, 0, STAGE_SECTION_BYTES).unwrap(),
            |at, _| if at >= 8192 { Err(Code::Io) } else { Ok(()) },
            |_, input| {
                written += input.len();
                Ok(())
            },
        );
        assert_eq!((failed, written), (Err(Code::Io), 8192));
    }
}
