//! Bounded loader-owned four-level paging. No huge pages or writable executable alias.
use crate::firmware::Allocation;
use door_boot_protocol::{KERNEL_TABLE_POOL_PAGES, Mapping};
pub const STACK_BASE: u64 = 0xffff_ffff_9000_1000;
pub struct PageTables {
    pub backing: Allocation,
    used: usize,
}
impl PageTables {
    pub fn new() -> Self {
        Self {
            backing: Allocation::new(KERNEL_TABLE_POOL_PAGES * 4096),
            used: 1,
        }
    }
    pub fn map(&mut self, m: Mapping, ram_framebuffer: bool) {
        for offset in (0..m.bytes).step_by(4096) {
            let virtual_address = m.virtual_start + offset;
            let physical = m.physical + offset;
            assert!(physical < 0x1_0000_0000);
            let indexes = [
                (virtual_address >> 39) & 511,
                (virtual_address >> 30) & 511,
                (virtual_address >> 21) & 511,
                (virtual_address >> 12) & 511,
            ];
            let mut table = self.backing.base();
            for index in indexes[..3].iter().copied() {
                let entry = (table + index * 8) as *mut u64;
                // SAFETY: all intermediate tables are exclusive pages from the bounded pool.
                let mut value = unsafe { entry.read() };
                if value == 0 {
                    assert!(self.used < KERNEL_TABLE_POOL_PAGES, "page table limit");
                    value = self.backing.base() + (self.used * 4096) as u64;
                    self.used += 1;
                    unsafe {
                        entry.write(value | 3);
                    }
                }
                table = value & 0x000f_ffff_ffff_f000;
            }
            let entry = (table + indexes[3] * 8) as *mut u64;
            assert_eq!(unsafe { entry.read() }, 0, "mapping overlap");
            let flags = 1
                | if m.permissions == 6 { 2 } else { 0 }
                | if m.permissions == 5 { 0 } else { 1u64 << 63 }
                // Loader-allocated RAMFB retains normal RAM cache attributes.
                // Firmware GOP MMIO keeps its established uncached policy.
                | if m.role == 7 && !ram_framebuffer { 0x18 } else { 0 };
            unsafe {
                entry.write(physical | flags);
            }
        }
    }
}
