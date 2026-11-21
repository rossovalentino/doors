//! Bounded PE resource directory (data directory 2) validation and lookup.
//!
//! The same allocation-free code runs over immutable file bytes in the planner
//! and, in the guest adapter, over the mapped image through a checked-copy
//! `Reader`. Readers bound every access by the declared directory size, so a
//! malformed (or, for writable sections, modified) tree can at worst fail.
//! The tree must have exactly three levels (type, name, language); a fourth
//! level, a language-level name or a data entry above the language level is
//! malformed. Work is bounded per table and in total, which also bounds trees
//! that share subdirectories.

use crate::{Error as PlanError, Limit, MAX_MODULE_BYTES, ModulePlan, Reason, Rva};

pub const MAX_ENTRIES: u32 = 4096;
pub const MAX_VISITS: usize = 16384;
pub const MAX_NAME_UNITS: usize = 255;
pub const TABLE_BYTES: u32 = 16;
pub const ENTRY_BYTES: u32 = 8;
pub const DATA_ENTRY_BYTES: u32 = 16;
/// Thread, user and system UI language of the declared US-English profile.
pub const PROFILE_LANGUAGE: u16 = 0x0409;
pub const LANG_NEUTRAL: u16 = 0;
pub const SUBLANG_SYS_DEFAULT: u16 = 2;

/// Predefined resource types (`MAKEINTRESOURCE` values).
pub mod kind {
    pub const CURSOR: u16 = 1;
    pub const BITMAP: u16 = 2;
    pub const ICON: u16 = 3;
    pub const MENU: u16 = 4;
    pub const DIALOG: u16 = 5;
    pub const STRING: u16 = 6;
    pub const FONTDIR: u16 = 7;
    pub const FONT: u16 = 8;
    pub const ACCELERATOR: u16 = 9;
    pub const RCDATA: u16 = 10;
    pub const MESSAGETABLE: u16 = 11;
    pub const GROUP_CURSOR: u16 = 12;
    pub const GROUP_ICON: u16 = 14;
    pub const VERSION: u16 = 16;
    pub const MANIFEST: u16 = 24;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Id<'a> {
    Integer(u16),
    /// Case-insensitive UTF-16 name, without a terminator.
    Name(&'a [u16]),
}

/// One `IMAGE_RESOURCE_DATA_ENTRY`. `offset` is its position inside the
/// resource directory (the guest HRSRC is directory address + offset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry {
    pub offset: u32,
    pub data_rva: u32,
    pub size: u32,
    pub code_page: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Malformed,
    Limit,
    /// The reader refused an in-bounds read (unmapped or inaccessible).
    Read,
    TypeNotFound,
    NameNotFound,
    LanguageNotFound,
    /// The module has no resource directory (ERROR_RESOURCE_DATA_NOT_FOUND).
    NoData,
}

/// Bounded byte source. `offset` is relative to the directory start; the
/// `Directory` wrapper already checked `offset + output.len() <= size`.
pub trait Reader {
    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Error>;
}

pub struct Directory<R> {
    reader: R,
    size: u32,
    visits: usize,
}

const SUBDIRECTORY: u32 = 0x8000_0000;

impl<R: Reader> Directory<R> {
    pub fn new(reader: R, size: u32) -> Result<Self, Error> {
        if !(TABLE_BYTES..=MAX_MODULE_BYTES).contains(&size) {
            return Err(Error::Malformed);
        }
        Ok(Self {
            reader,
            size,
            visits: 0,
        })
    }
    pub fn size(&self) -> u32 {
        self.size
    }
    fn bytes<const N: usize>(&mut self, offset: u32) -> Result<[u8; N], Error> {
        let end = offset.checked_add(N as u32).ok_or(Error::Malformed)?;
        if end > self.size {
            return Err(Error::Malformed);
        }
        let mut output = [0u8; N];
        self.reader.read(offset, &mut output)?;
        Ok(output)
    }
    fn visit(&mut self, count: usize) -> Result<(), Error> {
        self.visits = self.visits.checked_add(count).ok_or(Error::Limit)?;
        if self.visits > MAX_VISITS {
            return Err(Error::Limit);
        }
        Ok(())
    }
    /// `(named, ids)` counts of the table at `offset`.
    fn table(&mut self, offset: u32) -> Result<(u32, u32), Error> {
        if !offset.is_multiple_of(4) {
            return Err(Error::Malformed);
        }
        let header = self.bytes::<16>(offset)?;
        let named = u32::from(u16::from_le_bytes([header[12], header[13]]));
        let ids = u32::from(u16::from_le_bytes([header[14], header[15]]));
        let count = named + ids;
        if count > MAX_ENTRIES {
            return Err(Error::Limit);
        }
        let end = u64::from(offset) + u64::from(TABLE_BYTES) + u64::from(count * ENTRY_BYTES);
        if end > u64::from(self.size) {
            return Err(Error::Malformed);
        }
        self.visit(1 + count as usize)?;
        Ok((named, ids))
    }
    fn entry(&mut self, table: u32, index: u32) -> Result<(u32, u32), Error> {
        let raw = self.bytes::<8>(table + TABLE_BYTES + index * ENTRY_BYTES)?;
        Ok((
            u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
            u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]),
        ))
    }
    /// Validates a named-entry string and returns its unit count.
    fn name_units(&mut self, raw: u32) -> Result<u32, Error> {
        let offset = raw & !SUBDIRECTORY;
        if !offset.is_multiple_of(2) {
            return Err(Error::Malformed);
        }
        let length = u32::from(u16::from_le_bytes(self.bytes::<2>(offset)?));
        if length == 0 || length as usize > MAX_NAME_UNITS {
            return Err(Error::Malformed);
        }
        let end = u64::from(offset) + 2 + u64::from(length) * 2;
        if end > u64::from(self.size) {
            return Err(Error::Malformed);
        }
        Ok(length)
    }
    fn name_matches(&mut self, raw: u32, wanted: &[u16]) -> Result<bool, Error> {
        let length = self.name_units(raw)?;
        if length as usize != wanted.len() {
            return Ok(false);
        }
        let offset = (raw & !SUBDIRECTORY) + 2;
        for (index, unit) in wanted.iter().enumerate() {
            let stored = u16::from_le_bytes(self.bytes::<2>(offset + index as u32 * 2)?);
            if upcase(stored) != upcase(*unit) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Entries `[0, named)` must carry string names and `[named, ..)` IDs.
    fn check_kind(raw: u32, index: u32, named: u32) -> Result<(), Error> {
        let is_name = raw & SUBDIRECTORY != 0;
        if is_name != (index < named) || (!is_name && raw > 0xffff) {
            return Err(Error::Malformed);
        }
        Ok(())
    }
    pub fn data_entry(&mut self, offset: u32) -> Result<Entry, Error> {
        if !offset.is_multiple_of(4) {
            return Err(Error::Malformed);
        }
        let raw = self.bytes::<16>(offset)?;
        let field =
            |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        Ok(Entry {
            offset,
            data_rva: field(0),
            size: field(4),
            code_page: field(8),
        })
    }
    /// Validate the complete three-level tree. `data` checks every leaf's
    /// RVA extent against the owning image. Returns the number of leaves.
    pub fn validate(
        &mut self,
        mut data: impl FnMut(u32, u32) -> Result<(), Error>,
    ) -> Result<usize, Error> {
        self.visits = 0;
        let mut leaves = 0;
        let (types_named, types_ids) = self.table(0)?;
        for type_index in 0..types_named + types_ids {
            let (raw, target) = self.entry(0, type_index)?;
            Self::check_kind(raw, type_index, types_named)?;
            if raw & SUBDIRECTORY != 0 {
                self.name_units(raw)?;
            }
            let names = self.subdirectory(target)?;
            let (names_named, names_ids) = self.table(names)?;
            for name_index in 0..names_named + names_ids {
                let (raw, target) = self.entry(names, name_index)?;
                Self::check_kind(raw, name_index, names_named)?;
                if raw & SUBDIRECTORY != 0 {
                    self.name_units(raw)?;
                }
                let languages = self.subdirectory(target)?;
                let (languages_named, languages_ids) = self.table(languages)?;
                if languages_named != 0 || languages_ids == 0 {
                    return Err(Error::Malformed);
                }
                for language_index in 0..languages_ids {
                    let (raw, target) = self.entry(languages, language_index)?;
                    Self::check_kind(raw, language_index, 0)?;
                    if target & SUBDIRECTORY != 0 {
                        return Err(Error::Malformed);
                    }
                    let entry = self.data_entry(target)?;
                    data(entry.data_rva, entry.size)?;
                    leaves += 1;
                }
            }
            if names_named + names_ids == 0 {
                return Err(Error::Malformed);
            }
        }
        Ok(leaves)
    }
    fn subdirectory(&self, target: u32) -> Result<u32, Error> {
        if target & SUBDIRECTORY == 0 {
            return Err(Error::Malformed);
        }
        let offset = target & !SUBDIRECTORY;
        // A subdirectory never points back at the root table.
        if offset == 0 {
            return Err(Error::Malformed);
        }
        Ok(offset)
    }
    /// Find a subdirectory (levels 0 and 1) or data entry (level 2) by ID.
    fn lookup(&mut self, table: u32, id: Id<'_>) -> Result<Option<u32>, Error> {
        let (named, ids) = self.table(table)?;
        for index in 0..named + ids {
            let (raw, target) = self.entry(table, index)?;
            Self::check_kind(raw, index, named)?;
            let matched = match id {
                Id::Integer(value) => raw & SUBDIRECTORY == 0 && raw == u32::from(value),
                Id::Name(units) => raw & SUBDIRECTORY != 0 && self.name_matches(raw, units)?,
            };
            if matched {
                return Ok(Some(target));
            }
        }
        Ok(None)
    }
    /// Windows `FindResourceExW` semantics: a neutral language request walks
    /// the profile's fallback list and finally takes the first language; an
    /// explicit language tries itself, its neutral sublanguage and neutral.
    pub fn find(&mut self, kind: Id<'_>, name: Id<'_>, language: u16) -> Result<Entry, Error> {
        self.visits = 0;
        let names = self.lookup(0, kind)?.ok_or(Error::TypeNotFound)?;
        let names = self.subdirectory(names)?;
        let languages = self.lookup(names, name)?.ok_or(Error::NameNotFound)?;
        let languages = self.subdirectory(languages)?;
        for candidate in language_candidates(language).iter().flatten() {
            if let Some(target) = self.lookup(languages, Id::Integer(*candidate))? {
                return self.leaf(target);
            }
        }
        if language & 0x3ff == LANG_NEUTRAL {
            let (named, ids) = self.table(languages)?;
            if named == 0 && ids != 0 {
                let (_, target) = self.entry(languages, 0)?;
                return self.leaf(target);
            }
        }
        Err(Error::LanguageNotFound)
    }
    /// EnumResourceNames: each name under type `kind`, in directory order
    /// (string names, then IDs), until `each` returns false. A string name
    /// is handed over in `scratch`.
    pub fn names(
        &mut self,
        kind: Id<'_>,
        scratch: &mut [u16; MAX_NAME_UNITS],
        mut each: impl FnMut(Id<'_>) -> bool,
    ) -> Result<(), Error> {
        self.visits = 0;
        let names = self.lookup(0, kind)?.ok_or(Error::TypeNotFound)?;
        let names = self.subdirectory(names)?;
        let (named, ids) = self.table(names)?;
        for index in 0..named + ids {
            let (raw, _) = self.entry(names, index)?;
            Self::check_kind(raw, index, named)?;
            let go_on = if raw & SUBDIRECTORY != 0 {
                let length = self.name_units(raw)? as usize;
                let offset = (raw & !SUBDIRECTORY) + 2;
                for (i, slot) in scratch.iter_mut().enumerate().take(length) {
                    *slot = u16::from_le_bytes(self.bytes::<2>(offset + i as u32 * 2)?);
                }
                each(Id::Name(&scratch[..length]))
            } else {
                each(Id::Integer(raw as u16))
            };
            if !go_on {
                break;
            }
        }
        Ok(())
    }
    fn leaf(&mut self, target: u32) -> Result<Entry, Error> {
        if target & SUBDIRECTORY != 0 {
            return Err(Error::Malformed);
        }
        self.data_entry(target)
    }
    /// True only for an offset that is an actual data entry of this tree;
    /// arbitrary handles passed to LoadResource/SizeofResource never parse.
    pub fn contains_data_entry(&mut self, offset: u32) -> Result<bool, Error> {
        let mut found = false;
        self.visits = 0;
        let (types_named, types_ids) = self.table(0)?;
        for type_index in 0..types_named + types_ids {
            let (_, target) = self.entry(0, type_index)?;
            let names = self.subdirectory(target)?;
            let (names_named, names_ids) = self.table(names)?;
            for name_index in 0..names_named + names_ids {
                let (_, target) = self.entry(names, name_index)?;
                let languages = self.subdirectory(target)?;
                let (named, ids) = self.table(languages)?;
                for language_index in 0..named + ids {
                    let (_, target) = self.entry(languages, language_index)?;
                    if target == offset {
                        found = true;
                    }
                }
            }
        }
        Ok(found)
    }
}

/// Deduplicated candidate list (at most eight IDs).
pub fn language_candidates(language: u16) -> [Option<u16>; 8] {
    let mut list = [None; 8];
    let mut count = 0;
    let mut push = |value: u16| {
        if !list[..count].contains(&Some(value)) && count < list.len() {
            list[count] = Some(value);
            count += 1;
        }
    };
    let primary = language & 0x3ff;
    let sublanguage = language >> 10;
    push(language);
    push(primary);
    push(0);
    if primary == LANG_NEUTRAL {
        if sublanguage != SUBLANG_SYS_DEFAULT {
            push(PROFILE_LANGUAGE);
            push(PROFILE_LANGUAGE & 0x3ff);
        }
        push(PROFILE_LANGUAGE);
        push(PROFILE_LANGUAGE & 0x3ff);
        push(0x0409);
    }
    list
}

/// Uppercase used for resource names: ASCII and Latin-1 letters. Names made
/// by the resource compiler are stored uppercase; both sides are folded.
pub fn upcase(unit: u16) -> u16 {
    match unit {
        0x61..=0x7a => unit - 0x20,
        0xe0..=0xfe if unit != 0xf7 => unit - 0x20,
        0xff => 0x178,
        _ => unit,
    }
}

struct ImageReader<'p, 'a> {
    module: &'p ModulePlan<'a>,
    base: u32,
}
impl Reader for ImageReader<'_, '_> {
    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), Error> {
        let rva = self.base.checked_add(offset).ok_or(Error::Malformed)?;
        let bytes = self
            .module
            .image
            .rva_bytes(rva, output.len())
            .map_err(|_| Error::Read)?;
        output.copy_from_slice(bytes);
        Ok(())
    }
}

impl<'a> ModulePlan<'a> {
    /// Validated resource directory extent, if present.
    pub fn resource_directory(&self) -> Option<(Rva, u32)> {
        match self.directory(2) {
            Ok((0, 0)) | Err(_) => None,
            Ok((rva, bytes)) => Some((Rva(rva), bytes)),
        }
    }
    fn resources(&self) -> Result<Option<Directory<ImageReader<'_, 'a>>>, Error> {
        let (rva, bytes) = self.directory(2).map_err(|_| Error::Malformed)?;
        if (rva, bytes) == (0, 0) {
            return Ok(None);
        }
        if !rva.is_multiple_of(4) {
            return Err(Error::Malformed);
        }
        Directory::new(
            ImageReader {
                module: self,
                base: rva,
            },
            bytes,
        )
        .map(Some)
    }
    pub(crate) fn validate_resources(&self) -> Result<(), PlanError> {
        let (rva, _) = self.directory(2)?;
        let map = |error: Error| match error {
            Error::Limit => self.error(rva, Reason::ResourceLimit(Limit::Resources)),
            _ => self.error(rva, Reason::InvalidResource),
        };
        let Some(mut directory) = self.resources().map_err(map)? else {
            return Ok(());
        };
        directory
            .validate(|data, bytes| {
                // Leaves name readable image bytes inside one section (zero
                // fill is permitted); headers, gaps and padding are not.
                self.mapped_section(data, bytes.max(1))
                    .map(|_| ())
                    .map_err(|_| Error::Malformed)
            })
            .map(|_| ())
            .map_err(map)
    }
    /// Host/kernel lookup with `FindResourceExW` language semantics.
    pub fn find_resource(&self, kind: Id<'_>, name: Id<'_>, language: u16) -> Result<Entry, Error> {
        self.resources()?
            .ok_or(Error::NoData)?
            .find(kind, name, language)
    }
}
