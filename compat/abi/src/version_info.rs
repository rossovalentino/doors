//! A version resource (RT_VERSION's VS_VERSIONINFO) as the tree of blocks
//! VerQueryValue walks: each block is `wLength, wValueLength, wType`, a
//! NUL-terminated UTF-16 key, padding to 4 bytes, the value, padding to 4
//! bytes, then its children; offsets align from the resource's start. A
//! query is a backslash-separated key path ("\\", "\\VarFileInfo\\Translation",
//! "\\StringFileInfo\\040904B0\\ProductVersion") matched case-insensitively
//! one block at a time; empty components are skipped, as Windows does.

/// What a query found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Value {
    /// Byte offset of the value from the resource's start.
    pub offset: usize,
    /// wValueLength: characters (with the NUL) for text, bytes otherwise.
    pub length: u16,
    /// wType 1: the value is UTF-16 text.
    pub text: bool,
}

/// VS_FIXEDFILEINFO's signature (the root block's value).
pub const FIXED_SIGNATURE: u32 = 0xfeef_04bd;
/// The marker Windows writes after a copied 32-bit resource, before the
/// spare half VerQueryValueA converts text into.
pub const SPARE_SIGNATURE: [u8; 4] = *b"FE2X";

#[derive(Clone, Copy)]
struct Block {
    start: usize,
    end: usize,
    key: usize,
    key_units: usize,
    value: usize,
    value_length: u16,
    text: bool,
    children: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}
const fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}

/// The block at `start`, bounded by `limit` (its parent's end).
fn block(resource: &[u8], start: usize, limit: usize) -> Option<Block> {
    let length = usize::from(u16_at(resource, start)?);
    let end = start.checked_add(length)?;
    if length < 6 || end > limit || end > resource.len() {
        return None;
    }
    let value_length = u16_at(resource, start + 2)?;
    let text = u16_at(resource, start + 4)? == 1;
    let key = start + 6;
    let mut units = 0;
    while u16_at(resource, key + units * 2)? != 0 {
        units += 1;
        if key + units * 2 >= end {
            return None;
        }
    }
    let value = align4(key + units * 2 + 2);
    let value_bytes = usize::from(value_length) * if text { 2 } else { 1 };
    Some(Block {
        start,
        end,
        key,
        key_units: units,
        value,
        value_length,
        text,
        children: align4(value + value_bytes),
    })
}

fn fold(unit: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&unit) {
        unit - 0x20
    } else {
        unit
    }
}

fn child(resource: &[u8], parent: &Block, name: &[u16]) -> Option<Block> {
    let mut at = parent.children;
    while at + 6 <= parent.end {
        let candidate = block(resource, at, parent.end)?;
        let matches = candidate.key_units == name.len()
            && name.iter().enumerate().all(|(i, unit)| {
                u16_at(resource, candidate.key + i * 2).map(fold) == Some(fold(*unit))
            });
        if matches {
            return Some(candidate);
        }
        at = candidate.start + align4(candidate.end - candidate.start);
    }
    None
}

/// The resource's own length (the root's wLength), when it is well formed.
pub fn length(resource: &[u8]) -> Option<usize> {
    block(resource, 0, resource.len()).map(|b| b.end)
}

/// Look `path` (UTF-16, without its NUL) up in `resource`.
pub fn query(resource: &[u8], path: &[u16]) -> Option<Value> {
    let mut current = block(resource, 0, resource.len())?;
    for name in path
        .split(|unit| *unit == u16::from(b'\\'))
        .filter(|name| !name.is_empty())
    {
        current = child(resource, &current, name)?;
    }
    Some(Value {
        offset: current.value,
        length: current.value_length,
        text: current.text,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }
    /// A block: key, value bytes, text flag and children, padded as the
    /// resource compiler lays it out.
    fn node(key: &str, value: &[u8], text: bool, children: &[Vec<u8>]) -> Vec<u8> {
        let mut out = std::vec![0u8; 6];
        for unit in wide(key).into_iter().chain([0]) {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        out.extend_from_slice(value);
        for child in children {
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            out.extend_from_slice(child);
        }
        let value_length = if text { value.len() / 2 } else { value.len() } as u16;
        let length = out.len() as u16;
        out[0..2].copy_from_slice(&length.to_le_bytes());
        out[2..4].copy_from_slice(&value_length.to_le_bytes());
        out[4..6].copy_from_slice(&u16::from(text).to_le_bytes());
        out
    }
    fn text(value: &str) -> Vec<u8> {
        wide(value)
            .into_iter()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    fn resource() -> Vec<u8> {
        let mut fixed = std::vec![0u8; 52];
        fixed[..4].copy_from_slice(&FIXED_SIGNATURE.to_le_bytes());
        fixed[8..12].copy_from_slice(&0x000a_0000u32.to_le_bytes());
        let strings = node(
            "040904B0",
            &[],
            true,
            &[
                node("CompanyName", &text("Doors"), true, &[]),
                node("ProductVersion", &text("10.0.15"), true, &[]),
                node("Comments", &text(""), true, &[]),
            ],
        );
        let translation = node("Translation", &[0x09, 0x04, 0xb0, 0x04], false, &[]);
        node(
            "VS_VERSION_INFO",
            &fixed,
            false,
            &[
                node("StringFileInfo", &[], true, &[strings]),
                node("VarFileInfo", &[], true, &[translation]),
            ],
        )
    }

    #[test]
    fn the_root_is_the_fixed_file_information() {
        let r = resource();
        assert_eq!(length(&r), Some(r.len()));
        let value = query(&r, &wide("\\")).unwrap();
        assert_eq!((value.length, value.text), (52, false));
        assert_eq!(
            r[value.offset..value.offset + 4],
            FIXED_SIGNATURE.to_le_bytes()
        );
        assert_eq!(query(&r, &[]), Some(value));
    }

    #[test]
    fn strings_and_translations_resolve_by_case_insensitive_paths() {
        let r = resource();
        let value = query(&r, &wide("\\StringFileInfo\\040904b0\\productversion")).unwrap();
        assert!(value.text);
        assert_eq!(value.length, 8);
        assert_eq!(&r[value.offset..value.offset + 16], &text("10.0.15")[..]);
        let translation = query(&r, &wide("\\VarFileInfo\\\\Translation")).unwrap();
        assert_eq!((translation.length, translation.text), (4, false));
        assert_eq!(
            &r[translation.offset..translation.offset + 4],
            &[0x09, 0x04, 0xb0, 0x04]
        );
        let empty = query(&r, &wide("\\StringFileInfo\\040904B0\\Comments")).unwrap();
        assert_eq!(empty.length, 1);
    }

    #[test]
    fn missing_keys_and_truncated_resources_are_not_found() {
        let r = resource();
        assert_eq!(
            query(&r, &wide("\\StringFileInfo\\040904B0\\FileVersion")),
            None
        );
        assert_eq!(query(&r, &wide("\\StringFileInfo\\04070000")), None);
        // A prefix of a key is not the key.
        assert_eq!(query(&r, &wide("\\StringFile")), None);
        assert_eq!(query(&r[..40], &wide("\\")), None);
        assert_eq!(length(&[0u8; 4]), None);
    }
}
