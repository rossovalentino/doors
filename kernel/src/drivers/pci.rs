//! PCI configuration mechanism 1 for the selected q35 profile. Configuration
//! address/data transactions are serialized with local interrupts disabled.
use super::{Error, Result};
use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

static CONFIG_LOCK: AtomicBool = AtomicBool::new(false);
/// Modern (non-transitional) VirtIO PCI identities: vendor 0x1af4, device
/// 0x1040 + the VirtIO device type.
const ID: u32 = 0x1042_1af4;
pub const NET: u32 = 0x1041_1af4;
pub const SOUND: u32 = 0x1059_1af4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Address {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}
impl Address {
    fn selector(self, offset: u8) -> u32 {
        0x8000_0000
            | (u32::from(self.bus) << 16)
            | (u32::from(self.device) << 11)
            | (u32::from(self.function) << 8)
            | u32::from(offset & !3)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bar {
    pub physical: u64,
    pub bytes: u64,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Capability {
    pub bar: usize,
    pub offset: u32,
    pub length: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Capabilities {
    pub regions: [Capability; 4],
    pub multiplier: u32,
}

/// Non-cloneable authority for a single discovered function. Discovery leaves
/// DMA and address decoding disabled until all resources have been assigned.
pub struct Function {
    id: u32,
    address: Address,
    bars: [Option<Bar>; 6],
    pub(crate) caps: Capabilities,
}
impl Function {
    pub const fn address(&self) -> Address {
        self.address
    }
    pub const fn bars(&self) -> &[Option<Bar>; 6] {
        &self.bars
    }
    pub(crate) fn command(&self, dma: bool, memory: bool) -> Result<()> {
        if read32(self.address, 0) != self.id {
            return Err(Error::Removed);
        }
        let before = read32(self.address, 4) as u16;
        let command = (before & !7) | 0x400 | if memory { 2 } else { 0 } | if dma { 4 } else { 0 };
        write16(self.address, 4, command);
        if read32(self.address, 4) as u16 & 0x407 != command & 0x407 {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}

struct ConfigGuard {
    flags: u64,
}
impl ConfigGuard {
    fn take() -> Self {
        let flags: u64;
        // SAFETY: PCI entry points require CPL0; pair restores previous IF.
        unsafe {
            asm!("pushfq", "pop {}", "cli",out(reg) flags,options(nomem));
        }
        while CONFIG_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Self { flags }
    }
}
impl Drop for ConfigGuard {
    fn drop(&mut self) {
        CONFIG_LOCK.store(false, Ordering::Release);
        if self.flags & (1 << 9) != 0 {
            unsafe {
                asm!("sti", options(nomem, nostack));
            }
        }
    }
}
fn read32(address: Address, offset: u8) -> u32 {
    let _guard = ConfigGuard::take();
    let value: u32;
    unsafe {
        asm!("out dx,eax",in("dx")0xcf8u16,in("eax")address.selector(offset),options(nomem,nostack));
        asm!("in eax,dx",in("dx")0xcfcu16,out("eax")value,options(nomem,nostack));
    }
    value
}
fn write32(address: Address, offset: u8, value: u32) {
    let _guard = ConfigGuard::take();
    unsafe {
        asm!("out dx,eax",in("dx")0xcf8u16,in("eax")address.selector(offset),options(nomem,nostack));
        asm!("out dx,eax",in("dx")0xcfcu16,in("eax")value,options(nomem,nostack));
    }
}
fn write16(address: Address, offset: u8, value: u16) {
    let _guard = ConfigGuard::take();
    unsafe {
        asm!("out dx,eax",in("dx")0xcf8u16,in("eax")address.selector(offset),options(nomem,nostack));
        asm!("out dx,ax",in("dx")0xcfcu16+u16::from(offset&2),in("ax")value,options(nomem,nostack));
    }
}
fn dword(bytes: &[u8; 256], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn parse_capabilities(config: &[u8; 256]) -> Result<Capabilities> {
    if config[6] & 0x10 == 0 || config[14] & 0x7f != 0 {
        return Err(Error::Configuration);
    }
    let mut at = config[0x34] as usize;
    let mut visited = [false; 64];
    let mut regions = [None; 4];
    let mut multiplier = 0;
    for _ in 0..48 {
        if at == 0 {
            break;
        }
        if !(0x40..=0xfc).contains(&at) || at & 3 != 0 || visited[at / 4] {
            return Err(Error::Configuration);
        }
        visited[at / 4] = true;
        if config[at] == 9 {
            let length = config[at + 2] as usize;
            if length < 16 || at + length > 256 {
                return Err(Error::Configuration);
            }
            let kind = config[at + 3];
            let bar = config[at + 4] as usize;
            if (1..=4).contains(&kind) && bar < 6 && regions[kind as usize - 1].is_none() {
                // First supported memory BAR capability, as required by 4.1.4.1.
                if dword(config, 0x10 + bar * 4) & 1 == 0 {
                    if kind == 2 && length < 20 {
                        return Err(Error::Configuration);
                    }
                    let offset = dword(config, at + 8);
                    let span = dword(config, at + 12);
                    let (minimum, alignment) = match kind {
                        1 => (56, 4),
                        2 => (2, 2),
                        3 => (1, 1),
                        _ => (8, 4),
                    };
                    if span < minimum
                        || !offset.is_multiple_of(alignment)
                        || offset.checked_add(span).is_none()
                    {
                        return Err(Error::Configuration);
                    }
                    regions[kind as usize - 1] = Some(Capability {
                        bar,
                        offset,
                        length: span,
                    });
                    if kind == 2 {
                        multiplier = dword(config, at + 16);
                        if multiplier & 1 != 0 {
                            return Err(Error::Configuration);
                        }
                    }
                }
            }
        }
        at = config[at + 1] as usize;
    }
    if at != 0 {
        return Err(Error::Configuration);
    }
    let regions = [
        regions[0].ok_or(Error::Unsupported)?,
        regions[1].ok_or(Error::Unsupported)?,
        regions[2].ok_or(Error::Unsupported)?,
        regions[3].ok_or(Error::Unsupported)?,
    ];
    for (i, a) in regions.iter().enumerate() {
        for b in &regions[..i] {
            if a.bar == b.bar
                && u64::from(a.offset) < u64::from(b.offset) + u64::from(b.length)
                && u64::from(b.offset) < u64::from(a.offset) + u64::from(a.length)
            {
                return Err(Error::Configuration);
            }
        }
    }
    Ok(Capabilities {
        regions,
        multiplier,
    })
}

/// Discover the one modern block function, validate its capabilities, and size
/// only referenced memory BARs. The firmware-assigned address is never changed.
///
/// # Safety
/// Caller runs at CPL0 on q35 with PCI configuration mechanism 1, exclusively
/// owns unbound block functions, and does not race PCI enumeration/removal.
pub unsafe fn discover_block() -> Result<Function> {
    unsafe { discover(ID) }
}
/// Discover the one modern VirtIO function with `id` (`NET`, `SOUND`, ...)
/// exactly as `discover_block` does the data disk.
///
/// # Safety
/// As `discover_block`.
pub unsafe fn discover(id: u32) -> Result<Function> {
    let mut selected = None;
    for bus in 0..=255 {
        for device in 0..32 {
            let base = Address {
                bus,
                device,
                function: 0,
            };
            if read32(base, 0) as u16 == 0xffff {
                continue;
            }
            let count = if read32(base, 12) & 0x0080_0000 != 0 {
                8
            } else {
                1
            };
            for function in 0..count {
                let address = Address {
                    bus,
                    device,
                    function,
                };
                if read32(address, 0) == id && selected.replace(address).is_some() {
                    return Err(Error::Ambiguous);
                }
            }
        }
    }
    let address = selected.ok_or(Error::NotFound)?;
    let mut config = [0u8; 256];
    for (index, chunk) in config.chunks_exact_mut(4).enumerate() {
        chunk.copy_from_slice(&read32(address, (index * 4) as u8).to_le_bytes());
    }
    let caps = parse_capabilities(&config)?;
    let mut device = Function {
        id,
        address,
        bars: [None; 6],
        caps,
    };
    device.command(false, false)?;
    // Disable MSI and MSI-X as well as INTx: this version polls completion.
    let mut at = config[0x34] as usize;
    while at != 0 {
        let control = u16::from_le_bytes([config[at + 2], config[at + 3]]);
        if config[at] == 0x11 {
            write16(address, (at + 2) as u8, (control & !0x8000) | 0x4000);
        }
        if config[at] == 5 {
            write16(address, (at + 2) as u8, control & !1);
        }
        at = config[at + 1] as usize;
    }
    let mut upper = [false; 6];
    for index in 0..6 {
        if upper[index] {
            continue;
        }
        let low = dword(&config, 0x10 + index * 4);
        if low & 1 == 0 && low & 6 == 4 {
            if index == 5 {
                return Err(Error::Configuration);
            }
            upper[index + 1] = true;
        }
        if !caps.regions.iter().any(|cap| cap.bar == index) {
            continue;
        }
        if low & 1 != 0 || low & 6 != 0 && low & 6 != 4 {
            return Err(Error::Unsupported);
        }
        let wide = low & 6 == 4;
        let high = if wide {
            dword(&config, 0x14 + index * 4)
        } else {
            0
        };
        let offset = (0x10 + index * 4) as u8;
        write32(address, offset, u32::MAX);
        if wide {
            write32(address, offset + 4, u32::MAX);
        }
        let mask_low = read32(address, offset) & !15;
        let mask_high = if wide {
            read32(address, offset + 4)
        } else {
            u32::MAX
        };
        write32(address, offset, low);
        if wide {
            write32(address, offset + 4, high);
        }
        device.bars[index] = Some(decode_bar(low, high, mask_low, mask_high)?);
    }
    for cap in caps.regions {
        let bar = device.bars[cap.bar].ok_or(Error::Configuration)?;
        if u64::from(cap.offset) + u64::from(cap.length) > bar.bytes {
            return Err(Error::Configuration);
        }
    }
    for (i, a) in device.bars.iter().enumerate() {
        if let Some(a) = a {
            for b in device.bars[..i].iter().flatten() {
                if a.physical < b.physical + b.bytes && b.physical < a.physical + a.bytes {
                    return Err(Error::Configuration);
                }
            }
        }
    }
    Ok(device)
}

fn decode_bar(low: u32, high: u32, mask_low: u32, mask_high: u32) -> Result<Bar> {
    let mask = (u64::from(mask_high) << 32) | u64::from(mask_low & !15);
    let bytes = (!mask).checked_add(1).ok_or(Error::Configuration)?;
    let physical = (u64::from(high) << 32) | u64::from(low & !15);
    if !bytes.is_power_of_two()
        || !(4096..=0x0100_0000).contains(&bytes)
        || physical == 0
        || !physical.is_multiple_of(bytes)
        || physical.checked_add(bytes).is_none_or(|end| end > 1 << 52)
    {
        return Err(Error::Configuration);
    }
    Ok(Bar { physical, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> [u8; 256] {
        let mut c = [0; 256];
        c[6] = 0x10;
        c[0x34] = 0x40;
        for index in 0..4 {
            let at = 0x40 + index * 24;
            c[at] = 9;
            c[at + 1] = if index == 3 { 0 } else { (at + 24) as u8 };
            c[at + 2] = 20;
            c[at + 3] = index as u8 + 1;
            c[at + 4] = 4;
            c[at + 8..at + 12].copy_from_slice(&(index as u32 * 4096).to_le_bytes());
            c[at + 12..at + 16].copy_from_slice(&4096u32.to_le_bytes());
        }
        c
    }
    #[test]
    fn capability_list_rejects_cycle_bounds_and_overlap() {
        assert!(parse_capabilities(&config()).is_ok());
        let mut c = config();
        c[0x41] = 0x40;
        assert_eq!(parse_capabilities(&c), Err(Error::Configuration));
        let mut c = config();
        c[0x42] = 255;
        assert_eq!(parse_capabilities(&c), Err(Error::Configuration));
        let mut c = config();
        c[0x60..0x64].fill(0);
        assert_eq!(parse_capabilities(&c), Err(Error::Configuration));
    }
    #[test]
    fn pci_selector_encodes_bdf_and_dword_offset() {
        assert_eq!(
            Address {
                bus: 2,
                device: 5,
                function: 3
            }
            .selector(0x17),
            0x8002_2b14
        );
    }
    #[test]
    fn memory_bar_masks_reject_unassigned_unaligned_and_huge_apertures() {
        assert_eq!(
            decode_bar(0xf0000000, 0, 0xffffc000, u32::MAX),
            Ok(Bar {
                physical: 0xf0000000,
                bytes: 16384
            })
        );
        assert_eq!(
            decode_bar(0, 0, 0xffffc000, u32::MAX),
            Err(Error::Configuration)
        );
        assert_eq!(
            decode_bar(0xf0001000, 0, 0xffffc000, u32::MAX),
            Err(Error::Configuration)
        );
        assert_eq!(
            decode_bar(0xf0000000, 0, 0xffffa000, u32::MAX),
            Err(Error::Configuration)
        );
        assert_eq!(decode_bar(0, 1, 0, 0xffff_ffff), Err(Error::Configuration));
        assert_eq!(
            decode_bar(0, 0x00100000, 0xffffc000, u32::MAX),
            Err(Error::Configuration)
        );
        assert_eq!(
            decode_bar(0, 1, 0xffffc000, u32::MAX),
            Ok(Bar {
                physical: 1 << 32,
                bytes: 16384
            })
        );
    }
    #[test]
    fn capability_extensions_reserved_types_and_bar_numbers_are_ignored() {
        let mut c = config();
        c[0x34] = 0xd0;
        c[0xd0] = 9;
        c[0xd1] = 0x40;
        c[0xd2] = 24;
        c[0xd3] = 99;
        assert_eq!(parse_capabilities(&c), parse_capabilities(&config()));
        c[0xd3] = 1;
        c[0xd4] = 6;
        assert_eq!(parse_capabilities(&c), parse_capabilities(&config()));
    }
}
