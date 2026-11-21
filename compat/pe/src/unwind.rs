//! Structural validation only. The initial policy (P09) keeps its narrow
//! rules; the application policy (P18) accepts what `x64_unwind` walks:
//! versions 1 and 2, every code, chains and language handlers.
//! P09 contains faults instead of dispatching or
//! unwinding foreign exceptions. Required handlers/chains are explicitly refused,
//! except that the P13 Application policy accepts language-handler frames
//! (UNW_FLAG_EHANDLER/UHANDLER with an executable handler in the same module):
//! ordinary MSVC code and the pinned precompiled Rust `core`/`alloc` carry them.
//! The virtual unwinder still fails closed on such frames; handlers are never
//! invoked, so a fault there is contained exactly as before.
use crate::{Error, Feature, Limit, ModulePlan, Reason, dword, word};

impl ModulePlan<'_> {
    pub(crate) fn validate_unwind(&self, handlers: bool) -> Result<(), Error> {
        let (directory, bytes) = self.directory(3)?;
        if directory == 0 {
            return Ok(());
        }
        if !directory.is_multiple_of(4) || !bytes.is_multiple_of(12) {
            return Err(self.error(directory, Reason::InvalidUnwind));
        }
        // The application policy (P18): as many functions as the image
        // holds (CoreCLR has tens of thousands).
        let limit = if handlers { 1 << 20 } else { 4096 };
        if bytes / 12 > limit {
            return Err(self.error(directory, Reason::ResourceLimit(Limit::UnwindEntries)));
        }
        let mut previous_end = 0;
        for index in 0..bytes / 12 {
            let record = self.read(directory + index * 12, 12)?;
            let start = dword(record, 0);
            let end = dword(record, 4);
            let info = dword(record, 8);
            if start >= end || start < previous_end || !info.is_multiple_of(4) {
                return Err(self.error(directory + index * 12, Reason::InvalidUnwind));
            }
            self.executable(start, end - start)
                .map_err(|_| self.error(start, Reason::InvalidUnwind))?;
            if handlers {
                self.validate_application_info(info, 0)?;
            } else {
                self.validate_unwind_info(info, end - start, handlers)?;
            }
            previous_end = end;
        }
        Ok(())
    }
    /// An application image's UNWIND_INFO, as Windows' unwinder reads it
    /// (`x64_unwind`): versions 1 and 2, every unwind code with its slot
    /// count, any frame register, language handlers inside the module and
    /// chained infos (a RUNTIME_FUNCTION of executable code, followed to a
    /// bounded depth). Only what the unwinder could not walk is refused.
    fn validate_application_info(&self, rva: u32, depth: usize) -> Result<(), Error> {
        let header = self.read(rva, 4)?;
        let version = header[0] & 7;
        if version != 1 && version != 2 {
            return Err(self.error(rva, Reason::Unsupported(Feature::UnwindVersion)));
        }
        let flags = header[0] >> 3;
        let chained = flags & 4 != 0;
        if flags & !7 != 0 || (chained && flags & 3 != 0) {
            return Err(self.error(rva, Reason::InvalidUnwind));
        }
        let count = usize::from(header[2]);
        let data = self.read(rva, 4 + ((count + 1) & !1) * 2)?;
        let codes = &data[4..];
        let mut index = 0;
        while index < count {
            let op = codes[index * 2 + 1] & 15;
            let info = codes[index * 2 + 1] >> 4;
            let slots = crate::x64_unwind::opcode_slots(op, info)
                .map_err(|_| self.error(rva, Reason::InvalidUnwind))?;
            if index + slots > count {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            index += slots;
        }
        let at = rva + 4 + (((count + 1) & !1) * 2) as u32;
        if chained {
            if depth >= 32 {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            let record = self.read(at, 12)?;
            let (start, end, info) = (dword(record, 0), dword(record, 4), dword(record, 8));
            if start >= end || !info.is_multiple_of(4) {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            self.executable(start, end - start)
                .map_err(|_| self.error(rva, Reason::InvalidUnwind))?;
            return self.validate_application_info(info, depth + 1);
        }
        if flags != 0 {
            let handler = dword(self.read(at, 4)?, 0);
            self.executable(handler, 1)
                .map_err(|_| self.error(rva, Reason::InvalidUnwind))?;
        }
        Ok(())
    }
    fn validate_unwind_info(
        &self,
        rva: u32,
        function_bytes: u32,
        handlers: bool,
    ) -> Result<(), Error> {
        let header = self.read(rva, 4)?;
        if header[0] & 7 != 1 {
            return Err(self.error(rva, Reason::Unsupported(Feature::UnwindVersion)));
        }
        // Bits 0 and 1 of the flags: exception and termination handlers.
        let flags = header[0] >> 3;
        if flags != 0 && !(handlers && flags & !3 == 0) {
            return Err(self.error(rva, Reason::Unsupported(Feature::UnwindHandler)));
        }
        let prolog = header[1];
        let count = usize::from(header[2]);
        let frame = header[3] & 15;
        if u32::from(prolog) > function_bytes
            || (frame == 0 && header[3] != 0)
            || (frame != 0 && !nonvolatile(frame))
        {
            return Err(self.error(rva, Reason::InvalidUnwind));
        }
        let data = self.read(rva, 4 + ((count + 1) & !1) * 2)?;
        let codes = &data[4..];
        let mut index = 0;
        let mut previous_offset = prolog;
        let mut frame_operations = 0;
        while index < count {
            let offset = codes[index * 2];
            let op = codes[index * 2 + 1] & 15;
            let info = codes[index * 2 + 1] >> 4;
            if offset > previous_offset {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            previous_offset = offset;
            let slots = match op {
                0 if nonvolatile(info) => 1,
                1 if info == 0 => 2,
                1 if info == 1 => 3,
                2 => 1,
                3 if info == 0 && frame != 0 => {
                    frame_operations += 1;
                    1
                }
                4 if nonvolatile(info) => 2,
                5 if nonvolatile(info) => 3,
                8 if info >= 6 => 2,
                9 if info >= 6 => 3,
                10 if info <= 1 => 1,
                _ => return Err(self.error(rva, Reason::InvalidUnwind)),
            };
            if index + slots > count {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            if op == 1 {
                let bytes = if info == 0 {
                    u32::from(word(codes, (index + 1) * 2)) * 8
                } else {
                    dword(codes, (index + 1) * 2)
                };
                if bytes == 0 || !bytes.is_multiple_of(8) {
                    return Err(self.error(rva, Reason::InvalidUnwind));
                }
            }
            if (op == 5 && !dword(codes, (index + 1) * 2).is_multiple_of(8))
                || (op == 9 && !dword(codes, (index + 1) * 2).is_multiple_of(16))
            {
                return Err(self.error(rva, Reason::InvalidUnwind));
            }
            index += slots;
        }
        if (frame != 0 && frame_operations != 1) || (frame == 0 && frame_operations != 0) {
            return Err(self.error(rva, Reason::InvalidUnwind));
        }
        if flags != 0 {
            // The handler RVA follows the (even-padded) code array.
            let at = rva + 4 + (((count + 1) & !1) * 2) as u32;
            let handler = dword(self.read(at, 4)?, 0);
            self.executable(handler, 1)
                .map_err(|_| self.error(rva, Reason::InvalidUnwind))?;
        }
        Ok(())
    }
}

fn nonvolatile(register: u8) -> bool {
    matches!(register, 3 | 5 | 6 | 7 | 12..=15)
}
