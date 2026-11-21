//! Fault delivery and continuation. Privileged state and original FPU state stay
//! kernel-owned; a Windows CONTEXT never contains native pointers/CR3 authority.
use super::super::*;
use door_windows_abi::exception as abi;

/// Set in a page fault's error code (above the hardware bits) when the
/// fault consumed a PAGE_GUARD page: delivered as
/// STATUS_GUARD_PAGE_VIOLATION instead of an access violation.
pub(in crate::process) const GUARD_FAULT: u64 = 1 << 63;
/// Set when a thread's stack ran out (its guard reached the reservation's
/// last page): delivered as STATUS_STACK_OVERFLOW (P18 M0h).
pub(in crate::process) const STACK_OVERFLOW_FAULT: u64 = 1 << 62;
const STACK_OVERFLOW: u32 = 0xC000_00FD;

pub(in crate::process) struct Pending {
    address: u64,
    fx: [u8; 512],
}

/// The Windows exception a fault other than a page fault is: its code, and
/// how far the reported address (and RIP) goes back. An int3 trap leaves RIP
/// after the instruction; Windows reports the breakpoint itself. x87 (#MF)
/// and SIMD (#XM) faults name the first unmasked exception their status
/// word holds; a general-protection fault is an access violation (reading
/// address -1), or a privileged instruction for those a program may not
/// run (HLT, CLI, STI, IN, OUT), or a fail-fast (`int 0x29`).
fn translate(r: Record, vector: u64, rip: u64, fx: &[u8; 512]) -> Option<(u32, u64)> {
    const BREAKPOINT: u32 = 0x8000_0003;
    const PRIVILEGED: u32 = 0xc000_0096;
    const FAST_FAIL: u32 = 0xc000_0409;
    // STATUS_FLOAT_: invalid, denormal, divide by zero, overflow,
    // underflow, inexact, stack check.
    let float = |flags: u32, masks: u32, stack: bool| -> u32 {
        let pending = flags & !masks & 0x3f;
        match pending.trailing_zeros() {
            0 if stack => 0xc000_0092,
            0 => 0xc000_0090,
            1 => 0xc000_008d,
            2 => 0xc000_008e,
            3 => 0xc000_0091,
            4 => 0xc000_0093,
            5 => 0xc000_008f,
            _ => 0xc000_0090,
        }
    };
    match vector {
        3 => Some((BREAKPOINT, 1)),
        13 => {
            let mut opcode = [0u8; 2];
            let read =
                crate::process::with_vm(|v| crate::process::read(v, r, rip, &mut opcode)).is_ok();
            // `int 0x29` is __fastfail: STATUS_STACK_BUFFER_OVERRUN, as
            // Windows raises it (the gate is not open to user mode).
            if read && opcode == [0xcd, 0x29] {
                return Some((FAST_FAIL, 0));
            }
            let privileged =
                read && matches!(opcode[0], 0xf4 | 0xfa | 0xfb | 0xe4..=0xe7 | 0xec..=0xef);
            Some((if privileged { PRIVILEGED } else { 0 }, 0))
        }
        16 => {
            let status = u32::from(u16::from_le_bytes([fx[2], fx[3]]));
            let control = u32::from(u16::from_le_bytes([fx[0], fx[1]]));
            Some((float(status, control, status & 0x40 != 0), 0))
        }
        19 => {
            let mxcsr = u32::from_le_bytes([fx[24], fx[25], fx[26], fx[27]]);
            Some((float(mxcsr, mxcsr >> 7, false), 0))
        }
        _ => None,
    }
}
pub(in crate::process) unsafe fn deliver(
    context: *mut arch::Context,
    r: Record,
    vector: u64,
    error: u64,
    address: u64,
) -> Result<(), Status> {
    if !matches!(r.role, 107 | 110 | 115) {
        return Err(Status::AccessDenied);
    }
    // A fault while an earlier one is still pending replaces it: the
    // program resumed that context in user mode (a caught exception, RtlRestoreContext)
    // or faulted inside its handler (a nested exception), as Windows allows (P18).
    let dispatch = with_meta(|m| {
        m.threads[r.task.slot()].as_ref().ok_or(Status::Canceled)?;
        Ok(m.processes[r.pid.slot()]
            .as_ref()
            .ok_or(Status::Canceled)?
            .exception_entry)
    })?;
    if dispatch == 0 {
        return Err(Status::Unsupported);
    }
    let frame = unsafe { &mut *((*context).frame as *mut arch::TrapFrame) };
    // A fault in 32-bit code goes to the x86 dispatcher (`deliver32`).
    if frame.cs == u64::from(arch::USER_CODE32_SELECTOR) {
        return unsafe { deliver32(context, r, frame, dispatch, vector, error, address) };
    }
    let mut packet = [0u8; abi::PACKET_BYTES];
    let (code, parameters, count) = match vector {
        14 => (
            if error & STACK_OVERFLOW_FAULT != 0 {
                STACK_OVERFLOW
            } else if error & GUARD_FAULT != 0 {
                abi::GUARD_PAGE_VIOLATION
            } else {
                abi::ACCESS_VIOLATION
            },
            [
                if error & 16 != 0 {
                    8
                } else {
                    u64::from(error & 2 != 0)
                },
                address,
            ],
            2,
        ),
        6 => (0xc000_001d, [0, 0], 0),
        0 => (0xc000_0094, [0, 0], 0),
        _ => match translate(r, vector, frame.rip, unsafe { &(*context).fx }) {
            // A general-protection fault reads address -1.
            Some((0, _)) => (abi::ACCESS_VIOLATION, [0, u64::MAX], 2),
            Some((code, back)) => {
                frame.rip -= back;
                (code, [0, 0], 0)
            }
            None => return Err(Status::Unsupported),
        },
    };
    let registers = [
        frame.rax, frame.rcx, frame.rdx, frame.rbx, frame.rsp, frame.rbp, frame.rsi, frame.rdi,
        frame.r8, frame.r9, frame.r10, frame.r11, frame.r12, frame.r13, frame.r14, frame.r15,
        frame.rip,
    ];
    let integer = abi::IntegerContext {
        registers,
        flags: frame.rflags as u32,
    };
    if !abi::record(
        &mut packet[..abi::RECORD_BYTES],
        code,
        frame.rip,
        &parameters[..count],
    ) || !integer.encode(&mut packet[abi::CONTEXT_OFFSET..])
        || !abi::encode_float(
            unsafe { &(*context).fx },
            &mut packet[abi::CONTEXT_OFFSET..],
        )
    {
        return Err(Status::InvalidArgument);
    }
    let base = frame
        .rsp
        .checked_sub(abi::PACKET_BYTES as u64 + 64)
        .ok_or(Status::BadAddress)?
        & !15;
    let stack = base.checked_sub(40).ok_or(Status::BadAddress)?;
    let teb = unsafe { (*context).user_gs_base };
    with_vm(|vm| {
        vm.validate(r.pid.slot(), dispatch, 1, UserAccess::Execute)?;
        // A thread's growing stack reaches down far enough for the record
        // first, as the guard pages on the way would grow it.
        if let Ok(Some(limit)) = vm.windows_stack_reach(r.pid.slot(), stack, frame.rsp)
            && teb != 0
        {
            let _ = write(vm, r, teb + 16, &limit.to_le_bytes());
        }
        vm.validate(r.pid.slot(), stack, 40 + packet.len(), UserAccess::Write)?;
        write(vm, r, base, &packet)
    })?;
    with_meta(|m| {
        m.threads[r.task.slot()].as_mut().unwrap().exception = Some(Pending {
            address: base + abi::CONTEXT_OFFSET as u64,
            fx: unsafe { (*context).fx },
        })
    });
    frame.rip = dispatch;
    frame.rsp = stack;
    frame.rcx = base;
    frame.rdx = base + abi::CONTEXT_OFFSET as u64;
    frame.rflags = 0x202;
    log(format_args!(
        "DOOR_P10 EXCEPTION pid={} vector={vector} code={code:#x} address={address:#x}\n",
        r.pid.raw()
    ));
    // Where it happened (P18 diagnostics; the P10 line above stays exact).
    log(format_args!(
        "DOOR_P18 FAULT_AT pid={} rip={:#x} rsp={:#x}\n",
        r.pid.raw(),
        registers[16],
        registers[4]
    ));
    if code == STACK_OVERFLOW {
        stack_trail(r, registers[4]);
    }
    Ok(())
}
/// A stack overflow's crude backtrace: the first values above the stack
/// pointer that point into the system DLLs (0x1_8000_0000..0x1_b000_0000),
/// the executable (0x1_4000_0000..0x1_8000_0000) or a DLL ntdll's loader
/// mapped high (0x7f00_0000_0000..), which shows the recursion's cycle.
fn stack_trail(r: Record, rsp: u64) {
    let mut found = [0u64; 24];
    let mut count = 0;
    let mut chunk = [0u8; 512];
    let mut at = rsp & !7;
    'scan: for _ in 0..32 {
        if with_vm(|vm| read(vm, r, at, &mut chunk)).is_err() {
            break;
        }
        for word in chunk.chunks_exact(8) {
            let value = u64::from_le_bytes(word.try_into().unwrap());
            if (0x1_4000_0000..0x1_b000_0000).contains(&value)
                || (0x7f00_0000_0000..0x8000_0000_0000).contains(&value)
            {
                found[count] = value;
                count += 1;
                if count == found.len() {
                    break 'scan;
                }
            }
        }
        at += chunk.len() as u64;
    }
    log(format_args!(
        "DOOR_P18 STACK_TRAIL pid={} {:x?}\n",
        r.pid.raw(),
        &found[..count]
    ));
}

pub(in crate::process) unsafe fn resume(
    context: *mut arch::Context,
    r: Record,
    arguments: [u64; 6],
) -> Result<(), Status> {
    if !matches!(r.role, 107 | 110 | 115) {
        return Err(Status::AccessDenied);
    }
    if arguments[1..].iter().any(|value| *value != 0) {
        return Err(Status::InvalidArgument);
    }
    let expected = with_meta(|m| {
        m.threads[r.task.slot()]
            .as_ref()
            .and_then(|thread| thread.exception.as_ref())
            .map(|pending| pending.address)
    })
    .ok_or(Status::InvalidOperation)?;
    if arguments[0] != expected {
        return Err(Status::AccessDenied);
    }
    let mut bytes = [0u8; abi::CONTEXT_BYTES];
    with_vm(|vm| read(vm, r, arguments[0], &mut bytes))?;
    let integer = abi::IntegerContext::decode(&bytes).ok_or(Status::InvalidArgument)?;
    let regs = integer.registers;
    let frame = unsafe { &mut *((*context).frame as *mut arch::TrapFrame) };
    let mut next = *frame;
    next.rax = regs[0];
    next.rcx = regs[1];
    next.rdx = regs[2];
    next.rbx = regs[3];
    next.rsp = regs[4];
    next.rbp = regs[5];
    next.rsi = regs[6];
    next.rdi = regs[7];
    next.r8 = regs[8];
    next.r9 = regs[9];
    next.r10 = regs[10];
    next.r11 = regs[11];
    next.r12 = regs[12];
    next.r13 = regs[13];
    next.r14 = regs[14];
    next.r15 = regs[15];
    next.rip = regs[16];
    next.rflags = u64::from(integer.flags);
    if !arch::valid_user_return(unsafe { &*context }, &next, arch::kernel_cr3()) {
        return Err(Status::InvalidArgument);
    }
    with_vm(|vm| {
        vm.validate(r.pid.slot(), next.rip, 1, UserAccess::Execute)?;
        vm.validate(r.pid.slot(), next.rsp, 8, UserAccess::Write)
    })?;
    let pending = with_meta(|m| m.threads[r.task.slot()].as_mut().unwrap().exception.take())
        .ok_or(Status::InvalidOperation)?;
    // The CONTEXT's floating-point state when it carries a valid one (a
    // handler may have changed it), else the state saved at the fault.
    let fx = abi::decode_float(&bytes).unwrap_or(pending.fx);
    unsafe {
        (*context).fx = fx;
    }
    *frame = next;
    log(format_args!(
        "DOOR_P10 CONTINUE pid={} rip={:#x}\n",
        r.pid.raw(),
        next.rip
    ));
    Ok(())
}

/// EXCEPTION_RECORD32 and CONTEXT32 sizes and the CONTEXT32 fields written.
const RECORD32_BYTES: usize = 80;
const CONTEXT32_BYTES: usize = 716;
/// CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS | EXTENDED_REGISTERS.
const CONTEXT32_FLAGS: u32 = 0x0001_0027;
/// A fault in 32-bit (x86) code (P17): an EXCEPTION_RECORD32 and a
/// CONTEXT32 (its integer, control and segment registers, the FXSAVE image
/// as ExtendedRegisters) below the faulting stack, then 32-bit ntdll's
/// dispatcher (`DoorExceptionDispatch`) with ECX and EDX pointing at them.
/// The dispatcher continues in user mode (an x86 CONTEXT holds no state the
/// kernel must restore), so no continuation is kept here.
unsafe fn deliver32(
    context: *mut arch::Context,
    r: Record,
    frame: &mut arch::TrapFrame,
    dispatch: u64,
    vector: u64,
    error: u64,
    address: u64,
) -> Result<(), Status> {
    let (code, parameters, count): (u32, [u32; 2], usize) = match vector {
        14 => (
            if error & STACK_OVERFLOW_FAULT != 0 {
                STACK_OVERFLOW
            } else if error & GUARD_FAULT != 0 {
                abi::GUARD_PAGE_VIOLATION
            } else {
                abi::ACCESS_VIOLATION
            },
            [
                if error & 16 != 0 {
                    8
                } else {
                    u32::from(error & 2 != 0)
                },
                address as u32,
            ],
            2,
        ),
        6 => (0xc000_001d, [0, 0], 0),
        0 => (0xc000_0094, [0, 0], 0),
        _ => match translate(r, vector, frame.rip, unsafe { &(*context).fx }) {
            Some((0, _)) => (abi::ACCESS_VIOLATION, [0, u32::MAX], 2),
            Some((code, back)) => {
                frame.rip -= back;
                (code, [0, 0], 0)
            }
            None => return Err(Status::Unsupported),
        },
    };
    let low = |value: u64| value as u32;
    let mut packet = [0u8; RECORD32_BYTES + CONTEXT32_BYTES];
    let mut put = |at: usize, value: u32| packet[at..at + 4].copy_from_slice(&value.to_le_bytes());
    put(0, code);
    put(12, low(frame.rip));
    put(16, count as u32);
    for (index, value) in parameters[..count].iter().enumerate() {
        put(20 + index * 4, *value);
    }
    let c = RECORD32_BYTES;
    put(c, CONTEXT32_FLAGS);
    // SegGs and SegFs as WOW64 reports them (0x2B and the TEB selector 0x53).
    for (offset, value) in [
        (140, u32::from(arch::USER_DATA_SELECTOR)),
        (144, 0x53),
        (148, u32::from(arch::USER_DATA_SELECTOR)),
        (152, u32::from(arch::USER_DATA_SELECTOR)),
        (156, low(frame.rdi)),
        (160, low(frame.rsi)),
        (164, low(frame.rbx)),
        (168, low(frame.rdx)),
        (172, low(frame.rcx)),
        (176, low(frame.rax)),
        (180, low(frame.rbp)),
        (184, low(frame.rip)),
        (188, u32::from(arch::USER_CODE32_SELECTOR)),
        (192, low(frame.rflags)),
        (196, low(frame.rsp)),
        (200, u32::from(arch::USER_DATA_SELECTOR)),
    ] {
        put(c + offset, value);
    }
    packet[c + 204..c + 716].copy_from_slice(unsafe { &(*context).fx });
    let base = (frame.rsp & 0xffff_ffff)
        .checked_sub(packet.len() as u64 + 64)
        .ok_or(Status::BadAddress)?
        & !15;
    let stack = base.checked_sub(16).ok_or(Status::BadAddress)?;
    with_vm(|vm| {
        vm.validate(r.pid.slot(), dispatch, 1, UserAccess::Execute)?;
        vm.validate(r.pid.slot(), stack, 16 + packet.len(), UserAccess::Write)?;
        write(vm, r, base, &packet)
    })?;
    frame.rip = dispatch;
    frame.rsp = stack;
    frame.rcx = base;
    frame.rdx = base + RECORD32_BYTES as u64;
    frame.rflags = 0x202;
    log(format_args!(
        "DOOR_P17 EXCEPTION pid={} vector={vector} code={code:#x} address={address:#x}\n",
        r.pid.raw()
    ));
    Ok(())
}
