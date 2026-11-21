//! File versions as Windows Installer compares them: a PE file's
//! VS_FIXEDFILEINFO (the RT_VERSION resource) and the File table's
//! `major.minor.build.revision` strings.

/// `1.2.3.4` as a comparable number (missing parts are 0); None when it is
/// not a version (a companion file key, or empty).
pub fn parse(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut out = 0u64;
    for (index, part) in text.split('.').enumerate() {
        if index == 4 {
            return None;
        }
        let v: u16 = part.parse().ok()?;
        out |= u64::from(v) << (48 - 16 * index);
    }
    Some(out)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The file version of a PE image (`VS_FIXEDFILEINFO.dwFileVersionMS/LS`);
/// None for an unversioned file.
pub fn of_image(bytes: &[u8]) -> Option<u64> {
    if bytes.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = usize::from(u16_at(bytes, pe + 6)?);
    let optional = usize::from(u16_at(bytes, pe + 20)?);
    let opt = pe + 24;
    let directories = match u16_at(bytes, opt)? {
        0x20b => opt + 112,
        0x10b => opt + 96,
        _ => return None,
    };
    let resource_rva = u32_at(bytes, directories + 2 * 8)?;
    if resource_rva == 0 {
        return None;
    }
    let table = opt + optional;
    let to_offset = |rva: u32| -> Option<usize> {
        for i in 0..sections {
            let s = table + 40 * i;
            let va = u32_at(bytes, s + 12)?;
            let size = u32_at(bytes, s + 8)?.max(u32_at(bytes, s + 16)?);
            let raw = u32_at(bytes, s + 20)?;
            if rva >= va && rva < va + size {
                return Some((rva - va + raw) as usize);
            }
        }
        None
    };
    let root = to_offset(resource_rva)?;
    // One level: the directory entry with `id`, or the first.
    let entry = |dir: usize, id: Option<u32>| -> Option<u32> {
        let named = usize::from(u16_at(bytes, dir + 12)?);
        let ids = usize::from(u16_at(bytes, dir + 14)?);
        for i in 0..named + ids {
            let e = dir + 16 + 8 * i;
            let name = u32_at(bytes, e)?;
            if id.is_none_or(|id| name == id) {
                return u32_at(bytes, e + 4);
            }
        }
        None
    };
    let types = entry(root, Some(16))?;
    if types & 0x8000_0000 == 0 {
        return None;
    }
    let names = entry(root + (types & 0x7fff_ffff) as usize, None)?;
    if names & 0x8000_0000 == 0 {
        return None;
    }
    let languages = entry(root + (names & 0x7fff_ffff) as usize, None)?;
    if languages & 0x8000_0000 != 0 {
        return None;
    }
    let data = root + languages as usize;
    let at = to_offset(u32_at(bytes, data)?)?;
    let size = u32_at(bytes, data + 4)? as usize;
    let block = bytes.get(at..at + size)?;
    // VS_VERSIONINFO: header, "VS_VERSION_INFO", padding, the fixed info.
    let signature = (0..block.len().saturating_sub(4))
        .step_by(4)
        .find(|&i| u32_at(block, i) == Some(0xfeef_04bd))?;
    let ms = u32_at(block, signature + 8)?;
    let ls = u32_at(block, signature + 12)?;
    Some(u64::from(ms) << 32 | u64::from(ls))
}

/// `major.minor.build.revision`.
pub fn text(v: u64) -> alloc::string::String {
    alloc::format!(
        "{}.{}.{}.{}",
        v >> 48,
        (v >> 32) & 0xffff,
        (v >> 16) & 0xffff,
        v & 0xffff
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_compare_as_the_installer_does() {
        assert!(parse("1.2.3.4") < parse("1.2.10.0"));
        assert_eq!(parse("2"), parse("2.0.0.0"));
        assert_eq!(parse("fil_companion"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("1.2.3.4.5"), None);
        assert_eq!(text(parse("10.0.26100.1").unwrap()), "10.0.26100.1");
    }
}
