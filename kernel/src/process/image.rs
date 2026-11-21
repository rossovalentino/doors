//! Original native ELF loading policy, independent of process authority/startup.
use crate::mm::{
    self, FrameAllocator, PAGE_BYTES, PageCount, PageTables, Permissions, ProcessSpace, RegionId,
    USER_END, USER_MIN, VirtualAddress,
};
use door_image_format::ElfImage;

pub const MAX_SEGMENTS: usize = 8;
pub const MAX_FILE_BYTES: usize = door_initfs::MAX_NATIVE_FILE;
pub const MAX_MEMORY_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Format(door_image_format::Error),
    Policy,
    Memory(mm::Error),
}
impl From<mm::Error> for Error {
    fn from(value: mm::Error) -> Self {
        Self::Memory(value)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub address: u64,
    pub pages: u64,
    pub file_offset: usize,
    pub file_bytes: usize,
    pub permissions: Permissions,
}
pub struct ImagePlan<'a> {
    bytes: &'a [u8],
    entry: u64,
    segments: [Option<LoadSegment>; MAX_SEGMENTS],
    count: usize,
}
pub struct LoadedImage {
    pub entry: u64,
    pub regions: [Option<RegionId>; MAX_SEGMENTS],
    pub next_alias: VirtualAddress,
}
impl<'a> ImagePlan<'a> {
    /// Structural parsing and native policy checks complete before allocation.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::Policy);
        }
        let image = ElfImage::parse(bytes).map_err(Error::Format)?;
        let mut result = Self {
            bytes,
            entry: image.entry,
            segments: [None; MAX_SEGMENTS],
            count: 0,
        };
        let mut total = 0u64;
        let mut first = 0;
        let mut previous_end = 0;
        for index in 0..image.segment_count() {
            let segment = image.segment(index).map_err(Error::Format)?;
            if segment.kind != 1 {
                continue;
            }
            if result.count == MAX_SEGMENTS {
                return Err(Error::Policy);
            }
            let rounded = segment
                .memory_bytes
                .checked_add(PAGE_BYTES - 1)
                .ok_or(Error::Policy)?
                & !(PAGE_BYTES - 1);
            let end = segment
                .virtual_address
                .checked_add(rounded)
                .ok_or(Error::Policy)?;
            total = total.checked_add(rounded).ok_or(Error::Policy)?;
            if result.count == 0 {
                first = segment.virtual_address;
            }
            if segment.virtual_address < USER_MIN
                || end > USER_END
                || total > MAX_MEMORY_BYTES
                || end - first > MAX_MEMORY_BYTES
                || segment.virtual_address < previous_end
            {
                return Err(Error::Policy);
            }
            let permissions = match segment.flags {
                4 => Permissions::ReadOnly,
                5 => Permissions::ReadExecute,
                6 => Permissions::ReadWrite,
                _ => return Err(Error::Policy),
            };
            result.segments[result.count] = Some(LoadSegment {
                address: segment.virtual_address,
                pages: rounded / PAGE_BYTES,
                file_offset: segment.offset as usize,
                file_bytes: segment.file_bytes as usize,
                permissions,
            });
            result.count += 1;
            previous_end = end;
        }
        Ok(result)
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub fn segments(&self) -> impl Iterator<Item = LoadSegment> + '_ {
        self.segments.iter().flatten().copied()
    }
    /// Reserve, zero, copy and publish all segments. Allocation failure removes
    /// all this invocation's mappings/reservations, preserving earlier process
    /// regions. Structural tables may remain retained until process destruction.
    ///
    /// # Safety
    /// ProcessSpace's exclusive VM guard, trusted CR3 and IRQ-off contracts hold.
    /// No thread has started this process. Alias storage and one-page gaps are
    /// reserved by process policy for the whole image; input bytes stay immutable.
    pub unsafe fn load(
        &self,
        space: &mut ProcessSpace,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
        alias_start: VirtualAddress,
    ) -> Result<LoadedImage, Error> {
        unsafe { self.load_with_checkpoints(space, frames, kernel, alias_start, &mut || Ok(())) }
    }
    /// The normal loader with a bounded kernel diagnostic checkpoint before and
    /// after each segment construction. A checkpoint error uses the identical
    /// ownership rollback path as allocation or mapping failure. This callback
    /// is kernel policy and is never supplied by an ABI caller.
    ///
    /// # Safety
    /// All `load` preconditions hold. The callback must not mutate the process,
    /// allocator, kernel tables, input bytes, or external mapping ownership.
    pub unsafe fn load_with_checkpoints(
        &self,
        space: &mut ProcessSpace,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
        alias_start: VirtualAddress,
        checkpoint: &mut impl FnMut() -> Result<(), mm::Error>,
    ) -> Result<LoadedImage, Error> {
        let mut aliases = [alias_start; MAX_SEGMENTS];
        let mut next = alias_start;
        for (index, segment) in self.segments().enumerate() {
            aliases[index] = next;
            next = next.checked_add(PageCount::new(segment.pages + 1)?.bytes())?;
        }
        let mut regions = [None; MAX_SEGMENTS];
        let result = (|| {
            for (index, segment) in self.segments().enumerate() {
                checkpoint()?;
                let id = space.reserve(segment.address, PageCount::new(segment.pages)?)?;
                regions[index] = Some(id);
                let bytes =
                    &self.bytes[segment.file_offset..segment.file_offset + segment.file_bytes];
                unsafe {
                    space.commit(
                        id,
                        frames,
                        kernel,
                        aliases[index],
                        segment.permissions,
                        bytes,
                    )?
                };
                checkpoint()?;
            }
            Ok::<(), Error>(())
        })();
        if let Err(error) = result {
            for id in regions.into_iter().flatten().rev() {
                if space.get(id)?.permissions.is_some() {
                    unsafe { space.decommit(id, frames, kernel)? };
                }
                space.release(id)?;
            }
            return Err(error);
        }
        Ok(LoadedImage {
            entry: self.entry,
            regions,
            next_alias: next,
        })
    }
}
/// Capture a supplied executable image (up to `EXECUTABLE_MAX_BYTES`) as a
/// sequence of copies of at most `limit` bytes each: one architecture user
/// copy is bounded (`MAX_USER_COPY`, interrupts off), so a whole image must
/// never be requested as a single copy. The caller validates and pins the
/// complete source range first; `copy` receives each chunk's source offset.
pub fn capture_bounded<E>(
    output: &mut [u8],
    limit: usize,
    mut copy: impl FnMut(u64, &mut [u8]) -> Result<(), E>,
) -> Result<(), E> {
    for (index, chunk) in output.chunks_mut(limit.max(1)).enumerate() {
        copy((index * limit.max(1)) as u64, chunk)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{vec, vec::Vec};
    fn put(bytes: &mut [u8], offset: usize, value: u64, width: usize) {
        bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    fn image() -> Vec<u8> {
        let mut bytes = vec![0u8; 8200];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        for (offset, value, width) in [
            (16, 2, 2),
            (18, 62, 2),
            (20, 1, 4),
            (24, 0x400000, 8),
            (32, 64, 8),
            (52, 64, 2),
            (54, 56, 2),
            (56, 2, 2),
        ] {
            put(&mut bytes, offset, value, width);
        }
        for (header, address, offset, flags, memory) in [
            (64, 0x400000, 4096, 5, 4096),
            (120, 0x402000, 8192, 6, 8192),
        ] {
            for (relative, value, width) in [
                (0, 1, 4),
                (4, flags, 4),
                (8, offset, 8),
                (16, address, 8),
                (32, 8, 8),
                (40, memory, 8),
                (48, 4096, 8),
            ] {
                put(&mut bytes, header + relative, value, width);
            }
        }
        bytes[4096..4104].copy_from_slice(b"original");
        bytes
    }
    #[test]
    fn native_plan_preserves_rx_rw_and_zero_fill_extent() {
        let bytes = image();
        let plan = ImagePlan::parse(&bytes).unwrap();
        let segments: Vec<_> = plan.segments().collect();
        assert_eq!(plan.entry(), 0x400000);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].permissions, Permissions::ReadExecute);
        assert_eq!(segments[1].permissions, Permissions::ReadWrite);
        assert_eq!((segments[1].file_bytes, segments[1].pages), (8, 2));
    }
    #[test]
    fn native_plan_rejects_kernel_null_huge_and_sparse_images() {
        for address in [0, 0xffff_ffff_8000_0000] {
            let mut bytes = image();
            put(&mut bytes, 24, address, 8);
            put(&mut bytes, 64 + 16, address, 8);
            put(&mut bytes, 120 + 16, address + 8192, 8);
            assert!(matches!(ImagePlan::parse(&bytes), Err(Error::Policy)));
        }
        let mut bytes = image();
        put(&mut bytes, 120 + 40, MAX_MEMORY_BYTES, 8);
        assert!(matches!(ImagePlan::parse(&bytes), Err(Error::Policy)));
        let mut bytes = image();
        put(&mut bytes, 120 + 16, 0x2000000, 8);
        assert!(matches!(ImagePlan::parse(&bytes), Err(Error::Policy)));
    }
    #[test]
    fn native_plan_rejects_writable_entry_and_malformed_source_before_allocating() {
        let mut bytes = image();
        put(&mut bytes, 64 + 4, 7, 4);
        assert!(matches!(
            ImagePlan::parse(&bytes),
            Err(Error::Format(door_image_format::Error::Permissions))
        ));
        let mut bytes = image();
        put(&mut bytes, 120 + 32, 9000, 8);
        assert!(ImagePlan::parse(&bytes).is_err());
    }
    /// Regression: an 83768-byte door-tools image exceeded the 64 KiB single
    /// user-copy bound, so every launch failed with BadAddress (shell: Io).
    #[test]
    fn image_capture_larger_than_one_user_copy_is_complete_and_bounded() {
        const LIMIT: usize = 64 * 1024;
        for len in [1, LIMIT - 1, LIMIT, LIMIT + 1, 83768, 1024 * 1024] {
            let source: Vec<u8> = (0..len).map(|i| (i * 7 + i / 251) as u8).collect();
            let mut output = vec![0u8; len];
            let mut calls = Vec::new();
            capture_bounded(&mut output, LIMIT, |offset, chunk| {
                assert!(chunk.len() <= LIMIT && !chunk.is_empty());
                let start = offset as usize;
                chunk.copy_from_slice(&source[start..start + chunk.len()]);
                calls.push((start, chunk.len()));
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(output, source, "len {len}");
            assert_eq!(calls.len(), len.div_ceil(LIMIT));
        }
        let mut output = vec![0u8; 83768];
        let mut seen = 0;
        let failed = capture_bounded(&mut output, LIMIT, |offset, _| {
            seen += 1;
            if offset == LIMIT as u64 {
                Err("fault")
            } else {
                Ok(())
            }
        });
        assert_eq!((failed, seen), (Err("fault"), 2));
    }
}
