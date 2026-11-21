//! The Event Log (`door_service_protocol::eventlog`): Init keeps each log
//! as a ring of fixed slots in `/state/logs/<log>.evl` (system-owned, so
//! only Init writes it). File: `DOOREVL2`, `[next record u32][count u32]`,
//! then `RECORDS` slots of `SLOT_BYTES`: `[length u16][0 u16][event]`, the
//! event with record number r in slot (r - 1) % RECORDS. A `DOOREVL1` file
//! (256-byte slots, messages of at most 168 bytes, from before messages
//! grew) is copied into the new layout beside it, then renamed over it.
use super::log as trace;
use crate::server::Response;
use door_abi::{Handle, platform::Identity};
use door_service_client::{Disposition, READ, Storage, WRITE};
use door_service_protocol::{
    Code,
    eventlog::{self as wire, Event, Log, Partial},
};

const MAGIC: &[u8; 8] = b"DOOREVL2";
const MAGIC_V1: &[u8; 8] = b"DOOREVL1";
const HEADER: u64 = 16;
const SLOT_V1: u64 = 256;

/// What Init knows of one log's file.
#[derive(Clone, Copy, Default)]
pub(super) struct Ring {
    loaded: bool,
    next: u32,
    count: u32,
}
impl Ring {
    fn oldest(&self) -> u32 {
        self.next.saturating_sub(self.count)
    }
}
/// Message text a connection sent with REPORT_TEXT for its next REPORT.
#[derive(Clone, Copy)]
pub(super) struct Staged {
    channel: Handle,
    log: u8,
    len: usize,
    bytes: [u8; wire::MESSAGE_BYTES],
}
/// Connections that may stage text at once.
pub(super) const STAGES: usize = 4;
pub(super) const NO_STAGE: Option<Staged> = None;

fn slot(record: u32, slot_bytes: u64) -> u64 {
    HEADER + u64::from((record - 1) % wire::RECORDS) * slot_bytes
}
/// Where a DOOREVL1 file is rewritten before it replaces the old one.
fn upgrade_path(log: Log) -> &'static str {
    match log {
        Log::Application => "/state/logs/Application.ev2",
        Log::System => "/state/logs/System.ev2",
        Log::Security => "/state/logs/Security.ev2",
        Log::Setup => "/state/logs/Setup.ev2",
    }
}
fn storage_code(e: door_service_client::Error) -> Code {
    match e {
        door_service_client::Error::Service(c) => c,
        _ => Code::Io,
    }
}
fn parse_header(header: &[u8; HEADER as usize]) -> (u32, u32) {
    (
        u32::from_le_bytes(header[8..12].try_into().unwrap()).max(1),
        u32::from_le_bytes(header[12..16].try_into().unwrap()).min(wire::RECORDS),
    )
}
/// Copies a DOOREVL1 log (`next`, `count`) into the DOOREVL2 layout and
/// renames the copy over it.
fn upgrade(storage: &mut Storage, log: Log, next: u32, count: u32) -> Result<(), Code> {
    let temp = upgrade_path(log);
    let _ = storage.remove(temp);
    let old = storage
        .open(log.path(), Disposition::OpenExisting, READ, READ)
        .map_err(storage_code)?;
    let new = match storage.open(temp, Disposition::CreateNew, READ | WRITE, 0) {
        Ok(file) => file,
        Err(e) => {
            let _ = storage.close(old);
            return Err(storage_code(e));
        }
    };
    let ring = Ring {
        loaded: true,
        next,
        count,
    };
    let copied = (|| {
        for record in ring.oldest().max(1)..next {
            let mut bytes = [0u8; SLOT_V1 as usize];
            let n = storage
                .read_full(&old, slot(record, SLOT_V1), &mut bytes)
                .map_err(storage_code)?;
            let len = 4 + usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
            if n < len {
                return Err(Code::Corrupt);
            }
            storage
                .write_all(&new, slot(record, wire::SLOT_BYTES as u64), &bytes[..len])
                .map_err(storage_code)?;
        }
        storage
            .write_all(&new, 0, &header(&ring))
            .map_err(storage_code)
    })();
    let _ = storage.close(old);
    let _ = storage.close(new);
    copied?;
    storage.flush().map_err(storage_code)?;
    storage.remove(log.path()).map_err(storage_code)?;
    storage.rename(temp, log.path()).map_err(storage_code)?;
    storage.flush().map_err(storage_code)?;
    trace(format_args!(
        "DOOR_P16 EVENTLOG upgraded log={} records={count}",
        log.name()
    ));
    Ok(())
}
/// The header of `log` (a missing file: an empty log, created on the
/// first event).
fn load(storage: &mut Storage, log: Log, ring: &mut Ring) -> Result<(), Code> {
    if ring.loaded {
        return Ok(());
    }
    match storage.open(log.path(), Disposition::OpenExisting, READ, READ) {
        Ok(file) => {
            let mut header = [0u8; HEADER as usize];
            let read = storage.read(&file, 0, &mut header);
            let _ = storage.close(file);
            if read.map_err(storage_code)? != header.len() {
                return Err(Code::Corrupt);
            }
            let (next, count) = parse_header(&header);
            if &header[..8] == MAGIC_V1 {
                upgrade(storage, log, next, count)?;
            } else if &header[..8] != MAGIC {
                return Err(Code::Corrupt);
            }
            ring.next = next;
            ring.count = count;
        }
        Err(door_service_client::Error::Service(Code::NotFound)) => {
            // An upgrade stopped between removing the old file and
            // renaming the copy: the copy is complete.
            if storage.rename(upgrade_path(log), log.path()).is_ok() {
                let _ = storage.flush();
                return load(storage, log, ring);
            }
            ring.next = 1;
            ring.count = 0;
        }
        Err(e) => return Err(storage_code(e)),
    }
    ring.loaded = true;
    Ok(())
}
fn header(ring: &Ring) -> [u8; HEADER as usize] {
    let mut h = [0u8; HEADER as usize];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&ring.next.to_le_bytes());
    h[12..16].copy_from_slice(&ring.count.to_le_bytes());
    h
}
/// Appends `event` (its record number and time set here): the record.
pub(super) fn append(
    storage: &mut Storage,
    ring: &mut Ring,
    mut event: Event,
) -> Result<u32, Code> {
    load(storage, event.log, ring)?;
    let _ = storage.mkdir("/state/logs");
    let file = match storage.open(event.log.path(), Disposition::OpenExisting, READ | WRITE, 0) {
        Ok(file) => file,
        Err(door_service_client::Error::Service(Code::NotFound)) => storage
            .open(event.log.path(), Disposition::CreateNew, READ | WRITE, 0)
            .map_err(storage_code)?,
        Err(e) => return Err(storage_code(e)),
    };
    event.record = ring.next;
    event.time = door_sys::wall_time().unwrap_or(0);
    let mut bytes = [0u8; 4 + wire::STORED_BYTES];
    let n = {
        let encoded: &mut [u8; wire::STORED_BYTES] = (&mut bytes[4..]).try_into().unwrap();
        event.encode_all(encoded)
    };
    bytes[..2].copy_from_slice(&(n as u16).to_le_bytes());
    let mut next = *ring;
    next.next += 1;
    next.count = (next.count + 1).min(wire::RECORDS);
    let written = storage
        .write_all(
            &file,
            slot(event.record, wire::SLOT_BYTES as u64),
            &bytes[..4 + n],
        )
        .and_then(|()| storage.write_all(&file, 0, &header(&next)));
    let _ = storage.close(file);
    written.map_err(storage_code)?;
    let _ = storage.flush();
    *ring = next;
    Ok(event.record)
}
/// Event `record` of `log`.
pub(super) fn read(
    storage: &mut Storage,
    log: Log,
    ring: &mut Ring,
    record: u32,
) -> Result<Event, Code> {
    load(storage, log, ring)?;
    if record == 0 || record < ring.oldest() || record >= ring.next {
        return Err(Code::NotFound);
    }
    let file = storage
        .open(log.path(), Disposition::OpenExisting, READ, READ)
        .map_err(storage_code)?;
    let mut bytes = [0u8; 4 + wire::STORED_BYTES];
    let read = storage.read_full(&file, slot(record, wire::SLOT_BYTES as u64), &mut bytes);
    let _ = storage.close(file);
    let n = read.map_err(storage_code)?;
    let len = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
    if n < 4 + len || len > wire::STORED_BYTES {
        return Err(Code::Corrupt);
    }
    let event = Event::decode(&bytes[4..4 + len]).ok_or(Code::Corrupt)?;
    if event.record != record {
        return Err(Code::Corrupt);
    }
    Ok(event)
}
/// Empties `log` (record numbers keep counting).
pub(super) fn clear(storage: &mut Storage, log: Log, ring: &mut Ring) -> Result<(), Code> {
    load(storage, log, ring)?;
    let mut next = *ring;
    next.count = 0;
    if let Ok(file) = storage.open(log.path(), Disposition::OpenExisting, READ | WRITE, 0) {
        let written = storage.write_all(&file, 0, &header(&next));
        let _ = storage.close(file);
        written.map_err(storage_code)?;
        let _ = storage.flush();
    }
    *ring = next;
    Ok(())
}

impl super::Registry {
    /// Init's own events (System log: the Service Control Manager's state
    /// changes, as Windows' 7036).
    pub(super) fn system_event(&mut self, kind: u16, id: u32, source: &str, message: &str) {
        let Ok(mut storage) = Storage::connect_port(self.ports[0]) else {
            return;
        };
        let event = Event::new(Log::System, kind, 0, id, source, message);
        let _ = append(
            &mut storage,
            &mut self.event_logs[Log::System as usize],
            event,
        );
    }
    /// A connection went: its staged text goes.
    pub(super) fn eventlog_disconnected(&mut self, channel: Handle) {
        for stage in &mut self.event_text {
            if stage.is_some_and(|s| s.channel == channel) {
                *stage = None;
            }
        }
    }
    /// The text `channel` staged (taken).
    fn take_staged(&mut self, channel: Handle) -> Option<Staged> {
        self.event_text
            .iter_mut()
            .find(|s| s.is_some_and(|s| s.channel == channel))
            .and_then(Option::take)
    }
    /// Every Event Log request. Session processes report to Application
    /// and read Application, System and Setup; administrators also report
    /// to System, read Security and clear logs.
    pub(super) fn eventlog_request(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
    ) -> Response {
        if !self.admits(peer) {
            return Response::error(Code::Denied);
        }
        let admin = peer.groups & door_abi::platform::group::ADMINISTRATORS != 0;
        let Some(&first) = body.first() else {
            return Response::error(Code::InvalidRequest);
        };
        let Some(log) = Log::from_u8(first) else {
            return Response::error(Code::InvalidRequest);
        };
        let may_report = match log {
            Log::Application => true,
            Log::System => admin,
            Log::Security | Log::Setup => false,
        };
        let may_read = log != Log::Security || admin;
        if operation == wire::REPORT_TEXT {
            if !may_report {
                return Response::error(Code::Denied);
            }
            let part = &body[1..];
            if part.len() > wire::TEXT_CHUNK {
                return Response::error(Code::InvalidRequest);
            }
            let index = match self
                .event_text
                .iter()
                .position(|s| s.is_some_and(|s| s.channel == channel))
            {
                Some(i) => i,
                None => {
                    let Some(i) = self.event_text.iter().position(Option::is_none) else {
                        return Response::error(Code::Resources);
                    };
                    self.event_text[i] = Some(Staged {
                        channel,
                        log: first,
                        len: 0,
                        bytes: [0; wire::MESSAGE_BYTES],
                    });
                    i
                }
            };
            let Some(stage) = self.event_text[index].as_mut() else {
                return Response::error(Code::Io);
            };
            if stage.log != first || stage.len + part.len() > wire::MESSAGE_BYTES {
                self.event_text[index] = None;
                return Response::error(Code::InvalidRequest);
            }
            stage.bytes[stage.len..stage.len + part.len()].copy_from_slice(part);
            stage.len += part.len();
            return Response::empty();
        }
        let staged = if operation == wire::REPORT {
            self.take_staged(channel)
        } else {
            None
        };
        let Ok(mut storage) = Storage::connect_port(self.ports[0]) else {
            return Response::error(Code::Io);
        };
        let ring = &mut self.event_logs[log as usize];
        match operation {
            wire::REPORT => {
                if !may_report {
                    return Response::error(Code::Denied);
                }
                // The packet's part of the message, then what was staged.
                let Some(mut partial) = Partial::decode(&body[1..]) else {
                    return Response::error(Code::InvalidRequest);
                };
                if let Some(s) = &staged
                    && (s.log != first || !partial.push(&s.bytes[..s.len]))
                {
                    return Response::error(Code::InvalidRequest);
                }
                let Some(mut event) = partial.finish() else {
                    return Response::error(Code::InvalidRequest);
                };
                if event.log != log {
                    return Response::error(Code::InvalidRequest);
                }
                event.uid = peer.uid;
                match append(&mut storage, ring, event) {
                    Ok(record) => {
                        trace(format_args!(
                            "DOOR_P16 EVENTLOG report log={} id={} record={record} bytes={}",
                            log.name(),
                            event.id,
                            event.message_bytes().len()
                        ));
                        Response::bytes(&record.to_le_bytes())
                    }
                    Err(code) => Response::error(code),
                }
            }
            wire::READ | wire::READ_TEXT => {
                if !may_read {
                    return Response::error(Code::Denied);
                }
                let (record, offset) = match (operation, body.len()) {
                    (wire::READ, 5) => (u32::from_le_bytes(body[1..5].try_into().unwrap()), None),
                    (wire::READ_TEXT, 7) => (
                        u32::from_le_bytes(body[1..5].try_into().unwrap()),
                        Some(usize::from(u16::from_le_bytes([body[5], body[6]]))),
                    ),
                    _ => return Response::error(Code::InvalidRequest),
                };
                let event = match read(&mut storage, log, ring, record) {
                    Ok(event) => event,
                    Err(code) => return Response::error(code),
                };
                match offset {
                    None => {
                        let mut out = [0u8; wire::HEAD_BYTES];
                        let n = event.encode(&mut out);
                        Response::bytes(&out[..n])
                    }
                    Some(offset) => {
                        let message = event.message_bytes();
                        let start = offset.min(message.len());
                        let end = (start + wire::TEXT_CHUNK).min(message.len());
                        Response::bytes(&message[start..end])
                    }
                }
            }
            wire::INFO if body.len() == 1 => {
                if !may_read {
                    return Response::error(Code::Denied);
                }
                if let Err(code) = load(&mut storage, log, ring) {
                    return Response::error(code);
                }
                let mut out = [0u8; 8];
                let oldest = if ring.count == 0 { 0 } else { ring.oldest() };
                out[..4].copy_from_slice(&oldest.to_le_bytes());
                out[4..].copy_from_slice(&ring.count.to_le_bytes());
                Response::bytes(&out)
            }
            wire::CLEAR if body.len() == 1 && admin => match clear(&mut storage, log, ring) {
                Ok(()) => Response::empty(),
                Err(code) => Response::error(code),
            },
            wire::CLEAR => Response::error(Code::Denied),
            _ => Response::error(Code::InvalidRequest),
        }
    }
}
