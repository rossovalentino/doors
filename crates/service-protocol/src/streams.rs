//! Alternate data streams (`file:stream`, NTFS's named streams). Doors
//! keeps a file's named streams as small files in its user's profile,
//! named by the file's storage object ID, which stays with the file when
//! it is renamed or moved: `/users/<uid>/AppData/Local/Doors/Streams/
//! <object as 16 hex digits>.<stream name>`. The mark of the web is the
//! `Zone.Identifier` stream a downloader writes (`[ZoneTransfer]` with
//! `ZoneId=3` for the Internet).

/// Folders under the profile, outermost first.
pub const FOLDERS: [&str; 2] = ["AppData/Local/Doors", "AppData/Local/Doors/Streams"];
/// The mark of the web.
pub const ZONE_IDENTIFIER: &str = "Zone.Identifier";
/// URLZONE_INTERNET and URLZONE_UNTRUSTED: the zones Windows warns about.
pub const INTERNET: u8 = 3;
pub const UNTRUSTED: u8 = 4;
/// Longest stream name kept (the storage name limit is 48 units: 16 hex
/// digits and a dot come first).
pub const NAME_MAX: usize = 31;

/// A stream name Doors can keep: 1..=31 of `A-Z a-z 0-9 . _ -` and spaces.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b' '))
}
struct Out<'a> {
    buffer: &'a mut [u8],
    len: usize,
}
impl core::fmt::Write for Out<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        self.buffer
            .get_mut(self.len..end)
            .ok_or(core::fmt::Error)?
            .copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
/// `/users/<uid>/<FOLDERS[level]>` into `out`: its length.
pub fn folder(uid: u32, level: usize, out: &mut [u8]) -> Option<usize> {
    let mut o = Out {
        buffer: out,
        len: 0,
    };
    core::fmt::Write::write_fmt(&mut o, format_args!("/users/{uid}/{}", FOLDERS.get(level)?))
        .ok()?;
    Some(o.len)
}
/// The file holding stream `name` of storage object `object`: its length.
pub fn path(uid: u32, object: u64, name: &str, out: &mut [u8]) -> Option<usize> {
    if !valid_name(name) {
        return None;
    }
    let mut o = Out {
        buffer: out,
        len: 0,
    };
    core::fmt::Write::write_fmt(
        &mut o,
        format_args!("/users/{uid}/{}/{object:016x}.{name}", FOLDERS[1]),
    )
    .ok()?;
    Some(o.len)
}
/// The stream name a file in the streams folder holds for `object`.
pub fn stream_of(file_name: &str, object: u64) -> Option<&str> {
    let (id, name) = file_name.split_once('.')?;
    (id.len() == 16 && u64::from_str_radix(id, 16).ok()? == object && valid_name(name))
        .then_some(name)
}
/// The zone a `Zone.Identifier` stream names (`ZoneId=` in its
/// `[ZoneTransfer]` section).
pub fn zone(text: &[u8]) -> Option<u8> {
    let text = core::str::from_utf8(text).ok()?;
    let mut section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.eq_ignore_ascii_case("[ZoneTransfer]");
        } else if section
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("ZoneId")
        {
            return value.trim().parse().ok().filter(|z| *z <= 4);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_files_are_named_by_object_and_zones_parse_as_windows_writes_them() {
        let mut out = [0u8; 128];
        let n = path(1001, 0x2a, ZONE_IDENTIFIER, &mut out).unwrap();
        assert_eq!(
            core::str::from_utf8(&out[..n]).unwrap(),
            "/users/1001/AppData/Local/Doors/Streams/000000000000002a.Zone.Identifier"
        );
        let n = folder(1001, 1, &mut out).unwrap();
        assert_eq!(&out[..n], b"/users/1001/AppData/Local/Doors/Streams");
        assert_eq!(
            stream_of("000000000000002a.Zone.Identifier", 0x2a),
            Some("Zone.Identifier")
        );
        assert_eq!(stream_of("000000000000002b.Zone.Identifier", 0x2a), None);
        assert!(path(1, 1, "a/b", &mut out).is_none());
        assert!(path(1, 1, "", &mut out).is_none());
        assert!(!valid_name(&"x".repeat(NAME_MAX + 1)));
        assert_eq!(
            zone(b"[ZoneTransfer]\r\nReferrerUrl=https://example.com/\r\nZoneId=3\r\n"),
            Some(3)
        );
        assert_eq!(zone(b"[Other]\r\nZoneId=3\r\n"), None);
        assert_eq!(zone(b"[ZoneTransfer]\nzoneid = 4\n"), Some(4));
        assert_eq!(zone(b"[ZoneTransfer]\r\nZoneId=9\r\n"), None);
    }
}
