//! Immutable application resource pack (fonts, icon masks and bitmaps) that
//! the kernel validates once from the authenticated initfs and shares with
//! every desktop application process as one read-only section. No allocation,
//! no paths outside the pack, lookup by exact key.
//!
//! Layout (little endian):
//! ```text
//! 0   magic "DOORAST1"
//! 8   u32 entry count n (1..=MAX_ENTRIES)
//! 12  u32 total length L (exact; any trailing bytes up to the section are 0)
//! 16  n x 16-byte entries, sorted by key bytes, unique:
//!       u32 key offset, u32 key length (1..=MAX_KEY), u32 data offset, u32 data length
//! ..  keys (ASCII [A-Za-z0-9._/-], no leading '/', no "..", no "//"), then data
//! ```
//! Every key and data range lies inside `[HEADER + 16 n, L)`, ranges never
//! overlap and appear in entry order, so a valid pack has one interpretation.
pub const MAGIC: &[u8; 8] = b"DOORAST1";
pub const HEADER: usize = 16;
pub const ENTRY: usize = 16;
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_KEY: usize = 128;
/// One shared section; equal to the graphics section ceiling.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Pack<'a> {
    bytes: &'a [u8],
    count: usize,
}
fn word(bytes: &[u8], at: usize) -> Option<usize> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
}
/// Pack keys: relative asset paths such as
/// `icons/windows-shell/app-add.20.mask`.
pub fn valid_key(key: &[u8]) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && key[0] != b'/'
        && key[key.len() - 1] != b'/'
        && key
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'/'))
        && !key.windows(2).any(|w| w == b"//" || w == b"..")
}
impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        if bytes.get(..8)? != MAGIC {
            return None;
        }
        let count = word(bytes, 8)?;
        let length = word(bytes, 12)?;
        let table = HEADER.checked_add(count.checked_mul(ENTRY)?)?;
        if !(1..=MAX_ENTRIES).contains(&count)
            || length > MAX_BYTES
            || length > bytes.len()
            || table > length
            || bytes[length..].iter().any(|&v| v != 0)
        {
            return None;
        }
        // Keys first, then data, each contiguous in entry order.
        let mut end = table;
        for i in 0..count {
            let at = HEADER + i * ENTRY;
            let (offset, size) = (word(bytes, at)?, word(bytes, at + 4)?);
            if offset != end || !valid_key(bytes.get(offset..offset.checked_add(size)?)?) {
                return None;
            }
            end += size;
            if i > 0 {
                let previous = HEADER + (i - 1) * ENTRY;
                let before = &bytes
                    [word(bytes, previous)?..word(bytes, previous)? + word(bytes, previous + 4)?];
                if before >= &bytes[offset..offset + size] {
                    return None;
                }
            }
        }
        for i in 0..count {
            let at = HEADER + i * ENTRY + 8;
            let (offset, size) = (word(bytes, at)?, word(bytes, at + 4)?);
            if offset != end {
                return None;
            }
            end = offset.checked_add(size)?;
        }
        (end == length).then_some(Self {
            bytes: &bytes[..length],
            count,
        })
    }
    pub fn len(self) -> usize {
        self.count
    }
    pub fn is_empty(self) -> bool {
        self.count == 0
    }
    fn key(self, i: usize) -> &'a [u8] {
        let at = HEADER + i * ENTRY;
        let (offset, size) = (
            word(self.bytes, at).unwrap(),
            word(self.bytes, at + 4).unwrap(),
        );
        &self.bytes[offset..offset + size]
    }
    pub fn entry(self, i: usize) -> Option<(&'a str, &'a [u8])> {
        if i >= self.count {
            return None;
        }
        let at = HEADER + i * ENTRY + 8;
        let (offset, size) = (word(self.bytes, at)?, word(self.bytes, at + 4)?);
        Some((
            core::str::from_utf8(self.key(i)).ok()?,
            &self.bytes[offset..offset + size],
        ))
    }
    /// The bytes stored under `key`, by binary search over sorted keys.
    pub fn get(self, key: &str) -> Option<&'a [u8]> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.key(mid).cmp(key.as_bytes()) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return self.entry(mid).map(|e| e.1),
            }
        }
        None
    }
}
/// Pack key of a source-relative asset path: everything after the last
/// `assets/` component, so `../../../assets/icons/x.mask` and
/// `../../../../../assets/icons/x.mask` name the same entry.
pub fn key_of(path: &str) -> &str {
    path.rfind("assets/").map_or(path, |at| &path[at + 7..])
}
/// Exact pack length for `entries`, or None when they cannot form a pack
/// (count, key or size limits; keys must be valid, sorted and unique).
pub fn encoded_len(entries: &[(&str, &[u8])]) -> Option<usize> {
    if !(1..=MAX_ENTRIES).contains(&entries.len())
        || entries.iter().any(|(k, _)| !valid_key(k.as_bytes()))
        || entries
            .windows(2)
            .any(|w| w[0].0.as_bytes() >= w[1].0.as_bytes())
    {
        return None;
    }
    let mut length = HEADER + entries.len() * ENTRY;
    for (k, d) in entries {
        length = length.checked_add(k.len())?.checked_add(d.len())?;
    }
    (length <= MAX_BYTES && u32::try_from(length).is_ok()).then_some(length)
}
/// Serialize `entries` (the build side). `out` must be exactly
/// `encoded_len(entries)` bytes; the result always parses.
pub fn encode(entries: &[(&str, &[u8])], out: &mut [u8]) -> Option<()> {
    if encoded_len(entries)? != out.len() {
        return None;
    }
    let put = |out: &mut [u8], at: usize, v: usize| {
        out[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes())
    };
    out[..8].copy_from_slice(MAGIC);
    put(out, 8, entries.len());
    put(out, 12, out.len());
    let mut at = HEADER + entries.len() * ENTRY;
    for (i, (k, _)) in entries.iter().enumerate() {
        put(out, HEADER + i * ENTRY, at);
        put(out, HEADER + i * ENTRY + 4, k.len());
        out[at..at + k.len()].copy_from_slice(k.as_bytes());
        at += k.len();
    }
    for (i, (_, d)) in entries.iter().enumerate() {
        put(out, HEADER + i * ENTRY + 8, at);
        put(out, HEADER + i * ENTRY + 12, d.len());
        out[at..at + d.len()].copy_from_slice(d);
        at += d.len();
    }
    Some(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{vec, vec::Vec};
    /// Unchecked layout writer, so malformed orderings can be tested too.
    pub fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut at = HEADER + entries.len() * ENTRY;
        let mut keys = Vec::new();
        for (k, _) in entries {
            keys.push((at, k.len()));
            at += k.len();
        }
        let mut data = Vec::new();
        for (_, d) in entries {
            data.push((at, d.len()));
            at += d.len();
        }
        for i in 0..entries.len() {
            for v in [keys[i].0, keys[i].1, data[i].0, data[i].1] {
                out.extend_from_slice(&(v as u32).to_le_bytes());
            }
        }
        for (k, _) in entries {
            out.extend_from_slice(k.as_bytes());
        }
        for (_, d) in entries {
            out.extend_from_slice(d);
        }
        let n = out.len() as u32;
        out[12..16].copy_from_slice(&n.to_le_bytes());
        out
    }
    #[test]
    fn lookups_find_exact_keys_and_reject_malformed_packs() {
        let entries: [(&str, &[u8]); 3] = [
            ("fonts/a.dfa", b"AAA"),
            ("icons/b.20.mask", b"BB"),
            ("icons/c.bgra", b""),
        ];
        let mut bytes = build(&entries);
        let pack = Pack::parse(&bytes).unwrap();
        assert_eq!(pack.len(), 3);
        for (k, d) in entries {
            assert_eq!(pack.get(k), Some(d));
        }
        assert_eq!(pack.get("icons/b"), None);
        assert_eq!(
            pack.get(key_of("../../../assets/icons/b.20.mask")),
            Some(&b"BB"[..])
        );
        // Section padding is zero; anything else is rejected.
        bytes.extend_from_slice(&[0; 7]);
        assert!(Pack::parse(&bytes).is_some());
        bytes.push(1);
        assert!(Pack::parse(&bytes).is_none());
        // Unsorted, duplicate, unsafe keys and overlapping ranges fail.
        for bad in [
            build(&[("b", b"1"), ("a", b"2")]),
            build(&[("a", b"1"), ("a", b"2")]),
            build(&[("../x", b"1")]),
            build(&[("a x", b"1")]),
            build(&[("/x", b"1")]),
        ] {
            assert!(Pack::parse(&bad).is_none());
        }
        let good = build(&entries);
        for at in [8, 12, 16, 20, 24, 28] {
            let mut bad = good.clone();
            bad[at] ^= 1;
            assert!(Pack::parse(&bad).is_none(), "byte {at}");
        }
        assert!(Pack::parse(&good[..good.len() - 1]).is_none());
    }
    #[test]
    fn encoder_output_round_trips_and_matches_the_reference_layout() {
        let entries: [(&str, &[u8]); 3] = [
            ("fonts/cascadia-mono/CascadiaMono.ttf", b"TTF"),
            ("icons/b.20.mask", b"BB"),
            ("icons/c.bgra", b""),
        ];
        let mut out = vec![0; encoded_len(&entries).unwrap()];
        encode(&entries, &mut out).unwrap();
        assert_eq!(out, build(&entries));
        let pack = Pack::parse(&out).unwrap();
        for (i, (k, d)) in entries.iter().enumerate() {
            assert_eq!(pack.entry(i), Some((*k, *d)));
        }
        for bad in [
            &[][..],
            &[("b", &b"1"[..]), ("a", b"2")][..],
            &[("a", &b"1"[..]), ("a", b"2")][..],
            &[("a/../b", &b"1"[..])][..],
        ] {
            assert_eq!(encoded_len(bad), None);
        }
        assert_eq!(encode(&entries, &mut vec![0; out.len() + 1]), None);
    }
}
