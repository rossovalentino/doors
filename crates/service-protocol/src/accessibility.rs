//! door.accessibility v1 (`specs/contracts/native-ui-v1.md`, "Accessibility").
//!
//! Providers (every native window client) publish one semantic tree per
//! window in the bounded `door_accessibility::wire` format: a full tree, then
//! deltas. Messages are staged with BEGIN and CHUNKs (or one SECTION
//! snapshot) and published by COMMIT; the window server validates them before
//! anyone can read them. Only an authorized assistive client (CAP_ASSISTIVE,
//! granted from the kernel-authenticated identity, never from these bytes)
//! may follow focus, read trees and invoke actions; secure surfaces read as a
//! placeholder, protected (password) content is stripped, and a locked
//! session denies every read and action.
use crate::window::Event;
use crate::{BODY_BYTES, Code};

/// HELLO capability (bit 24): publish trees, receive ACTION events.
pub const CAP_ACCESSIBILITY: u32 = 1 << 24;
/// HELLO capability (bit 25): assistive client (screen reader, caption
/// panel). Granted only to authorized identities.
pub const CAP_ASSISTIVE: u32 = 1 << 25;
/// Boot file staged only in the desktop-diagnostics image (never in the
/// shipping image): Init then starts Captions in `ASSIST_DIAGNOSTIC_MODE`.
pub const ASSIST_DIAGNOSTICS_FILE: &str = "system/assist-diagnostics.conf";
/// Exact contents of `ASSIST_DIAGNOSTICS_FILE`.
pub const ASSIST_DIAGNOSTICS_FLAG: &[u8] = b"assist-diagnostics-v1\n";
/// Startup mode of a Captions process that may write the trees and captions
/// it reads (app text) to its serial diagnostics for guest acceptance
/// checks. Any other mode keeps them on screen only. It must be a startup
/// mode the kernel accepts (`StartupV2::validate`), or the launch fails.
pub const ASSIST_DIAGNOSTIC_MODE: u64 = door_abi::platform::mode::ASSIST_DIAGNOSTICS;
/// Opcodes 0x5200..=0x523F are reserved for accessibility.
pub const FIRST: u32 = 0x5200;
pub const LAST: u32 = 0x523F;

// ----- provider ---------------------------------------------------------
/// `[window u64][version u32][kind u32][bytes u32]`: stage one wire message
/// for an owned window (replaces the caller's earlier staging).
pub const PUBLISH_BEGIN: u32 = 0x5200;
/// `[offset u32][bytes]`: append up to `CHUNK_BYTES`, in order.
pub const PUBLISH_CHUNK: u32 = 0x5201;
/// `[window u64][version u32][kind u32][bytes u32]` plus one section handle
/// with READ: the server snapshots `bytes` from section offset 0.
pub const PUBLISH_SECTION: u32 = 0x5202;
/// Empty: validate and publish the staged message. Reply `[version u32]`.
/// `Changed`: a delta did not match the published version; send a full tree.
pub const PUBLISH_COMMIT: u32 = 0x5203;
/// `[window u64]`: withdraw a window's tree.
pub const WITHDRAW: u32 = 0x5204;
/// `[serial u32]`: the action request behind an ACTION event. Reply
/// `ActionRequest` (encoded).
pub const ACTION_GET: u32 = 0x5205;
/// `[serial u32][result u32]`: the provider performed (or refused) it.
pub const ACTION_DONE: u32 = 0x5206;

// ----- assistive client ---------------------------------------------------
/// Empty: reply `Focus` (encoded).
pub const FOCUS_GET: u32 = 0x5210;
/// `[window u64][scope u32][node u64]`: freeze a redacted copy for reading.
/// Reply `[serial u32][bytes u32]`.
pub const SNAPSHOT: u32 = 0x5211;
/// `[serial u32][offset u32]`: up to `READ_BYTES` of a snapshot.
pub const READ: u32 = 0x5212;
/// `[window u64][node u64][action u32][arg bytes]`: ask the provider to act.
/// Reply `[serial u32]`; the result arrives as ACTION_RESULT.
pub const INVOKE: u32 = 0x5213;
/// Empty: windows with published trees. Reply `[count u32]` then
/// `[window u64][version u32][flags u32]` each (`WINDOW_*` flags).
pub const WINDOWS: u32 = 0x5214;

pub const SCOPE_TREE: u32 = 0;
/// Just the focused node (cheap focus following).
pub const SCOPE_FOCUS: u32 = 1;
/// Just `node`.
pub const SCOPE_NODE: u32 = 2;

pub const WINDOW_FOCUSED: u32 = 1;
pub const WINDOW_SECURE: u32 = 2;

pub const KIND_FULL: u32 = 1;
pub const KIND_DELTA: u32 = 2;
/// Largest staged message (equals `door_accessibility::wire::MAX_MESSAGE_BYTES`).
pub const MAX_MESSAGE_BYTES: usize = 512 * 1024;
pub const CHUNK_BYTES: usize = BODY_BYTES - 4;
pub const READ_BYTES: usize = BODY_BYTES;
pub const ARG_BYTES: usize = BODY_BYTES - 20;
/// Snapshots kept per assistive client and pending actions per session.
pub const MAX_SNAPSHOTS: usize = 4;
pub const MAX_PENDING_ACTIONS: usize = 16;

pub const RESULT_DONE: u32 = 0;
pub const RESULT_REJECTED: u32 = 1;
/// The provider or window went away, or the session locked.
pub const RESULT_GONE: u32 = 2;

// ----- events (window Event layout) ---------------------------------------
/// Assistive: a window's tree changed. window, a = version.
pub const TREE_CHANGED: u32 = 0x120;
/// Assistive: focus moved. window, value = node (0 none), a = version.
pub const FOCUS_CHANGED: u32 = 0x121;
/// Provider: perform an action. window, value = node, a = action code
/// (`door_accessibility::Action::code`), b = serial (ACTION_GET for args).
pub const ACTION: u32 = 0x122;
/// Assistive: value = serial, a = `RESULT_*`.
pub const ACTION_RESULT: u32 = 0x123;
/// Assistive: a = 1 access granted (unlock), 0 revoked (session locked).
pub const ACCESS_CHANGED: u32 = 0x124;
pub const EVENT_FIRST: u32 = 0x120;
pub const EVENT_LAST: u32 = 0x12F;

/// Decode an accessibility event (clients that negotiated a capability).
pub fn decode(b: &[u8]) -> Result<Event, Code> {
    crate::clipboard::decode_event(b, TREE_CHANGED..=ACCESS_CHANGED)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// PUBLISH_BEGIN / PUBLISH_SECTION body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Publish {
    pub window: u64,
    pub version: u32,
    pub kind: u32,
    pub bytes: u32,
}
impl Publish {
    pub const BYTES: usize = 20;
    pub fn encode(self) -> [u8; Self::BYTES] {
        let mut o = [0; Self::BYTES];
        o[..8].copy_from_slice(&self.window.to_le_bytes());
        o[8..12].copy_from_slice(&self.version.to_le_bytes());
        o[12..16].copy_from_slice(&self.kind.to_le_bytes());
        o[16..20].copy_from_slice(&self.bytes.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let p = Self {
            window: u64_at(b, 0),
            version: u32_at(b, 8),
            kind: u32_at(b, 12),
            bytes: u32_at(b, 16),
        };
        if p.window == 0
            || p.version == 0
            || !matches!(p.kind, KIND_FULL | KIND_DELTA)
            || p.bytes == 0
            || p.bytes as usize > MAX_MESSAGE_BYTES
        {
            return Err(Code::InvalidRequest);
        }
        Ok(p)
    }
    /// The header of an encoded wire message (magic "DA11", format 1).
    pub fn of_message(window: u64, message: &[u8]) -> Result<Self, Code> {
        if message.len() < 40
            || message[..4] != *b"DA11"
            || u16::from_le_bytes([message[4], message[5]]) != 1
        {
            return Err(Code::InvalidRequest);
        }
        let kind = u32::from(u16::from_le_bytes([message[6], message[7]]));
        let p = Self {
            window,
            version: u32_at(message, 16),
            kind,
            bytes: message.len() as u32,
        };
        if u64_at(message, 8) != window {
            return Err(Code::InvalidRequest);
        }
        Self::decode(&p.encode())
    }
}
/// Chunk plan for a staged message: (offset, length) pairs.
pub fn chunks(total: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..total.div_ceil(CHUNK_BYTES))
        .map(move |i| (i * CHUNK_BYTES, CHUNK_BYTES.min(total - i * CHUNK_BYTES)))
}

/// FOCUS_GET reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    pub window: u64,
    pub node: u64,
    pub version: u32,
    pub flags: u32,
}
impl Focus {
    pub const BYTES: usize = 24;
    pub fn encode(self) -> [u8; Self::BYTES] {
        let mut o = [0; Self::BYTES];
        o[..8].copy_from_slice(&self.window.to_le_bytes());
        o[8..16].copy_from_slice(&self.node.to_le_bytes());
        o[16..20].copy_from_slice(&self.version.to_le_bytes());
        o[20..24].copy_from_slice(&self.flags.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            window: u64_at(b, 0),
            node: u64_at(b, 8),
            version: u32_at(b, 16),
            flags: u32_at(b, 20),
        })
    }
}

/// SNAPSHOT request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub window: u64,
    pub scope: u32,
    pub node: u64,
}
impl Snapshot {
    pub const BYTES: usize = 20;
    pub fn encode(self) -> [u8; Self::BYTES] {
        let mut o = [0; Self::BYTES];
        o[..8].copy_from_slice(&self.window.to_le_bytes());
        o[8..12].copy_from_slice(&self.scope.to_le_bytes());
        o[12..20].copy_from_slice(&self.node.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let s = Self {
            window: u64_at(b, 0),
            scope: u32_at(b, 8),
            node: u64_at(b, 12),
        };
        if s.window == 0 || s.scope > SCOPE_NODE || (s.scope == SCOPE_NODE) != (s.node != 0) {
            return Err(Code::InvalidRequest);
        }
        Ok(s)
    }
}

/// INVOKE request and ACTION_GET reply: the action and its argument
/// (SET_VALUE: UTF-8 text; SET_RANGE: i64 milli-units; SET_SELECTION: two
/// u32 offsets; others: empty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionRequest<'a> {
    pub window: u64,
    pub node: u64,
    pub action: u32,
    pub arg: &'a [u8],
}
/// Action codes (bit index of `door_accessibility::Actions`).
pub const ACTION_FOCUS: u32 = 0;
pub const ACTION_INVOKE: u32 = 1;
pub const ACTION_TOGGLE: u32 = 2;
pub const ACTION_EXPAND: u32 = 3;
pub const ACTION_COLLAPSE: u32 = 4;
pub const ACTION_SELECT: u32 = 5;
pub const ACTION_SET_VALUE: u32 = 6;
pub const ACTION_INCREMENT: u32 = 7;
pub const ACTION_DECREMENT: u32 = 8;
pub const ACTION_SCROLL_INTO_VIEW: u32 = 9;
pub const ACTION_SHOW_MENU: u32 = 10;
pub const ACTION_DISMISS: u32 = 11;
pub const ACTION_SET_RANGE: u32 = 12;
pub const ACTION_SET_SELECTION: u32 = 13;
impl<'a> ActionRequest<'a> {
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        let n = 20 + self.arg.len();
        if self.arg.len() > ARG_BYTES || out.len() < n {
            return Err(Code::InvalidRequest);
        }
        out[..8].copy_from_slice(&self.window.to_le_bytes());
        out[8..16].copy_from_slice(&self.node.to_le_bytes());
        out[16..20].copy_from_slice(&self.action.to_le_bytes());
        out[20..n].copy_from_slice(self.arg);
        Ok(n)
    }
    pub fn decode(b: &'a [u8]) -> Result<Self, Code> {
        if b.len() < 20 || b.len() > 20 + ARG_BYTES {
            return Err(Code::InvalidRequest);
        }
        let r = Self {
            window: u64_at(b, 0),
            node: u64_at(b, 8),
            action: u32_at(b, 16),
            arg: &b[20..],
        };
        let arg_ok = match r.action {
            ACTION_SET_VALUE => core::str::from_utf8(r.arg).is_ok_and(|s| !s.contains('\0')),
            ACTION_SET_RANGE => r.arg.len() == 8,
            ACTION_SET_SELECTION => r.arg.len() == 8,
            0..=ACTION_SET_SELECTION => r.arg.is_empty(),
            _ => false,
        };
        if r.window == 0 || r.node == 0 || !arg_ok {
            return Err(Code::InvalidRequest);
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    #[test]
    fn requests_roundtrip_and_reject_malformed_bytes() {
        let p = Publish {
            window: 7,
            version: 3,
            kind: KIND_DELTA,
            bytes: 900,
        };
        assert_eq!(Publish::decode(&p.encode()), Ok(p));
        assert!(Publish::decode(&Publish { kind: 9, ..p }.encode()).is_err());
        assert!(
            Publish::decode(
                &Publish {
                    bytes: MAX_MESSAGE_BYTES as u32 + 1,
                    ..p
                }
                .encode()
            )
            .is_err()
        );
        assert!(Publish::decode(&p.encode()[..19]).is_err());
        let total = CHUNK_BYTES * 2 + 5;
        let plan: std::vec::Vec<_> = chunks(total).collect();
        assert_eq!(
            plan,
            [
                (0, CHUNK_BYTES),
                (CHUNK_BYTES, CHUNK_BYTES),
                (2 * CHUNK_BYTES, 5)
            ]
        );
        let f = Focus {
            window: 1,
            node: 2,
            version: 3,
            flags: WINDOW_FOCUSED,
        };
        assert_eq!(Focus::decode(&f.encode()), Ok(f));
        let s = Snapshot {
            window: 4,
            scope: SCOPE_NODE,
            node: 9,
        };
        assert_eq!(Snapshot::decode(&s.encode()), Ok(s));
        assert!(Snapshot::decode(&Snapshot { node: 0, ..s }.encode()).is_err());
        assert!(Snapshot::decode(&Snapshot { scope: 7, ..s }.encode()).is_err());
        let mut buf = [0u8; BODY_BYTES];
        let a = ActionRequest {
            window: 4,
            node: 9,
            action: ACTION_SET_VALUE,
            arg: b"Alice",
        };
        let n = a.encode(&mut buf).unwrap();
        assert_eq!(ActionRequest::decode(&buf[..n]), Ok(a));
        let bad = ActionRequest {
            action: ACTION_INVOKE,
            ..a
        };
        let n = bad.encode(&mut buf).unwrap();
        assert!(
            ActionRequest::decode(&buf[..n]).is_err(),
            "invoke takes no argument"
        );
        let range = ActionRequest {
            action: ACTION_SET_RANGE,
            arg: &5000i64.to_le_bytes(),
            ..a
        };
        let n = range.encode(&mut buf).unwrap();
        assert!(ActionRequest::decode(&buf[..n]).is_ok());
        let e = Event {
            kind: FOCUS_CHANGED,
            window: 4,
            value: 9,
            a: 3,
            ..Event::default()
        };
        assert_eq!(decode(&e.encode()), Ok(e));
        assert!(decode(&Event { kind: 0x100, ..e }.encode()).is_err());
        assert!(
            crate::window::Event::decode(&e.encode()).is_err(),
            "never delivered to window-only clients"
        );
        let mut message = [0u8; 40];
        message[..4].copy_from_slice(b"DA11");
        message[4] = 1;
        message[6] = 1;
        message[8..16].copy_from_slice(&4u64.to_le_bytes());
        message[16..20].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            Publish::of_message(4, &message),
            Ok(Publish {
                window: 4,
                version: 2,
                kind: KIND_FULL,
                bytes: 40
            })
        );
        assert!(
            Publish::of_message(5, &message).is_err(),
            "window must match"
        );
    }
}
