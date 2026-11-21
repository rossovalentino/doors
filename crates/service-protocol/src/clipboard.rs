//! door.clipboard v1 (`specs/contracts/native-ui-v1.md`). The window server
//! brokers one clipboard per signed-in session, as Windows does. Data is
//! copied at set: COMMIT validates the staged formats and the server keeps a
//! private copy, so readers never call back into the owner, a hung or dead
//! owner cannot block a paste, and the data outlives its owner (Windows keeps
//! non-delayed clipboard data when the owner exits). Owners come from
//! authenticated channel metadata, never from these bytes.
//!
//! Staging is shared with drag and drop (`dnd::BEGIN` opens a drag staging):
//! BEGIN, then per format FORMAT (declared size) and its bytes through WRITE
//! chunks or one WRITE_SECTION snapshot, then COMMIT (or `dnd::START`).
use crate::window::{EVENT_BYTES, Event, Rect};
use crate::{BODY_BYTES, Code};
/// HELLO capability (bit 20; bits 20..=23 are reserved for native-ui):
/// clipboard requests and CHANGED events.
pub const CAP_CLIPBOARD: u32 = 1 << 20;
/// Opcodes 0x5100..=0x517F are reserved for the clipboard.
pub const FIRST: u32 = 0x5100;
pub const LAST: u32 = 0x517F;
/// `[window u64]`: open a staging set for the caller's window (replaces the
/// caller's earlier uncommitted staging).
pub const BEGIN: u32 = 0x5100;
/// `[format u32][bytes u32]`: declare one format and its exact size.
pub const FORMAT: u32 = 0x5101;
/// `[format u32][offset u32][bytes]`: append `WRITE_BYTES` at most, in order.
pub const WRITE: u32 = 0x5102;
/// `[format u32][offset u32][bytes u32]` plus one section handle with READ:
/// the server snapshots `bytes` from section offset 0 into the format.
pub const WRITE_SECTION: u32 = 0x5103;
/// Empty: replace the clipboard with the staged formats (none: empty the
/// clipboard, like EmptyClipboard). Reply `[sequence u32]`.
pub const COMMIT: u32 = 0x5104;
/// Empty: discard the caller's staging.
pub const ABORT: u32 = 0x5105;
/// Empty: reply `Info` (`INFO_BYTES`).
pub const INFO: u32 = 0x5106;
/// `[sequence u32][format u32][offset u32]`: up to `READ_BYTES` of a format.
/// Only the owner of the focused window may read (Changed when `sequence`
/// is no longer current).
pub const READ: u32 = 0x5107;
/// Empty: reply `[settings u32]`.
pub const SETTINGS_GET: u32 = 0x5108;
/// `[settings u32]`: Settings > System > Clipboard (workspace apps only).
/// Turning history off clears it, pinned items included (Windows 11).
pub const SETTINGS_SET: u32 = 0x5109;
/// Empty: Settings > System > Clipboard > Clear clipboard data: the current
/// clipboard and every unpinned history item (workspace apps only).
pub const CLEAR_DATA: u32 = 0x510a;
/// Largest WRITE chunk and READ reply.
pub const WRITE_BYTES: usize = BODY_BYTES - 8;
pub const READ_BYTES: usize = BODY_BYTES;
/// Formats (Windows counterparts in brackets). A set carries each at most once.
/// UTF-8 text without NUL [CF_UNICODETEXT].
pub const TEXT: u32 = 1;
/// NUL-separated absolute Door paths [CF_HDROP]. Paths only: a reader opens
/// them with its own identity, so file permissions are never bypassed.
pub const FILES: u32 = 2;
/// `[width u32][height u32]` then premultiplied BGRA rows [CF_DIBV5].
pub const IMAGE: u32 = 3;
/// PNG file bytes ["PNG"]. The server checks the signature, never decodes.
pub const PNG: u32 = 4;
/// UTF-8 HTML fragment ["HTML Format"].
pub const HTML: u32 = 5;
/// One effect u32 (`dnd::COPY` or `dnd::MOVE`) ["Preferred DropEffect"]:
/// File Explorer Cut is FILES with MOVE.
pub const DROP_EFFECT: u32 = 6;
pub const FORMATS: [u32; 6] = [TEXT, FILES, IMAGE, PNG, HTML, DROP_EFFECT];
pub const fn mask(format: u32) -> u32 {
    if format >= 1 && format <= DROP_EFFECT {
        1 << format
    } else {
        0
    }
}
pub const ALL_FORMATS: u32 = 0b111_1110;
/// One clipboard set (all formats together), and the largest PNG/text/HTML.
pub const MAX_CONTENT: usize = 8 * 1024 * 1024;
pub const MAX_IMAGE_SIDE: u32 = 8192;
pub const MAX_FILES: usize = 1024;
pub const MAX_FILE_PATH: usize = 1024;
pub const MAX_FILES_BYTES: usize = 256 * 1024;
/// Clipboard history (Windows 11): 25 items of at most 4 MB each, text, HTML
/// and bitmaps.
pub const HISTORY_ITEMS: usize = 25;
pub const HISTORY_ITEM_BYTES: usize = 4 * 1024 * 1024;
/// Everything the broker holds (clipboard, history, staging, drag offers).
pub const MAX_BROKER_BYTES: usize = 32 * 1024 * 1024;
/// Settings bits. Windows defaults: both off.
pub const SETTING_HISTORY: u32 = 1;
/// "Clipboard history across your devices": Doors has no cloud; it keeps
/// pinned items on this device only.
pub const SETTING_SYNC: u32 = 2;
pub const SETTINGS_KNOWN: u32 = SETTING_HISTORY | SETTING_SYNC;
/// Event kind (window Event layout, CAP_CLIPBOARD clients only): the
/// clipboard changed. a = sequence, b = format mask, detail = `Owner`.
pub const CHANGED: u32 = 0x100;
/// Owner state in `Info` and CHANGED.detail.
pub const OWNER_NONE: u32 = 0;
pub const OWNER_ALIVE: u32 = 1;
/// The owner exited; its copied data stays (Windows behaviour).
pub const OWNER_EXITED: u32 = 2;
pub const INFO_BYTES: usize = 12 + 4 * FORMATS.len();
/// GetClipboardSequenceNumber, the available formats, GetClipboardOwner and
/// each format's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub sequence: u32,
    pub formats: u32,
    pub owner: u32,
    pub sizes: [u32; FORMATS.len()],
}
impl Info {
    pub fn encode(self) -> [u8; INFO_BYTES] {
        let mut o = [0; INFO_BYTES];
        o[..4].copy_from_slice(&self.sequence.to_le_bytes());
        o[4..8].copy_from_slice(&self.formats.to_le_bytes());
        o[8..12].copy_from_slice(&self.owner.to_le_bytes());
        for (i, size) in self.sizes.iter().enumerate() {
            o[12 + 4 * i..16 + 4 * i].copy_from_slice(&size.to_le_bytes());
        }
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != INFO_BYTES {
            return Err(Code::InvalidRequest);
        }
        let word = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let info = Self {
            sequence: word(0),
            formats: word(4),
            owner: word(8),
            sizes: core::array::from_fn(|i| word(12 + 4 * i)),
        };
        let consistent = FORMATS
            .iter()
            .zip(info.sizes)
            .all(|(f, size)| (info.formats & mask(*f) != 0) == (size != 0));
        if info.formats & !ALL_FORMATS != 0 || info.owner > OWNER_EXITED || !consistent {
            return Err(Code::InvalidRequest);
        }
        Ok(info)
    }
    pub fn size(&self, format: u32) -> u32 {
        FORMATS
            .iter()
            .position(|f| *f == format)
            .map_or(0, |i| self.sizes[i])
    }
}
/// Largest size a format may declare.
pub const fn max_bytes(format: u32) -> usize {
    match format {
        FILES => MAX_FILES_BYTES,
        IMAGE => 8 + MAX_CONTENT,
        DROP_EFFECT => 4,
        TEXT | PNG | HTML => MAX_CONTENT,
        _ => 0,
    }
}
fn utf8(b: &[u8]) -> Result<&str, Code> {
    let s = core::str::from_utf8(b).map_err(|_| Code::InvalidRequest)?;
    if s.contains('\0') {
        return Err(Code::InvalidRequest);
    }
    Ok(s)
}
/// Validate one complete format payload (COMMIT and `dnd::START`).
pub fn validate(format: u32, b: &[u8]) -> Result<(), Code> {
    if mask(format) == 0 {
        return Err(Code::Unsupported);
    }
    if b.is_empty() || b.len() > max_bytes(format) {
        return Err(Code::InvalidRequest);
    }
    match format {
        TEXT | HTML => utf8(b).map(|_| ()),
        FILES => files(b).try_for_each(|p| p.map(|_| ())),
        IMAGE => {
            let (w, h) = image_size(b)?;
            if b.len() != 8 + (w as usize * h as usize * 4) {
                return Err(Code::InvalidRequest);
            }
            if b[8..]
                .chunks_exact(4)
                .any(|p| p[0].max(p[1]).max(p[2]) > p[3])
            {
                return Err(Code::InvalidRequest);
            }
            Ok(())
        }
        PNG => (b.len() > 8 && b[..8] == *b"\x89PNG\r\n\x1a\n")
            .then_some(())
            .ok_or(Code::InvalidRequest),
        DROP_EFFECT => match u32::from_le_bytes(b.try_into().map_err(|_| Code::InvalidRequest)?) {
            1 | 2 => Ok(()),
            _ => Err(Code::InvalidRequest),
        },
        _ => Err(Code::Unsupported),
    }
}
/// `[width][height]` of an IMAGE payload (1..=`MAX_IMAGE_SIDE` each).
pub fn image_size(b: &[u8]) -> Result<(u32, u32), Code> {
    if b.len() < 8 {
        return Err(Code::InvalidRequest);
    }
    let w = u32::from_le_bytes(b[..4].try_into().unwrap());
    let h = u32::from_le_bytes(b[4..8].try_into().unwrap());
    if !(1..=MAX_IMAGE_SIDE).contains(&w) || !(1..=MAX_IMAGE_SIDE).contains(&h) {
        return Err(Code::InvalidRequest);
    }
    Ok((w, h))
}
/// The paths of a FILES payload, each checked: absolute, at most
/// `MAX_FILE_PATH` bytes, no control characters, at most `MAX_FILES`.
pub fn files(b: &[u8]) -> impl Iterator<Item = Result<&str, Code>> {
    let count = b.split(|c| *c == 0).count();
    b.split(|c| *c == 0).map(move |p| {
        let s = utf8(p)?;
        if count > MAX_FILES
            || !s.starts_with('/')
            || s.len() > MAX_FILE_PATH
            || s.chars().any(char::is_control)
        {
            return Err(Code::InvalidRequest);
        }
        Ok(s)
    })
}
/// Encode a FILES payload into `out`; returns its length.
pub fn encode_files(paths: &[&str], out: &mut [u8]) -> Result<usize, Code> {
    let mut n = 0;
    for (i, p) in paths.iter().enumerate() {
        let sep = usize::from(i > 0);
        let end = n + sep + p.len();
        if end > out.len() {
            return Err(Code::Resources);
        }
        if sep == 1 {
            out[n] = 0;
        }
        out[n + sep..end].copy_from_slice(p.as_bytes());
        n = end;
    }
    validate(FILES, &out[..n])?;
    Ok(n)
}
/// Decode a native-ui event (window Event layout) whose kind is in `kinds`.
pub(crate) fn decode_event(b: &[u8], kinds: core::ops::RangeInclusive<u32>) -> Result<Event, Code> {
    if b.len() != EVENT_BYTES || b[56..].iter().any(|&v| v != 0) {
        return Err(Code::InvalidRequest);
    }
    let word = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let long = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    if !kinds.contains(&word(0)) {
        return Err(Code::InvalidRequest);
    }
    Ok(Event {
        kind: word(0),
        detail: word(4),
        window: long(8),
        value: long(16),
        rect: Rect::decode(&b[24..40])?,
        a: word(40),
        b: word(44),
        timestamp: long(48),
    })
}
/// CHANGED events (clients that negotiated CAP_CLIPBOARD).
pub fn decode(b: &[u8]) -> Result<Event, Code> {
    decode_event(b, CHANGED..=CHANGED)
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    #[test]
    fn formats_are_validated_before_the_server_keeps_them() {
        assert!(validate(TEXT, "Hello, 世界".as_bytes()).is_ok());
        assert!(validate(TEXT, b"a\0b").is_err());
        assert!(validate(TEXT, &[0xff, 0xfe]).is_err());
        assert!(validate(TEXT, b"").is_err());
        assert!(validate(HTML, b"<b>x</b>").is_ok());
        let mut out = [0u8; 64];
        let n = encode_files(&["/users/1001/a.txt", "/users/1001/Pictures"], &mut out).unwrap();
        assert_eq!(
            files(&out[..n]).collect::<Result<Vec<_>, _>>().unwrap(),
            ["/users/1001/a.txt", "/users/1001/Pictures"]
        );
        assert!(validate(FILES, b"relative/path").is_err());
        assert!(validate(FILES, b"/a\0\0/b").is_err(), "empty entry");
        assert!(validate(FILES, b"/a\n").is_err());
        let mut image = [0u8; 8 + 2 * 4];
        image[..4].copy_from_slice(&2u32.to_le_bytes());
        image[4..8].copy_from_slice(&1u32.to_le_bytes());
        image[8..].copy_from_slice(&[10, 20, 30, 40, 0, 0, 0, 0]);
        assert!(validate(IMAGE, &image).is_ok());
        image[8] = 41;
        assert!(validate(IMAGE, &image).is_err(), "not premultiplied");
        assert!(validate(IMAGE, &image[..12]).is_err(), "short rows");
        assert!(validate(PNG, b"\x89PNG\r\n\x1a\n\0").is_ok());
        assert!(validate(PNG, b"GIF89a....").is_err());
        assert!(validate(DROP_EFFECT, &2u32.to_le_bytes()).is_ok());
        assert!(validate(DROP_EFFECT, &4u32.to_le_bytes()).is_err());
        assert_eq!(validate(9, b"x"), Err(Code::Unsupported));
        assert!(validate(TEXT, &std::vec![b'a'; MAX_CONTENT + 1]).is_err());
        assert!(validate(TEXT, &std::vec![b'a'; MAX_CONTENT]).is_ok());
    }
    #[test]
    fn info_and_events_roundtrip_and_reject_inconsistent_bytes() {
        let info = Info {
            sequence: 7,
            formats: mask(TEXT) | mask(HTML),
            owner: OWNER_EXITED,
            sizes: [5, 0, 0, 0, 12, 0],
        };
        assert_eq!(Info::decode(&info.encode()), Ok(info));
        assert_eq!(info.size(HTML), 12);
        let bad = Info {
            sizes: [0; 6],
            ..info
        };
        assert!(Info::decode(&bad.encode()).is_err());
        let e = Event {
            kind: CHANGED,
            a: 7,
            b: info.formats,
            detail: OWNER_ALIVE,
            ..Event::default()
        };
        assert_eq!(decode(&e.encode()), Ok(e));
        assert!(decode(&Event { kind: 3, ..e }.encode()).is_err());
        assert!(
            crate::window::Event::decode(&e.encode()).is_err(),
            "never delivered to window-only clients"
        );
    }
}
