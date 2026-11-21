#![no_std]

#[cfg(test)]
extern crate std;

pub const BLOCK: usize = 512;
/// 256 MiB (the UEFI loader's largest boot allocation): the desktop archive
/// carries the on-demand Windows fonts (ADR 0018) and the optional IME
/// dictionaries (about 200 MB in all). Only the desktop profiles stage it.
pub const MAX_ARCHIVE: usize = 256 * 1024 * 1024;
pub const MAX_FILE: usize = 32 * 1024 * 1024;
/// Executable policy stays independent of immutable resource archive entries.
pub const MAX_NATIVE_FILE: usize = 16 * 1024 * 1024;
pub mod app_assets;
pub mod desktop_assets;
/// Entries an archive may hold (P18: 512, the desktop ships 62 Windows
/// system DLLs in two architectures).
pub const MAX_ENTRIES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Bounds,
    Header,
    Checksum,
    Path,
    Type,
    Limit,
    Duplicate,
    Terminator,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry<'a> {
    pub path: &'a str,
    pub data: &'a [u8],
    pub directory: bool,
}

pub struct Archive<'a> {
    bytes: &'a [u8],
    offsets: [usize; MAX_ENTRIES],
    count: usize,
}

impl<'a> Archive<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_ARCHIVE || !bytes.len().is_multiple_of(BLOCK) {
            return Err(Error::Limit);
        }
        let mut archive = Self {
            bytes,
            offsets: [0; MAX_ENTRIES],
            count: 0,
        };
        let mut offset: usize = 0;
        loop {
            let header = bytes
                .get(offset..offset.checked_add(BLOCK).ok_or(Error::Bounds)?)
                .ok_or(Error::Terminator)?;
            if header.iter().all(|b| *b == 0) {
                let tail = bytes.get(offset..).ok_or(Error::Bounds)?;
                if tail.len() < 2 * BLOCK || tail.iter().any(|b| *b != 0) {
                    return Err(Error::Terminator);
                }
                return Ok(archive);
            }
            if archive.count == MAX_ENTRIES {
                return Err(Error::Limit);
            }
            let (entry, next) = decode(bytes, offset)?;
            for index in 0..archive.count {
                let prior = archive.entry(index)?;
                if prior.path.trim_end_matches('/') == entry.path.trim_end_matches('/') {
                    return Err(Error::Duplicate);
                }
                if (!prior.directory
                    && entry
                        .path
                        .strip_prefix(prior.path)
                        .is_some_and(|tail| tail.starts_with('/')))
                    || (!entry.directory
                        && prior
                            .path
                            .strip_prefix(entry.path)
                            .is_some_and(|tail| tail.starts_with('/')))
                {
                    return Err(Error::Path);
                }
            }
            archive.offsets[archive.count] = offset;
            archive.count += 1;
            offset = next;
        }
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn entry(&self, index: usize) -> Result<Entry<'a>, Error> {
        if index >= self.count {
            return Err(Error::Bounds);
        }
        decode(self.bytes, self.offsets[index]).map(|(entry, _)| entry)
    }
    pub fn find(&self, path: &str) -> Option<Entry<'a>> {
        (0..self.count).find_map(|index| self.entry(index).ok().filter(|entry| entry.path == path))
    }
    /// Returns only a regular file; directories cannot satisfy a required file role.
    pub fn file(&self, path: &str) -> Option<&'a [u8]> {
        self.find(path)
            .filter(|entry| !entry.directory)
            .map(|entry| entry.data)
    }
}

fn octal(bytes: &[u8]) -> Result<usize, Error> {
    let end = bytes
        .iter()
        .position(|b| *b == 0 || *b == b' ')
        .unwrap_or(bytes.len());
    if end == 0 || bytes[end..].iter().any(|b| *b != 0 && *b != b' ') {
        return Err(Error::Header);
    }
    bytes[..end].iter().try_fold(0usize, |value, byte| {
        if !(b'0'..=b'7').contains(byte) {
            return Err(Error::Header);
        }
        value
            .checked_mul(8)
            .and_then(|v| v.checked_add(usize::from(*byte - b'0')))
            .ok_or(Error::Bounds)
    })
}
fn valid_path(path: &str, directory: bool) -> bool {
    if path.len() >= 100 {
        return false;
    }
    let path = if directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    };
    !path.is_empty()
        && path.len() < 100
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
fn decode(bytes: &[u8], offset: usize) -> Result<(Entry<'_>, usize), Error> {
    let h = bytes
        .get(offset..offset.checked_add(BLOCK).ok_or(Error::Bounds)?)
        .ok_or(Error::Bounds)?;
    if &h[257..263] != b"ustar\0" || &h[263..265] != b"00" || h[345..].iter().any(|b| *b != 0) {
        return Err(Error::Header);
    }
    let checksum: usize = h
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if (148..156).contains(&i) {
                32
            } else {
                usize::from(*b)
            }
        })
        .sum();
    if checksum != octal(&h[148..156])? {
        return Err(Error::Checksum);
    }
    let directory = match h[156] {
        b'0' => false,
        b'5' => true,
        _ => return Err(Error::Type),
    };
    if h[157..257].iter().any(|b| *b != 0) {
        return Err(Error::Type);
    }
    for range in [100..108, 108..116, 116..124, 136..148, 329..337, 337..345] {
        octal(&h[range])?;
    }
    let end = h[..100].iter().position(|b| *b == 0).ok_or(Error::Path)?;
    if h[end..100].iter().any(|b| *b != 0) {
        return Err(Error::Path);
    }
    let path = core::str::from_utf8(&h[..end]).map_err(|_| Error::Path)?;
    if !valid_path(path, directory) {
        return Err(Error::Path);
    }
    let size = octal(&h[124..136])?;
    if size > MAX_FILE || (directory && size != 0) {
        return Err(Error::Limit);
    }
    let start = offset.checked_add(BLOCK).ok_or(Error::Bounds)?;
    let end = start.checked_add(size).ok_or(Error::Bounds)?;
    let next = start
        .checked_add(size.div_ceil(BLOCK) * BLOCK)
        .ok_or(Error::Bounds)?;
    let data = bytes.get(start..end).ok_or(Error::Bounds)?;
    let padding = bytes.get(end..next).ok_or(Error::Bounds)?;
    if padding.iter().any(|b| *b != 0) {
        return Err(Error::Header);
    }
    Ok((
        Entry {
            path,
            data,
            directory,
        },
        next,
    ))
}

/// Encodes the fixed deterministic profile; callers append padded data and two zero blocks.
pub fn encode_header(
    header: &mut [u8; BLOCK],
    path: &str,
    size: usize,
    directory: bool,
) -> Result<(), Error> {
    if !valid_path(path, directory) {
        return Err(Error::Path);
    }
    if size > MAX_FILE || (directory && size != 0) {
        return Err(Error::Limit);
    }
    header.fill(0);
    header[..path.len()].copy_from_slice(path.as_bytes());
    for (start, end, value) in [
        (100, 108, if directory { 0o755 } else { 0o644 }),
        (108, 116, 0),
        (116, 124, 0),
        (124, 136, size),
        (136, 148, 0),
        (329, 337, 0),
        (337, 345, 0),
    ] {
        write_octal(&mut header[start..end], value);
    }
    header[156] = if directory { b'5' } else { b'0' };
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[148..156].fill(b' ');
    let checksum = header.iter().map(|b| usize::from(*b)).sum();
    write_octal(&mut header[148..155], checksum);
    header[155] = b' ';
    Ok(())
}
fn write_octal(bytes: &mut [u8], mut value: usize) {
    let last = bytes.len() - 1;
    bytes.fill(b'0');
    bytes[last] = 0;
    for byte in bytes[..last].iter_mut().rev() {
        *byte = b'0' + (value & 7) as u8;
        value >>= 3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> [u8; 2048] {
        let mut bytes = [0; 2048];
        let mut h = [0; BLOCK];
        encode_header(&mut h, "system/boot.txt", 3, false).unwrap();
        bytes[..BLOCK].copy_from_slice(&h);
        bytes[BLOCK..BLOCK + 3].copy_from_slice(b"OS\n");
        bytes
    }
    #[test]
    fn roundtrip_and_truncation() {
        let bytes = fixture();
        let archive = Archive::parse(&bytes).unwrap();
        assert_eq!(archive.len(), 1);
        assert_eq!(archive.find("system/boot.txt").unwrap().data, b"OS\n");
        for end in 0..bytes.len() {
            assert!(Archive::parse(&bytes[..end]).is_err());
        }
    }
    #[test]
    fn required_file_rejects_directory_and_name_boundary_preserves_output() {
        let mut bytes = [0; 1536];
        let mut header = [0; BLOCK];
        encode_header(&mut header, "system/boot.txt", 0, true).unwrap();
        bytes[..BLOCK].copy_from_slice(&header);
        let archive = Archive::parse(&bytes).unwrap();
        assert!(archive.find("system/boot.txt").is_some());
        assert!(archive.file("system/boot.txt").is_none());
        assert_eq!(
            Archive::parse(&fixture()).unwrap().file("system/boot.txt"),
            Some(b"OS\n".as_slice())
        );
        let mut name = [b'a'; 100];
        name[99] = b'/';
        header.fill(0xa5);
        assert_eq!(
            encode_header(&mut header, core::str::from_utf8(&name).unwrap(), 0, true),
            Err(Error::Path)
        );
        assert_eq!(header, [0xa5; BLOCK]);
        name[98] = b'/';
        encode_header(
            &mut header,
            core::str::from_utf8(&name[..99]).unwrap(),
            0,
            true,
        )
        .unwrap();
        bytes[..BLOCK].copy_from_slice(&header);
        assert!(Archive::parse(&bytes).is_ok());
    }
    #[test]
    fn rejects_paths_links_checksum_and_duplicate() {
        for path in ["/root", "../root", "a/../b", "a//b", "a\\b", "c:file", "a/"] {
            assert!(encode_header(&mut [0; BLOCK], path, 1, false).is_err());
        }
        let mut bytes = fixture();
        bytes[0] ^= 1;
        assert!(matches!(Archive::parse(&bytes), Err(Error::Checksum)));
        let bytes = fixture();
        let mut twice = [0; 3072];
        twice[..1024].copy_from_slice(&bytes[..1024]);
        twice[1024..2048].copy_from_slice(&bytes[..1024]);
        assert!(matches!(Archive::parse(&twice), Err(Error::Duplicate)));
        assert!(encode_header(&mut [0; BLOCK], "dir/", 1, true).is_err());
    }
    #[test]
    fn unsupported_types_prefixes_padding_and_limits_are_rejected() {
        for kind in [b'1', b'2', b'3', b'x', b'g'] {
            let mut bytes = fixture();
            bytes[156] = kind;
            bytes[148..156].fill(b' ');
            let checksum = bytes[..512].iter().map(|b| usize::from(*b)).sum();
            write_octal(&mut bytes[148..155], checksum);
            bytes[155] = b' ';
            assert!(matches!(Archive::parse(&bytes), Err(Error::Type)));
        }
        let mut bytes = fixture();
        bytes[345] = b'a';
        assert!(Archive::parse(&bytes).is_err());
        let mut bytes = fixture();
        bytes[600] = 1;
        assert!(Archive::parse(&bytes).is_err());
        assert!(encode_header(&mut [0; 512], "file", MAX_FILE + 1, false).is_err());
        let mut bytes = fixture();
        bytes[2047] = 1;
        assert!(matches!(Archive::parse(&bytes), Err(Error::Terminator)));
        let mut conflict = [0; 2048];
        let mut header = [0; 512];
        encode_header(&mut header, "a", 0, false).unwrap();
        conflict[..512].copy_from_slice(&header);
        encode_header(&mut header, "a/b", 0, false).unwrap();
        conflict[512..1024].copy_from_slice(&header);
        assert!(matches!(Archive::parse(&conflict), Err(Error::Path)));
    }
    #[test]
    fn embedded_hd_service_file_boundary_is_shared_by_encoder_and_parser() {
        let mut bytes = std::vec![0; BLOCK + MAX_FILE + 2 * BLOCK];
        let mut header = [0; BLOCK];
        encode_header(&mut header, "bin/window.elf", MAX_FILE, false).unwrap();
        bytes[..BLOCK].copy_from_slice(&header);
        bytes[BLOCK + MAX_FILE - 1] = 73;
        let archive = Archive::parse(&bytes).unwrap();
        let data = archive.file("bin/window.elf").unwrap();
        assert_eq!(data.len(), MAX_FILE);
        assert_eq!(data[MAX_FILE - 1], 73);
        write_octal(&mut bytes[124..136], MAX_FILE + 1);
        bytes[148..156].fill(b' ');
        let checksum = bytes[..BLOCK].iter().map(|b| usize::from(*b)).sum();
        write_octal(&mut bytes[148..155], checksum);
        bytes[155] = b' ';
        assert!(matches!(Archive::parse(&bytes), Err(Error::Limit)));
    }
}
