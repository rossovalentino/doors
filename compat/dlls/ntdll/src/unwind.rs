//! Public x64 caller recovery over bounded mapped PE metadata. All application
//! memory crosses USER_COPY; no foreign unwind enters native Rust/kernel frames.
use door_pe_loader::virtual_unwind::{self as engine, Context, Function, Memory};

struct Captured;
impl Memory for Captured {
    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), engine::Error> {
        checked_copy(output.as_mut_ptr() as u64, address, output.len())
    }
}
fn read<const N: usize>(address: u64) -> Result<[u8; N], engine::Error> {
    let mut bytes = [0; N];
    Captured.read(address, &mut bytes)?;
    Ok(bytes)
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed metadata extent"),
    )
}
fn put(pointer: u64, bytes: &[u8]) -> Result<(), engine::Error> {
    checked_copy(pointer, bytes.as_ptr() as u64, bytes.len())
}
fn checked_copy(destination: u64, source: u64, bytes: usize) -> Result<(), engine::Error> {
    if bytes > 4096 {
        return Err(engine::Error::Bounds);
    }
    // The kernel validates and captures both complete ranges under the VM lease.
    unsafe { door_sys::user_copy(destination, source, bytes) }.map_err(|_| engine::Error::Memory)
}
#[derive(Clone, Copy)]
struct Image {
    base: u64,
    size: u32,
    table: u32,
    entries: u32,
}
impl Image {
    fn parse(base: u64) -> Result<Self, engine::Error> {
        let dos = read::<64>(base)?;
        let nt = u32_at(&dos, 60);
        if &dos[..2] != b"MZ" || !(64..=4096).contains(&nt) {
            return Err(engine::Error::Malformed);
        }
        let pe = read::<264>(
            base.checked_add(u64::from(nt))
                .ok_or(engine::Error::Bounds)?,
        )?;
        if &pe[..4] != b"PE\0\0"
            || pe[4..6] != [0x64, 0x86]
            || pe[24..26] != [0x0b, 2]
            || u32_at(&pe, 132) < 4
        {
            return Err(engine::Error::Malformed);
        }
        let size = u32_at(&pe, 80);
        let table = u32_at(&pe, 160);
        let bytes = u32_at(&pe, 164);
        if size == 0
            || size > 16 * 1024 * 1024
            || !bytes.is_multiple_of(12)
            || bytes / 12 > 4096
            || table.checked_add(bytes).is_none_or(|end| end > size)
        {
            return Err(engine::Error::Bounds);
        }
        Ok(Self {
            base,
            size,
            table,
            entries: bytes / 12,
        })
    }
    fn entry(self, index: u32) -> Result<(u64, Function), engine::Error> {
        if index >= self.entries {
            return Err(engine::Error::Bounds);
        }
        let pointer = self.base + u64::from(self.table) + u64::from(index) * 12;
        let bytes = read::<12>(pointer)?;
        let value = Function {
            begin: u32_at(&bytes, 0),
            end: u32_at(&bytes, 4),
            unwind: u32_at(&bytes, 8),
        };
        if value.begin >= value.end || value.end > self.size || value.unwind >= self.size {
            return Err(engine::Error::Malformed);
        }
        Ok((pointer, value))
    }
    fn lookup(self, pc: u64) -> Result<Option<(u64, Function)>, engine::Error> {
        let mut previous_end = 0;
        let mut found = None;
        for index in 0..self.entries {
            let (pointer, function) = self.entry(index)?;
            if function.begin < previous_end {
                return Err(engine::Error::Malformed);
            }
            previous_end = function.end;
            if (self.base + u64::from(function.begin)..self.base + u64::from(function.end))
                .contains(&pc)
            {
                found = Some((pointer, function));
            }
        }
        Ok(found)
    }
}
fn image(pc: u64) -> Result<Option<Image>, engine::Error> {
    // The module inventory the bootstrap published from DOORWIN1/DOORWIN2.
    let table = super::modules::inventory().ok_or(engine::Error::Malformed)?;
    for base in table.images[..table.count]
        .iter()
        .chain(super::modules::loaded())
        .map(|m| m.base)
    {
        if base == 0x184000000 && !super::loader::dynamic_present() {
            continue;
        }
        let image = Image::parse(base)?;
        if (base..base
            .checked_add(u64::from(image.size))
            .ok_or(engine::Error::Bounds)?)
            .contains(&pc)
        {
            return Ok(Some(image));
        }
    }
    Ok(None)
}
fn invalid() -> ! {
    // An invalid function table must not masquerade as a successful leaf unwind.
    door_sys::exit(0xc00000ff)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorLookupFunctionEntry(pc: u64, base: u64, history: u64) -> u64 {
    RtlLookupFunctionEntry(pc, base, history)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualUnwind(
    kind: u32,
    base: u64,
    pc: u64,
    entry: u64,
    context: u64,
    data: u64,
    frame: u64,
    pointers: u64,
) -> u64 {
    RtlVirtualUnwind(kind, base, pc, entry, context, data, frame, pointers)
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlLookupFunctionEntry(pc: u64, base: u64, _history: u64) -> u64 {
    let result = (|| {
        let Some(image) = image(pc)? else {
            return Ok(0);
        };
        let Some((pointer, _)) = image.lookup(pc)? else {
            return Ok(0);
        };
        put(base, &image.base.to_le_bytes())?;
        Ok::<_, engine::Error>(pointer)
    })();
    result.unwrap_or_else(|_| invalid())
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlVirtualUnwind(
    kind: u32,
    base: u64,
    pc: u64,
    entry: u64,
    context: u64,
    handler_data: u64,
    establisher: u64,
    pointers: u64,
) -> u64 {
    let result = (|| {
        if kind != 0 || pointers != 0 {
            return Err(engine::Error::Unsupported);
        }
        let image = image(pc)?.ok_or(engine::Error::Bounds)?;
        let (expected, function) = image.lookup(pc)?.ok_or(engine::Error::Bounds)?;
        if image.base != base || expected != entry {
            return Err(engine::Error::Malformed);
        }
        let mut bytes = read::<1232>(context)?;
        if u32_at(&bytes, 48) & 0x100003 != 0x100003 {
            return Err(engine::Error::Unsupported);
        }
        let mut state = Context {
            registers: core::array::from_fn(|i| {
                u64::from_le_bytes(
                    bytes[120 + i * 8..128 + i * 8]
                        .try_into()
                        .expect("context register"),
                )
            }),
            rip: u64::from_le_bytes(bytes[248..256].try_into().expect("context rip")),
            xmm: core::array::from_fn(|i| {
                bytes[416 + i * 16..432 + i * 16]
                    .try_into()
                    .expect("context xmm")
            }),
        };
        let frame = engine::unwind(&mut Captured, base, image.size, pc, function, &mut state)?;
        for (i, value) in state.registers.iter().enumerate() {
            bytes[120 + i * 8..128 + i * 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[248..256].copy_from_slice(&state.rip.to_le_bytes());
        for (i, value) in state.xmm.iter().enumerate() {
            bytes[416 + i * 16..432 + i * 16].copy_from_slice(value);
        }
        put(context, &bytes)?;
        put(handler_data, &0u64.to_le_bytes())?;
        put(establisher, &frame.to_le_bytes())?;
        Ok::<_, engine::Error>(0)
    })();
    result.unwrap_or_else(|_| invalid())
}
