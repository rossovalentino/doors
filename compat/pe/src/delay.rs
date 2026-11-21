//! Delay-load descriptors (IMAGE_DELAYLOAD_DESCRIPTOR). The P13 application
//! policy binds them eagerly: the target DLL must be part of the validated
//! bundle, the module-handle slot receives its base and every delay IAT slot
//! receives the resolved export before the image runs. The image's own
//! `__delayLoadHelper2` is therefore never entered for a bound slot, and a
//! missing delay-load DLL fails the load instead of faulting on first call.
use crate::{
    Error, Feature, Limit, LoadPolicy, MAX_DELAY_DESCRIPTORS, ModulePlan, Permissions, Reason,
    dword,
};

pub(crate) const DESCRIPTOR_BYTES: u32 = 32;

#[derive(Clone, Copy)]
pub(crate) struct DelayImport<'a> {
    pub name: &'a str,
    pub descriptor: u32,
    pub handle: u32,
    pub iat: u32,
    pub names: u32,
    pub count: usize,
}

impl<'a> ModulePlan<'a> {
    /// Number of descriptors before the all-zero terminator. The terminator
    /// may follow the declared directory extent but must be file-backed.
    pub(crate) fn delay_count(&self) -> Result<usize, Error> {
        let (directory, bytes) = self.directory(13)?;
        if directory == 0 {
            return Ok(0);
        }
        if !directory.is_multiple_of(4) || bytes < DESCRIPTOR_BYTES - 4 {
            return Err(self.error(directory, Reason::InvalidDelayImport));
        }
        for index in 0..=MAX_DELAY_DESCRIPTORS as u32 {
            let at = directory
                .checked_add(index * DESCRIPTOR_BYTES)
                .ok_or_else(|| self.error(directory, Reason::InvalidDelayImport))?;
            let descriptor = self.read(at, DESCRIPTOR_BYTES as usize)?;
            if descriptor.iter().all(|byte| *byte == 0) {
                if index == 0 {
                    return Err(self.error(directory, Reason::InvalidDelayImport));
                }
                return Ok(index as usize);
            }
        }
        Err(self.error(directory, Reason::ResourceLimit(Limit::DelayImports)))
    }
    pub(crate) fn delay_import(&self, index: usize) -> Result<DelayImport<'a>, Error> {
        let (directory, _) = self.directory(13)?;
        let at = directory + index as u32 * DESCRIPTOR_BYTES;
        let d = self.read(at, DESCRIPTOR_BYTES as usize)?;
        match dword(d, 0) {
            1 => {}
            0 => return Err(self.error(at, Reason::Unsupported(Feature::LegacyDelayImports))),
            _ => return Err(self.error(at, Reason::InvalidDelayImport)),
        }
        let name = self
            .image
            .string(dword(d, 4))
            .map_err(|_| self.error(at, Reason::InvalidDelayImport))?;
        let handle = dword(d, 8);
        let iat = dword(d, 12);
        let names = dword(d, 16);
        let bound = dword(d, 20);
        let unload = dword(d, 24);
        let width = self.pointer_bytes();
        for value in [handle, iat, names] {
            if value == 0 || !value.is_multiple_of(width) {
                return Err(self.error(at, Reason::InvalidDelayImport));
            }
        }
        // The loader writes the HMODULE slot exactly like __delayLoadHelper2
        // would; it must be ordinary writable image data.
        let slot = self
            .mapped_section(handle, width)
            .map_err(|_| self.error(handle, Reason::InvalidDelayImport))?;
        if crate::layout::section_permissions(self, slot)? != Permissions::ReadWrite {
            return Err(self.error(handle, Reason::InvalidDelayImport));
        }
        let limit = self.policy.limits().imports;
        let mut count = 0;
        loop {
            if count > limit {
                return Err(self.error(names, Reason::ResourceLimit(Limit::Imports)));
            }
            let offset = count as u32 * width;
            let thunk_rva = names
                .checked_add(offset)
                .ok_or_else(|| self.error(names, Reason::InvalidDelayImport))?;
            if self.pointer_at(thunk_rva)? == 0 {
                break;
            }
            let slot = iat
                .checked_add(offset)
                .ok_or_else(|| self.error(iat, Reason::InvalidDelayImport))?;
            self.mapped_section(slot, width)
                .map_err(|_| self.error(slot, Reason::InvalidDelayImport))?;
            count += 1;
        }
        if count == 0 {
            return Err(self.error(names, Reason::InvalidDelayImport));
        }
        // Optional bound/unload copies are only read by the image's own unload
        // helper; they must still describe in-image storage of the same size.
        for table in [bound, unload] {
            if table != 0
                && (!table.is_multiple_of(width)
                    || self.mapped_section(table, count as u32 * width).is_err())
            {
                return Err(self.error(table, Reason::InvalidDelayImport));
            }
        }
        Ok(DelayImport {
            name,
            descriptor: at,
            handle,
            iat,
            names,
            count,
        })
    }
    pub(crate) fn validate_delay_imports(&self, policy: LoadPolicy) -> Result<(), Error> {
        if !policy.application() {
            return Ok(());
        }
        let count = self.delay_count()?;
        let mut total = 0usize;
        for index in 0..count {
            let import = self.delay_import(index)?;
            total += import.count;
            if total > policy.limits().imports {
                return Err(self.error(import.names, Reason::ResourceLimit(Limit::Imports)));
            }
            for function in 0..import.count {
                self.thunk_symbol(import.names + function as u32 * self.pointer_bytes())?;
            }
        }
        Ok(())
    }
}
