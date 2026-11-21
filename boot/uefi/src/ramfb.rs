//! Original QEMU RAM framebuffer bootstrap. The firmware's tiny default RAMFB
//! modes do not cover native 4K. Reserve our own bounded backing and publish it
//! through the documented fw_cfg DMA interface before leaving firmware.
use crate::firmware::Allocation;
use core::{
    arch::asm,
    sync::atomic::{Ordering, compiler_fence},
};
use door_boot_protocol::Framebuffer;

pub fn framebuffer(width: u32, height: u32) -> Option<Framebuffer> {
    // Only the explicit ramfb QEMU device provides this writable item.
    let (selector, size) = unsafe { door_boot_debug::fw_cfg_entry("etc/ramfb") }?;
    assert_eq!(size, 28, "RAMFB configuration size");
    let mut features = [0; 4];
    unsafe {
        asm!("out dx, ax", in("dx") 0x510u16, in("ax") 1u16,
            options(nomem, nostack, preserves_flags));
        for byte in &mut features {
            *byte = door_boot_debug::in8(0x511);
        }
    }
    assert!(
        u32::from_le_bytes(features) & 2 != 0,
        "RAMFB requires fw_cfg DMA"
    );
    assert!((640..=3840).contains(&width) && (480..=2400).contains(&height));
    let bytes = width as usize * height as usize * 4;
    let backing = Allocation::new(bytes);
    // LoaderData is retained by Door's BootData ownership, and the framebuffer
    // has an explicit live role7 mapping. OS loaders must not allocate UEFI's
    // EfiReservedMemoryType. Descriptor and payload remain loader-owned too.
    // A failed transfer never
    // returns firmware memory that a device could still reference.
    let mut transfer = Allocation::new(4096);
    let base = transfer.base();
    let data = transfer.data_mut();
    data[..4].copy_from_slice(&((u32::from(selector) << 16) | 8 | 16).to_be_bytes());
    data[4..8].copy_from_slice(&28u32.to_be_bytes());
    data[8..16].copy_from_slice(&(base + 16).to_be_bytes());
    data[16..24].copy_from_slice(&backing.base().to_be_bytes());
    data[24..28].copy_from_slice(&0x3432_5258u32.to_be_bytes()); // XRGB8888
    data[28..32].copy_from_slice(&0u32.to_be_bytes());
    data[32..36].copy_from_slice(&width.to_be_bytes());
    data[36..40].copy_from_slice(&height.to_be_bytes());
    data[40..44].copy_from_slice(&(width * 4).to_be_bytes());
    compiler_fence(Ordering::SeqCst);
    // QEMU's address register is big endian; writing its low half triggers DMA.
    unsafe {
        asm!("out dx, eax", in("dx") 0x514u16, in("eax") ((base >> 32) as u32).to_be(),
            options(nostack, preserves_flags));
        asm!("out dx, eax", in("dx") 0x518u16, in("eax") (base as u32).to_be(),
            options(nostack, preserves_flags));
    }
    let control = base as *const u32;
    let mut done = false;
    for _ in 0..1_000_000 {
        let status = u32::from_be(unsafe { control.read_volatile() });
        assert_eq!(status & 1, 0, "RAMFB DMA rejected");
        if status == 0 {
            done = true;
            break;
        }
        core::hint::spin_loop();
    }
    assert!(done, "RAMFB DMA timeout");
    compiler_fence(Ordering::SeqCst);
    Some(Framebuffer {
        physical: backing.base(),
        virtual_start: backing.base(),
        bytes: bytes as u64,
        width,
        height,
        stride: width,
        format: 1,
    })
}
