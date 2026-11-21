//! The Windows process environment block: "Name=Value\0" entries in
//! case-insensitive name order (hidden "=X:" drive entries first, as
//! Windows sorts them), ended by an empty entry. ntdll keeps the process's
//! block; kernel32's Get/Set/ExpandEnvironmentStrings work on it.
use crate::Error;

/// Capacity in UTF-16 units (Windows' block has no fixed limit; 32 KiB of
/// units holds every variable a session sets with room for applications).
pub const UNITS: usize = 16 * 1024;
/// The longest value GetEnvironmentVariable reports (Windows: 32767).
pub const VALUE_MAX: usize = 32767;
const EQUALS: u16 = b'=' as u16;
const PERCENT: u16 = b'%' as u16;

pub struct Block {
    units: [u16; UNITS],
    /// Units of the entries, each with its NUL (the final empty entry is
    /// at `used`).
    used: usize,
}
impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}
fn upper(u: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&u) {
        u - 32
    } else {
        u
    }
}
fn compare(a: &[u16], b: &[u16]) -> core::cmp::Ordering {
    a.iter().map(|u| upper(*u)).cmp(b.iter().map(|u| upper(*u)))
}
/// A name Windows accepts: not empty, no '=' after the first unit.
pub fn valid_name(name: &[u16]) -> bool {
    !name.is_empty() && !name[1..].contains(&EQUALS) && !name.contains(&0)
}
impl Block {
    pub const fn new() -> Self {
        Self {
            units: [0; UNITS],
            used: 0,
        }
    }
    /// Each entry (without its NUL) and where it starts.
    fn entries(&self) -> impl Iterator<Item = (usize, &[u16])> {
        let mut at = 0;
        core::iter::from_fn(move || {
            if at >= self.used {
                return None;
            }
            let start = at;
            let length = self.units[at..self.used].iter().position(|u| *u == 0)?;
            at += length + 1;
            Some((start, &self.units[start..start + length]))
        })
    }
    /// The name and value of an entry ("=C:=C:\x" has name "=C:").
    fn split(entry: &[u16]) -> (&[u16], &[u16]) {
        match entry.iter().skip(1).position(|u| *u == EQUALS) {
            Some(i) => (&entry[..i + 1], &entry[i + 2..]),
            None => (entry, &[]),
        }
    }
    pub fn get(&self, name: &[u16]) -> Option<&[u16]> {
        self.entries()
            .map(|(_, e)| Self::split(e))
            .find(|(n, _)| compare(n, name).is_eq())
            .map(|(_, v)| v)
    }
    /// Sets (`Some`) or removes (`None`) a variable.
    pub fn set(&mut self, name: &[u16], value: Option<&[u16]>) -> Result<(), Error> {
        if !valid_name(name) || value.is_some_and(|v| v.contains(&0) || v.len() > VALUE_MAX) {
            return Err(Error::Limit);
        }
        // Where the entry is, or goes.
        let mut at = self.used;
        let mut old = 0;
        for (start, entry) in self.entries() {
            let order = compare(Self::split(entry).0, name);
            if order.is_eq() {
                at = start;
                old = entry.len() + 1;
                break;
            }
            if order.is_gt() {
                at = start;
                break;
            }
        }
        let new = value.map_or(0, |v| name.len() + 1 + v.len() + 1);
        // The final empty entry needs one unit.
        if self.used - old + new + 1 > UNITS {
            return Err(Error::Limit);
        }
        self.units.copy_within(at + old..self.used, at + new);
        self.used = self.used - old + new;
        if let Some(v) = value {
            let entry = &mut self.units[at..at + new];
            entry[..name.len()].copy_from_slice(name);
            entry[name.len()] = EQUALS;
            entry[name.len() + 1..new - 1].copy_from_slice(v);
            entry[new - 1] = 0;
        }
        self.units[self.used] = 0;
        Ok(())
    }
    /// `set` with an ASCII name and a value given in pieces.
    pub fn set_text(&mut self, name: &str, value: &[&str]) -> Result<(), Error> {
        let mut n = [0u16; 64];
        let mut v = [0u16; 512];
        let mut nl = 0;
        for u in name.encode_utf16() {
            *n.get_mut(nl).ok_or(Error::Limit)? = u;
            nl += 1;
        }
        let mut vl = 0;
        for u in value.iter().flat_map(|p| p.encode_utf16()) {
            *v.get_mut(vl).ok_or(Error::Limit)? = u;
            vl += 1;
        }
        self.set(&n[..nl], Some(&v[..vl]))
    }
    /// The first unit (the block PEB.ProcessParameters.Environment names).
    pub fn as_ptr(&self) -> *const u16 {
        self.units.as_ptr()
    }
    /// The whole block with its final empty entry (GetEnvironmentStrings).
    pub fn strings(&self) -> &[u16] {
        // An empty block is two NULs, as Windows reports one.
        if self.used == 0 {
            &[0, 0]
        } else {
            &self.units[..self.used + 1]
        }
    }
    /// ExpandEnvironmentStrings: each %NAME% of a defined variable becomes
    /// its value; others stay as written. Calls `out` for each unit and
    /// returns the unit count (without the NUL).
    pub fn expand(&self, source: &[u16], mut out: impl FnMut(u16)) -> usize {
        let mut count = 0;
        let mut emit = |u: u16| {
            out(u);
            count += 1;
        };
        let mut i = 0;
        while i < source.len() {
            if source[i] == PERCENT
                && let Some(length) = source[i + 1..].iter().position(|u| *u == PERCENT)
                && length > 0
                && let Some(value) = self.get(&source[i + 1..i + 1 + length])
            {
                value.iter().for_each(|u| emit(*u));
                i += length + 2;
                continue;
            }
            emit(source[i]);
            i += 1;
        }
        count
    }
}

/// The variables of a Windows 11 session for `user` on `computer` (what
/// cmd.exe's `set` lists), with the profile folder `profile`
/// (C:\Users\<folder>).
pub fn session_defaults(
    block: &mut Block,
    user: &str,
    profile: &str,
    computer: &str,
    processors: &str,
) -> Result<(), Error> {
    let p = [profile];
    let local = [profile, "\\AppData\\Local"];
    let home = profile.get(2..).unwrap_or("\\");
    let pairs: [(&str, &[&str]); 37] = [
        ("ALLUSERSPROFILE", &["C:\\ProgramData"]),
        ("APPDATA", &[profile, "\\AppData\\Roaming"]),
        ("CommonProgramFiles", &["C:\\Program Files\\Common Files"]),
        (
            "CommonProgramFiles(x86)",
            &["C:\\Program Files (x86)\\Common Files"],
        ),
        ("CommonProgramW6432", &["C:\\Program Files\\Common Files"]),
        ("COMPUTERNAME", &[computer]),
        ("ComSpec", &["C:\\Windows\\system32\\cmd.exe"]),
        (
            "DriverData",
            &["C:\\Windows\\System32\\Drivers\\DriverData"],
        ),
        ("HOMEDRIVE", &["C:"]),
        ("HOMEPATH", &[home]),
        ("LOCALAPPDATA", &local),
        ("LOGONSERVER", &["\\\\", computer]),
        ("NUMBER_OF_PROCESSORS", &[processors]),
        ("OS", &["Windows_NT"]),
        (
            "Path",
            &[
                "C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;",
                profile,
                "\\AppData\\Local\\Microsoft\\WindowsApps;",
            ],
        ),
        (
            "PATHEXT",
            &[".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC"],
        ),
        ("PROCESSOR_ARCHITECTURE", &["AMD64"]),
        (
            "PROCESSOR_IDENTIFIER",
            &["Intel64 Family 6 Model 85 Stepping 7, GenuineIntel"],
        ),
        ("PROCESSOR_LEVEL", &["6"]),
        ("PROCESSOR_REVISION", &["5507"]),
        ("ProgramData", &["C:\\ProgramData"]),
        ("ProgramFiles", &["C:\\Program Files"]),
        ("ProgramFiles(x86)", &["C:\\Program Files (x86)"]),
        ("ProgramW6432", &["C:\\Program Files"]),
        (
            "PSModulePath",
            &[
                "C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules",
            ],
        ),
        ("PUBLIC", &["C:\\Users\\Public"]),
        ("SESSIONNAME", &["Console"]),
        ("SystemDrive", &["C:"]),
        ("SystemRoot", &["C:\\Windows"]),
        ("TEMP", &[profile, "\\AppData\\Local\\Temp"]),
        ("TMP", &[profile, "\\AppData\\Local\\Temp"]),
        ("USERDOMAIN", &[computer]),
        ("USERDOMAIN_ROAMINGPROFILE", &[computer]),
        ("USERNAME", &[user]),
        ("USERPROFILE", &p),
        ("windir", &["C:\\Windows"]),
        ("OneDrive", &[]),
    ];
    for (name, value) in pairs {
        // OneDrive is set only for a signed-in Microsoft account.
        if !value.is_empty() {
            block.set_text(name, value)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{boxed::Box, string::String, vec::Vec};
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn text(u: &[u16]) -> String {
        String::from_utf16(u).unwrap()
    }
    #[test]
    fn get_set_remove_and_order() {
        let mut b = Box::new(Block::new());
        assert_eq!(b.strings(), &[0, 0]);
        b.set(&w("Path"), Some(&w("C:\\Windows"))).unwrap();
        b.set(&w("ALPHA"), Some(&w("1"))).unwrap();
        b.set(&w("=C:"), Some(&w("C:\\Users"))).unwrap();
        assert_eq!(text(b.get(&w("path")).unwrap()), "C:\\Windows");
        assert_eq!(text(b.get(&w("=c:")).unwrap()), "C:\\Users");
        b.set(&w("PATH"), Some(&w("x"))).unwrap();
        assert_eq!(text(b.strings()), "=C:=C:\\Users\0ALPHA=1\0PATH=x\0\0");
        b.set(&w("alpha"), None).unwrap();
        assert!(b.get(&w("ALPHA")).is_none());
        assert_eq!(text(b.strings()), "=C:=C:\\Users\0PATH=x\0\0");
        assert_eq!(b.set(&w("A=B"), Some(&w("1"))), Err(Error::Limit));
        assert_eq!(b.set(&[], Some(&w("1"))), Err(Error::Limit));
        // An empty value is a defined variable.
        b.set(&w("EMPTY"), Some(&[])).unwrap();
        assert_eq!(b.get(&w("EMPTY")), Some(&[][..]));
    }
    #[test]
    fn expansion_and_defaults() {
        let mut b = Box::new(Block::new());
        session_defaults(&mut b, "User", "C:\\Users\\1001", "DOORS-PC", "4").unwrap();
        assert_eq!(text(b.get(&w("HOMEPATH")).unwrap()), "\\Users\\1001");
        assert_eq!(text(b.get(&w("USERNAME")).unwrap()), "User");
        assert_eq!(text(b.get(&w("USERPROFILE")).unwrap()), "C:\\Users\\1001");
        assert!(b.get(&w("OneDrive")).is_none());
        let mut out = Vec::new();
        let n = b.expand(&w("%SystemRoot%\\x %NOPE% 100% %%"), |u| out.push(u));
        assert_eq!(text(&out), "C:\\Windows\\x %NOPE% 100% %%");
        assert_eq!(n, out.len());
        out.clear();
        b.expand(&w("%TEMP%"), |u| out.push(u));
        assert_eq!(text(&out), "C:\\Users\\1001\\AppData\\Local\\Temp");
    }
    #[test]
    fn capacity_is_enforced() {
        let mut b = Box::new(Block::new());
        let big = std::vec![b'x' as u16; 9000];
        b.set(&w("A"), Some(&big)).unwrap();
        assert_eq!(b.set(&w("B"), Some(&big)), Err(Error::Limit));
        assert!(b.get(&w("B")).is_none());
        b.set(&w("A"), None).unwrap();
        b.set(&w("B"), Some(&big)).unwrap();
    }
}
