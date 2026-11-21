//! P14 file operations, Recycle Bin and change subscriptions.
//!
//! The engine (`door_file_ops`) runs inside this single serialized owner:
//! each request performs one bounded step whose data and recovery record
//! commit in one redo transaction. Operations belong to the authenticated
//! user; IDs and paths in requests confer no authority.
use super::*;
use door_file_ops::{self as ops, Bin};
use door_service_protocol::files::{
    ConflictInfo, ConflictPolicy, Decision, FileOpMode, OpControl, OpProgress, OpState,
    RecycleEntry, RecycleOutcome, RecycleUsage, WATCH_FOLDER, WATCH_SETTINGS, WatchState,
};
use door_storage_core::persistence::{Change, fileops::OpVolume};

/// Live change subscriptions service-wide and per channel.
pub const WATCHES: usize = 16;
pub const WATCHES_PER_CHANNEL: usize = 4;
/// Operation records kept per user (finished ones are pruned first).
const OPS_PER_USER: usize = 16;

#[derive(Clone, Copy)]
pub(super) enum Target {
    Folder(u64),
    Settings(u32),
}
#[derive(Clone, Copy)]
pub(super) struct Watch {
    channel: Handle,
    pid: u64,
    session: u64,
    epoch: u64,
    token: u64,
    subject: Subject,
    target: Target,
    seen: u64,
}
pub(super) fn op_code(error: ops::Error) -> Code {
    use ops::Error as E;
    match error {
        E::NotFound => Code::NotFound,
        E::Exists => Code::Exists,
        E::Denied => Code::Denied,
        E::NoSpace | E::TooLarge => Code::Resources,
        E::Invalid => Code::InvalidRequest,
        E::Sharing => Code::SharingViolation,
        E::Changed => Code::Changed,
        E::Corrupt => Code::Corrupt,
        E::Io | E::Poisoned => Code::Io,
        E::Busy => Code::WouldBlock,
    }
}
fn progress(p: ops::Progress) -> OpProgress {
    let mode = match p.mode {
        ops::Mode::Copy => FileOpMode::Copy,
        ops::Mode::Move => FileOpMode::Move,
        ops::Mode::Delete => FileOpMode::Delete,
    };
    let state = match p.state {
        ops::State::Scanning => OpState::Scanning,
        ops::State::Running => OpState::Running,
        ops::State::Paused => OpState::Paused,
        ops::State::Conflict => OpState::Conflict,
        ops::State::Done => OpState::Done,
        ops::State::Cancelled => OpState::Cancelled,
        ops::State::Failed => OpState::Failed,
    };
    let conflict = match (p.state, p.conflict) {
        (ops::State::Conflict, Some((src, dst))) => Some(ConflictInfo {
            source_bytes: src.bytes,
            dest_bytes: dst.bytes,
            source_modified: src.modified,
            dest_modified: dst.modified,
            directory: src.directory,
        }),
        // The conflicting item vanished meanwhile: still a decision to make.
        (ops::State::Conflict, None) => Some(ConflictInfo::default()),
        _ => None,
    };
    OpProgress::new(
        p.op,
        mode,
        state,
        (p.items_total, p.items_done),
        p.skipped,
        p.failures,
        (p.bytes_total, p.bytes_done),
        p.last_error.map(op_code),
        conflict,
        p.current.as_str(),
    )
}
fn reply(p: ops::Progress) -> Response {
    let mut bytes = [0; door_service_protocol::files::PROGRESS_BYTES];
    let len = progress(p).encode(&mut bytes);
    Response::bytes(&bytes[..len])
}
fn utf8(bytes: &[u8]) -> Result<&str, Code> {
    if bytes.len() > PATH_BYTES || bytes.contains(&0) {
        return Err(Code::InvalidRequest);
    }
    core::str::from_utf8(bytes).map_err(|_| Code::InvalidRequest)
}
fn op_id(body: &[u8]) -> Result<u64, Code> {
    let id = u64::from_le_bytes(
        body.get(..8)
            .ok_or(Code::InvalidRequest)?
            .try_into()
            .unwrap(),
    );
    if id == 0 {
        return Err(Code::InvalidRequest);
    }
    Ok(id)
}
impl<'a> Broker<'a> {
    fn bin(&self, uid: u32) -> Bin {
        let total = self.volume.usage().map_or(0, |(total, _)| total);
        Bin {
            uid,
            limit: ops::default_limit(total),
        }
    }
    /// Allocate an operation ID and make room for one more of `uid`'s
    /// records, pruning that user's finished operations first.
    fn new_op(&mut self, user: Subject, uid: u32) -> Result<u64, Code> {
        let mut vol = OpVolume {
            volume: &mut self.volume,
            user,
            system: self.system,
        };
        let mut count = 0;
        let mut after = 0;
        let mut finished = [0u64; OPS_PER_USER];
        let mut prunable = 0;
        while let Some(p) = self.engine.next(&mut vol, after, uid).map_err(op_code)? {
            after = p.op;
            count += 1;
            if p.state.finished() && prunable < finished.len() {
                finished[prunable] = p.op;
                prunable += 1;
            }
        }
        for op in &finished[..prunable] {
            if count < OPS_PER_USER {
                break;
            }
            self.engine.dismiss(&mut vol, *op, uid).map_err(op_code)?;
            count -= 1;
        }
        if count >= OPS_PER_USER {
            return Err(Code::Resources);
        }
        let id = self.next_op;
        self.next_op = id.checked_add(1).ok_or(Code::Resources)?;
        Ok(id)
    }
    pub(super) fn file_ops(
        &mut self,
        channel: Handle,
        peer: Identity,
        subject: Subject,
        operation: u32,
        body: &[u8],
    ) -> Result<Response, Code> {
        if operation == opcode::WATCH
            || operation == opcode::WATCH_POLL
            || operation == opcode::UNWATCH
        {
            return self.watches(channel, peer, subject, operation, body);
        }
        // File operations and the bin belong to signed-in users.
        if !server::session_peer(peer) {
            return Err(Code::Denied);
        }
        let uid = peer.uid;
        match operation {
            opcode::FILEOP_BEGIN => {
                if body.len() < 2 {
                    return Err(Code::InvalidRequest);
                }
                let mode = ops::Mode::decode(body[0]).ok_or(Code::InvalidRequest)?;
                let policy = ops::Policy::decode(body[1]).ok_or(Code::InvalidRequest)?;
                let dest = utf8(&body[2..])?;
                if (mode == ops::Mode::Delete) != dest.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                let op = self.new_op(subject, uid)?;
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                self.engine
                    .begin(&mut vol, op, uid, mode, policy, dest)
                    .map_err(op_code)?;
                Ok(Response::scalar(op))
            }
            opcode::FILEOP_ADD => {
                let op = op_id(body)?;
                let source = utf8(&body[8..])?;
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                self.engine
                    .add(&mut vol, op, uid, source)
                    .map_err(op_code)?;
                Ok(Response::empty())
            }
            opcode::FILEOP_STEP | opcode::FILEOP_QUERY if body.len() == 8 => {
                let op = op_id(body)?;
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                let p = if operation == opcode::FILEOP_STEP {
                    self.engine.step(&mut vol, op, uid)
                } else {
                    self.engine.query(&mut vol, op, uid)
                }
                .map_err(op_code)?;
                Ok(reply(p))
            }
            opcode::FILEOP_RESOLVE if body.len() == 10 => {
                let op = op_id(body)?;
                let decision = match body[8] {
                    1 => ops::Decision::Replace,
                    2 => ops::Decision::Skip,
                    3 => ops::Decision::KeepBoth,
                    _ => return Err(Code::InvalidRequest),
                };
                if body[9] > 1 {
                    return Err(Code::InvalidRequest);
                }
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                self.engine
                    .resolve(&mut vol, op, uid, decision, body[9] == 1)
                    .map_err(op_code)?;
                Ok(Response::empty())
            }
            opcode::FILEOP_CONTROL if body.len() == 9 => {
                let op = op_id(body)?;
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                let e = &mut self.engine;
                match body[8] {
                    0 => e.pause(&mut vol, op, uid).map_err(op_code)?,
                    1 => e.resume(&mut vol, op, uid).map_err(op_code)?,
                    2 => {
                        return e.cancel(&mut vol, op, uid).map(reply).map_err(op_code);
                    }
                    3 => {
                        e.dismiss(&mut vol, op, uid).map_err(op_code)?;
                        return Ok(Response::empty());
                    }
                    _ => return Err(Code::InvalidRequest),
                }
                e.query(&mut vol, op, uid).map(reply).map_err(op_code)
            }
            opcode::FILEOP_LIST if body.len() == 8 => {
                let after = u64at(body, 0);
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                match self.engine.next(&mut vol, after, uid).map_err(op_code)? {
                    Some(p) => Ok(reply(p)),
                    None => Ok(Response::empty()),
                }
            }
            opcode::RECYCLE => {
                let path = utf8(body)?;
                let bin = self.bin(uid);
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                // A Windows program names the item the Windows way (C:\...).
                let native;
                let path = if path.starts_with('/') {
                    path
                } else {
                    native = ops::native_from_windows(&mut vol, path).map_err(op_code)?;
                    native.as_str()
                };
                let outcome = match bin.recycle(&mut vol, path).map_err(op_code)? {
                    ops::RecycleOutcome::Recycled(id) => RecycleOutcome::Recycled(id),
                    ops::RecycleOutcome::TooBig => RecycleOutcome::TooBig,
                };
                Ok(Response::bytes(&outcome.encode()))
            }
            opcode::RECYCLE_LIST if body.len() == 8 => {
                let bin = self.bin(uid);
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                match bin.next(&mut vol, u64at(body, 0)).map_err(op_code)? {
                    Some(info) => {
                        let entry = RecycleEntry::new(
                            info.id,
                            info.deleted,
                            info.bytes,
                            info.items,
                            info.directory,
                            info.original.as_str(),
                        );
                        let mut out = [0; door_service_protocol::BODY_BYTES];
                        let len = entry.encode(&mut out);
                        Ok(Response::bytes(&out[..len]))
                    }
                    None => Ok(Response::empty()),
                }
            }
            opcode::RECYCLE_RESTORE if body.len() == 8 => {
                let id = op_id(body)?;
                let bin = self.bin(uid);
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                bin.restore(&mut vol, id).map_err(op_code)?;
                Ok(Response::empty())
            }
            opcode::RECYCLE_PURGE if body.len() == 8 => {
                let id = u64at(body, 0);
                let bin = self.bin(uid);
                let path = bin.path().map_err(op_code)?;
                // Sources first, so nothing is planned for an empty bin.
                let mut sources: [Option<ops::PathBuf<{ ops::PATH_BYTES }>>; 2] = [None, None];
                {
                    let mut vol = OpVolume {
                        volume: &mut self.volume,
                        user: subject,
                        system: self.system,
                    };
                    if id == 0 {
                        match ops::Volume::child(&mut vol, path.as_str(), 0) {
                            Ok(Some(_)) => {
                                sources[0] =
                                    Some(ops::PathBuf::from(path.as_str()).map_err(op_code)?)
                            }
                            Ok(None) | Err(ops::Error::NotFound) => {
                                return Ok(Response::scalar(0));
                            }
                            Err(e) => return Err(op_code(e)),
                        }
                    } else {
                        let (info, item) = bin.entry_paths(&mut vol, id).map_err(op_code)?;
                        if ops::Volume::stat(&mut vol, info.as_str()).is_ok() {
                            sources[0] = Some(info);
                        }
                        sources[1] = item;
                        if sources.iter().all(Option::is_none) {
                            return Err(Code::NotFound);
                        }
                    }
                }
                let op = self.new_op(subject, uid)?;
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                self.engine
                    .begin(&mut vol, op, uid, ops::Mode::Delete, ops::Policy::Ask, "")
                    .map_err(op_code)?;
                for source in sources.iter().flatten() {
                    self.engine
                        .add(&mut vol, op, uid, source.as_str())
                        .map_err(op_code)?;
                }
                Ok(Response::scalar(op))
            }
            opcode::RECYCLE_USAGE if body.is_empty() => {
                let bin = self.bin(uid);
                let mut vol = OpVolume {
                    volume: &mut self.volume,
                    user: subject,
                    system: self.system,
                };
                let usage = bin.usage(&mut vol).map_err(op_code)?;
                Ok(Response::bytes(
                    &RecycleUsage {
                        items: usage.items,
                        bytes: usage.bytes,
                        limit: usage.limit,
                    }
                    .encode(),
                ))
            }
            _ => Err(Code::InvalidRequest),
        }
    }
    fn watch_slot(&self, channel: Handle, peer: Identity, token: u64) -> Result<usize, Code> {
        self.watches
            .iter()
            .position(|w| {
                w.is_some_and(|w| {
                    w.channel == channel
                        && w.pid == peer.pid
                        && w.session == peer.session
                        && w.epoch == peer.epoch
                        && w.token == token
                })
            })
            .ok_or(Code::InvalidHandle)
    }
    fn watches(
        &mut self,
        channel: Handle,
        peer: Identity,
        subject: Subject,
        operation: u32,
        body: &[u8],
    ) -> Result<Response, Code> {
        match operation {
            opcode::WATCH if !body.is_empty() => {
                let target = match body[0] {
                    WATCH_FOLDER => {
                        let path = utf8(&body[1..])?;
                        if !path.starts_with('/') {
                            return Err(Code::InvalidRequest);
                        }
                        Target::Folder(
                            self.volume
                                .watch_target(subject, Path::Native(path))
                                .map_err(persistence_error)?,
                        )
                    }
                    WATCH_SETTINGS if body.len() == 1 => Target::Settings(peer.uid),
                    _ => return Err(Code::InvalidRequest),
                };
                if self
                    .watches
                    .iter()
                    .filter(|w| w.is_some_and(|w| w.channel == channel))
                    .count()
                    >= WATCHES_PER_CHANNEL
                {
                    return Err(Code::Resources);
                }
                let slot = self
                    .watches
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Code::Resources)?;
                let token = self.next;
                self.next = token.checked_add(1).ok_or(Code::Resources)?;
                self.watches[slot] = Some(Watch {
                    channel,
                    pid: peer.pid,
                    session: peer.session,
                    epoch: peer.epoch,
                    token,
                    subject,
                    target,
                    seen: self.volume.change_sequence(),
                });
                Ok(Response::scalar(token))
            }
            opcode::WATCH_POLL if body.len() == 8 => {
                let slot = self.watch_slot(channel, peer, u64at(body, 0))?;
                let mut watch = self.watches[slot].unwrap();
                let mut state = WatchState::default();
                if let Target::Folder(object) = watch.target {
                    if !self.volume.object_alive(object) {
                        state.gone = true;
                    } else if let Err(error) = self.volume.watch_allowed(watch.subject, object) {
                        // Rights changed since subscribing: stop observing.
                        self.watches[slot] = None;
                        return Err(persistence_error(error));
                    }
                }
                let complete = self.volume.changes_since(watch.seen, |change| {
                    state.changed |= match (watch.target, change) {
                        (Target::Folder(a), Change::Folder(b)) => a == b,
                        (Target::Settings(a), Change::Settings(b)) => a == b || b == 0,
                        _ => false,
                    };
                });
                state.overflow = !complete;
                state.changed |= state.overflow || state.gone;
                watch.seen = self.volume.change_sequence();
                state.sequence = watch.seen;
                self.watches[slot] = Some(watch);
                Ok(Response::bytes(&state.encode()))
            }
            opcode::UNWATCH if body.len() == 8 => {
                let slot = self.watch_slot(channel, peer, u64at(body, 0))?;
                self.watches[slot] = None;
                Ok(Response::empty())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
    pub(super) fn drop_watches(&mut self, channel: Handle) {
        for w in &mut self.watches {
            if w.is_some_and(|w| w.channel == channel) {
                *w = None;
            }
        }
    }
}
// Keep wire enums and engine enums in step.
const _: () = {
    assert!(FileOpMode::Delete as u8 == ops::Mode::Delete as u8);
    assert!(ConflictPolicy::KeepBoth as u8 == ops::Policy::KeepBoth as u8);
    assert!(Decision::KeepBoth as u8 == ops::Decision::KeepBoth as u8);
    assert!(OpControl::Dismiss as u8 == 3);
};
