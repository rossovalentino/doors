use crate::{Error, MAX_IMAGE_BYTES, extent, overlaps, u16_at, u32_at, u64_at};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Section {
    pub virtual_address: u32,
    pub virtual_bytes: u32,
    pub raw_offset: u32,
    pub raw_bytes: u32,
    pub flags: u32,
}
impl Section {
    pub fn mapped_bytes(self) -> u32 {
        self.virtual_bytes.max(self.raw_bytes)
    }
}

/// The image's machine: x64 (PE32+), or x86 (PE32) for 32-bit programs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Machine {
    X64,
    X86,
}
impl Machine {
    /// The width of a pointer, an import thunk or a TLS callback slot.
    pub const fn pointer_bytes(self) -> usize {
        match self {
            Self::X64 => 8,
            Self::X86 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PeImage<'a> {
    bytes: &'a [u8],
    sections: usize,
    count: usize,
    optional: usize,
    /// Where the data directory table starts (after the optional header's
    /// fixed fields, which are 112 bytes in PE32+ and 96 in PE32).
    table: usize,
    pub machine: Machine,
    pub entry_rva: u32,
    pub image_base: u64,
    pub subsystem: u16,
    pub characteristics: u16,
    pub image_bytes: u32,
    headers_bytes: u32,
    directories: usize,
}
impl<'a> PeImage<'a> {
    /// Structural parse for inspectors and initial loaders. A delay-import
    /// directory is refused: it must never become an empty dependency set.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, false, false)
    }
    /// `parse` for x64 or x86 (PE32) images, which a loader selecting the
    /// process's personality tells apart by `machine`.
    pub fn parse_any(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, false, true)
    }
    /// `parse_with_delay_imports` for x64 or x86 images.
    pub fn parse_any_with_delay_imports(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, true, true)
    }
    /// Same validation, but leaves a present delay-import directory to a loader
    /// that validates and binds it itself. Its extent is still checked as
    /// file-backed RVA bytes; no dependency is inferred from its absence.
    pub fn parse_with_delay_imports(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::parse_with(bytes, true, false)
    }
    fn parse_with(bytes: &'a [u8], delay_imports: bool, x86: bool) -> Result<Self, Error> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(Error::Limit);
        }
        if bytes.get(..2).ok_or(Error::Truncated)? != b"MZ" {
            return Err(Error::BadMagic);
        }
        let pe = u32_at(bytes, 0x3c)? as usize;
        if pe < 64
            || bytes
                .get(pe..pe.checked_add(24).ok_or(Error::Overflow)?)
                .ok_or(Error::Truncated)?[..4]
                != *b"PE\0\0"
        {
            return Err(Error::BadMagic);
        }
        let machine = match u16_at(bytes, pe + 4)? {
            0x8664 => Machine::X64,
            0x014c if x86 => Machine::X86,
            _ => return Err(Error::Unsupported),
        };
        let count = usize::from(u16_at(bytes, pe + 6)?);
        if count == 0 || count > 96 {
            return Err(Error::Limit);
        }
        let optional_bytes = usize::from(u16_at(bytes, pe + 20)?);
        let optional = pe + 24;
        extent(optional as u64, optional_bytes as u64, bytes.len() as u64)?;
        // PE32+ (0x20b) for x64, PE32 (0x10b) for x86: the fixed fields
        // before the data directories differ (the image base is 4 bytes in
        // PE32, which also has BaseOfData; the stack and heap sizes too).
        let (magic, fixed) = match machine {
            Machine::X64 => (0x20b, 112),
            Machine::X86 => (0x10b, 96),
        };
        if optional_bytes < fixed || u16_at(bytes, optional)? != magic {
            return Err(Error::Unsupported);
        }
        let directories = u32_at(bytes, optional + fixed - 4)? as usize;
        if directories > 16 || fixed + directories * 8 > optional_bytes {
            return Err(Error::Limit);
        }
        let sections = optional + optional_bytes;
        extent(sections as u64, (count * 40) as u64, bytes.len() as u64)?;
        let file_align = u32_at(bytes, optional + 36)?;
        let section_align = u32_at(bytes, optional + 32)?;
        if !file_align.is_power_of_two()
            || !(512..=65536).contains(&file_align)
            || !section_align.is_power_of_two()
            || section_align < 4096
            || section_align < file_align
        {
            return Err(Error::Alignment);
        }
        let image = Self {
            bytes,
            sections,
            count,
            optional,
            table: optional + fixed,
            machine,
            directories,
            entry_rva: u32_at(bytes, optional + 16)?,
            image_base: match machine {
                Machine::X64 => u64_at(bytes, optional + 24)?,
                Machine::X86 => u64::from(u32_at(bytes, optional + 28)?),
            },
            subsystem: u16_at(bytes, optional + 68)?,
            characteristics: u16_at(bytes, pe + 22)?,
            image_bytes: u32_at(bytes, optional + 56)?,
            headers_bytes: u32_at(bytes, optional + 60)?,
        };
        if image.headers_bytes as usize > bytes.len()
            || (image.headers_bytes as usize) < sections + count * 40
            || !image.headers_bytes.is_multiple_of(file_align)
            || !image.image_bytes.is_multiple_of(section_align)
            || !image.image_base.is_multiple_of(65536)
            || image
                .image_base
                .checked_add(u64::from(image.image_bytes))
                .is_none()
        {
            return Err(Error::Bounds);
        }
        let mut entry_valid = false;
        for index in 0..count {
            let section = image.section(index)?;
            let size = section.mapped_bytes();
            if size == 0 {
                return Err(Error::Bounds);
            }
            extent(
                u64::from(section.virtual_address),
                u64::from(size),
                u64::from(image.image_bytes),
            )?;
            extent(
                u64::from(section.raw_offset),
                u64::from(section.raw_bytes),
                bytes.len() as u64,
            )?;
            if section.virtual_address < image.headers_bytes
                || !section.virtual_address.is_multiple_of(section_align)
                || (section.raw_bytes != 0
                    && (section.raw_offset < image.headers_bytes
                        || !section.raw_offset.is_multiple_of(file_align)
                        || !section.raw_bytes.is_multiple_of(file_align)))
            {
                return Err(Error::Alignment);
            }
            if section.flags & 0xa000_0000 == 0xa000_0000 {
                return Err(Error::Permissions);
            }
            for prior in 0..index {
                let other = image.section(prior)?;
                if overlaps(
                    u64::from(section.virtual_address),
                    u64::from(size),
                    u64::from(other.virtual_address),
                    u64::from(other.mapped_bytes()),
                ) || overlaps(
                    u64::from(section.raw_offset),
                    u64::from(section.raw_bytes),
                    u64::from(other.raw_offset),
                    u64::from(other.raw_bytes),
                ) {
                    return Err(Error::Overlap);
                }
            }
            if section.flags & 0x2000_0000 != 0
                && image.entry_rva >= section.virtual_address
                && u64::from(image.entry_rva)
                    < u64::from(section.virtual_address) + u64::from(section.raw_bytes)
            {
                entry_valid = true;
            }
        }
        // DLLs may intentionally have no entry point; executables may not.
        if !(entry_valid || image.entry_rva == 0 && image.characteristics & 0x2000 != 0) {
            return Err(Error::InvalidEntry);
        }
        for directory in 0..directories {
            let (rva, size) = image.directory(directory)?;
            if rva == 0 && size == 0 {
                continue;
            }
            if rva == 0 || size == 0 {
                return Err(Error::Bounds);
            }
            if directory == 4 {
                extent(u64::from(rva), u64::from(size), bytes.len() as u64)?;
            } else {
                image.rva_bytes(rva, size as usize)?;
            }
        }
        image.validate_imports(delay_imports)?;
        image.validate_exports()?;
        Ok(image)
    }
    pub fn section_count(&self) -> usize {
        self.count
    }
    pub fn headers_bytes(&self) -> u32 {
        self.headers_bytes
    }
    pub fn dll_characteristics(&self) -> Result<u16, Error> {
        u16_at(self.bytes, self.optional + 70)
    }
    /// Translate a completely file-backed RVA extent, without interpreting it
    /// as an address in a loaded process.
    pub fn file_offset(&self, rva: u32, length: usize) -> Result<u32, Error> {
        self.rva_bytes(rva, length)?;
        if rva < self.headers_bytes {
            return Ok(rva);
        }
        for index in 0..self.count {
            let section = self.section(index)?;
            if rva >= section.virtual_address
                && u64::from(rva - section.virtual_address) + length as u64
                    <= u64::from(section.raw_bytes)
            {
                return section
                    .raw_offset
                    .checked_add(rva - section.virtual_address)
                    .ok_or(Error::Overflow);
            }
        }
        Err(Error::Bounds)
    }
    pub fn section(&self, index: usize) -> Result<Section, Error> {
        if index >= self.count {
            return Err(Error::Bounds);
        }
        let s = self.sections + index * 40;
        Ok(Section {
            virtual_bytes: u32_at(self.bytes, s + 8)?,
            virtual_address: u32_at(self.bytes, s + 12)?,
            raw_bytes: u32_at(self.bytes, s + 16)?,
            raw_offset: u32_at(self.bytes, s + 20)?,
            flags: u32_at(self.bytes, s + 36)?,
        })
    }
    pub fn directory(&self, index: usize) -> Result<(u32, u32), Error> {
        if index >= 16 {
            return Err(Error::Bounds);
        }
        if index >= self.directories {
            return Ok((0, 0));
        }
        Ok((
            u32_at(self.bytes, self.table + index * 8)?,
            u32_at(self.bytes, self.table + 4 + index * 8)?,
        ))
    }
    pub fn rva_bytes(&self, rva: u32, length: usize) -> Result<&'a [u8], Error> {
        if rva < self.headers_bytes {
            let range = extent(u64::from(rva), length as u64, u64::from(self.headers_bytes))?;
            return self.bytes.get(range).ok_or(Error::Bounds);
        }
        for index in 0..self.count {
            let section = self.section(index)?;
            if rva >= section.virtual_address {
                let relative = u64::from(rva - section.virtual_address);
                if relative.checked_add(length as u64).ok_or(Error::Overflow)?
                    <= u64::from(section.raw_bytes)
                {
                    let range = extent(
                        u64::from(section.raw_offset) + relative,
                        length as u64,
                        self.bytes.len() as u64,
                    )?;
                    return self.bytes.get(range).ok_or(Error::Bounds);
                }
            }
        }
        Err(Error::Bounds)
    }
    /// A complete mapped section extent, including its zero-filled tail. This
    /// does not assert file backing, readability or executable permissions.
    pub fn mapped_section(&self, rva: u32, length: u32) -> Result<Section, Error> {
        let end = rva.checked_add(length).ok_or(Error::Overflow)?;
        for index in 0..self.count {
            let section = self.section(index)?;
            if rva >= section.virtual_address
                && u64::from(end)
                    <= u64::from(section.virtual_address) + u64::from(section.mapped_bytes())
            {
                return Ok(section);
            }
        }
        Err(Error::Bounds)
    }
    pub fn string(&self, rva: u32) -> Result<&'a str, Error> {
        for length in 1..=256 {
            let value = self.rva_bytes(rva, length)?;
            if value[length - 1] == 0 {
                let string = core::str::from_utf8(&value[..length - 1])
                    .map_err(|_| Error::MalformedString)?;
                if string.is_empty() || !string.bytes().all(|byte| byte.is_ascii_graphic()) {
                    return Err(Error::MalformedString);
                }
                return Ok(string);
            }
        }
        Err(Error::MalformedString)
    }
    pub fn import_count(&self) -> Result<usize, Error> {
        let (rva, size) = self.directory(1)?;
        if rva == 0 {
            return Ok(0);
        }
        let count = (size as usize / 20).min(1025);
        for index in 0..count {
            let descriptor = self.rva_bytes(
                rva.checked_add((index * 20) as u32)
                    .ok_or(Error::Overflow)?,
                20,
            )?;
            if descriptor.iter().all(|byte| *byte == 0) {
                return Ok(index);
            }
        }
        Err(Error::Limit)
    }
    pub fn import_name(&self, index: usize) -> Result<&'a str, Error> {
        if index >= self.import_count()? {
            return Err(Error::Bounds);
        }
        let (rva, _) = self.directory(1)?;
        let descriptor = self.rva_bytes(
            rva.checked_add((index * 20) as u32)
                .ok_or(Error::Overflow)?,
            20,
        )?;
        self.string(u32_at(descriptor, 12)?)
    }
    pub fn export_count(&self) -> Result<usize, Error> {
        let (rva, _) = self.directory(0)?;
        if rva == 0 {
            return Ok(0);
        }
        Ok(u32_at(self.rva_bytes(rva, 40)?, 24)? as usize)
    }
    pub fn export_name(&self, index: usize) -> Result<&'a str, Error> {
        if index >= self.export_count()? {
            return Err(Error::Bounds);
        }
        let (rva, _) = self.directory(0)?;
        let address = u32_at(self.rva_bytes(rva, 40)?, 32)?;
        let pointer = self.rva_bytes(
            address
                .checked_add((index * 4) as u32)
                .ok_or(Error::Overflow)?,
            4,
        )?;
        self.string(u32_at(pointer, 0)?)
    }
    fn validate_imports(&self, delay_imports: bool) -> Result<(), Error> {
        for index in 0..self.import_count()? {
            self.import_name(index)?;
        }
        // Delay imports are not parsed in this initial profile: never silently
        // treat them as an empty dependency set.
        if !delay_imports && self.directory(13)? != (0, 0) {
            return Err(Error::Unsupported);
        }
        Ok(())
    }
    fn validate_exports(&self) -> Result<(), Error> {
        let (rva, size) = self.directory(0)?;
        if rva != 0 {
            if size < 40 {
                return Err(Error::Bounds);
            }
            let table = self.rva_bytes(rva, 40)?;
            let functions = u32_at(table, 20)? as usize;
            let names = u32_at(table, 24)? as usize;
            if functions > 4096 || names > functions {
                return Err(Error::Limit);
            }
            let addresses = self.rva_bytes(u32_at(table, 28)?, functions * 4)?;
            self.rva_bytes(u32_at(table, 32)?, names * 4)?;
            let ordinals = self.rva_bytes(u32_at(table, 36)?, names * 2)?;
            for index in 0..names {
                let ordinal = usize::from(u16_at(ordinals, index * 2)?);
                if ordinal >= functions {
                    return Err(Error::Bounds);
                }
                let address = u32_at(addresses, ordinal * 4)?;
                if address == 0 {
                    return Err(Error::Bounds);
                }
                // Data exports may point into a zero-filled section, rather
                // than bytes stored in the image file. Forwarder strings remain
                // file-backed and are interpreted by the selected loader.
                if address >= rva && u64::from(address) < u64::from(rva) + u64::from(size) {
                    self.rva_bytes(address, 1)?;
                } else {
                    self.mapped_section(address, 1)?;
                }
            }
        }
        if self.export_count()? > 4096 {
            return Err(Error::Limit);
        }
        for index in 0..self.export_count()? {
            self.export_name(index)?;
        }
        Ok(())
    }
}

impl<'a> PeImage<'a> {
    /// The application manifest: the RT_MANIFEST (24) resource's first
    /// name and language, as the loader reads it.
    pub fn manifest(&self) -> Option<&'a [u8]> {
        let (rva, size) = self.directory(2).ok()?;
        if rva == 0 || size < 16 {
            return None;
        }
        let root = self.rva_bytes(rva, size as usize).ok()?;
        // One directory level: the entry with `id` (or the first), its
        // offset and whether it is a subdirectory.
        let entry = |at: usize, id: Option<u32>| -> Option<(usize, bool)> {
            let named = usize::from(u16_at(root, at + 12).ok()?);
            let ids = usize::from(u16_at(root, at + 14).ok()?);
            for index in 0..(named + ids).min(256) {
                let e = at + 16 + index * 8;
                let name = u32_at(root, e).ok()?;
                if id.is_none_or(|id| name == id) {
                    let target = u32_at(root, e + 4).ok()?;
                    return Some(((target & 0x7fff_ffff) as usize, target & 0x8000_0000 != 0));
                }
            }
            None
        };
        let (names, dir) = entry(0, Some(24))?;
        if !dir {
            return None;
        }
        let (languages, dir) = entry(names, None)?;
        if !dir {
            return None;
        }
        let (data, dir) = entry(languages, None)?;
        if dir {
            return None;
        }
        let data_rva = u32_at(root, data).ok()?;
        let data_size = u32_at(root, data + 4).ok()? as usize;
        if data_size > 64 * 1024 {
            return None;
        }
        self.rva_bytes(data_rva, data_size).ok()
    }
}

/// A manifest's requestedExecutionLevel (User Account Control).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionLevel {
    /// No requestedExecutionLevel element.
    Unspecified,
    AsInvoker,
    HighestAvailable,
    RequireAdministrator,
}
/// The requestedExecutionLevel of a manifest (UTF-8, or UTF-16 with its
/// byte order mark).
pub fn execution_level(manifest: &[u8]) -> ExecutionLevel {
    let mut text = [0u8; 8192];
    let mut n = 0;
    if manifest.starts_with(&[0xff, 0xfe]) {
        for pair in manifest[2..].chunks_exact(2) {
            if n == text.len() {
                break;
            }
            text[n] = if pair[1] == 0 { pair[0] } else { b'?' };
            n += 1;
        }
    } else {
        n = manifest.len().min(text.len());
        text[..n].copy_from_slice(&manifest[..n]);
    }
    let text = &text[..n];
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let Some(at) = find(text, b"requestedExecutionLevel") else {
        return ExecutionLevel::Unspecified;
    };
    let rest = &text[at..];
    let end = rest.iter().position(|b| *b == b'>').unwrap_or(rest.len());
    let element = &rest[..end];
    let Some(level) = find(element, b"level=") else {
        return ExecutionLevel::Unspecified;
    };
    let value = &element[level + 6..];
    let value = value
        .strip_prefix(b"\"")
        .or_else(|| value.strip_prefix(b"'"))
        .unwrap_or(value);
    if value.starts_with(b"requireAdministrator") {
        ExecutionLevel::RequireAdministrator
    } else if value.starts_with(b"highestAvailable") {
        ExecutionLevel::HighestAvailable
    } else if value.starts_with(b"asInvoker") {
        ExecutionLevel::AsInvoker
    } else {
        ExecutionLevel::Unspecified
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;
    #[test]
    fn execution_levels_read_as_windows_reads_them() {
        let m = br#"<?xml version="1.0"?><assembly><trustInfo><security><requestedPrivileges><requestedExecutionLevel level="requireAdministrator" uiAccess="false"/></requestedPrivileges></security></trustInfo></assembly>"#;
        assert_eq!(execution_level(m), ExecutionLevel::RequireAdministrator);
        assert_eq!(
            execution_level(b"<requestedExecutionLevel uiAccess='false' level='asInvoker'/>"),
            ExecutionLevel::AsInvoker
        );
        assert_eq!(
            execution_level(b"<requestedExecutionLevel level=\"highestAvailable\"/>"),
            ExecutionLevel::HighestAvailable
        );
        assert_eq!(execution_level(b"<assembly/>"), ExecutionLevel::Unspecified);
        let wide: alloc_free_utf16::Wide = alloc_free_utf16::Wide::of(
            b"<requestedExecutionLevel level=\"requireAdministrator\"/>",
        );
        assert_eq!(
            execution_level(wide.bytes()),
            ExecutionLevel::RequireAdministrator
        );
    }
    mod alloc_free_utf16 {
        pub struct Wide([u8; 256], usize);
        impl Wide {
            pub fn of(ascii: &[u8]) -> Self {
                let mut b = [0u8; 256];
                b[0] = 0xff;
                b[1] = 0xfe;
                for (i, c) in ascii.iter().enumerate() {
                    b[2 + 2 * i] = *c;
                }
                Self(b, 2 + 2 * ascii.len())
            }
            pub fn bytes(&self) -> &[u8] {
                &self.0[..self.1]
            }
        }
    }
}
