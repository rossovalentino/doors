//! Bounded compiler TLS metadata. Addresses in source directories are preferred
//! image VAs, never trusted pointers into the loader's own address space.
use crate::{Error, Machine, ModulePlan, Permissions, Reason, Rva, dword};
pub const TEMPLATE_BYTES: usize = 256;
pub const MAX_CALLBACKS: usize = 8;
#[derive(Clone, Copy)]
pub struct TlsPlan<'a> {
    pub template: &'a [u8],
    pub template_rva: Rva,
    pub zero_bytes: u32,
    pub index: Rva,
    pub callbacks: [Rva; MAX_CALLBACKS],
    pub callback_count: usize,
}
impl<'a> ModulePlan<'a> {
    pub fn tls(&self) -> Result<Option<TlsPlan<'a>>, Error> {
        let (rva, bytes) = self.directory(9)?;
        if (rva, bytes) == (0, 0) {
            return Ok(None);
        }
        // IMAGE_TLS_DIRECTORY64 (40 bytes) or IMAGE_TLS_DIRECTORY32 (24):
        // four pointer-sized addresses, then SizeOfZeroFill and
        // Characteristics.
        let width = self.pointer_bytes() as usize;
        let size = match self.machine() {
            Machine::X64 => 40,
            Machine::X86 => 24,
        };
        if rva == 0 || bytes != size {
            return Err(self.error(rva, Reason::InvalidLayout));
        }
        let data = self.read(rva, size as usize)?;
        let convert = |value: u64| -> Result<u32, Error> {
            value
                .checked_sub(self.preferred_base().0)
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v < self.image_bytes())
                .ok_or(self.error(rva, Reason::InvalidLayout))
        };
        let start = convert(self.pointer_in(data, 0))?;
        let end = convert(self.pointer_in(data, width))?;
        let length = end
            .checked_sub(start)
            .ok_or(self.error(rva, Reason::InvalidLayout))?;
        let zero_bytes = dword(data, 4 * width);
        let characteristics = dword(data, 4 * width + 4);
        if length
            .checked_add(zero_bytes)
            .is_none_or(|v| v as usize > TEMPLATE_BYTES)
            || characteristics & !0x00f00000 != 0
            || (characteristics >> 20) > 5
        {
            return Err(self.error(rva, Reason::InvalidLayout));
        }
        let index = Rva(convert(self.pointer_in(data, 2 * width))?);
        let mut writable = false;
        self.walk_runs(|run| {
            if run.permissions == Permissions::ReadWrite
                && index.0 >= run.rva.0
                && index
                    .0
                    .checked_add(4)
                    .is_some_and(|end| end <= run.rva.0 + run.bytes)
            {
                writable = true;
            }
            Ok::<_, core::convert::Infallible>(())
        })
        .map_err(crate::layout::infallible_plan_error)?;
        if !writable || !index.0.is_multiple_of(4) {
            return Err(self.error(index.0, Reason::InvalidLayout));
        }
        let mut plan = TlsPlan {
            template: self.read(start, length as usize)?,
            template_rva: Rva(start),
            zero_bytes,
            index,
            callbacks: [Rva(0); MAX_CALLBACKS],
            callback_count: 0,
        };
        let callbacks = self.pointer_in(data, 3 * width);
        if callbacks != 0 {
            let first = convert(callbacks)?;
            for i in 0..=MAX_CALLBACKS {
                let slot = first
                    .checked_add(i as u32 * width as u32)
                    .ok_or(self.error(first, Reason::InvalidLayout))?;
                let value = self.pointer_at(slot)?;
                if value == 0 {
                    return Ok(Some(plan));
                }
                if i == MAX_CALLBACKS {
                    return Err(self.error(slot, Reason::InvalidLayout));
                }
                let callback = Rva(convert(value)?);
                let mut executable = false;
                self.walk_runs(|run| {
                    if run.permissions == Permissions::ReadExecute
                        && callback.0 >= run.rva.0
                        && run
                            .file
                            .is_some_and(|file| callback.0 - run.rva.0 < file.bytes)
                    {
                        executable = true;
                    }
                    Ok::<_, core::convert::Infallible>(())
                })
                .map_err(crate::layout::infallible_plan_error)?;
                if !executable {
                    return Err(self.error(callback.0, Reason::InvalidLayout));
                }
                plan.callbacks[i] = callback;
                plan.callback_count += 1;
            }
        }
        Ok(Some(plan))
    }
}
