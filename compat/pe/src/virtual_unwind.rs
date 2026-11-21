//! Transactional version-one x64 virtual stack unwinding. This computes caller
//! state; it neither invokes language handlers nor unwinds native Rust frames.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Bounds,
    Memory,
    Malformed,
    Unsupported,
}
pub trait Memory {
    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), Error>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Context {
    pub registers: [u64; 16],
    pub rip: u64,
    pub xmm: [[u8; 16]; 16],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Function {
    pub begin: u32,
    pub end: u32,
    pub unwind: u32,
}

fn nonvolatile(register: usize) -> bool {
    matches!(register, 3 | 5 | 6 | 7 | 12..=15)
}
fn address(base: u64, offset: u64, bytes: usize) -> Result<u64, Error> {
    let value = base.checked_add(offset).ok_or(Error::Bounds)?;
    if value < 0x10000
        || value
            .checked_add(bytes as u64)
            .is_none_or(|end| end > 0x8000_0000_0000)
    {
        return Err(Error::Bounds);
    }
    Ok(value)
}
fn word(memory: &mut impl Memory, pointer: u64) -> Result<u64, Error> {
    let mut bytes = [0; 8];
    memory.read(address(pointer, 0, 8)?, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn slots(op: u8, info: usize) -> Result<usize, Error> {
    match op {
        0 if nonvolatile(info) => Ok(1),
        1 if info == 0 => Ok(2),
        1 if info == 1 => Ok(3),
        2 => Ok(1),
        3 if info == 0 => Ok(1),
        4 if nonvolatile(info) => Ok(2),
        5 if nonvolatile(info) => Ok(3),
        8 if (6..16).contains(&info) => Ok(2),
        9 if (6..16).contains(&info) => Ok(3),
        10 => Err(Error::Unsupported),
        _ => Err(Error::Malformed),
    }
}
fn short(bytes: &[u8], offset: usize) -> u64 {
    u64::from(u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("checked unwind operand"),
    ))
}
fn long(bytes: &[u8], offset: usize) -> u64 {
    u64::from(u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked unwind operand"),
    ))
}

/// Supports validated v1 body/prolog frames and terminal RET epilogues. Other
/// epilogue forms, machine frames, chained data and language handlers fail closed.
/// A failed read/metadata operation leaves the caller's context unchanged.
pub fn unwind(
    memory: &mut impl Memory,
    base: u64,
    image_bytes: u32,
    control: u64,
    function: Function,
    context: &mut Context,
) -> Result<u64, Error> {
    if function.begin >= function.end
        || function.end > image_bytes
        || !function.unwind.is_multiple_of(4)
        || function
            .unwind
            .checked_add(4)
            .is_none_or(|end| end > image_bytes)
    {
        return Err(Error::Bounds);
    }
    let start = address(base, u64::from(function.begin), 1)?;
    let end = address(base, u64::from(function.end), 0)?;
    if !(start..end).contains(&control) {
        return Err(Error::Bounds);
    }
    let pc_offset = control - start;
    let mut header = [0; 4];
    memory.read(address(base, u64::from(function.unwind), 4)?, &mut header)?;
    if header[0] != 1 {
        return Err(Error::Unsupported);
    }
    let count = usize::from(header[2]);
    let code_bytes = ((count + 1) & !1) * 2;
    if u64::from(function.unwind) + 4 + code_bytes as u64 > u64::from(image_bytes)
        || u32::from(header[1]) > function.end - function.begin
    {
        return Err(Error::Bounds);
    }
    let mut codes = [0; 512];
    memory.read(
        address(base, u64::from(function.unwind) + 4, code_bytes)?,
        &mut codes[..code_bytes],
    )?;
    let frame = usize::from(header[3] & 15);
    if (frame == 0 && header[3] != 0) || (frame != 0 && !nonvolatile(frame)) {
        return Err(Error::Malformed);
    }
    let mut index = 0;
    let mut previous = header[1];
    let mut frame_offset = None;
    while index < count {
        let offset = codes[index * 2];
        let op = codes[index * 2 + 1] & 15;
        let info = usize::from(codes[index * 2 + 1] >> 4);
        let length = slots(op, info)?;
        if offset > previous || index + length > count {
            return Err(Error::Malformed);
        }
        if op == 3 && (frame_offset.replace(offset).is_some() || frame == 0) {
            return Err(Error::Malformed);
        }
        previous = offset;
        index += length;
    }
    if (frame != 0) != frame_offset.is_some() {
        return Err(Error::Malformed);
    }
    let mut next = *context;
    let mut instruction = [0; 3];
    let instruction_bytes = (end - control).min(3) as usize;
    memory.read(control, &mut instruction[..instruction_bytes])?;
    // A terminal RET has already restored the stack/nonvolatile registers.
    if instruction[0] == 0xc3 {
        let establisher = next.registers[4];
        next.rip = word(memory, next.registers[4])?;
        next.registers[4] = address(next.registers[4], 8, 0)?;
        *context = next;
        return Ok(establisher);
    }
    if pc_offset >= u64::from(header[1])
        && (matches!(instruction[0], 0xc2 | 0xe9 | 0xeb | 0x58..=0x5f)
            || matches!(instruction[0], 0x26 | 0x2e | 0x36 | 0x3e | 0x64..=0x67 | 0xf0..=0xf3)
            || (instruction[0] == 0xff && matches!((instruction[1] >> 3) & 7, 4 | 5))
            || (matches!(instruction[0], 0xf2 | 0xf3 | 0x40..=0x4f)
                && (matches!(
                    instruction[1],
                    0x26 | 0x2e | 0x36 | 0x3e | 0x40..=0x4f | 0x64..=0x67 | 0xf0..=0xf3 | 0xc2 | 0xc3 | 0x81 | 0x83 | 0x8d | 0x58..=0x5f
                ) || (instruction[1] == 0xff && matches!((instruction[2] >> 3) & 7, 4 | 5)))))
    {
        return Err(Error::Unsupported);
    }
    let established = frame_offset.is_some_and(|offset| pc_offset >= u64::from(offset));
    let establisher = if established {
        next.registers[frame]
            .checked_sub(u64::from(header[3] >> 4) * 16)
            .ok_or(Error::Bounds)?
    } else {
        next.registers[4]
    };
    address(establisher, 0, 0)?;
    index = 0;
    while index < count {
        let offset = codes[index * 2];
        let op = codes[index * 2 + 1] & 15;
        let info = usize::from(codes[index * 2 + 1] >> 4);
        let length = slots(op, info)?;
        if pc_offset >= u64::from(offset) {
            match op {
                0 => {
                    next.registers[info] = word(memory, next.registers[4])?;
                    next.registers[4] = address(next.registers[4], 8, 0)?;
                }
                1 => {
                    let allocation = if info == 0 {
                        short(&codes, (index + 1) * 2) * 8
                    } else {
                        long(&codes, (index + 1) * 2)
                    };
                    if allocation == 0 || !allocation.is_multiple_of(8) {
                        return Err(Error::Malformed);
                    }
                    next.registers[4] = address(next.registers[4], allocation, 0)?;
                }
                2 => next.registers[4] = address(next.registers[4], info as u64 * 8 + 8, 0)?,
                3 => next.registers[4] = establisher,
                4 | 5 => {
                    let offset = if op == 4 {
                        short(&codes, (index + 1) * 2) * 8
                    } else {
                        long(&codes, (index + 1) * 2)
                    };
                    if !offset.is_multiple_of(8) {
                        return Err(Error::Malformed);
                    }
                    next.registers[info] = word(memory, address(establisher, offset, 8)?)?;
                }
                8 | 9 => {
                    let offset = if op == 8 {
                        short(&codes, (index + 1) * 2) * 16
                    } else {
                        long(&codes, (index + 1) * 2)
                    };
                    if !offset.is_multiple_of(16) {
                        return Err(Error::Malformed);
                    }
                    memory.read(address(establisher, offset, 16)?, &mut next.xmm[info])?;
                }
                _ => return Err(Error::Unsupported),
            }
        }
        index += length;
    }
    next.rip = word(memory, next.registers[4])?;
    next.registers[4] = address(next.registers[4], 8, 0)?;
    *context = next;
    Ok(establisher)
}
