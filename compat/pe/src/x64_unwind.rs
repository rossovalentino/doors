//! x64 virtual unwinding with Windows' semantics (P18 M0c): what
//! RtlVirtualUnwind does for one frame. Unwind info versions 1 and 2,
//! chained unwind info, every unwind code (machine frames included), the
//! epilogue rule (an instruction pointer inside `add/lea rsp; pop...; ret`
//! or a tail-call `jmp` is unwound by simulating the remaining epilogue),
//! language handlers (exception and termination handlers with their data),
//! and the addresses nonvolatile registers were restored from
//! (KNONVOLATILE_CONTEXT_POINTERS). Every read goes through `Memory`;
//! a failed read leaves the caller's context unchanged.
pub use crate::virtual_unwind::{Context, Error, Function, Memory};

/// RtlVirtualUnwind's HandlerType flags.
pub const UNW_FLAG_NHANDLER: u32 = 0;
pub const UNW_FLAG_EHANDLER: u32 = 1;
pub const UNW_FLAG_UHANDLER: u32 = 2;
pub const UNW_FLAG_CHAININFO: u8 = 4;

const PUSH_NONVOL: u8 = 0;
const ALLOC_LARGE: u8 = 1;
const ALLOC_SMALL: u8 = 2;
const SET_FPREG: u8 = 3;
const SAVE_NONVOL: u8 = 4;
const SAVE_NONVOL_FAR: u8 = 5;
const EPILOG: u8 = 6;
const SPARE: u8 = 7;
const SAVE_XMM128: u8 = 8;
const SAVE_XMM128_FAR: u8 = 9;
const PUSH_MACHFRAME: u8 = 10;
/// Bound on chained unwind records followed for one frame.
const MAX_CHAIN: usize = 32;
/// Bound on instructions simulated in one epilogue.
const MAX_EPILOG: usize = 64;

/// Where each restored nonvolatile register was read from (0: not
/// restored from memory by this unwind).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pointers {
    pub integer: [u64; 16],
    pub float: [u64; 16],
}
/// One unwound frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame {
    /// The establisher frame (the frame pointer the language handler gets).
    pub establisher: u64,
    /// The language handler and the address of its data, when the caller
    /// asked for that kind of handler and the frame has one.
    pub handler: Option<(u64, u64)>,
}

fn user(address: u64, bytes: u64) -> Result<u64, Error> {
    if address < 0x10000
        || address
            .checked_add(bytes)
            .is_none_or(|end| end > 0x8000_0000_0000)
    {
        return Err(Error::Bounds);
    }
    Ok(address)
}
fn read_u64(memory: &mut impl Memory, address: u64) -> Result<u64, Error> {
    let mut bytes = [0; 8];
    memory.read(user(address, 8)?, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
fn read_u32(memory: &mut impl Memory, address: u64) -> Result<u32, Error> {
    let mut bytes = [0; 4];
    memory.read(user(address, 4)?, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}
pub(crate) fn opcode_slots(op: u8, info: u8) -> Result<usize, Error> {
    Ok(match op {
        ALLOC_LARGE if info == 0 => 2,
        ALLOC_LARGE if info == 1 => 3,
        ALLOC_LARGE => return Err(Error::Malformed),
        SAVE_NONVOL | SAVE_XMM128 | EPILOG => 2,
        SAVE_NONVOL_FAR | SAVE_XMM128_FAR => 3,
        PUSH_NONVOL | ALLOC_SMALL | SET_FPREG | PUSH_MACHFRAME | SPARE => 1,
        _ => return Err(Error::Malformed),
    })
}

/// An UNWIND_INFO header and its codes.
struct Info {
    version: u8,
    flags: u8,
    prolog: u8,
    count: usize,
    frame_register: usize,
    frame_offset: u64,
    codes: [u8; 512],
    /// Address just past the code array (the handler RVA or the chained
    /// RUNTIME_FUNCTION).
    tail: u64,
}
fn info(memory: &mut impl Memory, base: u64, rva: u32) -> Result<Info, Error> {
    let at = base.checked_add(u64::from(rva)).ok_or(Error::Bounds)?;
    let mut header = [0; 4];
    memory.read(user(at, 4)?, &mut header)?;
    let version = header[0] & 7;
    if version != 1 && version != 2 {
        return Err(Error::Unsupported);
    }
    let count = usize::from(header[2]);
    let mut codes = [0; 512];
    let code_bytes = count * 2;
    memory.read(user(at + 4, code_bytes as u64)?, &mut codes[..code_bytes])?;
    let frame_register = usize::from(header[3] & 15);
    Ok(Info {
        version,
        flags: header[0] >> 3,
        prolog: header[1],
        count,
        frame_register,
        frame_offset: u64::from(header[3] >> 4) * 16,
        codes,
        tail: at + 4 + (((count + 1) & !1) * 2) as u64,
    })
}
fn u16_at(codes: &[u8; 512], slot: usize) -> u64 {
    u64::from(u16::from_le_bytes([codes[slot * 2], codes[slot * 2 + 1]]))
}
fn u32_at(codes: &[u8; 512], slot: usize) -> u64 {
    u64::from(u32::from_le_bytes([
        codes[slot * 2],
        codes[slot * 2 + 1],
        codes[slot * 2 + 2],
        codes[slot * 2 + 3],
    ]))
}

/// Instruction bytes at `pc` (up to 16, fewer at the end of readable
/// memory).
fn code(memory: &mut impl Memory, pc: u64) -> Result<[u8; 16], Error> {
    let mut bytes = [0; 16];
    // Read what is readable: shrink at a boundary failure.
    let mut length = 16;
    while length > 0 {
        if memory
            .read(user(pc, length as u64)?, &mut bytes[..length])
            .is_ok()
        {
            return Ok(bytes);
        }
        length /= 2;
    }
    Err(Error::Memory)
}
/// Whether `pc` (in `function` at `base`) is in an epilogue: an optional
/// `add rsp,imm` or `lea rsp,[reg+disp]`, pops of nonvolatile registers,
/// then `ret`, `ret imm16`, `rep ret`, or a jump (to inside the function:
/// keep checking there; outside it: a tail call ending the epilogue).
fn in_epilogue(
    memory: &mut impl Memory,
    base: u64,
    function: Function,
    pc: u64,
) -> Result<bool, Error> {
    let inside = |target: u64| {
        target >= base + u64::from(function.begin) && target < base + u64::from(function.end)
    };
    let mut pc = pc;
    let first = code(memory, pc)?;
    if first[0] & 0xf8 == 0x48 {
        match first[1] {
            0x81 if first[0] == 0x48 && first[2] == 0xc4 => pc += 7,
            0x83 if first[0] == 0x48 && first[2] == 0xc4 => pc += 4,
            0x8d => {
                if first[0] & 0x06 != 0 || (first[2] >> 3) & 7 != 4 || first[2] & 7 == 4 {
                    return Ok(false);
                }
                match first[2] >> 6 {
                    1 => pc += 4,
                    2 => pc += 7,
                    _ => return Ok(false),
                }
            }
            _ => {}
        }
    }
    for _ in 0..MAX_EPILOG {
        let b = code(memory, pc)?;
        let mut at = 0;
        if b[0] & 0xf0 == 0x40 {
            at = 1;
        }
        match b[at] {
            0x58..=0x5f => {
                pc += at as u64 + 1;
                continue;
            }
            0xc2 | 0xc3 => return Ok(true),
            0xf3 => return Ok(b[at + 1] == 0xc3),
            0xe9 => {
                let displacement = i32::from_le_bytes([b[at + 1], b[at + 2], b[at + 3], b[at + 4]]);
                let target = (pc + at as u64 + 5).wrapping_add(displacement as i64 as u64);
                if !inside(target) {
                    return Ok(true);
                }
                pc = target;
                continue;
            }
            0xeb => {
                let target = (pc + at as u64 + 2).wrapping_add(b[at + 1] as i8 as i64 as u64);
                if !inside(target) {
                    return Ok(true);
                }
                pc = target;
                continue;
            }
            // jmp qword ptr [rip+disp32] / rex.W jmp through memory: tail calls.
            0xff if b[at + 1] == 0x25 || (at == 1 && (b[at + 1] >> 3) & 7 == 4) => return Ok(true),
            _ => return Ok(false),
        }
    }
    Ok(false)
}
/// Simulate the rest of an epilogue starting at `pc`.
fn run_epilogue(
    memory: &mut impl Memory,
    base: u64,
    function: Function,
    mut pc: u64,
    context: &mut Context,
    pointers: &mut Pointers,
) -> Result<(), Error> {
    let inside = |target: u64| {
        target >= base + u64::from(function.begin) && target < base + u64::from(function.end)
    };
    for _ in 0..MAX_EPILOG {
        let b = code(memory, pc)?;
        let mut at = 0;
        let mut rex = 0;
        if b[0] & 0xf0 == 0x40 {
            rex = b[0] & 15;
            at = 1;
        }
        match b[at] {
            0x58..=0x5f => {
                let register = usize::from(b[at] - 0x58) + usize::from(rex & 1) * 8;
                let rsp = context.registers[4];
                context.registers[register] = read_u64(memory, rsp)?;
                pointers.integer[register] = rsp;
                context.registers[4] = rsp + 8;
                pc += at as u64 + 1;
            }
            0x81 => {
                let value = i32::from_le_bytes([b[at + 2], b[at + 3], b[at + 4], b[at + 5]]);
                context.registers[4] = context.registers[4].wrapping_add(value as i64 as u64);
                pc += at as u64 + 6;
            }
            0x83 => {
                context.registers[4] =
                    context.registers[4].wrapping_add(b[at + 2] as i8 as i64 as u64);
                pc += at as u64 + 3;
            }
            0x8d => {
                let register = usize::from(b[at + 1] & 7) + usize::from(rex & 1) * 8;
                if b[at + 1] >> 6 == 1 {
                    context.registers[4] =
                        context.registers[register].wrapping_add(b[at + 2] as i8 as i64 as u64);
                    pc += at as u64 + 3;
                } else {
                    let value = i32::from_le_bytes([b[at + 2], b[at + 3], b[at + 4], b[at + 5]]);
                    context.registers[4] =
                        context.registers[register].wrapping_add(value as i64 as u64);
                    pc += at as u64 + 6;
                }
            }
            0xc2 => {
                let rsp = context.registers[4];
                context.rip = read_u64(memory, rsp)?;
                context.registers[4] =
                    rsp + 8 + u64::from(u16::from_le_bytes([b[at + 1], b[at + 2]]));
                return Ok(());
            }
            0xc3 | 0xf3 => {
                let rsp = context.registers[4];
                context.rip = read_u64(memory, rsp)?;
                context.registers[4] = rsp + 8;
                return Ok(());
            }
            0xe9 | 0xeb => {
                let (length, displacement) = if b[at] == 0xe9 {
                    (
                        5,
                        i32::from_le_bytes([b[at + 1], b[at + 2], b[at + 3], b[at + 4]]) as i64,
                    )
                } else {
                    (2, b[at + 1] as i8 as i64)
                };
                let target = (pc + at as u64 + length).wrapping_add(displacement as u64);
                if inside(target) {
                    pc = target;
                } else {
                    // A tail call: the return address is on top, as at a ret.
                    let rsp = context.registers[4];
                    context.rip = read_u64(memory, rsp)?;
                    context.registers[4] = rsp + 8;
                    return Ok(());
                }
            }
            0xff => {
                let rsp = context.registers[4];
                context.rip = read_u64(memory, rsp)?;
                context.registers[4] = rsp + 8;
                return Ok(());
            }
            _ => return Err(Error::Malformed),
        }
    }
    Err(Error::Malformed)
}

/// Unwind one frame of `function` (a RUNTIME_FUNCTION of the image or
/// dynamic table at `base`) at `pc`: `context` becomes the caller's,
/// `pointers` record where nonvolatile registers came from, and the
/// language handler of kind `handler_type` is returned when the frame has
/// one and `pc` is past the prolog.
pub fn virtual_unwind(
    memory: &mut impl Memory,
    handler_type: u32,
    base: u64,
    pc: u64,
    function: Function,
    context: &mut Context,
    pointers: &mut Pointers,
) -> Result<Frame, Error> {
    let mut next = *context;
    let mut recorded = *pointers;
    let mut function = function;
    let mut frame = next.registers[4];
    let mut establisher = frame;
    let mut machine_frame = false;
    let mut in_prolog = false;
    let mut last: Option<Info> = None;
    for chain in 0..MAX_CHAIN {
        if function.begin >= function.end {
            return Err(Error::Malformed);
        }
        let info = info(memory, base, function.unwind)?;
        if info.frame_register != 0 {
            frame = next.registers[info.frame_register]
                .checked_sub(info.frame_offset)
                .ok_or(Error::Bounds)?;
        }
        let start = base + u64::from(function.begin);
        let prolog_offset = if pc >= start && pc < start + u64::from(info.prolog) {
            in_prolog = chain == 0;
            Some(pc - start)
        } else {
            if chain == 0 && info.count > 0 && in_epilogue(memory, base, function, pc)? {
                run_epilogue(memory, base, function, pc, &mut next, &mut recorded)?;
                *context = next;
                *pointers = recorded;
                return Ok(Frame {
                    establisher: frame,
                    handler: None,
                });
            }
            None
        };
        let mut i = 0;
        while i < info.count {
            let offset = u64::from(info.codes[i * 2]);
            let op = info.codes[i * 2 + 1] & 15;
            let op_info = info.codes[i * 2 + 1] >> 4;
            let slots = opcode_slots(op, op_info)?;
            if i + slots > info.count {
                return Err(Error::Malformed);
            }
            if prolog_offset.is_some_and(|p| p < offset) {
                i += slots;
                continue;
            }
            let register = usize::from(op_info);
            match op {
                PUSH_NONVOL => {
                    let rsp = next.registers[4];
                    next.registers[register] = read_u64(memory, rsp)?;
                    recorded.integer[register] = rsp;
                    next.registers[4] = rsp + 8;
                }
                ALLOC_LARGE => {
                    let amount = if op_info == 0 {
                        u16_at(&info.codes, i + 1) * 8
                    } else {
                        u32_at(&info.codes, i + 1)
                    };
                    next.registers[4] =
                        next.registers[4].checked_add(amount).ok_or(Error::Bounds)?;
                }
                ALLOC_SMALL => next.registers[4] += u64::from(op_info) * 8 + 8,
                SET_FPREG => {
                    next.registers[4] = frame;
                    establisher = frame;
                }
                SAVE_NONVOL | SAVE_NONVOL_FAR => {
                    let offset = if op == SAVE_NONVOL {
                        u16_at(&info.codes, i + 1) * 8
                    } else {
                        u32_at(&info.codes, i + 1)
                    };
                    let at = frame.checked_add(offset).ok_or(Error::Bounds)?;
                    next.registers[register] = read_u64(memory, at)?;
                    recorded.integer[register] = at;
                }
                SAVE_XMM128 | SAVE_XMM128_FAR => {
                    let offset = if op == SAVE_XMM128 {
                        u16_at(&info.codes, i + 1) * 16
                    } else {
                        u32_at(&info.codes, i + 1)
                    };
                    let at = frame.checked_add(offset).ok_or(Error::Bounds)?;
                    memory.read(user(at, 16)?, &mut next.xmm[register])?;
                    recorded.float[register] = at;
                }
                PUSH_MACHFRAME => {
                    // Interrupt/trap frame: RIP at [rsp], RSP at [rsp+24]
                    // (after an error code when info is 1).
                    let mut rsp = next.registers[4];
                    if op_info != 0 {
                        rsp += 8;
                    }
                    next.rip = read_u64(memory, rsp)?;
                    next.registers[4] = read_u64(memory, rsp + 24)?;
                    machine_frame = true;
                }
                // Version 2 epilogue descriptors: nothing to do in a body.
                EPILOG if info.version == 2 => {}
                SPARE => {}
                _ => return Err(Error::Malformed),
            }
            i += slots;
        }
        let chained = info.flags & UNW_FLAG_CHAININFO != 0;
        let tail = info.tail;
        last = Some(info);
        if !chained {
            break;
        }
        if chain + 1 == MAX_CHAIN {
            return Err(Error::Malformed);
        }
        function = Function {
            begin: read_u32(memory, tail)?,
            end: read_u32(memory, tail + 4)?,
            unwind: read_u32(memory, tail + 8)?,
        };
    }
    if !machine_frame {
        let rsp = next.registers[4];
        next.rip = read_u64(memory, rsp)?;
        next.registers[4] = rsp + 8;
    }
    let last = last.ok_or(Error::Malformed)?;
    let handler = if handler_type != 0 && u32::from(last.flags) & handler_type != 0 && !in_prolog {
        let rva = read_u32(memory, last.tail)?;
        Some((base + u64::from(rva), last.tail + 4))
    } else {
        None
    };
    *context = next;
    *pointers = recorded;
    Ok(Frame {
        establisher,
        handler,
    })
}
