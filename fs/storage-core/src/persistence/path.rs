use super::{Error, Result};
/// A file or folder name: up to 255 UTF-16 units, as NTFS (catalog v6).
pub const NAME_UNITS: usize = 255;
/// Registry key and value names keep the catalog's and hive v1's
/// 48-unit records until hive v2.
pub const REGISTRY_NAME_UNITS: usize = 48;
/// Names catalog versions 1 to 5 hold.
pub const CATALOG_NAME_UNITS: usize = 48;
/// A name of at most `N` UTF-16 units: a file or folder name (`Name`), or a
/// registry key or value name (`RegName`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Name<const N: usize = NAME_UNITS> {
    pub(crate) len: u8,
    pub(crate) units: [u16; N],
}
/// A registry key or value name.
pub type RegName = Name<REGISTRY_NAME_UNITS>;
impl<const N: usize> Name<N> {
    pub const EMPTY: Self = Self {
        len: 0,
        units: [0; N],
    };
    pub fn new(units: &[u16]) -> Result<Self> {
        if units.is_empty()
            || units.len() > N
            || units.iter().any(|&u| {
                u == 0
                    || u == 47
                    || u == 92
                    || u == 58
                    || u < 32
                    || [34, 42, 60, 62, 63, 124].contains(&u)
            })
        {
            return Err(Error::InvalidName);
        }
        let mut n = Self::EMPTY;
        n.len = units.len() as u8;
        n.units[..units.len()].copy_from_slice(units);
        Ok(n)
    }
    /// A registry key name (one path component): Windows forbids only the
    /// backslash, its separator, so MIME types ("text/plain"), drive-like
    /// and wildcard characters are valid key names.
    pub fn key(units: &[u16]) -> Result<Self> {
        if units.is_empty() || units.len() > N || units.iter().any(|&u| u == 0 || u == 92) {
            return Err(Error::InvalidName);
        }
        Ok(Self::raw(units))
    }
    /// A registry value name: any characters but NUL (backslashes and
    /// colons included, as in AppCompatFlags' path-named values).
    pub fn value(units: &[u16]) -> Result<Self> {
        if units.is_empty() || units.len() > N || units.contains(&0) {
            return Err(Error::InvalidName);
        }
        Ok(Self::raw(units))
    }
    fn raw(units: &[u16]) -> Self {
        let mut n = Self::EMPTY;
        n.len = units.len() as u8;
        n.units[..units.len()].copy_from_slice(units);
        n
    }
    pub fn ascii(bytes: &str) -> Result<Self> {
        if !bytes.is_ascii() {
            return Err(Error::InvalidName);
        }
        let mut u = [0; N];
        if bytes.len() > u.len() {
            return Err(Error::InvalidName);
        }
        for (o, b) in u.iter_mut().zip(bytes.bytes()) {
            *o = u16::from(b);
        }
        Self::new(&u[..bytes.len()])
    }
    pub fn units(&self) -> &[u16] {
        &self.units[..self.len as usize]
    }
    pub(crate) fn same(self, other: Self, fold: bool) -> bool {
        self.len == other.len
            && self.units().iter().zip(other.units()).all(|(&a, &b)| {
                if fold {
                    fold_ascii(a) == fold_ascii(b)
                } else {
                    a == b
                }
            })
    }
}
pub(crate) fn fold_ascii(u: u16) -> u16 {
    if (65..=90).contains(&u) { u + 32 } else { u }
}
#[derive(Clone, Copy)]
pub enum Path<'a> {
    Native(&'a str),
    Windows(&'a [u16]),
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub(crate) cwd: u64,
    pub(crate) drive_cwd: u64,
    pub(crate) mount: u64,
}
impl Context {
    pub const fn root() -> Self {
        Self {
            cwd: 10,
            drive_cwd: 10,
            mount: 0,
        }
    }
}
pub(crate) struct Parsed {
    pub start: u64,
    pub fold: bool,
    pub components: [Name; 16],
    pub len: usize,
}
fn device(name: Name) -> bool {
    let s = name.units();
    let stop = s.iter().position(|&u| u == 46).unwrap_or(s.len());
    let s = &s[..stop];
    let eq = |b: &[u8]| {
        s.len() == b.len()
            && s.iter()
                .zip(b)
                .all(|(&u, &v)| fold_ascii(u) == u16::from(v))
    };
    eq(b"con")
        || eq(b"prn")
        || eq(b"aux")
        || eq(b"nul")
        || s.len() == 4
            && (eq_prefix(s, b"com") || eq_prefix(s, b"lpt"))
            && ((49..=57).contains(&s[3]) || [0xb9, 0xb2, 0xb3].contains(&s[3]))
}
fn eq_prefix(s: &[u16], b: &[u8]) -> bool {
    s.iter()
        .zip(b)
        .all(|(&u, &v)| fold_ascii(u) == u16::from(v))
}
pub(crate) fn parse(path: Path<'_>, context: Context) -> Result<Parsed> {
    let mut result = Parsed {
        start: 1,
        fold: false,
        components: [Name::EMPTY; 16],
        len: 0,
    };
    let mut native = [0u16; 1024];
    let (mut units, windows, mut extended) = match path {
        Path::Native(s) => {
            if !s.starts_with('/') || s.len() > native.len() {
                return Err(Error::InvalidName);
            }
            let mut count = 0;
            for u in s.encode_utf16() {
                if count == native.len() {
                    return Err(Error::InvalidName);
                }
                native[count] = u;
                count += 1;
            }
            (&native[..count], false, false)
        }
        Path::Windows(s) => (s, true, false),
    };
    if units.is_empty() || units.len() > 1024 || units.contains(&0) {
        return Err(Error::InvalidName);
    }
    if windows {
        result.fold = true;
        result.start = context.cwd;
        if units.starts_with(&[92, 92, 63, 92]) || units.starts_with(&[92, 63, 63, 92]) {
            extended = true;
            units = &units[4..];
        } else if units.len() >= 2 && units[..2].iter().all(|&u| u == 47 || u == 92) {
            // Ordinary DOS paths convert '/' separators before namespace
            // classification. Mixed or forward-slash UNC prefixes must never
            // become a single-rooted path into the local C: view.
            return Err(Error::Unsupported);
        }
        if units.len() >= 2 && units[1] == 58 {
            if fold_ascii(units[0]) != 99 {
                return Err(Error::Unsupported);
            }
            units = &units[2..];
            result.start = context.drive_cwd;
            if units
                .first()
                .is_some_and(|&u| u == 92 || (!extended && u == 47))
            {
                result.start = 10;
                units = &units[1..];
            } else if extended {
                return Err(Error::InvalidName);
            }
        } else if extended {
            return Err(Error::Unsupported);
        } else if units.first().is_some_and(|&u| u == 92 || u == 47) {
            result.start = 10;
            units = &units[1..];
        }
    } else {
        units = &units[1..];
    }
    for raw in units.split(|&u| u == 47 && (!windows || !extended) || u == 92 && windows) {
        if raw.is_empty() {
            continue;
        }
        let mut part = raw;
        if windows && !extended && part != [46] && part != [46, 46] {
            while part.last().is_some_and(|&u| u == 32 || u == 46) {
                part = &part[..part.len() - 1];
            }
        }
        if (part == [46] || part == [46, 46]) && extended {
            return Err(Error::Unsupported);
        }
        let name = Name::new(part)?;
        if windows && device(name) {
            return Err(Error::Unsupported);
        }
        if result.len == 16 {
            return Err(Error::InvalidName);
        }
        result.components[result.len] = name;
        result.len += 1;
    }
    Ok(result)
}
