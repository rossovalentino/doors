//! Windows virtual-memory flags and records shared by ntdll and the kernel
//! (P18 M0b). The kernel's reservations and page tables are authoritative;
//! this is range rounding, flag checks and the VM_QUERY record.
pub const COMMIT: u32 = 0x1000;
pub const RESERVE: u32 = 0x2000;
pub const DECOMMIT: u32 = 0x4000;
pub const RELEASE: u32 = 0x8000;
/// MEM_RESET / MEM_RESET_UNDO: the contents may be discarded / are wanted
/// again. Both are accepted and keep the pages (always allowed).
pub const RESET: u32 = 0x8_0000;
pub const RESET_UNDO: u32 = 0x100_0000;
pub const TOP_DOWN: u32 = 0x10_0000;
/// MEM_WRITE_WATCH, MEM_PHYSICAL, MEM_LARGE_PAGES: not provided.
pub const WRITE_WATCH: u32 = 0x20_0000;
pub const PHYSICAL: u32 = 0x40_0000;
pub const LARGE_PAGES: u32 = 0x2000_0000;
pub const PAGE: u64 = 4096;
/// VM_RESERVE's flags argument.
pub const RESERVE_TOP_DOWN: u64 = 1;
/// VM_RESERVE flag: a mapped image (MEMORY_BASIC_INFORMATION Type MEM_IMAGE),
/// as ntdll's loader reserves executables and DLLs.
pub const RESERVE_IMAGE: u64 = 2;
/// VM_RESERVE flag: a placeholder (VirtualAlloc2 MEM_RESERVE_PLACEHOLDER;
/// protection PAGE_NOACCESS).
pub const RESERVE_PLACEHOLDER: u64 = 8;
/// VM_RESERVE flag alone: the placeholder of exactly `[address, address +
/// bytes)` becomes an ordinary reservation (MEM_REPLACE_PLACEHOLDER).
pub const RESERVE_REPLACE: u64 = 16;
/// VM_WINDOWS_RELEASE flags (a[2]; a[1] the bytes): MEM_PRESERVE_PLACEHOLDER
/// and MEM_COALESCE_PLACEHOLDERS.
pub const RELEASE_PRESERVE_PLACEHOLDER: u64 = 1;
pub const RELEASE_COALESCE_PLACEHOLDERS: u64 = 2;
/// MEM_* allocation types of VirtualAlloc2, VirtualFree and MapViewOfFile3.
pub const RESERVE_PLACEHOLDER_TYPE: u32 = 0x4_0000;
pub const REPLACE_PLACEHOLDER: u32 = 0x4000;
pub const PRESERVE_PLACEHOLDER: u32 = 0x2;
pub const COALESCE_PLACEHOLDERS: u32 = 0x1;
pub const UNMAP_WITH_TRANSIENT_BOOST: u32 = 0x1;

/// Windows sections (P18): VM_SECTION_* flags.
pub mod section {
    /// VM_SECTION_CREATE: SEC_RESERVE (pages are committed later through
    /// views; otherwise SEC_COMMIT, every page committed).
    pub const RESERVE: u64 = 1;
    /// VM_SECTION_MAP (besides `RESERVE_TOP_DOWN`): replace the empty
    /// private reservation of exactly the view's range
    /// (MEM_REPLACE_PLACEHOLDER).
    pub const REPLACE: u64 = 4;
    /// VM_SECTION_UNMAP: leave an empty reservation in the view's place
    /// (MEM_PRESERVE_PLACEHOLDER).
    pub const PRESERVE: u64 = 1;
    /// VM_SECTION_MAP: the view is an image (MEMORY_BASIC_INFORMATION Type
    /// MEM_IMAGE; ntdll lays out SEC_IMAGE sections).
    pub const IMAGE: u64 = 8;
    /// CreateFileMapping flProtect attributes.
    pub const SEC_FILE: u32 = 0x80_0000;
    pub const SEC_IMAGE: u32 = 0x100_0000;
    pub const SEC_RESERVE: u32 = 0x400_0000;
    pub const SEC_COMMIT: u32 = 0x800_0000;
    pub const SEC_NOCACHE: u32 = 0x1000_0000;
    pub const SEC_LARGE_PAGES: u32 = 0x8000_0000;
    pub const SEC_IMAGE_NO_EXECUTE: u32 = 0x1100_0000;
    /// FILE_MAP_* desired access.
    pub const FILE_MAP_COPY: u32 = 0x1;
    pub const FILE_MAP_WRITE: u32 = 0x2;
    pub const FILE_MAP_READ: u32 = 0x4;
    pub const FILE_MAP_ALL_ACCESS: u32 = 0xF001F;
    pub const FILE_MAP_EXECUTE: u32 = 0x20;
    pub const FILE_MAP_RESERVE: u32 = 0x8000_0000;
    /// The PAGE_* protection of a view mapped with `access` (FILE_MAP_*),
    /// None for an invalid combination. FILE_MAP_COPY is copy on write
    /// (WRITECOPY, EXECUTE_WRITECOPY with FILE_MAP_EXECUTE).
    pub const fn view_protection(access: u32) -> Option<u32> {
        let all = access & FILE_MAP_ALL_ACCESS == FILE_MAP_ALL_ACCESS;
        let write = access & FILE_MAP_WRITE != 0 || all;
        let copy = access & FILE_MAP_COPY != 0 && !write;
        let read = access & (FILE_MAP_READ | FILE_MAP_WRITE | FILE_MAP_COPY) != 0 || all;
        let execute = access & FILE_MAP_EXECUTE != 0;
        Some(match (read, write, copy, execute) {
            (_, true, _, true) => super::page::EXECUTE_READWRITE,
            (_, true, _, false) => super::page::READWRITE,
            (_, false, true, true) => super::page::EXECUTE_WRITECOPY,
            (_, false, true, false) => super::page::WRITECOPY,
            (true, false, false, true) => super::page::EXECUTE_READ,
            (true, false, false, false) => super::page::READONLY,
            (false, false, false, true) => super::page::EXECUTE,
            (false, false, false, false) => return None,
        })
    }
}

/// PAGE_* values.
pub mod page {
    pub const NOACCESS: u32 = 0x01;
    pub const READONLY: u32 = 0x02;
    pub const READWRITE: u32 = 0x04;
    pub const WRITECOPY: u32 = 0x08;
    pub const EXECUTE: u32 = 0x10;
    pub const EXECUTE_READ: u32 = 0x20;
    pub const EXECUTE_READWRITE: u32 = 0x40;
    pub const EXECUTE_WRITECOPY: u32 = 0x80;
    pub const GUARD: u32 = 0x100;
    pub const NOCACHE: u32 = 0x200;
    pub const WRITECOMBINE: u32 = 0x400;
    /// Control Flow Guard markers (no effect here).
    pub const TARGETS_INVALID: u32 = 0x4000_0000;
}

pub fn range(address: u64, bytes: u64, reserve: bool) -> Option<(u64, u64)> {
    if bytes == 0 {
        return None;
    }
    let mask = if reserve { 0xffff } else { PAGE - 1 };
    let start = address & !mask;
    let end = address.checked_add(bytes)?.checked_add(PAGE - 1)? & !(PAGE - 1);
    if end > 0x8000_0000_0000 || end <= start {
        return None;
    }
    Some((start, end - start))
}

/// Whether `value` is a PAGE_* protection VirtualAlloc/VirtualProtect
/// accept for private memory: one base value, optionally PAGE_GUARD (not
/// with NOACCESS) and the CFG marker. NOCACHE/WRITECOMBINE are refused.
pub fn protection_valid(value: u32) -> bool {
    let value = value & !page::TARGETS_INVALID;
    let guard = value & page::GUARD != 0;
    let base = value & !page::GUARD;
    matches!(
        base,
        page::NOACCESS
            | page::READONLY
            | page::READWRITE
            | page::WRITECOPY
            | page::EXECUTE
            | page::EXECUTE_READ
            | page::EXECUTE_READWRITE
            | page::EXECUTE_WRITECOPY
    ) && !(guard && base == page::NOACCESS)
}

/// MEMORY_BASIC_INFORMATION (64-bit layout), as VM_QUERY writes it.
pub const QUERY_BYTES: usize = 48;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BasicInformation {
    pub base: u64,
    pub allocation_base: u64,
    pub allocation_protect: u32,
    pub size: u64,
    pub state: u32,
    pub protect: u32,
    pub kind: u32,
}
impl BasicInformation {
    pub fn encode(&self) -> [u8; QUERY_BYTES] {
        let mut out = [0; QUERY_BYTES];
        out[0..8].copy_from_slice(&self.base.to_le_bytes());
        out[8..16].copy_from_slice(&self.allocation_base.to_le_bytes());
        out[16..20].copy_from_slice(&self.allocation_protect.to_le_bytes());
        // 20..24: PartitionId and padding.
        out[24..32].copy_from_slice(&self.size.to_le_bytes());
        out[32..36].copy_from_slice(&self.state.to_le_bytes());
        out[36..40].copy_from_slice(&self.protect.to_le_bytes());
        out[40..44].copy_from_slice(&self.kind.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8; QUERY_BYTES]) -> Self {
        let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        Self {
            base: u64_at(0),
            allocation_base: u64_at(8),
            allocation_protect: u32_at(16),
            size: u64_at(24),
            state: u32_at(32),
            protect: u32_at(36),
            kind: u32_at(40),
        }
    }
    /// The 28-byte MEMORY_BASIC_INFORMATION of a 32-bit process, or None
    /// when an address does not fit 32 bits.
    pub fn encode32(&self) -> Option<[u8; 28]> {
        let mut out = [0; 28];
        let narrow = |v: u64| u32::try_from(v).ok();
        out[0..4].copy_from_slice(&narrow(self.base)?.to_le_bytes());
        out[4..8].copy_from_slice(&narrow(self.allocation_base)?.to_le_bytes());
        out[8..12].copy_from_slice(&self.allocation_protect.to_le_bytes());
        // The size is clamped to the 32-bit address space.
        let size = self.size.min(0x1_0000_0000 - self.base) as u32;
        out[12..16].copy_from_slice(&size.to_le_bytes());
        out[16..20].copy_from_slice(&self.state.to_le_bytes());
        out[20..24].copy_from_slice(&self.protect.to_le_bytes());
        out[24..28].copy_from_slice(&self.kind.to_le_bytes());
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_include_straddling_pages_and_reservation_granularity() {
        assert_eq!(range(0, 1, true), Some((0, 4096)));
        assert_eq!(
            range(0x5000_1001, 4096, true),
            Some((0x5000_0000, 3 * PAGE))
        );
        assert_eq!(range(0x5000_0fff, 2, false), Some((0x5000_0000, 2 * PAGE)));
        assert_eq!(range(0x5000_1000, PAGE, false), Some((0x5000_1000, PAGE)));
    }
    #[test]
    fn overflow_empty_and_kernel_ranges_never_become_valid_extents() {
        for (address, bytes) in [
            (0, 0),
            (u64::MAX, 1),
            (0x8000_0000_0000, PAGE),
            (0, u64::MAX),
        ] {
            assert_eq!(range(address, bytes, true), None);
            assert_eq!(range(address, bytes, false), None);
        }
    }
    #[test]
    fn every_private_protection_is_accepted_and_device_modes_are_not() {
        for value in [
            1,
            2,
            4,
            8,
            0x10,
            0x20,
            0x40,
            0x80,
            0x104,
            0x140,
            0x4000_0040,
        ] {
            assert!(protection_valid(value), "{value:#x}");
        }
        for value in [0, 3, 0x101, 0x204, 0x404, 0x44, 0x1000] {
            assert!(!protection_valid(value), "{value:#x}");
        }
    }
    #[test]
    fn query_records_round_trip() {
        let q = BasicInformation {
            base: 0x1_2345_6000,
            allocation_base: 0x1_2345_0000,
            allocation_protect: 4,
            size: 0x3000,
            state: COMMIT,
            protect: 0x40,
            kind: 0x2_0000,
        };
        assert_eq!(BasicInformation::decode(&q.encode()), q);
        assert_eq!(q.encode32(), None);
        let low = BasicInformation {
            base: 0x40_0000,
            ..q
        };
        let low = BasicInformation {
            allocation_base: 0x40_0000,
            ..low
        };
        let bytes = low.encode32().unwrap();
        assert_eq!(&bytes[12..16], &0x3000u32.to_le_bytes());
    }
}
