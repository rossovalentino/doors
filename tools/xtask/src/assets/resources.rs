//! Read-only, bounded PE resource directory reader. No executable is loaded.
const DIRECTORY: u32 = 0x8000_0000;
const MAX_RESOURCE: usize = 4 * 1024 * 1024;
fn slice(bytes: &[u8], at: usize, length: usize) -> Result<&[u8], String> {
    bytes
        .get(at..at.checked_add(length).ok_or("resource extent overflow")?)
        .ok_or("resource extent outside file".into())
}
fn word(bytes: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(slice(bytes, at, 2)?.try_into().unwrap()))
}
fn dword(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(slice(bytes, at, 4)?.try_into().unwrap()))
}
pub struct Image<'a> {
    blob: &'a [u8],
    rva: u32,
}
impl<'a> Image<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() > 128 * 1024 * 1024 || slice(bytes, 0, 2)? != b"MZ" {
            return Err("resource PE header".into());
        }
        let pe = dword(bytes, 0x3c)? as usize;
        if slice(bytes, pe, 4)? != b"PE\0\0" {
            return Err("resource PE signature".into());
        }
        let count = word(bytes, pe + 6)? as usize;
        let optional_bytes = word(bytes, pe + 20)? as usize;
        let optional = pe + 24;
        let header = slice(bytes, optional, optional_bytes)?;
        let directory = match word(header, 0)? {
            0x20b => 112,
            0x10b => 96,
            _ => return Err("resource PE format".into()),
        };
        if count == 0 || count > 96 || dword(header, directory - 4)? < 3 {
            return Err("resource PE directory count".into());
        }
        let rva = dword(header, directory + 16)?;
        let size = dword(header, directory + 20)? as usize;
        if rva == 0 || !(16..=128 * 1024 * 1024).contains(&size) {
            return Err("resource root bounds".into());
        }
        let sections = slice(bytes, optional + optional_bytes, count * 40)?;
        let mut blob = None;
        for section in sections.chunks_exact(40) {
            let va = dword(section, 12)?;
            let raw_size = dword(section, 16)?;
            let raw_at = dword(section, 20)?;
            let Some(relative) = rva.checked_sub(va) else {
                continue;
            };
            if u64::from(relative) + size as u64 > u64::from(raw_size) {
                continue;
            }
            let at = u64::from(raw_at) + u64::from(relative);
            let current = slice(
                bytes,
                usize::try_from(at).map_err(|_| "resource offset width")?,
                size,
            )?;
            if blob.replace(current).is_some() {
                return Err("ambiguous resource sections".into());
            }
        }
        Ok(Self {
            blob: blob.ok_or("resource RVA not file-backed")?,
            rva,
        })
    }
    fn entries(&self, at: usize) -> Result<&[u8], String> {
        let header = slice(self.blob, at, 16)?;
        let count = usize::from(word(header, 12)?) + usize::from(word(header, 14)?);
        if count == 0 || count > 32768 {
            return Err("resource entry count".into());
        }
        slice(self.blob, at + 16, count * 8)
    }
    fn child(&self, at: usize, id: u16) -> Result<usize, String> {
        let mut found = None;
        for entry in self.entries(at)?.chunks_exact(8) {
            if dword(entry, 0)? == u32::from(id) {
                let value = dword(entry, 4)?;
                if value & DIRECTORY == 0 || found.replace((value & !DIRECTORY) as usize).is_some()
                {
                    return Err("resource child is missing/duplicated directory".into());
                }
            }
        }
        found.ok_or("resource ID not found".into())
    }
    /// Numeric IDs and names of the `kind` entries (e.g. 14 RT_GROUP_ICON),
    /// in directory order (names first, as PE stores them).
    pub fn list(&self, kind: u16) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for entry in self.entries(self.child(0, kind)?)?.chunks_exact(8) {
            let id = dword(entry, 0)?;
            if id & DIRECTORY == 0 {
                out.push(id.to_string());
                continue;
            }
            let at = (id & !DIRECTORY) as usize;
            let length = (word(self.blob, at)? as usize).min(96);
            let units: Vec<u16> = slice(self.blob, at + 2, length * 2)?
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            out.push(String::from_utf16_lossy(&units));
        }
        Ok(out)
    }
    pub fn read(&self, kind: u16, id: u16) -> Result<Vec<u8>, String> {
        let directory = self.child(self.child(0, kind)?, id)?;
        self.read_directory(directory)
    }
    pub fn read_named(&self, kind: u16, name: &str) -> Result<Vec<u8>, String> {
        let mut selected = None;
        for entry in self.entries(self.child(0, kind)?)?.chunks_exact(8) {
            let id = dword(entry, 0)?;
            if id & DIRECTORY == 0 {
                continue;
            }
            let at = (id & !DIRECTORY) as usize;
            let length = word(self.blob, at)? as usize;
            if length > 96 {
                return Err("resource name bound".into());
            }
            let units: Vec<_> = slice(self.blob, at + 2, length * 2)?
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                .collect();
            if String::from_utf16(&units).is_ok_and(|s| s == name) {
                let value = dword(entry, 4)?;
                if value & DIRECTORY == 0
                    || selected.replace((value & !DIRECTORY) as usize).is_some()
                {
                    return Err("named resource duplicate/directory".into());
                }
            }
        }
        self.read_directory(selected.ok_or("resource name not found")?)
    }
    fn read_directory(&self, directory: usize) -> Result<Vec<u8>, String> {
        let mut selected = None;
        let mut languages = std::collections::BTreeSet::new();
        for entry in self.entries(directory)?.chunks_exact(8) {
            let language = dword(entry, 0)?;
            let data = dword(entry, 4)?;
            if language > u32::from(u16::MAX)
                || data & DIRECTORY != 0
                || !languages.insert(language)
            {
                return Err("invalid/duplicate resource language leaf".into());
            }
            let rank = (
                if language == 0 {
                    0
                } else if language == 1033 {
                    1
                } else {
                    2
                },
                language,
            );
            if selected.is_none_or(|(prior, _)| rank < prior) {
                selected = Some((rank, data as usize));
            }
        }
        let (_, at) = selected.ok_or("resource language missing")?;
        let data = slice(self.blob, at, 16)?;
        let payload = dword(data, 0)?
            .checked_sub(self.rva)
            .ok_or("resource payload RVA")? as usize;
        let size = dword(data, 4)? as usize;
        if size == 0 || size > MAX_RESOURCE || dword(data, 12)? != 0 {
            return Err("resource payload bounds/reserved".into());
        }
        Ok(slice(self.blob, payload, size)?.to_vec())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn image() -> Vec<u8> {
        let mut b = vec![0; 603];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&64u32.to_le_bytes());
        b[64..68].copy_from_slice(b"PE\0\0");
        b[70..72].copy_from_slice(&1u16.to_le_bytes());
        b[84..86].copy_from_slice(&136u16.to_le_bytes());
        b[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        b[196..200].copy_from_slice(&3u32.to_le_bytes());
        b[216..220].copy_from_slice(&0x1000u32.to_le_bytes());
        b[220..224].copy_from_slice(&91u32.to_le_bytes());
        b[236..240].copy_from_slice(&0x1000u32.to_le_bytes());
        b[240..244].copy_from_slice(&91u32.to_le_bytes());
        b[244..248].copy_from_slice(&512u32.to_le_bytes());
        let r = &mut b[512..];
        for (at, id, next) in [
            (0, 14, DIRECTORY | 24),
            (24, 7, DIRECTORY | 48),
            (48, 0, 72),
        ] {
            r[at + 14..at + 16].copy_from_slice(&1u16.to_le_bytes());
            r[at + 16..at + 20].copy_from_slice(&(id as u32).to_le_bytes());
            r[at + 20..at + 24].copy_from_slice(&next.to_le_bytes());
        }
        r[72..76].copy_from_slice(&0x1058u32.to_le_bytes());
        r[76..80].copy_from_slice(&3u32.to_le_bytes());
        r[88..91].copy_from_slice(&[4, 5, 6]);
        b
    }
    #[test]
    fn resource_only_image_has_checked_rva_tree_and_payload_without_executable_entry() {
        let b = image();
        assert_eq!(Image::parse(&b).unwrap().read(14, 7).unwrap(), [4, 5, 6]);
        assert!(Image::parse(&b).unwrap().read(14, 8).is_err());
        for end in 0..b.len() {
            assert!(Image::parse(&b[..end]).is_err(), "{end}");
        }
        let mut bad = b.clone();
        bad[512 + 76..512 + 80].copy_from_slice(&100u32.to_le_bytes());
        assert!(Image::parse(&bad).unwrap().read(14, 7).is_err());
        bad = b.clone();
        bad[512 + 68..512 + 72].copy_from_slice(&(DIRECTORY | 48).to_le_bytes());
        assert!(Image::parse(&bad).unwrap().read(14, 7).is_err());
        bad = b.clone();
        bad[512 + 84] = 1;
        assert!(Image::parse(&bad).unwrap().read(14, 7).is_err());
    }
}
