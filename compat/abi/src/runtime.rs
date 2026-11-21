//! Freestanding runtime pieces every Rust-only compatibility DLL needs when
//! it links without a C runtime (ws2_32, iphlpapi, dnsapi, ...): a panic
//! handler that fails fast, the `_fltused` linkage marker and `strlen`
//! (referenced by `alloc`'s C strings). `memcpy`/`memset`/`memmove`/
//! `memcmp` come from ntdll's exports. Enable feature `guest-runtime`.
#![allow(unsafe_code)]

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn NtTerminateProcess(process: usize, code: u32) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn NtWriteFile(
        handle: usize,
        event: usize,
        routine: usize,
        context: usize,
        status: usize,
        buffer: usize,
        bytes: u32,
        offset: usize,
        key: usize,
    ) -> u32;
}

/// "<file>:<line>: compatibility DLL panic" on the standard error of a
/// console process (ignored without one), so a fast-failed process leaves
/// its cause in the console and the session transcript.
fn report(location: Option<&core::panic::Location<'_>>) {
    let mut text = [0u8; 160];
    let mut n = 0;
    let mut push = |bytes: &[u8]| {
        for b in bytes {
            if n < text.len() {
                text[n] = *b;
                n += 1;
            }
        }
    };
    if let Some(location) = location {
        push(location.file().as_bytes());
        push(b":");
        let mut digits = [0u8; 10];
        let mut line = location.line();
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (line % 10) as u8;
            line /= 10;
            if line == 0 {
                break;
            }
        }
        push(&digits[i..]);
        push(b": ");
    }
    push(b"compatibility DLL panic\r\n");
    let mut status = [0u64; 2];
    // SAFETY: both buffers are this frame's; a missing console fails the call.
    unsafe {
        NtWriteFile(
            crate::narrow(crate::ERROR_HANDLE),
            0,
            0,
            0,
            crate::narrow(status.as_mut_ptr() as u64),
            crate::narrow(text.as_ptr() as u64),
            n as u32,
            0,
            0,
        );
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    report(info.location());
    // STATUS_STACK_BUFFER_OVERRUN-style fast fail: no unwinding, no callbacks.
    unsafe {
        NtTerminateProcess(crate::narrow(u64::MAX), 0xc000_0409);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Linkage marker required by Windows compiler-builtins, not an FP operation.
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

/// x86 (P17): the precompiled `core`'s landing pads name MSVC's C++ frame
/// handler. No foreign or Rust unwind is ever dispatched here (panics abort),
/// so reaching it ends the process.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
extern "C" fn __CxxFrameHandler3() -> ! {
    unsafe {
        NtTerminateProcess(crate::narrow(u64::MAX), 0xc000_0409);
    }
    loop {
        core::hint::spin_loop();
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strlen(text: *const u8) -> usize {
    let mut n = 0;
    while unsafe { *text.add(n) } != 0 {
        n += 1;
    }
    n
}
