//! GetFullPathName's algorithm (P18 M0c), over UTF-16 units: the path
//! made absolute against the current directory, separators unified, `.`
//! and `..` resolved without climbing above the root, and trailing dots
//! and spaces dropped from each component, as Windows does. `\\?\` paths
//! pass through untouched; `\\.\` device paths are normalized after the
//! device prefix.

/// Longest result produced (in units, without the terminator).
pub const MAX_UNITS: usize = 32_767;
const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;
const DOT: u16 = b'.' as u16;
const SPACE: u16 = b' ' as u16;
const QUESTION: u16 = b'?' as u16;

fn separator(unit: u16) -> bool {
    unit == BACKSLASH || unit == SLASH
}
fn drive_letter(units: &[u16]) -> bool {
    units.len() >= 2 && units[1] == COLON && matches!(units[0], 0x41..=0x5a | 0x61..=0x7a)
}

/// The full path of `path` with current directory `current` (itself a
/// full path such as `C:\Users\me`) into `out`: the number of units, or
/// None when the input is empty or the result does not fit.
pub fn full_path(path: &[u16], current: &[u16], out: &mut [u16]) -> Option<usize> {
    if path.is_empty() {
        return None;
    }
    let mut writer = Writer { out, len: 0 };
    // \\?\ : verbatim.
    if path.len() >= 4
        && separator(path[0])
        && separator(path[1])
        && path[2] == QUESTION
        && separator(path[3])
    {
        writer.extend(path)?;
        return Some(writer.len);
    }
    // The root the components are resolved under, and the rest.
    let (root_len, rest): (usize, &[u16]);
    if separator(path[0]) && path.len() >= 2 && separator(path[1]) {
        // UNC (\\server\share) or a device path (\\.\name).
        writer.push(BACKSLASH)?;
        writer.push(BACKSLASH)?;
        let mut parts = 0;
        let mut i = 2;
        let mut start = 2;
        let device = path.len() > 2 && path[2] == DOT && (path.len() == 3 || separator(path[3]));
        let wanted = if device { 1 } else { 2 };
        while i <= path.len() && parts < wanted {
            if i == path.len() || separator(path[i]) {
                if i > start {
                    if parts > 0 {
                        writer.push(BACKSLASH)?;
                    }
                    writer.extend(&path[start..i])?;
                    parts += 1;
                }
                start = i + 1;
            }
            i += 1;
        }
        root_len = writer.len;
        rest = path.get(start.min(path.len())..).unwrap_or(&[]);
    } else if drive_letter(path) && path.len() >= 3 && separator(path[2]) {
        writer.extend(&[path[0] & !0x20, COLON])?;
        root_len = 2;
        rest = &path[3..];
    } else if drive_letter(path) {
        // C:relative: the current directory when it is on that drive.
        let same = drive_letter(current) && (current[0] & !0x20) == (path[0] & !0x20);
        if same {
            writer.extend(&[current[0] & !0x20, COLON])?;
            root_len = 2;
            components(&mut writer, root_len, &current[2..])?;
        } else {
            writer.extend(&[path[0] & !0x20, COLON])?;
            root_len = 2;
        }
        rest = &path[2..];
    } else if separator(path[0]) {
        // \rooted: the current directory's drive or share.
        let mut root = [0u16; 512];
        let mut current_root = Writer {
            out: &mut root,
            len: 0,
        };
        if drive_letter(current) {
            current_root.extend(&[current[0] & !0x20, COLON])?;
        } else {
            // A UNC current directory: its \\server\share.
            let mut scratch = [0u16; 512];
            let n = full_path(current, &[], &mut scratch)?;
            let mut seps = 0;
            let end = scratch[..n]
                .iter()
                .position(|u| {
                    if *u == BACKSLASH {
                        seps += 1;
                    }
                    seps == 4
                })
                .unwrap_or(n);
            current_root.extend(&scratch[..end])?;
        }
        let len = current_root.len;
        writer.extend(&root[..len])?;
        root_len = writer.len;
        rest = &path[1..];
    } else {
        // Relative: under the current directory.
        let n = {
            let mut scratch = [0u16; 1024];
            let n = full_path(current, &[], &mut scratch)?;
            writer.extend(&scratch[..n])?;
            n
        };
        let root = if drive_letter(&writer.out[..2]) {
            2
        } else {
            root_of_unc(&writer.out[..n])
        };
        // Re-resolve against the current directory's own root.
        let mut base = [0u16; 1024];
        base[..n].copy_from_slice(&writer.out[..n]);
        writer.len = root;
        components(&mut writer, root, &base[root..n])?;
        root_len = root;
        rest = path;
    }
    components(&mut writer, root_len, rest)?;
    // A bare root, or a path the caller ended with a separator, keeps one.
    if writer.len == root_len || path.last().is_some_and(|u| separator(*u)) {
        writer.push(BACKSLASH)?;
    }
    Some(writer.len)
}
fn root_of_unc(full: &[u16]) -> usize {
    let mut seps = 0;
    full.iter()
        .position(|u| {
            if *u == BACKSLASH {
                seps += 1;
            }
            seps == 4
        })
        .unwrap_or(full.len())
}

struct Writer<'a> {
    out: &'a mut [u16],
    len: usize,
}
impl Writer<'_> {
    fn push(&mut self, unit: u16) -> Option<()> {
        if self.len >= self.out.len() || self.len >= MAX_UNITS {
            return None;
        }
        self.out[self.len] = unit;
        self.len += 1;
        Some(())
    }
    fn extend(&mut self, units: &[u16]) -> Option<()> {
        for unit in units {
            self.push(*unit)?;
        }
        Some(())
    }
}
/// Append the components of `rest` under the root (`root_len` units of
/// `writer`), resolving `.` and `..`.
fn components(writer: &mut Writer<'_>, root_len: usize, rest: &[u16]) -> Option<()> {
    for component in rest.split(|u| separator(*u)) {
        // Trailing dots and spaces go (".." and "." are kept to resolve).
        let all_dots = component.iter().all(|u| *u == DOT);
        let trimmed = if all_dots {
            component
        } else {
            let end = component
                .iter()
                .rposition(|u| *u != DOT && *u != SPACE)
                .map_or(0, |p| p + 1);
            &component[..end]
        };
        match trimmed {
            [] => {}
            [DOT] => {}
            [DOT, DOT] => {
                // Back to the previous separator, never above the root.
                let mut at = writer.len;
                while at > root_len && writer.out[at - 1] != BACKSLASH {
                    at -= 1;
                }
                writer.len = at.saturating_sub(1).max(root_len);
            }
            _ if all_dots => {}
            name => {
                writer.push(BACKSLASH)?;
                writer.extend(name)?;
            }
        }
    }
    Some(())
}
/// Where the last component starts in a full path (None when it ends
/// with a separator).
pub fn file_part(full: &[u16]) -> Option<usize> {
    if full.last().is_none_or(|u| *u == BACKSLASH) {
        return None;
    }
    Some(
        full.iter()
            .rposition(|u| *u == BACKSLASH)
            .map_or(0, |p| p + 1),
    )
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{string::String, vec::Vec};
    fn full(path: &str, current: &str) -> Option<String> {
        let p: Vec<u16> = path.encode_utf16().collect();
        let c: Vec<u16> = current.encode_utf16().collect();
        let mut out = [0u16; 1024];
        let n = full_path(&p, &c, &mut out)?;
        Some(String::from_utf16(&out[..n]).unwrap())
    }
    #[test]
    fn windows_resolution_rules() {
        let cwd = r"C:\Users\me\work";
        for (input, expected) in [
            (r"C:\a\b\..\c", r"C:\a\c"),
            (r"c:/a//b/./c", r"C:\a\b\c"),
            (r"file.txt", r"C:\Users\me\work\file.txt"),
            (r"..\other", r"C:\Users\me\other"),
            (r"..\..\..\..\..", r"C:\"),
            (r"\rooted\x", r"C:\rooted\x"),
            (r"C:rel", r"C:\Users\me\work\rel"),
            (r"D:rel", r"D:\rel"),
            (r"C:\dir\", r"C:\dir\"),
            (r"C:\name. . ", r"C:\name"),
            (r"C:\", r"C:\"),
            (r"C:", r"C:\Users\me\work"),
            (r"\\server\share\a\..\b", r"\\server\share\b"),
            (r"\\?\C:\x\..\y", r"\\?\C:\x\..\y"),
            (r"\\.\pipe\name", r"\\.\pipe\name"),
        ] {
            assert_eq!(full(input, cwd).as_deref(), Some(expected), "{input}");
        }
        assert_eq!(full("", cwd), None);
        assert_eq!(
            full(r"x", r"\\server\share\dir").as_deref(),
            Some(r"\\server\share\dir\x")
        );
        assert_eq!(
            full(r"\y", r"\\server\share\dir").as_deref(),
            Some(r"\\server\share\y")
        );
    }
    #[test]
    fn the_file_part_is_the_last_component() {
        let path: Vec<u16> = r"C:\a\b.txt".encode_utf16().collect();
        assert_eq!(file_part(&path), Some(5));
        let dir: Vec<u16> = r"C:\a\".encode_utf16().collect();
        assert_eq!(file_part(&dir), None);
    }
}
