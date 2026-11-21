#![no_std]
//! Original rpcrt4.dll (P18): UUIDs (random version 4 from the kernel
//! entropy source, string conversion both ways). Doors has no RPC runtime;
//! only the UUID and RPC string functions are exported.
//!
//! The same sources build the x64 and the x86 DLL.

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// Marks 16 random bytes as an RFC 4122 version 4 UUID (in GUID byte
/// order: Data3's high nibble is the version).
pub fn version4(b: &mut [u8; 16]) {
    b[7] = (b[7] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
}
fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}
/// "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" as a UUID (GUID byte order).
pub fn parse(t: &[u8; 36]) -> Option<[u8; 16]> {
    if [8, 13, 18, 23].iter().any(|&i| t[i] != b'-') {
        return None;
    }
    let digits: [u8; 32] = {
        let mut d = [0u8; 32];
        let mut n = 0;
        for (i, &c) in t.iter().enumerate() {
            if [8, 13, 18, 23].contains(&i) {
                continue;
            }
            d[n] = hex(c)?;
            n += 1;
        }
        d
    };
    let byte = |i: usize| digits[2 * i] << 4 | digits[2 * i + 1];
    let mut b = [0u8; 16];
    // Data1 (4 bytes), Data2, Data3 little-endian; Data4 as written.
    for (to, from) in [
        (0, 3),
        (1, 2),
        (2, 1),
        (3, 0),
        (4, 5),
        (5, 4),
        (6, 7),
        (7, 6),
    ] {
        b[to] = byte(from);
    }
    for (i, slot) in b.iter_mut().enumerate().skip(8) {
        *slot = byte(i);
    }
    Some(b)
}
/// The lower-case text of a UUID.
pub fn format(b: &[u8; 16]) -> [u8; 36] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = [0u8; 36];
    let mut n = 0;
    for (k, &i) in order.iter().enumerate() {
        if [4, 6, 8, 10].contains(&k) {
            out[n] = b'-';
            n += 1;
        }
        out[n] = HEX[usize::from(b[i] >> 4)];
        out[n + 1] = HEX[usize::from(b[i] & 0xF)];
        n += 2;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_text() {
        let text = *b"6b29fc40-ca47-1067-b31d-00dd010662da";
        let uuid = parse(&text).unwrap();
        assert_eq!(&uuid[..4], &[0x40, 0xfc, 0x29, 0x6b]);
        assert_eq!(uuid[8], 0xb3);
        assert_eq!(format(&uuid), text);
        assert!(parse(b"6b29fc40-ca47-1067-b31d_00dd010662da").is_none());
        let mut r = [0xFFu8; 16];
        version4(&mut r);
        assert_eq!(format(&r)[14], b'4');
    }
}
