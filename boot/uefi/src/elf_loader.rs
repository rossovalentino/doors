use crate::firmware::Allocation;
use door_boot_protocol::Mapping;
use door_image_format::ElfImage;

pub struct LoadedKernel {
    pub _backing: Allocation,
    pub entry: u64,
    pub base: u64,
    pub bytes: u64,
    pub mappings: [Option<Mapping>; 8],
}
pub fn load(bytes: &[u8]) -> LoadedKernel {
    let elf = ElfImage::parse(bytes).expect("kernel ELF validation");
    let mut base = u64::MAX;
    let mut end = 0;
    for index in 0..elf.segment_count() {
        let s = elf.segment(index).expect("ELF segment");
        if s.kind != 1 {
            continue;
        }
        assert!(s.virtual_address >= 0xffff_ffff_8000_0000);
        base = base.min(s.virtual_address);
        end = end.max(
            s.virtual_address
                .checked_add(s.memory_bytes)
                .expect("segment end"),
        );
    }
    let length = (end - base).div_ceil(4096) * 4096;
    assert!(length <= 32 * 1024 * 1024);
    let mut backing = Allocation::new(length as usize);
    let mut mappings = [None; 8];
    let mut count = 0;
    for index in 0..elf.segment_count() {
        let s = elf.segment(index).expect("ELF segment");
        if s.kind != 1 {
            continue;
        }
        let offset = (s.virtual_address - base) as usize;
        let source = &bytes[s.offset as usize..(s.offset + s.file_bytes) as usize];
        backing.data_mut()[offset..offset + source.len()].copy_from_slice(source);
        assert!(count < mappings.len());
        mappings[count] = Some(Mapping {
            virtual_start: s.virtual_address,
            physical: backing.base() + offset as u64,
            bytes: s.memory_bytes.div_ceil(4096) * 4096,
            permissions: s.flags,
            role: 1,
        });
        count += 1;
    }
    LoadedKernel {
        _backing: backing,
        entry: elf.entry,
        base,
        bytes: length,
        mappings,
    }
}
