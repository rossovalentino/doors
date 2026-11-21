//! What both sides held at the last successful sync of each item: the
//! local size and time, and the server's entity tag. Stored as text in
//! `%LOCALAPPDATA%\NoneDrive\index` (one item per line, tab-separated;
//! Windows names cannot contain tabs or line breaks).
use alloc::{format, string::String, vec::Vec};

/// One synced item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    /// Relative to the sync folder, `/`-separated.
    pub path: String,
    pub folder: bool,
    /// The local file's size and change stamp (Storage version) when it
    /// last matched the server (0 for folders).
    pub size: u64,
    pub stamp: u64,
    /// The server's entity tag at that time (`sync::remote_tag`).
    pub etag: String,
}

pub const HEADER: &str = "NoneDrive index 1";

fn clean(s: &str) -> bool {
    !s.contains(['\t', '\n', '\r'])
}
/// The index as text. Items whose fields cannot be stored are left out
/// (they read as never synced, so the next sync compares them again).
pub fn encode(items: &[Known]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for k in items {
        if !clean(&k.path) || !clean(&k.etag) || k.path.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            if k.folder { 'D' } else { 'F' },
            k.size,
            k.stamp,
            k.etag,
            k.path
        ));
    }
    out
}
/// The items of an index file; None if it is damaged (the engine then
/// treats every item as never synced: nothing is deleted on either side,
/// and differing files are kept both).
pub fn decode(text: &str) -> Option<Vec<Known>> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let mut out = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut f = line.splitn(5, '\t');
        let folder = match f.next()? {
            "D" => true,
            "F" => false,
            _ => return None,
        };
        let size = f.next()?.parse().ok()?;
        let stamp = f.next()?.parse().ok()?;
        let etag = String::from(f.next()?);
        let path = String::from(f.next()?);
        if !crate::href::valid_relative(&path) || path.is_empty() {
            return None;
        }
        out.push(Known {
            path,
            folder,
            size,
            stamp,
            etag,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    #[test]
    fn the_index_round_trips_and_refuses_damage() {
        let items = vec![
            Known {
                path: "Docs".into(),
                folder: true,
                size: 0,
                stamp: 0,
                etag: "\"d1\"".into(),
            },
            Known {
                path: "Docs/a b.txt".into(),
                folder: false,
                size: 12,
                stamp: 7,
                etag: "W/\"x\"".into(),
            },
        ];
        let text = encode(&items);
        assert!(text.starts_with("NoneDrive index 1\n"));
        assert_eq!(decode(&text), Some(items.clone()));
        // An unstorable etag is left out rather than corrupting the file.
        let mut bad = items.clone();
        bad[1].etag = "a\tb".into();
        assert_eq!(decode(&encode(&bad)).map(|v| v.len()), Some(1));
        assert_eq!(decode("garbage"), None);
        assert_eq!(decode("NoneDrive index 1\nX\t0\t0\t\tp\n"), None);
        assert_eq!(decode("NoneDrive index 1\nF\t0\t0\t\t../p\n"), None);
        assert_eq!(decode("NoneDrive index 1\nF\tx\t0\t\tp\n"), None);
    }
}
