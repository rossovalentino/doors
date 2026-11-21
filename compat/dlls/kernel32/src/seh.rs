//! Exception entry points kernel32 forwards to ntdll on Windows (P18 M0c):
//! RaiseException and the Rtl unwinding, context and function-table
//! functions for x64, and the unhandled-exception filter for both widths.

#[cfg(target_arch = "x86_64")]
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlRaiseException(record: u64);
    fn DoorSetUnhandledExceptionFilter(filter: u64) -> u64;
    fn DoorUnhandledExceptionFilter(pointers: u64) -> i32;
}

/// RaiseException (x64): EXCEPTION_RECORD with ExceptionAddress at this
/// function, as Windows sets it; at most 15 arguments, none without an
/// array. A handler that continues execution returns here.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn RaiseException(code: u32, flags: u32, count: u32, arguments: usize) {
    let mut record = [0u64; 19];
    record[0] = u64::from(code) | (u64::from(flags & 1) << 32);
    record[2] = RaiseException as *const () as usize as u64;
    let count = if arguments == 0 { 0 } else { count.min(15) };
    record[3] = u64::from(count);
    for index in 0..count as usize {
        record[4 + index] =
            unsafe { core::ptr::read_volatile((arguments as *const u64).add(index)) };
    }
    unsafe { RtlRaiseException(record.as_mut_ptr() as u64) };
}
// RtlCaptureContext, RtlRestoreContext, RtlUnwind(Ex), RtlPcToFileHeader and
// the function-table calls are export forwarders to ntdll (`tools/xtask`
// `dlls::forwarders`), as Windows binds them: a wrapper here would capture
// its own frame, and an import of the same name resolves to the local
// definition (an endless self-call).

#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn SetUnhandledExceptionFilter(filter: usize) -> usize {
    unsafe { DoorSetUnhandledExceptionFilter(filter as u64) as usize }
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn UnhandledExceptionFilter(pointers: usize) -> i32 {
    unsafe { DoorUnhandledExceptionFilter(pointers as u64) }
}

/// x86: the filter is recorded; 32-bit unhandled faults end the process
/// in ntdll's SEH chain walk (P17), which does not consult it yet.
#[cfg(target_arch = "x86")]
static FILTER: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "system" fn SetUnhandledExceptionFilter(filter: usize) -> usize {
    FILTER.swap(filter, core::sync::atomic::Ordering::AcqRel)
}
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "system" fn UnhandledExceptionFilter(pointers: usize) -> i32 {
    let filter = FILTER.load(core::sync::atomic::Ordering::Acquire);
    if filter == 0 {
        return 1;
    }
    let f: extern "system" fn(usize) -> i32 = unsafe { core::mem::transmute(filter) };
    f(pointers)
}

// ---- CONTEXT helpers (x64) ----------------------------------------------
// The kernel saves and restores the legacy FXSAVE state (x87 and SSE);
// AVX state is not kept per thread, so XState reports x87|SSE only and a
// CONTEXT_XSTATE request is refused. CoreCLR then uses plain CONTEXTs.

#[cfg(target_arch = "x86_64")]
mod context {
    pub const BYTES: usize = 1232;
    pub const FLAGS: usize = 48;
    pub const AMD64: u32 = 0x0010_0000;
    pub const CONTROL: u32 = 0x1;
    pub const INTEGER: u32 = 0x2;
    pub const SEGMENTS: u32 = 0x4;
    pub const FLOATING_POINT: u32 = 0x8;
    pub const DEBUG_REGISTERS: u32 = 0x10;
    pub const XSTATE: u32 = 0x40;
    /// XSTATE_MASK_LEGACY (x87 | SSE).
    pub const LEGACY: u64 = 3;
}
#[cfg(target_arch = "x86_64")]
fn set_error(value: u32) {
    #[link(name = "ntdll", kind = "dylib")]
    unsafe extern "system" {
        fn RtlSetLastWin32Error(value: u32);
    }
    unsafe { RtlSetLastWin32Error(value) };
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn GetEnabledXStateFeatures() -> u64 {
    context::LEGACY
}
/// InitializeContext: a zeroed, 16-byte aligned CONTEXT inside `buffer`
/// with `flags`; the length needed when `buffer` is null or too small.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn InitializeContext(
    buffer: usize,
    flags: u32,
    context: usize,
    length: usize,
) -> i32 {
    if flags
        & !context::AMD64
        & !(context::CONTROL
            | context::INTEGER
            | context::SEGMENTS
            | context::FLOATING_POINT
            | context::DEBUG_REGISTERS
            | context::XSTATE)
        != 0
        || flags & context::AMD64 == 0
    {
        set_error(87);
        return 0;
    }
    if flags & context::XSTATE != 0 {
        set_error(50);
        return 0;
    }
    let needed = (context::BYTES + 15) as u32;
    let available = unsafe { (length as *const u32).read_unaligned() };
    if buffer == 0 || available < needed {
        unsafe { (length as *mut u32).write_unaligned(needed) };
        set_error(122);
        return 0;
    }
    let aligned = (buffer + 15) & !15;
    unsafe {
        core::ptr::write_bytes(aligned as *mut u8, 0, context::BYTES);
        ((aligned + context::FLAGS) as *mut u32).write_unaligned(flags);
        (context as *mut usize).write_unaligned(aligned);
    }
    1
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn InitializeContext2(
    buffer: usize,
    flags: u32,
    context: usize,
    length: usize,
    _compaction_mask: u64,
) -> i32 {
    InitializeContext(buffer, flags, context, length)
}
/// CopyContext: the parts of `source` that `flags` names into
/// `destination`.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn CopyContext(destination: usize, flags: u32, source: usize) -> i32 {
    if flags & context::XSTATE != 0 {
        set_error(50);
        return 0;
    }
    let copy = |offset: usize, bytes: usize| unsafe {
        core::ptr::copy(
            (source + offset) as *const u8,
            (destination + offset) as *mut u8,
            bytes,
        )
    };
    if flags & context::CONTROL != 0 {
        copy(56, 2); // SegCs
        copy(66, 2); // SegSs
        copy(68, 4); // EFlags
        copy(152, 8); // Rsp
        copy(248, 8); // Rip
    }
    if flags & context::INTEGER != 0 {
        copy(120, 32); // Rax, Rcx, Rdx, Rbx
        copy(160, 88); // Rbp .. R15
    }
    if flags & context::SEGMENTS != 0 {
        copy(58, 8); // SegDs, SegEs, SegFs, SegGs
    }
    if flags & context::FLOATING_POINT != 0 {
        copy(52, 4); // MxCsr
        copy(256, 512); // FltSave
    }
    if flags & context::DEBUG_REGISTERS != 0 {
        copy(72, 48); // Dr0..Dr3, Dr6, Dr7
    }
    let merged = unsafe { ((destination + context::FLAGS) as *const u32).read_unaligned() } | flags;
    unsafe { ((destination + context::FLAGS) as *mut u32).write_unaligned(merged) };
    1
}
/// LocateXStateFeature: x87 (0) and SSE (1) live in FltSave.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn LocateXStateFeature(context: usize, feature: u32, length: usize) -> usize {
    let (offset, bytes) = match feature {
        0 => (256, 160),
        1 => (256 + 160, 256),
        _ => return 0,
    };
    if length != 0 {
        unsafe { (length as *mut u32).write_unaligned(bytes) };
    }
    context + offset
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn SetXStateFeaturesMask(_context: usize, mask: u64) -> i32 {
    if mask & !context::LEGACY != 0 {
        set_error(87);
        return 0;
    }
    1
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn GetXStateFeaturesMask(_context: usize, mask: usize) -> i32 {
    unsafe { (mask as *mut u64).write_unaligned(context::LEGACY) };
    1
}

#[cfg(target_arch = "x86_64")]
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn NtGetContextThread(thread: u64, context: u64) -> u32;
    fn NtSetContextThread(thread: u64, context: u64) -> u32;
    fn RtlNtStatusToDosError(status: u32) -> u32;
}
#[cfg(target_arch = "x86_64")]
fn context_result(status: u32) -> i32 {
    if status == 0 {
        1
    } else {
        set_error(unsafe { RtlNtStatusToDosError(status) });
        0
    }
}
/// GetThreadContext: a suspended thread of this process (or the caller).
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadContext(thread: usize, context: usize) -> i32 {
    context_result(unsafe { NtGetContextThread(thread as u64, context as u64) })
}
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadContext(thread: usize, context: usize) -> i32 {
    context_result(unsafe { NtSetContextThread(thread as u64, context as u64) })
}
