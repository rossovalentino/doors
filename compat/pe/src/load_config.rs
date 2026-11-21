//! IMAGE_LOAD_CONFIG_DIRECTORY64 under the P13 application policy, and
//! IMAGE_LOAD_CONFIG_DIRECTORY32 under the P17 one (read into the same
//! field offsets; its SafeSEH handler table is validated and kept for the
//! x86 exception dispatcher).
//!
//! Every field the image or a Windows loader could dereference is checked to
//! stay inside the image. Features that need loader services Door does not
//! provide are refused explicitly: x86 SafeSEH/lock-prefix/edit-list tables,
//! code-integrity requirements, dynamic value relocations, CHPE/ARM64EC hybrid
//! metadata and enclaves. CFG is not enforced (P18 admits the
//! `IMAGE_DLLCHARACTERISTICS_GUARD_CF` bit in `layout.rs`): as Windows does
//! without enforcement, the image's own non-enforcing check/dispatch routines stay,
//! which is what Door does. The `/GS` security cookie is honoured: a slot that
//! still holds zero or the compiler default is rewritten before entry.
use crate::{
    DEFAULT_SECURITY_COOKIE, DEFAULT_SECURITY_COOKIE_X86, Error, Feature, Fixup, FixupKind,
    Machine, ModulePlan, Permissions, Reason, Rva, VisitError, dword, qword, word,
};

/// Bound on the declared structure size (current SDKs use 0x140..0x148).
pub const MAX_LOAD_CONFIG_BYTES: u32 = 4096;
/// Fields interpreted by this loader, through `UmaFunctionPointers`.
const KNOWN_BYTES: usize = 0x150;
/// Absolute pointers (VAs) the image may dereference.
const POINTERS: [usize; 17] = [
    88, 112, 120, 128, 160, 176, 208, 216, 232, 256, 264, 280, 288, 296, 304, 312, 320,
];
/// Function-pointer slots the image calls through (`__guard_*_fptr` style).
const SLOTS: [usize; 8] = [112, 120, 216, 232, 280, 288, 296, 312];
/// (table, count) pairs of guard metadata with the GuardFlags stride.
const TABLES: [(usize, usize); 4] = [(128, 136), (160, 168), (176, 184), (264, 272)];
/// IMAGE_LOAD_CONFIG_DIRECTORY32's fields: (x86 offset, the same field's
/// IMAGE_LOAD_CONFIG_DIRECTORY64 offset, bytes). Pointers and sizes widen
/// from 4 to 8 bytes; everything else keeps its width.
const X86_FIELDS: [(usize, usize, usize); 50] = [
    (0, 0, 4),
    (4, 4, 4),
    (8, 8, 2),
    (10, 10, 2),
    (12, 12, 4),
    (16, 16, 4),
    (20, 20, 4),
    (24, 24, 4),
    (28, 32, 4),
    (32, 40, 4),
    (36, 48, 4),
    (40, 56, 4),
    (44, 72, 4),
    (48, 64, 4),
    (52, 76, 2),
    (54, 78, 2),
    (56, 80, 4),
    (60, 88, 4),
    (64, 96, 4),
    (68, 104, 4),
    (72, 112, 4),
    (76, 120, 4),
    (80, 128, 4),
    (84, 136, 4),
    (88, 144, 4),
    (92, 148, 12),
    (104, 160, 4),
    (108, 168, 4),
    (112, 176, 4),
    (116, 184, 4),
    (120, 192, 4),
    (124, 200, 4),
    (128, 208, 4),
    (132, 216, 4),
    (136, 224, 4),
    (140, 228, 2),
    (142, 230, 2),
    (144, 232, 4),
    (148, 240, 4),
    (152, 244, 4),
    (156, 248, 4),
    (160, 256, 4),
    (164, 264, 4),
    (168, 272, 4),
    (172, 280, 4),
    (176, 288, 4),
    (180, 296, 4),
    (184, 304, 4),
    (188, 312, 4),
    (192, 320, 4),
];
/// Bound on a SafeSEH handler table (MSVC emits one entry per handler).
pub const MAX_SAFE_SEH: u32 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadConfig {
    /// Declared structure size (first field).
    pub size: u32,
    pub security_cookie: Option<Rva>,
    pub guard_flags: u32,
    pub dependent_load_flags: u16,
    /// x86: the SafeSEH table (the RVAs of the image's registered exception
    /// handlers, sorted) and its entry count. An x86 image without one
    /// admits any handler inside it, as Windows does for images built
    /// without /SAFESEH (unless IMAGE_DLLCHARACTERISTICS_NO_SEH).
    pub safe_seh: Option<(Rva, u32)>,
}

impl ModulePlan<'_> {
    pub fn load_config(&self) -> Result<Option<LoadConfig>, Error> {
        let (rva, bytes) = self.directory(10)?;
        if (rva, bytes) == (0, 0) {
            return Ok(None);
        }
        let invalid = |at: usize| {
            self.error(
                rva.saturating_add(at as u32),
                Reason::InvalidLoadConfiguration,
            )
        };
        if !rva.is_multiple_of(4) || !(4..=MAX_LOAD_CONFIG_BYTES).contains(&bytes) {
            return Err(invalid(0));
        }
        let size = dword(self.read(rva, 4)?, 0);
        if !(8..=MAX_LOAD_CONFIG_BYTES).contains(&size) {
            return Err(invalid(0));
        }
        // Windows interprets the structure by its own Size field; fields past it
        // read as zero. The whole declared structure must be file-backed.
        let raw = self.read(rva, size as usize)?;
        let mut fields = [0u8; KNOWN_BYTES];
        let x86 = self.machine() == Machine::X86;
        if x86 {
            for (from, to, bytes) in X86_FIELDS {
                if from + bytes <= raw.len() {
                    fields[to..to + bytes].copy_from_slice(&raw[from..from + bytes]);
                }
            }
        } else {
            let known = raw.len().min(KNOWN_BYTES);
            fields[..known].copy_from_slice(&raw[..known]);
        }
        let field = |at: usize| qword(&fields, at);
        // Lock-prefix tables and edit lists stay refused; SafeSEH is x86's
        // own exception-handler registration.
        if field(40) != 0 || field(80) != 0 || !x86 && (field(96) != 0 || field(104) != 0) {
            return Err(self.error(rva, Reason::Unsupported(Feature::LegacyTables)));
        }
        let width = self.pointer_bytes();
        if word(&fields, 148) != 0 {
            return Err(self.error(rva, Reason::Unsupported(Feature::CodeIntegrity)));
        }
        if field(192) != 0 || dword(&fields, 224) != 0 || word(&fields, 228) != 0 {
            return Err(self.error(rva, Reason::Unsupported(Feature::DynamicRelocations)));
        }
        if field(200) != 0 {
            return Err(self.error(rva, Reason::Unsupported(Feature::HybridImage)));
        }
        if field(248) != 0 {
            return Err(self.error(rva, Reason::Unsupported(Feature::Enclave)));
        }
        if word(&fields, 230) != 0 || dword(&fields, 244) != 0 {
            return Err(invalid(230));
        }
        for at in POINTERS {
            if field(at) != 0 {
                self.preferred_rva(field(at), rva + at as u32, Reason::InvalidLoadConfiguration)?;
            }
        }
        for at in SLOTS {
            if field(at) == 0 {
                continue;
            }
            let slot = self.preferred_rva(field(at), rva, Reason::InvalidLoadConfiguration)?;
            if !slot.is_multiple_of(width) || self.mapped_section(slot, width).is_err() {
                return Err(invalid(at));
            }
        }
        let guard_flags = dword(&fields, 144);
        let stride = 4 + (guard_flags >> 28);
        for (table, count) in TABLES {
            let entries = field(count);
            if entries == 0 {
                continue;
            }
            let start = match field(table) {
                0 => return Err(invalid(table)),
                value => self.preferred_rva(value, rva, Reason::InvalidLoadConfiguration)?,
            };
            let length = u32::try_from(entries)
                .ok()
                .and_then(|entries| entries.checked_mul(stride))
                .ok_or_else(|| invalid(count))?;
            self.mapped_section(start, length)
                .map_err(|_| invalid(table))?;
        }
        let security_cookie = match field(88) {
            0 => None,
            value => {
                let slot = self.preferred_rva(value, rva, Reason::InvalidLoadConfiguration)?;
                let section = self.mapped_section(slot, width).map_err(|_| invalid(88))?;
                if !slot.is_multiple_of(width)
                    || crate::layout::section_permissions(self, section)? != Permissions::ReadWrite
                {
                    return Err(invalid(88));
                }
                Some(Rva(slot))
            }
        };
        // SafeSEH: sorted, distinct handler RVAs inside the image.
        let safe_seh = match (x86, field(96), field(104)) {
            (true, _, 0) | (false, _, _) => None,
            (true, 0, _) => return Err(invalid(96)),
            (true, table, count) => {
                let start = self.preferred_rva(table, rva, Reason::InvalidLoadConfiguration)?;
                let count = u32::try_from(count)
                    .ok()
                    .filter(|count| *count <= MAX_SAFE_SEH)
                    .ok_or_else(|| invalid(104))?;
                let entries = self
                    .read(start, count as usize * 4)
                    .map_err(|_| invalid(96))?;
                let mut previous = None;
                for index in 0..count as usize {
                    let handler = dword(entries, index * 4);
                    if handler >= self.image_bytes() || previous.is_some_and(|p| p >= handler) {
                        return Err(invalid(96));
                    }
                    previous = Some(handler);
                }
                Some((Rva(start), count))
            }
        };
        Ok(Some(LoadConfig {
            size,
            security_cookie,
            guard_flags,
            dependent_load_flags: word(&fields, 78),
            safe_seh,
        }))
    }
    /// Current pointer-sized value of an in-section slot, including
    /// zero-fill tails.
    fn slot_value(&self, rva: u32) -> Result<u64, Error> {
        let width = self.pointer_bytes();
        let section = self.mapped_section(rva, width)?;
        let relative = rva - section.virtual_address;
        let mut bytes = [0u8; 8];
        let backed = section.raw_bytes.saturating_sub(relative).min(width);
        if backed != 0 {
            bytes[..backed as usize].copy_from_slice(self.read(rva, backed as usize)?);
        }
        Ok(qword(&bytes, 0))
    }
    /// The compiler's default `/GS` cookie for this image's machine.
    fn default_cookie(&self) -> u64 {
        match self.machine() {
            Machine::X64 => DEFAULT_SECURITY_COOKIE,
            Machine::X86 => DEFAULT_SECURITY_COOKIE_X86,
        }
    }
    /// The cookie slot the loader rewrites: it holds zero or the default.
    /// An image that deliberately ships another value keeps it, as on Windows.
    pub(crate) fn cookie_slot(&self) -> Result<Option<u32>, Error> {
        let Some(config) = self.load_config()? else {
            return Ok(None);
        };
        let Some(Rva(slot)) = config.security_cookie else {
            return Ok(None);
        };
        let value = self.slot_value(slot)?;
        Ok((value == 0 || value == self.default_cookie()).then_some(slot))
    }
    pub(crate) fn walk_cookie<E>(
        &self,
        seed: Option<u64>,
        mut visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        let Some(slot) = self.cookie_slot()? else {
            return Ok(());
        };
        let value = match (seed, self.machine()) {
            (Some(seed), Machine::X64) => diversify(seed, self.index, self.source.base.0),
            (Some(seed), Machine::X86) => diversify32(seed, self.index, self.source.base.0),
            (None, _) => self.slot_value(slot)?,
        };
        visitor(Fixup {
            module: self.index,
            rva: Rva(slot),
            address: self.address(slot),
            value,
            kind: FixupKind::SecurityCookie,
            width: self.pointer_bytes() as u8,
        })
        .map_err(VisitError::Callback)
    }
}

/// Per-module cookie from the caller's seed (SplitMix64 finaliser). The x64
/// Windows loader keeps the top 16 bits clear and never leaves zero or the
/// compiler default; this mixing provides diversity, not new entropy.
pub fn diversify(seed: u64, module: usize, base: u64) -> u64 {
    let mut value =
        seed ^ base.rotate_left(17) ^ (module as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    value &= 0x0000_ffff_ffff_ffff;
    if value == 0 || value == DEFAULT_SECURITY_COOKIE {
        value ^= 0x0000_5a5a_0001;
    }
    value
}
/// `diversify` for an x86 image's 32-bit cookie: never zero or the x86
/// compiler default.
pub fn diversify32(seed: u64, module: usize, base: u64) -> u64 {
    let wide = diversify(seed, module, base);
    let mut value = (wide ^ (wide >> 32)) & 0xffff_ffff;
    if value == 0 || value == DEFAULT_SECURITY_COOKIE_X86 {
        value ^= 0x5a5a_0001;
    }
    value
}
