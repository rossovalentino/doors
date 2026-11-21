//! x86 (P17) exception dispatch, as WOW64's 32-bit ntdll does it. The
//! kernel delivers a fault in 32-bit code as an EXCEPTION_RECORD32 and a
//! CONTEXT32 below the faulting stack (`DoorExceptionDispatch`, ECX and
//! EDX point at them); RtlRaiseException captures its caller's registers
//! into one. Dispatch runs the vectored handlers, then the frames of the
//! thread's SEH chain (`fs:[0]`, each inside the TEB32's stack range,
//! handlers called `(record, frame, context, dispatcher)`), and continues
//! by loading the (possibly edited) CONTEXT32 in user mode: the FXSAVE
//! image, the integer registers, EFLAGS, ESP and EIP. An exception no
//! handler takes ends the process with its code, as Windows does once the
//! default unhandled-exception filter has run. RtlUnwind calls each frame
//! above its target with EXCEPTION_UNWINDING set and unlinks it.
use door_windows_abi::x86;

/// EXCEPTION_RECORD32 and CONTEXT32 sizes, CONTEXT_i386 | CONTROL |
/// INTEGER | SEGMENTS | EXTENDED_REGISTERS, and where ExtendedRegisters
/// (the FXSAVE image) starts.
const RECORD_BYTES: usize = 80;
const CONTEXT_BYTES: usize = 716;
const CONTEXT_FLAGS: u32 = 0x0001_0027;
const CONTEXT_EXTENDED: u32 = 0x0001_0020;
const EXTENDED: u32 = 204;
/// CONTEXT32 field offsets.
const SEG_GS: u32 = 140;
const SEG_FS: u32 = 144;
const SEG_ES: u32 = 148;
const SEG_DS: u32 = 152;
const EDI: u32 = 156;
const ESI: u32 = 160;
const EBX: u32 = 164;
const EDX: u32 = 168;
const ECX: u32 = 172;
const EAX: u32 = 176;
const EBP: u32 = 180;
const EIP: u32 = 184;
const SEG_CS: u32 = 188;
const EFLAGS: u32 = 192;
const ESP: u32 = 196;
const SEG_SS: u32 = 200;
/// ExceptionFlags.
const NONCONTINUABLE: u32 = 1;
const UNWINDING: u32 = 2;
const EXIT_UNWIND: u32 = 4;
const STACK_INVALID: u32 = 8;
/// EXCEPTION_DISPOSITION.
const CONTINUE_EXECUTION: u32 = 0;
const CONTINUE_SEARCH: u32 = 1;
const NESTED_EXCEPTION: u32 = 2;
const COLLIDED_UNWIND: u32 = 3;
/// The NTSTATUS codes the dispatcher itself ends a process with.
const NONCONTINUABLE_EXCEPTION: u32 = 0xc000_0025;
const INVALID_DISPOSITION: u32 = 0xc000_0026;
const STATUS_UNWIND: u32 = 0xc000_0027;
const BAD_STACK: u32 = 0xc000_0028;
const INVALID_UNWIND_TARGET: u32 = 0xc000_0029;
/// The EFLAGS bits a CONTEXT32 may set (the arithmetic flags and DF), and
/// the ones continuing always sets (IF and the reserved bit 1).
const EFLAGS_USER: u32 = 0x0cd5;
const EFLAGS_FIXED: u32 = 0x0202;

fn teb(offset: usize) -> u32 {
    let value: u32;
    // SAFETY: FS is this thread's mapped TEB32.
    unsafe {
        core::arch::asm!("mov {value}, fs:[{offset}]", value = out(reg) value,
        offset = in(reg) offset, options(nostack, readonly, preserves_flags));
    }
    value
}
fn set_chain(head: u32) {
    // SAFETY: as `teb`; NT_TIB32.ExceptionList is the thread's own.
    unsafe {
        core::arch::asm!("mov fs:[{offset}], {head}", head = in(reg) head,
        offset = const x86::TEB_EXCEPTION_LIST, options(nostack, preserves_flags));
    }
}
/// A 32-bit word of this process's memory (the record, context and frames
/// the dispatch was handed, already in range).
fn word(address: u32) -> u32 {
    unsafe { core::ptr::read_volatile(address as usize as *const u32) }
}
fn set_word(address: u32, value: u32) {
    unsafe { core::ptr::write_volatile(address as usize as *mut u32, value) }
}
/// A frame of the SEH chain is a registration record on this thread's
/// stack.
fn valid_frame(frame: u32) -> bool {
    let (base, limit) = (teb(x86::TEB_STACK_BASE), teb(x86::TEB_STACK_LIMIT));
    frame & 3 == 0 && frame >= limit && frame.checked_add(8).is_some_and(|end| end <= base)
}
/// The exception ends the process (no handler took it, or a handler broke
/// the dispatch rules).
fn terminate(code: u32) -> ! {
    door_sys::exit(i64::from(code))
}

/// Calls `function(a, b, c, d)` the way Windows calls an exception handler:
/// cdecl or stdcall alike (the stack pointer is restored from EBP), the
/// non-volatile registers kept, the direction flag clear.
#[unsafe(naked)]
unsafe extern "C" fn call4(function: u32, a: u32, b: u32, c: u32, d: u32) -> u32 {
    core::arch::naked_asm!(
        "push ebp",
        "mov ebp, esp",
        "push ebx",
        "push esi",
        "push edi",
        "push dword ptr [ebp + 24]",
        "push dword ptr [ebp + 20]",
        "push dword ptr [ebp + 16]",
        "push dword ptr [ebp + 12]",
        "cld",
        "call dword ptr [ebp + 8]",
        "lea esp, [ebp - 12]",
        "pop edi",
        "pop esi",
        "pop ebx",
        "pop ebp",
        "ret",
    )
}
/// Continues with `context` (a CONTEXT32): the FXSAVE image at `fx` (a
/// 16-byte aligned copy, or 0 for none), then the integer registers; ECX,
/// EAX, EFLAGS and EIP travel on the target stack, below its ESP.
#[unsafe(naked)]
unsafe extern "C" fn restore(context: u32, fx: u32) -> ! {
    core::arch::naked_asm!(
        "mov ecx, [esp + 4]",
        "mov eax, [esp + 8]",
        "test eax, eax",
        "jz 2f",
        "fxrstor [eax]",
        "2:",
        "mov edx, [ecx + {esp}]",
        "sub edx, 16",
        "mov eax, [ecx + {ecx}]",
        "mov [edx], eax",
        "mov eax, [ecx + {eax}]",
        "mov [edx + 4], eax",
        "mov eax, [ecx + {eflags}]",
        "mov [edx + 8], eax",
        "mov eax, [ecx + {eip}]",
        "mov [edx + 12], eax",
        "mov edi, [ecx + {edi}]",
        "mov esi, [ecx + {esi}]",
        "mov ebx, [ecx + {ebx}]",
        "mov ebp, [ecx + {ebp}]",
        "mov esp, edx",
        "mov edx, [ecx + {edx}]",
        "pop ecx",
        "pop eax",
        "popfd",
        "ret",
        esp = const ESP,
        ecx = const ECX,
        eax = const EAX,
        eflags = const EFLAGS,
        eip = const EIP,
        edi = const EDI,
        esi = const ESI,
        ebx = const EBX,
        ebp = const EBP,
        edx = const EDX,
    )
}
#[repr(C, align(16))]
struct Fx([u8; 512]);
/// Continue execution with `context`.
fn resume(context: u32) -> ! {
    let mut fx = Fx([0; 512]);
    let flags = word(context);
    let fx_address = if flags & CONTEXT_EXTENDED == CONTEXT_EXTENDED {
        for (index, chunk) in fx.0.chunks_exact_mut(4).enumerate() {
            chunk.copy_from_slice(&word(context + EXTENDED + 4 * index as u32).to_le_bytes());
        }
        // MXCSR may only hold the bits MXCSR_MASK allows (else FXRSTOR faults).
        let mask = u32::from_le_bytes([fx.0[28], fx.0[29], fx.0[30], fx.0[31]]);
        let mask = if mask == 0 { 0xffbf } else { mask };
        let mxcsr = u32::from_le_bytes([fx.0[24], fx.0[25], fx.0[26], fx.0[27]]) & mask;
        fx.0[24..28].copy_from_slice(&mxcsr.to_le_bytes());
        fx.0.as_ptr() as u32
    } else {
        0
    };
    set_word(
        context + EFLAGS,
        (word(context + EFLAGS) & EFLAGS_USER) | EFLAGS_FIXED,
    );
    unsafe { restore(context, fx_address) }
}

/// Runs the handlers for (`record`, `context`): continues execution when
/// one says so, else returns (the exception is unhandled).
fn dispatch(record: u32, context: u32) {
    let mut pointers = [record, context];
    if let Some(handlers) = super::exception::vectored() {
        for function in handlers.into_iter().take_while(|function| *function != 0) {
            let result = unsafe { call4(function as u32, pointers.as_mut_ptr() as u32, 0, 0, 0) };
            if result as i32 == -1 {
                resume(context);
            }
        }
    }
    let mut frame = teb(x86::TEB_EXCEPTION_LIST);
    while frame != x86::CHAIN_END {
        if !valid_frame(frame) {
            set_word(record + 4, word(record + 4) | STACK_INVALID);
            return;
        }
        let mut dispatcher = 0u32;
        let handler = word(frame + 4);
        let result = unsafe {
            call4(
                handler,
                record,
                frame,
                context,
                &mut dispatcher as *mut u32 as u32,
            )
        };
        match result {
            CONTINUE_EXECUTION if word(record + 4) & NONCONTINUABLE != 0 => {
                terminate(NONCONTINUABLE_EXCEPTION)
            }
            CONTINUE_EXECUTION => resume(context),
            CONTINUE_SEARCH | NESTED_EXCEPTION => {}
            _ => terminate(INVALID_DISPOSITION),
        }
        frame = word(frame);
    }
}

/// The kernel's entry for a fault in 32-bit code: ECX the record, EDX the
/// context, both on the faulting thread's stack.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn DoorExceptionDispatch() -> ! {
    core::arch::naked_asm!("push edx", "push ecx", "call {f}", "ud2", f = sym fault)
}
extern "C" fn fault(record: u32, context: u32) -> ! {
    dispatch(record, context);
    terminate(word(record))
}

#[repr(C, align(16))]
struct Context([u8; CONTEXT_BYTES]);
impl Context {
    fn new() -> Self {
        let mut context = Self([0; CONTEXT_BYTES]);
        context.put(0, CONTEXT_FLAGS);
        let (gs, fs, es, ds, cs, ss): (u32, u32, u32, u32, u32, u32);
        // SAFETY: reading segment selectors has no effect.
        unsafe {
            core::arch::asm!(
                "mov {gs:e}, gs", "mov {fs:e}, fs", "mov {es:e}, es",
                "mov {ds:e}, ds", "mov {cs:e}, cs", "mov {ss:e}, ss",
                gs = out(reg) gs, fs = out(reg) fs, es = out(reg) es,
                ds = out(reg) ds, cs = out(reg) cs, ss = out(reg) ss,
                options(nomem, nostack, preserves_flags)
            );
        }
        for (offset, value) in [
            (SEG_GS, gs),
            (SEG_FS, fs),
            (SEG_ES, es),
            (SEG_DS, ds),
            (SEG_CS, cs),
            (SEG_SS, ss),
        ] {
            context.put(offset, value & 0xffff);
        }
        let mut fx = Fx([0; 512]);
        unsafe {
            core::arch::asm!("fxsave [{}]", in(reg) fx.0.as_mut_ptr(), options(nostack, preserves_flags));
        }
        context.0[EXTENDED as usize..].copy_from_slice(&fx.0);
        context
    }
    fn put(&mut self, offset: u32, value: u32) {
        let at = offset as usize;
        self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn address(&mut self) -> u32 {
        self.0.as_mut_ptr() as u32
    }
}

/// RtlRaiseException: dispatch `record` from the caller's own state, so
/// that continuing returns from this call with every register as it was.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "system" fn RtlRaiseException(_record: u32) {
    core::arch::naked_asm!(
        "pushfd",
        "push ebp",
        "push eax",
        "push ecx",
        "push edx",
        "push ebx",
        "push esi",
        "push edi",
        "mov eax, esp",
        "push eax",
        "call {f}",
        "ud2",
        f = sym raise,
    )
}
/// `saved`: EDI, ESI, EBX, EDX, ECX, EAX, EBP, EFLAGS, the return address,
/// the record; the caller's stack pointer once this stdcall returns is
/// just past them.
extern "C" fn raise(saved: u32) -> ! {
    let at = |index: u32| word(saved + 4 * index);
    let record = at(9);
    let mut context = Context::new();
    for (offset, index) in [
        (EDI, 0),
        (ESI, 1),
        (EBX, 2),
        (EDX, 3),
        (ECX, 4),
        (EAX, 5),
        (EBP, 6),
        (EFLAGS, 7),
        (EIP, 8),
    ] {
        context.put(offset, at(index));
    }
    context.put(ESP, saved + 40);
    set_word(record + 12, at(8));
    let address = context.address();
    dispatch(record, address);
    terminate(word(record))
}

/// RtlUnwind: call every frame above `target` (all of them when it is 0,
/// an exit unwind) with EXCEPTION_UNWINDING set, unlinking each, then
/// return `value` (Windows continues its caller with EAX = `value`).
#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwind(target: u32, target_ip: u32, record: u32, value: u32) -> u32 {
    DoorUnwind(target, target_ip, record, value)
}
/// RtlUnwind under the name 32-bit kernel32's own RtlUnwind export calls.
#[unsafe(no_mangle)]
pub extern "system" fn DoorUnwind(target: u32, _target_ip: u32, record: u32, value: u32) -> u32 {
    let mut own = [0u32; RECORD_BYTES / 4];
    let record = if record == 0 {
        own[0] = STATUS_UNWIND;
        own[3] = RtlUnwind as *const () as usize as u32;
        own.as_mut_ptr() as u32
    } else {
        record
    };
    let exit = if target == 0 { EXIT_UNWIND } else { 0 };
    set_word(record + 4, word(record + 4) | UNWINDING | exit);
    let mut context = Context::new();
    let frame_address = &value as *const u32 as u32;
    context.put(ESP, frame_address);
    context.put(EAX, value);
    let context = context.address();
    let mut frame = teb(x86::TEB_EXCEPTION_LIST);
    while frame != x86::CHAIN_END && frame != target {
        if target != 0 && target < frame {
            terminate(INVALID_UNWIND_TARGET);
        }
        if !valid_frame(frame) {
            terminate(BAD_STACK);
        }
        let mut dispatcher = 0u32;
        let handler = word(frame + 4);
        let result = unsafe {
            call4(
                handler,
                record,
                frame,
                context,
                &mut dispatcher as *mut u32 as u32,
            )
        };
        if !matches!(result, CONTINUE_SEARCH | COLLIDED_UNWIND) {
            terminate(INVALID_DISPOSITION);
        }
        let next = word(frame);
        set_chain(next);
        frame = next;
    }
    if frame != target && target != 0 && target != x86::CHAIN_END {
        terminate(INVALID_UNWIND_TARGET);
    }
    value
}
