use crate::{
    Fixup, FixupKind, Limit, LoadPolicy, Machine, ModulePlan, Reason, Rva, VisitError, dword, word,
};

impl ModulePlan<'_> {
    pub(crate) fn walk_relocations<E>(
        &self,
        policy: LoadPolicy,
        mut visitor: impl FnMut(Fixup) -> Result<(), E>,
    ) -> Result<(), VisitError<E>> {
        let (directory, bytes) = self.directory(5)?;
        if directory == 0 {
            return Ok(());
        }
        // DIR64 records in x64 images, HIGHLOW in x86 ones: an absolute
        // address the width of the image's pointers.
        let (expected, fixup_kind) = match self.machine() {
            Machine::X64 => (10, FixupKind::Dir64),
            Machine::X86 => (3, FixupKind::HighLow),
        };
        let width = self.pointer_bytes();
        let mut cursor = 0;
        let mut previous_page = None;
        let mut previous_target = None;
        let mut entries = 0;
        while cursor < bytes {
            if bytes - cursor < 8 {
                return Err(self
                    .error(directory + cursor, Reason::InvalidRelocation)
                    .into());
            }
            let block = self.read(directory + cursor, 8)?;
            let page = dword(block, 0);
            let block_bytes = dword(block, 4);
            if !page.is_multiple_of(4096)
                || page >= self.image_bytes()
                || previous_page.is_some_and(|previous| previous >= page)
                || block_bytes < 8
                || !block_bytes.is_multiple_of(4)
                || block_bytes > bytes - cursor
            {
                return Err(self
                    .error(directory + cursor, Reason::InvalidRelocation)
                    .into());
            }
            previous_page = Some(page);
            let records = self.read(directory + cursor + 8, block_bytes as usize - 8)?;
            for index in 0..records.len() / 2 {
                entries += 1;
                if entries > policy.limits().relocations * 2 {
                    return Err(self
                        .error(
                            directory + cursor,
                            Reason::ResourceLimit(Limit::Relocations),
                        )
                        .into());
                }
                let record = word(records, index * 2);
                let kind = record >> 12;
                if kind == 0 {
                    continue;
                } // IMAGE_REL_BASED_ABSOLUTE padding.
                let rva = page + u32::from(record & 0xfff);
                // Windows applies a record at any 12-bit offset of its page:
                // one near the page's end writes into the next page (common
                // in x86 code, whose absolute operands are unaligned). The
                // whole target must still lie in one mapped section.
                if kind != expected
                    || previous_target.is_some_and(|previous| previous + width > rva)
                {
                    return Err(self.error(rva, Reason::InvalidRelocation).into());
                }
                previous_target = Some(rva);
                let section = self.mapped_section(rva, width)?;
                let relative = rva - section.virtual_address;
                let mut original = [0_u8; 8];
                let backed = section.raw_bytes.saturating_sub(relative).min(width);
                if backed != 0 {
                    original[..backed as usize].copy_from_slice(self.read(rva, backed as usize)?);
                }
                let pointer = self.pointer_in(&original, 0);
                // Initial policy accepts image-relative absolute pointers only.
                // A relocation cannot introduce an arbitrary kernel/user address.
                let relative = pointer
                    .checked_sub(self.image.image_base)
                    .filter(|offset| *offset < u64::from(self.image_bytes()))
                    .ok_or_else(|| self.error(rva, Reason::InvalidRelocation))?;
                let value = self.source.base.0 + relative;
                visitor(Fixup {
                    module: self.index,
                    rva: Rva(rva),
                    address: self.address(rva),
                    value,
                    kind: fixup_kind,
                    width: width as u8,
                })
                .map_err(VisitError::Callback)?;
            }
            cursor += block_bytes;
        }
        Ok(())
    }
}
