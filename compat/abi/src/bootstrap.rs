//! Versioned kernel-to-ntdll startup record `DOORWIN2` of the P13
//! application path (manifests 10 and 15). The frozen P09/P10 fixtures keep
//! `DOORWIN1` (`lib.rs`); ntdll accepts each version only for its manifests.
//!
//! It is written by the trusted personality into a read-only page and captured
//! by ntdll through the checked-copy boundary. Version 2 carries up to fifteen
//! DLL records (the executable is described separately), each with its image
//! extent and validated resource directory, so the adapter never re-derives
//! module identity from an application-supplied pointer.
use crate::{Error, address};

pub const BOOTSTRAP_BYTES: usize = 1024;
pub const BOOTSTRAP_VERSION: u32 = 2;
/// DLL records, excluding the executable (one less than the planner's cap).
pub const MAX_MODULES: usize = 15;
const MAGIC: &[u8; 8] = b"DOORWIN2";
const MODULES_AT: usize = 88;
const MODULE_BYTES: usize = 32;
/// Bytes of one encoded `ModuleInit` (the bootstrap record's, and
/// WINDOWS_PROVIDER_LOAD's output).
pub const MODULE_INIT_BYTES: usize = MODULE_BYTES;
const END: usize = MODULES_AT + MAX_MODULES * MODULE_BYTES;
/// Mapped only by an explicit dynamic load (the P10 lifecycle fixture DLL).
pub const MODULE_DEFERRED: u32 = 1;
const _: () = assert!(END <= BOOTSTRAP_BYTES && BOOTSTRAP_BYTES <= crate::COPY_BYTES);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModuleInit {
    pub base: u64,
    /// A DLL may have no entry point. Nonzero entries receive DLL_PROCESS_ATTACH.
    pub entry: u64,
    pub image_bytes: u32,
    /// Validated resource directory extent, `(0, 0)` when absent.
    pub resource_rva: u32,
    pub resource_bytes: u32,
    pub flags: u32,
}
impl ModuleInit {
    pub fn deferred(&self) -> bool {
        self.flags & MODULE_DEFERRED != 0
    }
    pub fn contains(&self, address: u64) -> bool {
        address >= self.base && address - self.base < u64::from(self.image_bytes)
    }
    fn validate(&self, executable: bool) -> Result<(), Error> {
        if !address(self.base)
            || !self.base.is_multiple_of(0x1_0000)
            || (self.entry != 0 && !address(self.entry))
            || self.image_bytes == 0
            || !self.image_bytes.is_multiple_of(4096)
            || self
                .base
                .checked_add(u64::from(self.image_bytes))
                .is_none_or(|end| end > 0x8000_0000_0000)
        {
            return Err(Error::Address);
        }
        if (executable && self.flags != 0) || self.flags & !MODULE_DEFERRED != 0 {
            return Err(Error::Reserved);
        }
        let resources = (self.resource_rva, self.resource_bytes);
        if resources != (0, 0)
            && (self.resource_rva == 0
                || self.resource_bytes == 0
                || u64::from(self.resource_rva) + u64::from(self.resource_bytes)
                    > u64::from(self.image_bytes))
        {
            return Err(Error::Address);
        }
        Ok(())
    }
    /// One record, as the bootstrap record and WINDOWS_PROVIDER_LOAD write it.
    pub fn encode_record(&self, output: &mut [u8]) {
        self.encode(output)
    }
    /// One record written by `encode_record`, validated.
    pub fn decode_record(input: &[u8]) -> Result<Self, Error> {
        if input.len() != MODULE_BYTES {
            return Err(Error::Address);
        }
        let module = Self::decode(input);
        module.validate(false)?;
        Ok(module)
    }
    fn encode(&self, output: &mut [u8]) {
        output[..8].copy_from_slice(&self.base.to_le_bytes());
        output[8..16].copy_from_slice(&self.entry.to_le_bytes());
        output[16..20].copy_from_slice(&self.image_bytes.to_le_bytes());
        output[20..24].copy_from_slice(&self.resource_rva.to_le_bytes());
        output[24..28].copy_from_slice(&self.resource_bytes.to_le_bytes());
        output[28..32].copy_from_slice(&self.flags.to_le_bytes());
    }
    fn decode(input: &[u8]) -> Self {
        let get32 = |at: usize| u32::from_le_bytes(input[at..at + 4].try_into().unwrap());
        Self {
            base: u64::from_le_bytes(input[..8].try_into().unwrap()),
            entry: u64::from_le_bytes(input[8..16].try_into().unwrap()),
            image_bytes: get32(16),
            resource_rva: get32(20),
            resource_bytes: get32(24),
            flags: get32(28),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bootstrap {
    pub native_startup: u64,
    pub executable_entry: u64,
    pub executable_base: u64,
    pub teb: u64,
    pub peb: u64,
    pub parameters: u64,
    /// PE optional-header subsystem of the executable (2 GUI, 3 console).
    pub subsystem: u16,
    /// Extent and resources of the executable; `base`/`entry` repeat the
    /// fields above and `flags` is zero.
    pub executable: ModuleInit,
    pub module_count: u32,
    /// Dependency initialization order, excluding the executable.
    pub modules: [ModuleInit; MAX_MODULES],
}

impl Bootstrap {
    fn validate(&self) -> Result<(), Error> {
        if self.module_count as usize > MAX_MODULES {
            return Err(Error::Limit);
        }
        for value in [
            self.native_startup,
            self.executable_entry,
            self.executable_base,
            self.teb,
            self.peb,
            self.parameters,
        ] {
            if !address(value) {
                return Err(Error::Address);
            }
        }
        if !matches!(self.subsystem, 2 | 3) {
            return Err(Error::Version);
        }
        if self.executable.base != self.executable_base
            || self.executable.entry != self.executable_entry
            || !self.executable.contains(self.executable_entry)
        {
            return Err(Error::Address);
        }
        self.executable.validate(true)?;
        for (index, module) in self.modules.iter().enumerate() {
            if index < self.module_count as usize {
                module.validate(false)?;
                if module.entry != 0 && !module.contains(module.entry) {
                    return Err(Error::Address);
                }
                let overlaps = |other: &ModuleInit| {
                    module.base < other.base + u64::from(other.image_bytes)
                        && other.base < module.base + u64::from(module.image_bytes)
                };
                if overlaps(&self.executable) || self.modules[..index].iter().any(overlaps) {
                    return Err(Error::Address);
                }
            } else if *module != ModuleInit::default() {
                return Err(Error::Reserved);
            }
        }
        Ok(())
    }
    /// The executable followed by every DLL record, deferred ones included.
    pub fn images(&self) -> impl Iterator<Item = &ModuleInit> {
        core::iter::once(&self.executable).chain(&self.modules[..self.module_count as usize])
    }
    /// Module whose image contains `address`, if any.
    pub fn module_at(&self, address: u64) -> Option<&ModuleInit> {
        self.images().find(|module| module.contains(address))
    }
    pub fn encode(&self, output: &mut [u8]) -> Result<(), Error> {
        if output.len() != BOOTSTRAP_BYTES {
            return Err(Error::Size);
        }
        self.validate()?;
        output.fill(0);
        output[..8].copy_from_slice(MAGIC);
        output[8..12].copy_from_slice(&BOOTSTRAP_VERSION.to_le_bytes());
        output[12..16].copy_from_slice(&(BOOTSTRAP_BYTES as u32).to_le_bytes());
        for (index, value) in [
            self.native_startup,
            self.executable_entry,
            self.executable_base,
            self.teb,
            self.peb,
            self.parameters,
        ]
        .iter()
        .enumerate()
        {
            output[16 + index * 8..24 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        output[64..68].copy_from_slice(&self.module_count.to_le_bytes());
        output[68..70].copy_from_slice(&self.subsystem.to_le_bytes());
        output[72..76].copy_from_slice(&self.executable.image_bytes.to_le_bytes());
        output[76..80].copy_from_slice(&self.executable.resource_rva.to_le_bytes());
        output[80..84].copy_from_slice(&self.executable.resource_bytes.to_le_bytes());
        for (index, module) in self.modules.iter().enumerate() {
            let at = MODULES_AT + index * MODULE_BYTES;
            module.encode(&mut output[at..at + MODULE_BYTES]);
        }
        Ok(())
    }
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != BOOTSTRAP_BYTES {
            return Err(Error::Size);
        }
        if &input[..8] != MAGIC
            || input[8..12] != BOOTSTRAP_VERSION.to_le_bytes()
            || input[12..16] != (BOOTSTRAP_BYTES as u32).to_le_bytes()
        {
            return Err(Error::Version);
        }
        if input[70..72]
            .iter()
            .chain(&input[84..88])
            .chain(&input[END..])
            .any(|b| *b != 0)
        {
            return Err(Error::Reserved);
        }
        let get = |at: usize| u64::from_le_bytes(input[at..at + 8].try_into().unwrap());
        let get32 = |at: usize| u32::from_le_bytes(input[at..at + 4].try_into().unwrap());
        let result = Self {
            native_startup: get(16),
            executable_entry: get(24),
            executable_base: get(32),
            teb: get(40),
            peb: get(48),
            parameters: get(56),
            module_count: get32(64),
            subsystem: u16::from_le_bytes([input[68], input[69]]),
            executable: ModuleInit {
                base: get(32),
                entry: get(24),
                image_bytes: get32(72),
                resource_rva: get32(76),
                resource_bytes: get32(80),
                flags: 0,
            },
            modules: core::array::from_fn(|index| {
                let at = MODULES_AT + index * MODULE_BYTES;
                ModuleInit::decode(&input[at..at + MODULE_BYTES])
            }),
        };
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Bootstrap {
        let mut modules = [ModuleInit::default(); MAX_MODULES];
        for (index, module) in modules.iter_mut().enumerate().take(12) {
            *module = ModuleInit {
                base: 0x1_8000_0000 + index as u64 * 0x0200_0000,
                entry: if index % 2 == 0 {
                    0
                } else {
                    0x1_8000_1000 + index as u64 * 0x0200_0000
                },
                image_bytes: 0x10_0000,
                resource_rva: if index == 3 { 0x8000 } else { 0 },
                resource_bytes: if index == 3 { 0x200 } else { 0 },
                flags: u32::from(index == 2),
            };
        }
        Bootstrap {
            native_startup: 0x7100_0000,
            executable_entry: 0x1_4200_1000,
            executable_base: 0x1_4200_0000,
            teb: crate::TEB_ADDRESS,
            peb: crate::PEB_ADDRESS,
            parameters: crate::PARAMETERS_ADDRESS,
            subsystem: 2,
            executable: ModuleInit {
                base: 0x1_4200_0000,
                entry: 0x1_4200_1000,
                image_bytes: 0x4000,
                resource_rva: 0x3000,
                resource_bytes: 0x100,
                flags: 0,
            },
            module_count: 12,
            modules,
        }
    }
    #[test]
    fn twelve_module_gui_bundle_roundtrips_with_resource_extents() {
        let value = sample();
        let mut bytes = [0; BOOTSTRAP_BYTES];
        value.encode(&mut bytes).unwrap();
        assert_eq!(Bootstrap::decode(&bytes), Ok(value));
        assert_eq!(value.images().count(), 13);
        assert_eq!(
            value.module_at(0x1_8600_0010).unwrap().resource_bytes,
            0x200
        );
        assert!(value.module_at(0x1_4200_3fff).is_some());
        assert!(value.module_at(0x1_4200_4000).is_none());
        assert!(value.modules[2].deferred());
        for at in [70, 84, 87, END, BOOTSTRAP_BYTES - 1] {
            let mut bad = bytes;
            bad[at] = 1;
            assert_eq!(Bootstrap::decode(&bad), Err(Error::Reserved), "byte {at}");
        }
        let mut old = bytes;
        old[..8].copy_from_slice(b"DOORWIN1");
        assert_eq!(Bootstrap::decode(&old), Err(Error::Version));
        for end in [0, 256, BOOTSTRAP_BYTES - 1] {
            assert_eq!(Bootstrap::decode(&bytes[..end]), Err(Error::Size));
        }
    }
    #[test]
    fn records_reject_limits_overlaps_escaping_resources_and_flags() {
        let mut bytes = [0; BOOTSTRAP_BYTES];
        let mut bad = sample();
        bad.module_count = 16;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Limit));
        let mut bad = sample();
        bad.modules[13].base = 0x1_9000_0000;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Reserved));
        let mut bad = sample();
        bad.modules[1].base = bad.modules[0].base;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
        let mut bad = sample();
        bad.modules[3].resource_bytes = 0x10_0000;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
        let mut bad = sample();
        bad.modules[1].entry = bad.modules[2].base;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
        let mut bad = sample();
        bad.modules[0].flags = 2;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Reserved));
        let mut bad = sample();
        bad.executable.flags = MODULE_DEFERRED;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Reserved));
        let mut bad = sample();
        bad.subsystem = 1;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Version));
        let mut bad = sample();
        bad.executable_entry = 0x1_4200_5000;
        bad.executable.entry = bad.executable_entry;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
        let mut bad = sample();
        bad.modules[4].image_bytes = 0x1001;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
    }
}
