//! The Event Log (Init hosts it): the classic Windows logs Application,
//! System, Security and Setup, each a ring of `RECORDS` events persisted in
//! `/state/logs/<log>.evl`, overwritten oldest-first when full ("Overwrite
//! events as needed", Windows' default). advapi32's ReportEvent and
//! ReadEventLog, `eventcreate`, `wevtutil` and Event Viewer use it.
//!
//! A message holds up to `MESSAGE_BYTES`; one packet carries an event's
//! header, source and the start of its message (`Event::encode`), and the
//! rest goes in pieces: REPORT_TEXT before REPORT, READ_TEXT after READ.

/// Report an event (`Event::encode` body, `record` and `time` ignored):
/// any session process into Application; Init and administrators into
/// System. A message longer than the packet holds sends the rest with
/// REPORT_TEXT first. Reply: the new record number `[u32]`.
pub const REPORT: u32 = 0x7010;
/// Read one event: `[log u8][record u32]`; reply `Event::encode`
/// (NotFound past the newest or before the oldest).
pub const READ: u32 = 0x7011;
/// `[log u8]`: reply `[oldest u32][count u32]` (record numbers start at 1
/// and keep counting across overwrites, as Windows').
pub const INFO: u32 = 0x7012;
/// `[log u8]`: empty it (administrators only).
pub const CLEAR: u32 = 0x7013;
/// `[log u8][bytes]`: the next part of the message the connection's next
/// REPORT carries the start of (Init keeps it until that REPORT, at most
/// `MESSAGE_BYTES` in all).
pub const REPORT_TEXT: u32 = 0x7014;
/// `[log u8][record u32][offset u16]`: at most `TEXT_CHUNK` bytes of the
/// event's message from `offset` (empty at its end).
pub const READ_TEXT: u32 = 0x7015;
/// The first and last Event Log operations (Init routes the range).
pub const FIRST: u32 = 0x7010;
pub const LAST: u32 = 0x701f;

/// Events each log keeps.
pub const RECORDS: u32 = 512;
/// Source name and message limits (UTF-8 bytes).
pub const SOURCE_BYTES: usize = 40;
pub const MESSAGE_BYTES: usize = 2048;
/// An event's header.
pub const HEADER_BYTES: usize = 28;
/// The first packet of an event: a REPORT body after its log byte, and a
/// READ reply.
pub const HEAD_BYTES: usize = crate::BODY_BYTES - 1;
/// What READ_TEXT answers at most, and REPORT_TEXT carries after its log
/// byte.
pub const TEXT_CHUNK: usize = 224;
/// A whole event (Init's file keeps it in one slot).
pub const STORED_BYTES: usize = HEADER_BYTES + SOURCE_BYTES + MESSAGE_BYTES;
/// One stored record (the file's slot size): `[length u16][0 u16][event]`.
pub const SLOT_BYTES: usize = 2176;
const _: () = assert!(4 + STORED_BYTES <= SLOT_BYTES && HEADER_BYTES + SOURCE_BYTES < HEAD_BYTES);
const _: () = assert!(TEXT_CHUNK < crate::BODY_BYTES && MESSAGE_BYTES <= u16::MAX as usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Log {
    Application = 0,
    System = 1,
    Security = 2,
    Setup = 3,
}
impl Log {
    pub const ALL: [Self; 4] = [Self::Application, Self::System, Self::Security, Self::Setup];
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(usize::from(v)).copied()
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Application => "Application",
            Self::System => "System",
            Self::Security => "Security",
            Self::Setup => "Setup",
        }
    }
    /// A log by its name, any case ("application", "SYSTEM").
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|l| l.name().eq_ignore_ascii_case(name))
    }
    pub fn path(self) -> &'static str {
        match self {
            Self::Application => "/state/logs/Application.evl",
            Self::System => "/state/logs/System.evl",
            Self::Security => "/state/logs/Security.evl",
            Self::Setup => "/state/logs/Setup.evl",
        }
    }
}
/// EVENTLOG_*_TYPE.
pub mod kind {
    pub const SUCCESS: u16 = 0x0000;
    pub const ERROR: u16 = 0x0001;
    pub const WARNING: u16 = 0x0002;
    pub const INFORMATION: u16 = 0x0004;
    pub const AUDIT_SUCCESS: u16 = 0x0008;
    pub const AUDIT_FAILURE: u16 = 0x0010;
}

/// One event: `[record u32][time u64 FILETIME][id u32][kind u16]
/// [category u16][log u8][source_len u8][message_len u16][user uid u32]
/// [source][message]` (a packet may carry only the message's start;
/// `message_len` is always the whole message's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub record: u32,
    pub time: u64,
    pub id: u32,
    pub kind: u16,
    pub category: u16,
    pub log: Log,
    pub uid: u32,
    source: [u8; SOURCE_BYTES],
    source_len: usize,
    message: [u8; MESSAGE_BYTES],
    message_len: usize,
}
/// `text` cut to `max` bytes at a character boundary.
fn cut(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
impl Event {
    pub fn new(log: Log, kind: u16, category: u16, id: u32, source: &str, message: &str) -> Self {
        let source = cut(source, SOURCE_BYTES);
        let message = cut(message, MESSAGE_BYTES);
        let mut e = Self {
            record: 0,
            time: 0,
            id,
            kind,
            category,
            log,
            uid: 0,
            source: [0; SOURCE_BYTES],
            source_len: source.len(),
            message: [0; MESSAGE_BYTES],
            message_len: message.len(),
        };
        e.source[..source.len()].copy_from_slice(source.as_bytes());
        e.message[..message.len()].copy_from_slice(message.as_bytes());
        e
    }
    pub fn source(&self) -> &str {
        core::str::from_utf8(&self.source[..self.source_len]).unwrap_or("")
    }
    pub fn message(&self) -> &str {
        core::str::from_utf8(self.message_bytes()).unwrap_or("")
    }
    pub fn message_bytes(&self) -> &[u8] {
        &self.message[..self.message_len]
    }
    /// The header and source, then `message` bytes of the message.
    fn write(&self, out: &mut [u8], message: usize) -> usize {
        out[..4].copy_from_slice(&self.record.to_le_bytes());
        out[4..12].copy_from_slice(&self.time.to_le_bytes());
        out[12..16].copy_from_slice(&self.id.to_le_bytes());
        out[16..18].copy_from_slice(&self.kind.to_le_bytes());
        out[18..20].copy_from_slice(&self.category.to_le_bytes());
        out[20] = self.log as u8;
        out[21] = self.source_len as u8;
        out[22..24].copy_from_slice(&(self.message_len as u16).to_le_bytes());
        out[24..28].copy_from_slice(&self.uid.to_le_bytes());
        let mut at = HEADER_BYTES;
        out[at..at + self.source_len].copy_from_slice(&self.source[..self.source_len]);
        at += self.source_len;
        out[at..at + message].copy_from_slice(&self.message[..message]);
        at + message
    }
    /// The event's first packet: everything but the part of a long
    /// message past `HEAD_BYTES` (`head_message` bytes of it go).
    pub fn encode(&self, out: &mut [u8; HEAD_BYTES]) -> usize {
        self.write(out, self.head_message())
    }
    /// How much of the message `encode` carries.
    pub fn head_message(&self) -> usize {
        self.message_len
            .min(HEAD_BYTES - HEADER_BYTES - self.source_len)
    }
    /// The whole event.
    pub fn encode_all(&self, out: &mut [u8; STORED_BYTES]) -> usize {
        self.write(out, self.message_len)
    }
    /// An event `b` holds all of.
    pub fn decode(b: &[u8]) -> Option<Self> {
        Partial::decode(b)?.finish()
    }
    /// Event Viewer's Level column.
    pub fn level(&self) -> &'static str {
        match self.kind {
            kind::ERROR => "Error",
            kind::WARNING => "Warning",
            kind::AUDIT_SUCCESS => "Audit Success",
            kind::AUDIT_FAILURE => "Audit Failure",
            _ => "Information",
        }
    }
}
/// An event whose message is still arriving: the first packet's part, then
/// each following part (`push`) until `finish`.
#[derive(Clone, Copy, Debug)]
pub struct Partial {
    event: Event,
    total: usize,
}
impl Partial {
    /// The header, source and whatever of the message `b` holds (no more
    /// than its length).
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER_BYTES {
            return None;
        }
        let s = usize::from(b[21]);
        let total = usize::from(u16::from_le_bytes([b[22], b[23]]));
        if s > SOURCE_BYTES || total > MESSAGE_BYTES || b.len() < HEADER_BYTES + s {
            return None;
        }
        let source = core::str::from_utf8(&b[HEADER_BYTES..HEADER_BYTES + s]).ok()?;
        let part = &b[HEADER_BYTES + s..];
        let part = &part[..part.len().min(total)];
        let w = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let mut event = Event::new(
            Log::from_u8(b[20])?,
            u16::from_le_bytes([b[16], b[17]]),
            u16::from_le_bytes([b[18], b[19]]),
            w(12),
            source,
            "",
        );
        event.record = w(0);
        event.time = u64::from_le_bytes(b[4..12].try_into().ok()?);
        event.uid = w(24);
        event.message[..part.len()].copy_from_slice(part);
        event.message_len = part.len();
        Some(Self { event, total })
    }
    /// Bytes of the message received so far.
    pub fn received(&self) -> usize {
        self.event.message_len
    }
    /// The whole message's length.
    pub fn total(&self) -> usize {
        self.total
    }
    pub fn record(&self) -> u32 {
        self.event.record
    }
    /// Appends the next part (false when it runs past the message).
    pub fn push(&mut self, part: &[u8]) -> bool {
        let at = self.event.message_len;
        if at + part.len() > self.total {
            return false;
        }
        self.event.message[at..at + part.len()].copy_from_slice(part);
        self.event.message_len += part.len();
        true
    }
    /// The event, once its whole message (UTF-8) is here.
    pub fn finish(self) -> Option<Event> {
        (self.event.message_len == self.total
            && core::str::from_utf8(self.event.message_bytes()).is_ok())
        .then_some(self.event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events_round_trip_and_cut_long_text() {
        let mut e = Event::new(
            Log::Application,
            kind::ERROR,
            0,
            100,
            "Doors Probe",
            "Something went wrong: ü",
        );
        e.record = 7;
        e.time = 133_000_000_000_000_000;
        e.uid = 1001;
        let mut b = [0; HEAD_BYTES];
        let n = e.encode(&mut b);
        assert_eq!(Event::decode(&b[..n]), Some(e));
        assert_eq!(e.level(), "Error");
        assert!(Event::decode(&b[..n - 1]).is_none());
        let bytes = [b'x'; 4000];
        let long = core::str::from_utf8(&bytes).unwrap();
        let cut = Event::new(Log::System, kind::INFORMATION, 0, 1, long, long);
        assert_eq!(
            (cut.source().len(), cut.message().len()),
            (SOURCE_BYTES, MESSAGE_BYTES)
        );
        assert_eq!(Log::named("system"), Some(Log::System));
        assert_eq!(Log::named("Nope"), None);
    }
    #[test]
    fn long_messages_travel_in_parts() {
        // 3-byte characters, so the packet boundaries split them.
        let mut bytes = [0u8; 1500];
        for piece in bytes.chunks_mut(5) {
            piece.copy_from_slice("€ab".as_bytes());
        }
        let text = core::str::from_utf8(&bytes).unwrap();
        let e = Event::new(Log::Application, kind::WARNING, 2, 42, "Doors", text);
        assert_eq!(e.message().len(), 1500);
        let mut head = [0; HEAD_BYTES];
        let n = e.encode(&mut head);
        assert_eq!(n, HEAD_BYTES);
        let mut p = Partial::decode(&head[..n]).unwrap();
        assert_eq!((p.received(), p.total()), (e.head_message(), 1500));
        assert!(p.finish().is_none());
        let mut at = p.received();
        while at < 1500 {
            let end = (at + TEXT_CHUNK).min(1500);
            assert!(p.push(&e.message_bytes()[at..end]));
            at = end;
        }
        assert!(!p.push(b"x"));
        assert_eq!(p.finish(), Some(e));
        let mut all = [0; STORED_BYTES];
        let n = e.encode_all(&mut all);
        assert_eq!(Event::decode(&all[..n]), Some(e));
        // A slot written before messages grew (u8 length, then 0) reads
        // the same.
        let short = Event::new(
            Log::System,
            kind::ERROR,
            0,
            7031,
            "Service Control Manager",
            "x",
        );
        let n = short.encode(&mut head);
        assert_eq!(head[23], 0);
        assert_eq!(Event::decode(&head[..n]), Some(short));
    }
}
