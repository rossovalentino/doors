//! One PE image at a time, for a user-mode loader (P18, ntdll's LdrLoadDll):
//! parse the headers of a file, lay it out as mapped (headers and sections
//! at their RVAs, the rest zero), apply base relocations for any base,
//! walk the import descriptors, look exports up by name or ordinal
//! (forwarders reported as such), and give each section's page protection
//! and the TLS directory. Everything works on byte slices: the file as
//! read, and the image as mapped (`size_of_image` bytes), so the same code
//! serves PE32 and PE32+ and runs under host tests.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// Not a PE file, or a header field out of range.
    Malformed,
    /// A machine, magic or layout this loader does not take.
    Unsupported,
    /// A table points outside the image.
    Bounds,
}

pub const MAX_SECTIONS: usize = 96;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;
const IMAGE_SCN_CNT_UNINITIALIZED_DATA: u32 = 0x80;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Section {
    pub rva: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}
impl Section {
    /// The section's extent in the mapped image (at least its raw data).
    pub fn mapped_size(&self) -> u32 {
        self.virtual_size.max(self.raw_size)
    }
    /// PAGE_* protection of the section's pages. Writable sections are
    /// private copies here, so READWRITE rather than WRITECOPY.
    pub fn protection(&self) -> u32 {
        let c = self.characteristics;
        match (
            c & IMAGE_SCN_MEM_EXECUTE != 0,
            c & IMAGE_SCN_MEM_READ != 0,
            c & IMAGE_SCN_MEM_WRITE != 0,
        ) {
            (true, _, true) => 0x40,      // EXECUTE_READWRITE
            (true, true, false) => 0x20,  // EXECUTE_READ
            (true, false, false) => 0x10, // EXECUTE
            (false, _, true) => 0x04,     // READWRITE
            (false, true, false) => 0x02, // READONLY
            (false, false, false) => 0x01,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Headers {
    /// 0x8664 (x64) or 0x14c (x86).
    pub machine: u16,
    pub pe32_plus: bool,
    pub characteristics: u16,
    pub image_base: u64,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub entry: u32,
    pub subsystem: u16,
    pub dll_characteristics: u16,
    pub stack_reserve: u64,
    pub stack_commit: u64,
    pub directories: [(u32, u32); 16],
    pub sections: [Section; MAX_SECTIONS],
    pub section_count: usize,
}
impl Headers {
    pub fn sections(&self) -> &[Section] {
        &self.sections[..self.section_count]
    }
    pub fn directory(&self, index: usize) -> (u32, u32) {
        self.directories[index]
    }
    pub fn is_dll(&self) -> bool {
        self.characteristics & 0x2000 != 0
    }
    pub fn pointer_bytes(&self) -> usize {
        if self.pe32_plus { 8 } else { 4 }
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or(Error::Bounds)?
            .try_into()
            .expect("two bytes"),
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(Error::Bounds)?
            .try_into()
            .expect("four bytes"),
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .ok_or(Error::Bounds)?
            .try_into()
            .expect("eight bytes"),
    ))
}

/// The headers of a PE file (or of a mapped image: the header bytes are
/// the same).
pub fn parse_headers(file: &[u8]) -> Result<Headers, Error> {
    if file.get(..2) != Some(b"MZ") {
        return Err(Error::Malformed);
    }
    let nt = u32_at(file, 0x3c)? as usize;
    if file.get(nt..nt + 4) != Some(b"PE\0\0") {
        return Err(Error::Malformed);
    }
    let machine = u16_at(file, nt + 4)?;
    let count = u16_at(file, nt + 6)? as usize;
    let optional_size = u16_at(file, nt + 20)? as usize;
    let characteristics = u16_at(file, nt + 22)?;
    let optional = nt + 24;
    let magic = u16_at(file, optional)?;
    let pe32_plus = match (magic, machine) {
        // x86 with PE32+ headers: an IL-only image as the x64 loader
        // converted it (see convert_il_only).
        (0x20b, 0x8664 | 0x14c) => true,
        (0x10b, 0x14c) => false,
        _ => return Err(Error::Unsupported),
    };
    let (image_base, stack_reserve, stack_commit, directory_count, directory_at) = if pe32_plus {
        (
            u64_at(file, optional + 24)?,
            u64_at(file, optional + 72)?,
            u64_at(file, optional + 80)?,
            u32_at(file, optional + 108)? as usize,
            optional + 112,
        )
    } else {
        (
            u64::from(u32_at(file, optional + 28)?),
            u64::from(u32_at(file, optional + 72)?),
            u64::from(u32_at(file, optional + 76)?),
            u32_at(file, optional + 92)? as usize,
            optional + 96,
        )
    };
    let mut directories = [(0, 0); 16];
    for (i, slot) in directories
        .iter_mut()
        .enumerate()
        .take(directory_count.min(16))
    {
        *slot = (
            u32_at(file, directory_at + i * 8)?,
            u32_at(file, directory_at + i * 8 + 4)?,
        );
    }
    let size_of_image = u32_at(file, optional + 56)?;
    let size_of_headers = u32_at(file, optional + 60)?;
    if count > MAX_SECTIONS || size_of_image == 0 || size_of_headers > size_of_image {
        return Err(Error::Unsupported);
    }
    let mut sections = [Section::default(); MAX_SECTIONS];
    let table = optional + optional_size;
    for (i, section) in sections.iter_mut().enumerate().take(count) {
        let at = table + i * 40;
        *section = Section {
            virtual_size: u32_at(file, at + 8)?,
            rva: u32_at(file, at + 12)?,
            raw_size: u32_at(file, at + 16)?,
            raw_offset: u32_at(file, at + 20)?,
            characteristics: u32_at(file, at + 36)?,
        };
        if u64::from(section.rva) + u64::from(section.mapped_size())
            > u64::from(size_of_image) + 0xfff
        {
            return Err(Error::Bounds);
        }
    }
    Ok(Headers {
        machine,
        pe32_plus,
        characteristics,
        image_base,
        size_of_image,
        size_of_headers,
        entry: u32_at(file, optional + 16)?,
        subsystem: u16_at(file, optional + 68)?,
        dll_characteristics: u16_at(file, optional + 70)?,
        stack_reserve,
        stack_commit,
        directories,
        sections,
        section_count: count,
    })
}

const COMIMAGE_FLAGS_ILONLY: u32 = 1;
const COMIMAGE_FLAGS_32BITREQUIRED: u32 = 2;

/// Whether the mapped image is IL-only .NET code with no 32-bit
/// requirement (COMIMAGE_FLAGS_ILONLY without 32BITREQUIRED): an AnyCPU
/// assembly, which Windows loads into a process of either width.
pub fn is_il_only(image: &[u8], headers: &Headers) -> bool {
    let (rva, size) = headers.directory(14);
    if rva == 0 || size < 20 {
        return false;
    }
    u32_at(image, rva as usize + 16).is_ok_and(|flags| {
        flags & COMIMAGE_FLAGS_ILONLY != 0 && flags & COMIMAGE_FLAGS_32BITREQUIRED == 0
    })
}

const PE32_PLUS_OPTIONAL_BYTES: usize = 240;

/// Rewrite a mapped IL-only image's headers as the Windows x64 loader
/// does before an AnyCPU assembly runs in a 64-bit process: PE32+ optional
/// header (the section table moves after it), and no imports, import
/// address table or entry point, since nothing native in it runs (the x86
/// stub that would call mscoree). Resources, security, relocations, debug
/// and the COM descriptor stay; the machine stays x86, so its HIGHLOW
/// relocation still validates. The headers as now mapped are returned.
pub fn convert_il_only(image: &mut [u8], headers: &Headers) -> Result<Headers, Error> {
    let nt = u32_at(image, 0x3c)? as usize;
    let optional = nt + 24;
    if headers.pe32_plus {
        image
            .get_mut(optional + 16..optional + 20)
            .ok_or(Error::Bounds)?
            .fill(0);
        for directory in [1usize, 12] {
            let at = optional + 112 + directory * 8;
            image.get_mut(at..at + 8).ok_or(Error::Bounds)?.fill(0);
        }
        return parse_headers(image);
    }
    let old_size = usize::from(u16_at(image, nt + 20)?);
    let table_bytes = headers.section_count * 40;
    // The headers' page ends where the first section starts.
    let limit = headers
        .sections()
        .iter()
        .map(|s| s.rva as usize)
        .filter(|rva| *rva != 0)
        .min()
        .unwrap_or(image.len())
        .min(image.len());
    if optional + PE32_PLUS_OPTIONAL_BYTES + table_bytes > limit
        || optional + old_size + table_bytes > limit
    {
        return Err(Error::Bounds);
    }
    let old: [u8; 96] = image[optional..optional + 96]
        .try_into()
        .map_err(|_| Error::Bounds)?;
    let mut new = [0u8; PE32_PLUS_OPTIONAL_BYTES];
    // Magic through DllCharacteristics share their offsets, except that
    // BaseOfData and the 32-bit ImageBase become the 64-bit ImageBase.
    new[..72].copy_from_slice(&old[..72]);
    new[0..2].copy_from_slice(&0x20bu16.to_le_bytes());
    new[16..20].fill(0);
    new[24..32].copy_from_slice(&headers.image_base.to_le_bytes());
    for (i, at) in [72usize, 76, 80, 84].into_iter().enumerate() {
        let value = u64::from(u32::from_le_bytes(old[at..at + 4].try_into().unwrap()));
        new[72 + i * 8..80 + i * 8].copy_from_slice(&value.to_le_bytes());
    }
    new[104..108].copy_from_slice(&old[88..92]);
    new[108..112].copy_from_slice(&16u32.to_le_bytes());
    for directory in [2usize, 4, 5, 6, 14] {
        let (rva, size) = headers.directory(directory);
        let at = 112 + directory * 8;
        new[at..at + 4].copy_from_slice(&rva.to_le_bytes());
        new[at + 4..at + 8].copy_from_slice(&size.to_le_bytes());
    }
    image.copy_within(
        optional + old_size..optional + old_size + table_bytes,
        optional + PE32_PLUS_OPTIONAL_BYTES,
    );
    image[optional..optional + PE32_PLUS_OPTIONAL_BYTES].copy_from_slice(&new);
    image[nt + 20..nt + 22].copy_from_slice(&(PE32_PLUS_OPTIONAL_BYTES as u16).to_le_bytes());
    parse_headers(image)
}

/// Lay `file` out in `image` (`size_of_image` bytes, zeroed by the
/// caller's fresh commit): headers, then each section's raw data at its
/// RVA. Uninitialized data stays zero.
pub fn map(file: &[u8], headers: &Headers, image: &mut [u8]) -> Result<(), Error> {
    map_from(file.len() as u64, headers, image, |offset, out| {
        let start = usize::try_from(offset).map_err(|_| Error::Bounds)?;
        out.copy_from_slice(file.get(start..start + out.len()).ok_or(Error::Bounds)?);
        Ok(())
    })
}
/// `map` reading the file through `read(offset, destination)` (a file of
/// `file_bytes`): each part is read straight into its place in the image,
/// so a large image never needs a second copy of its file in memory.
pub fn map_from(
    file_bytes: u64,
    headers: &Headers,
    image: &mut [u8],
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    let header_bytes = (headers.size_of_headers as u64).min(file_bytes) as usize;
    read(0, image.get_mut(..header_bytes).ok_or(Error::Bounds)?)?;
    for section in headers.sections() {
        if section.characteristics & IMAGE_SCN_CNT_UNINITIALIZED_DATA != 0
            && section.raw_offset == 0
        {
            continue;
        }
        let length = section.raw_size.min(section.mapped_size()) as usize;
        if length == 0 {
            continue;
        }
        if u64::from(section.raw_offset) + length as u64 > file_bytes {
            return Err(Error::Bounds);
        }
        read(
            u64::from(section.raw_offset),
            image
                .get_mut(section.rva as usize..section.rva as usize + length)
                .ok_or(Error::Bounds)?,
        )?;
    }
    Ok(())
}

/// Apply base relocations for loading at `new_base` (a no-op at the
/// preferred base). HIGHLOW, DIR64, HIGH, LOW and HIGHADJ entries.
pub fn relocate(image: &mut [u8], headers: &Headers, new_base: u64) -> Result<(), Error> {
    let delta = new_base.wrapping_sub(headers.image_base);
    if delta == 0 {
        return Ok(());
    }
    let (rva, size) = headers.directory(5);
    if rva == 0 || size == 0 {
        // Stripped relocations: the image can only run at its own base.
        return Err(Error::Unsupported);
    }
    let end = rva as usize + size as usize;
    let mut block = rva as usize;
    while block + 8 <= end {
        let page = u32_at(image, block)? as usize;
        let bytes = u32_at(image, block + 4)? as usize;
        if bytes < 8 || block + bytes > end {
            return Err(Error::Malformed);
        }
        let mut entry = block + 8;
        while entry + 2 <= block + bytes {
            let value = u16_at(image, entry)?;
            let at = page + usize::from(value & 0xfff);
            match value >> 12 {
                0 => {}
                3 => {
                    let v = u32_at(image, at)?.wrapping_add(delta as u32);
                    image[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
                10 => {
                    let v = u64_at(image, at)?.wrapping_add(delta);
                    image[at..at + 8].copy_from_slice(&v.to_le_bytes());
                }
                1 => {
                    let v = u16_at(image, at)?.wrapping_add((delta >> 16) as u16);
                    image[at..at + 2].copy_from_slice(&v.to_le_bytes());
                }
                2 => {
                    let v = u16_at(image, at)?.wrapping_add(delta as u16);
                    image[at..at + 2].copy_from_slice(&v.to_le_bytes());
                }
                4 => {
                    // HIGHADJ: the next entry holds the low half.
                    let low = u16_at(image, entry + 2)?;
                    let full = (u32::from(u16_at(image, at)?) << 16) + u32::from(low);
                    let adjusted = full.wrapping_add(delta as u32).wrapping_add(0x8000) >> 16;
                    image[at..at + 2].copy_from_slice(&(adjusted as u16).to_le_bytes());
                    entry += 2;
                }
                _ => return Err(Error::Unsupported),
            }
            entry += 2;
        }
        block += bytes;
    }
    Ok(())
}

/// The NUL-terminated ASCII string at `rva`.
pub fn string_at(image: &[u8], rva: u32) -> Result<&[u8], Error> {
    let tail = image.get(rva as usize..).ok_or(Error::Bounds)?;
    let length = tail.iter().position(|b| *b == 0).ok_or(Error::Bounds)?;
    Ok(&tail[..length])
}

/// One imported symbol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Import<'a> {
    Name { hint: u16, name: &'a [u8] },
    Ordinal(u16),
}
/// Walk every import of the mapped `image`: `resolve(dll, symbol)` gives
/// the address written into that IAT slot. Stops at the first error.
pub fn bind_imports<E>(
    image: &mut [u8],
    headers: &Headers,
    mut resolve: impl FnMut(&[u8], Import<'_>) -> Result<u64, E>,
) -> Result<(), BindError<E>> {
    let (rva, _) = headers.directory(1);
    if rva == 0 {
        return Ok(());
    }
    let width = headers.pointer_bytes();
    let ordinal_flag = if headers.pe32_plus {
        1u64 << 63
    } else {
        1u64 << 31
    };
    let mut descriptor = rva as usize;
    loop {
        let lookup = u32_at(image, descriptor)?;
        let name = u32_at(image, descriptor + 12)?;
        let first = u32_at(image, descriptor + 16)?;
        if name == 0 && first == 0 {
            return Ok(());
        }
        let mut dll = [0u8; 256];
        let dll_name = string_at(image, name)?;
        let n = dll_name.len().min(dll.len());
        dll[..n].copy_from_slice(&dll_name[..n]);
        let table = if lookup != 0 { lookup } else { first } as usize;
        let mut index = 0usize;
        loop {
            let at = table + index * width;
            let entry = if width == 8 {
                u64_at(image, at)?
            } else {
                u64::from(u32_at(image, at)?)
            };
            if entry == 0 {
                break;
            }
            let address = if entry & ordinal_flag != 0 {
                resolve(&dll[..n], Import::Ordinal(entry as u16))
            } else {
                let by_name = (entry & 0x7fff_ffff) as u32;
                let hint = u16_at(image, by_name as usize)?;
                let symbol = string_at(image, by_name + 2)?;
                let mut copy = [0u8; 512];
                let m = symbol.len().min(copy.len());
                copy[..m].copy_from_slice(&symbol[..m]);
                resolve(
                    &dll[..n],
                    Import::Name {
                        hint,
                        name: &copy[..m],
                    },
                )
            }
            .map_err(BindError::Resolve)?;
            let slot = first as usize + index * width;
            if width == 8 {
                image
                    .get_mut(slot..slot + 8)
                    .ok_or(Error::Bounds)?
                    .copy_from_slice(&address.to_le_bytes());
            } else {
                image
                    .get_mut(slot..slot + 4)
                    .ok_or(Error::Bounds)?
                    .copy_from_slice(&(address as u32).to_le_bytes());
            }
            index += 1;
        }
        descriptor += 20;
    }
}
#[derive(Debug, Eq, PartialEq)]
pub enum BindError<E> {
    Image(Error),
    Resolve(E),
}
impl<E> From<Error> for BindError<E> {
    fn from(error: Error) -> Self {
        Self::Image(error)
    }
}

/// An export: its RVA, or a forwarder ("DLL.Symbol" or "DLL.#ordinal").
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Export<'a> {
    Rva(u32),
    Forwarder(&'a [u8]),
}
/// Look `symbol` up in the mapped image's export directory.
pub fn export<'a>(
    image: &'a [u8],
    headers: &Headers,
    symbol: Import<'_>,
) -> Result<Option<Export<'a>>, Error> {
    let (rva, size) = headers.directory(0);
    if rva == 0 {
        return Ok(None);
    }
    let directory = rva as usize;
    let base = u32_at(image, directory + 16)?;
    let functions = u32_at(image, directory + 20)?;
    let names = u32_at(image, directory + 24)?;
    let address_table = u32_at(image, directory + 28)? as usize;
    let name_table = u32_at(image, directory + 32)? as usize;
    let ordinal_table = u32_at(image, directory + 36)? as usize;
    let index = match symbol {
        Import::Ordinal(ordinal) => {
            let index = u32::from(ordinal).checked_sub(base);
            match index {
                Some(i) if i < functions => i,
                _ => return Ok(None),
            }
        }
        Import::Name { hint, name } => {
            let name_matches = |i: u32| -> Result<bool, Error> {
                let at = u32_at(image, name_table + i as usize * 4)?;
                Ok(string_at(image, at)? == name)
            };
            let found = if u32::from(hint) < names && name_matches(u32::from(hint))? {
                Some(u32::from(hint))
            } else {
                // Names are sorted: binary search.
                let (mut low, mut high) = (0u32, names);
                let mut found = None;
                while low < high {
                    let middle = low + (high - low) / 2;
                    let at = u32_at(image, name_table + middle as usize * 4)?;
                    match string_at(image, at)?.cmp(name) {
                        core::cmp::Ordering::Less => low = middle + 1,
                        core::cmp::Ordering::Greater => high = middle,
                        core::cmp::Ordering::Equal => {
                            found = Some(middle);
                            break;
                        }
                    }
                }
                found.or_else(|| (0..names).find(|i| name_matches(*i).unwrap_or(false)))
            };
            let Some(position) = found else {
                return Ok(None);
            };
            u32::from(u16_at(image, ordinal_table + position as usize * 2)?)
        }
    };
    let target = u32_at(image, address_table + index as usize * 4)?;
    if target == 0 {
        return Ok(None);
    }
    if size != 0 && target >= rva && target < rva + size {
        return Ok(Some(Export::Forwarder(string_at(image, target)?)));
    }
    Ok(Some(Export::Rva(target)))
}

/// The TLS directory (addresses as relocated in the image).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tls {
    pub start: u64,
    pub end: u64,
    pub index: u64,
    pub callbacks: u64,
    pub zero_fill: u32,
}
pub fn tls(image: &[u8], headers: &Headers) -> Result<Option<Tls>, Error> {
    let (rva, _) = headers.directory(9);
    if rva == 0 {
        return Ok(None);
    }
    let at = rva as usize;
    Ok(Some(if headers.pe32_plus {
        Tls {
            start: u64_at(image, at)?,
            end: u64_at(image, at + 8)?,
            index: u64_at(image, at + 16)?,
            callbacks: u64_at(image, at + 24)?,
            zero_fill: u32_at(image, at + 32)?,
        }
    } else {
        Tls {
            start: u64::from(u32_at(image, at)?),
            end: u64::from(u32_at(image, at + 4)?),
            index: u64::from(u32_at(image, at + 8)?),
            callbacks: u64::from(u32_at(image, at + 12)?),
            zero_fill: u32_at(image, at + 16)?,
        }
    }))
}
