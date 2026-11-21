use crate::{Error, MAX_IMAGE_BYTES, canonical, extent, overlaps, u16_at, u32_at, u64_at};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Segment {
    pub kind: u32,
    pub flags: u32,
    pub offset: u64,
    pub virtual_address: u64,
    pub file_bytes: u64,
    pub memory_bytes: u64,
    pub alignment: u64,
}

#[derive(Debug)]
pub struct ElfImage<'a> {
    bytes: &'a [u8],
    pub entry: u64,
    headers: usize,
    count: usize,
}

impl<'a> ElfImage<'a> {
    /// Initial static x64 executable profile. Rejects interpreters/dynamic linking.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::Limit);
        }
        let header = bytes.get(..64).ok_or(Error::Truncated)?;
        if &header[..4] != b"\x7fELF" {
            return Err(Error::BadMagic);
        }
        if header[4..7] != [2, 1, 1]
            || !matches!(header[7], 0 | 3)
            || header[8] != 0
            || u16_at(header, 16)? != 2
            || u16_at(header, 18)? != 62
            || u32_at(header, 20)? != 1
        {
            return Err(Error::Unsupported);
        }
        if header[9..16].iter().any(|byte| *byte != 0)
            || u32_at(header, 48)? != 0
            || u16_at(header, 52)? != 64
            || u16_at(header, 54)? != 56
        {
            return Err(Error::Unsupported);
        }
        let count = usize::from(u16_at(header, 56)?);
        if count == 0 || count > 128 {
            return Err(Error::Limit);
        }
        let headers = extent(u64_at(header, 32)?, (count * 56) as u64, bytes.len() as u64)?;
        if headers.start < 64 || !headers.start.is_multiple_of(8) {
            return Err(Error::Alignment);
        }
        let image = Self {
            bytes,
            entry: u64_at(header, 24)?,
            headers: headers.start,
            count,
        };
        let mut entry_valid = false;
        let mut loads = 0;
        let mut memory_total = 0_u64;
        for index in 0..count {
            let segment = image.segment(index)?;
            if matches!(segment.kind, 2 | 3) {
                return Err(Error::DynamicDependency);
            }
            if !matches!(segment.kind, 0 | 1 | 4 | 6 | 0x6474_e551 | 0x6474_e552) {
                return Err(Error::Unsupported);
            }
            if segment.kind != 0 {
                extent(segment.offset, segment.file_bytes, bytes.len() as u64)?;
            }
            if segment.kind == 0x6474_e551 && segment.flags & 1 != 0 {
                return Err(Error::Permissions);
            }
            if segment.kind != 1 {
                continue;
            }
            loads += 1;
            if segment.memory_bytes == 0 || segment.file_bytes > segment.memory_bytes {
                return Err(Error::Bounds);
            }
            extent(segment.offset, segment.file_bytes, bytes.len() as u64)?;
            let end = segment
                .virtual_address
                .checked_add(segment.memory_bytes)
                .ok_or(Error::Overflow)?;
            memory_total = memory_total
                .checked_add(segment.memory_bytes)
                .ok_or(Error::Overflow)?;
            if memory_total > 1024 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            if !canonical(segment.virtual_address)
                || !canonical(end - 1)
                || (segment.virtual_address < 0x0000_8000_0000_0000 && end > 0x0000_8000_0000_0000)
            {
                return Err(Error::Bounds);
            }
            if !segment.alignment.is_power_of_two()
                || segment.alignment < 4096
                || !segment.virtual_address.is_multiple_of(4096)
                || !segment.offset.is_multiple_of(4096)
                || segment.offset % segment.alignment != segment.virtual_address % segment.alignment
            {
                return Err(Error::Alignment);
            }
            if segment.flags & !7 != 0 || segment.flags & 4 == 0 || segment.flags & 3 == 3 {
                return Err(Error::Permissions);
            }
            for prior in 0..index {
                let other = image.segment(prior)?;
                if other.kind == 1 && other.virtual_address > segment.virtual_address {
                    return Err(Error::Unsupported);
                }
                if other.kind == 1
                    && (overlaps(
                        segment.virtual_address,
                        segment.memory_bytes,
                        other.virtual_address,
                        other.memory_bytes,
                    ) || overlaps(
                        segment.offset,
                        segment.file_bytes,
                        other.offset,
                        other.file_bytes,
                    ))
                {
                    return Err(Error::Overlap);
                }
            }
            if segment.flags & 1 != 0
                && image.entry >= segment.virtual_address
                && image.entry < segment.virtual_address + segment.file_bytes
            {
                entry_valid = true;
            }
        }
        if loads == 0 || !entry_valid {
            return Err(Error::InvalidEntry);
        }
        let section_count = usize::from(u16_at(header, 60)?);
        if section_count > 4096 {
            return Err(Error::Limit);
        }
        if section_count != 0 {
            if u16_at(header, 58)? != 64 {
                return Err(Error::Unsupported);
            }
            let sections = extent(
                u64_at(header, 40)?,
                (section_count * 64) as u64,
                bytes.len() as u64,
            )?;
            for index in 0..section_count {
                let s = sections.start + index * 64;
                let kind = u32_at(bytes, s + 4)?;
                let length = u64_at(bytes, s + 32)?;
                if kind != 0 && kind != 8 {
                    extent(u64_at(bytes, s + 24)?, length, bytes.len() as u64)?;
                }
                if matches!(kind, 4 | 9) && length != 0 {
                    return Err(Error::Unsupported);
                }
                if kind == 6 {
                    return Err(Error::DynamicDependency);
                }
            }
        }
        Ok(image)
    }
    pub fn segment_count(&self) -> usize {
        self.count
    }
    pub fn segment(&self, index: usize) -> Result<Segment, Error> {
        if index >= self.count {
            return Err(Error::Bounds);
        }
        let p = self.headers + index * 56;
        Ok(Segment {
            kind: u32_at(self.bytes, p)?,
            flags: u32_at(self.bytes, p + 4)?,
            offset: u64_at(self.bytes, p + 8)?,
            virtual_address: u64_at(self.bytes, p + 16)?,
            file_bytes: u64_at(self.bytes, p + 32)?,
            memory_bytes: u64_at(self.bytes, p + 40)?,
            alignment: u64_at(self.bytes, p + 48)?,
        })
    }
}
