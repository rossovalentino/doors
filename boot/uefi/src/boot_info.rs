use door_boot_protocol::{
    Framebuffer, Header, Mapping, MemoryRegion, Module, Ownership, PLATFORM_BYTES, PhysicalRange,
    Platform, REQUIRED, STARTUP_BYTES, STARTUP_KIND, Startup, TableDescriptor,
};
use uefi::mem::memory_map::{MemoryMap, MemoryMapMut, MemoryMapOwned};

pub struct Payload<'a> {
    pub modules: &'a [Module; 2],
    pub maps: &'a [Mapping],
    pub fb: Framebuffer,
    pub platform: &'a Platform,
    pub startup: Startup,
}
pub fn encode(
    buffer: &mut [u8],
    physical: u64,
    mut memory: MemoryMapOwned,
    payload: Payload<'_>,
) -> usize {
    let Payload {
        modules,
        maps,
        fb,
        platform,
        startup,
    } = payload;
    memory.sort();
    let mut cursor = 64 + 6 * 32;
    let modules_offset = cursor;
    for module in modules {
        module
            .encode(&mut buffer[cursor..cursor + 80])
            .expect("module encoding");
        cursor += 80;
    }
    let maps_offset = cursor;
    for map in maps {
        map.encode(&mut buffer[cursor..cursor + 40])
            .expect("mapping encoding");
        cursor += 40;
    }
    let framebuffer_offset = cursor;
    fb.encode(&mut buffer[cursor..cursor + 40])
        .expect("framebuffer encoding");
    cursor += 40;
    let platform_offset = cursor;
    platform
        .encode(&mut buffer[cursor..cursor + PLATFORM_BYTES])
        .expect("platform encoding");
    cursor += PLATFORM_BYTES;
    let startup_offset = cursor;
    startup
        .encode(&mut buffer[cursor..cursor + STARTUP_BYTES])
        .expect("startup encoding");
    cursor += STARTUP_BYTES;
    let memory_offset = cursor;
    for descriptor in memory.entries() {
        if descriptor.page_count == 0 {
            continue;
        }
        let bytes = descriptor
            .page_count
            .checked_mul(4096)
            .expect("firmware range");
        let ownership =
            if descriptor.att.bits() & (1 << 63) != 0 || matches!(descriptor.ty.0, 5 | 6) {
                Ownership::FirmwareRuntime
            } else {
                match descriptor.ty.0 {
                    7 => Ownership::UsableAfterHandoff,
                    1 | 2 => Ownership::BootData,
                    3 | 4 => Ownership::ReclaimableAfterCopy,
                    11 | 12 => Ownership::FramebufferMmio,
                    _ => Ownership::PermanentlyReserved,
                }
            };
        let region = MemoryRegion {
            range: PhysicalRange::new(descriptor.phys_start, bytes, 4096)
                .expect("firmware memory extent"),
            firmware_attributes: descriptor.att.bits(),
            firmware_type: descriptor.ty.0,
            ownership,
        };
        assert!(cursor + 40 <= buffer.len());
        region
            .encode(&mut buffer[cursor..cursor + 40])
            .expect("memory encoding");
        cursor += 40;
    }
    let tables = [
        (1, memory_offset, cursor - memory_offset, 40),
        (2, modules_offset, 160, 80),
        (3, maps_offset, maps.len() * 40, 40),
        (4, framebuffer_offset, 40, 40),
        (5, platform_offset, PLATFORM_BYTES, PLATFORM_BYTES as u32),
        (
            STARTUP_KIND,
            startup_offset,
            STARTUP_BYTES,
            STARTUP_BYTES as u32,
        ),
    ];
    Header {
        total_bytes: cursor as u64,
        table_count: 6,
        tables_offset: 64,
        self_phys: physical,
    }
    .encode(buffer)
    .expect("boot header");
    for (index, (kind, offset, length, entry_bytes)) in tables.into_iter().enumerate() {
        TableDescriptor {
            kind,
            revision: if kind == 5 { 2 } else { 1 },
            flags: REQUIRED,
            offset: offset as u64,
            length: length as u64,
            entry_bytes,
        }
        .encode(&mut buffer[64 + index * 32..96 + index * 32], cursor as u64)
        .expect("boot descriptor");
    }
    // The memory-map owner has no firmware-dependent free after exit. Its backing
    // remains reserved as BootData; this phase reclaims no loader allocation.
    cursor
}
