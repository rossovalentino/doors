//! Activation contexts (the first part of P18 M0g): programs create and
//! activate them from their own or a DLL's manifest (an application host
//! does at start-up). Doors redirects no assembly — each system DLL exists
//! in one version, comctl32's being the v6 controls — so a context is a
//! handle that activates and deactivates, answers basic and run-level
//! queries (asInvoker), and finds no section string (the loader's ordinary
//! search applies).
use core::sync::atomic::{AtomicUsize, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(value: u32);
}

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_SXS_KEY_NOT_FOUND: u32 = 14007;
/// Context handles: distinct, never NULL or INVALID_HANDLE_VALUE, clear
/// of the kernel's handles (from 0x1_0000) and the DLLs' small ones.
const FIRST: usize = 0x00AC_7000;
static NEXT: AtomicUsize = AtomicUsize::new(0);
static COOKIE: AtomicUsize = AtomicUsize::new(1);

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
/// `CreateActCtxW`: ACTCTXW's cbSize is checked; its manifest names no
/// assembly Doors would redirect to.
#[unsafe(no_mangle)]
pub extern "system" fn CreateActCtxW(context: usize) -> usize {
    let minimum = 4 + 4 + core::mem::size_of::<usize>();
    if context == 0 || (unsafe { (context as *const u32).read_unaligned() } as usize) < minimum {
        fail(ERROR_INVALID_PARAMETER);
        return usize::MAX;
    }
    FIRST + 4 * (NEXT.fetch_add(1, Ordering::Relaxed) % 0x1000)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateActCtxA(context: usize) -> usize {
    CreateActCtxW(context)
}
#[unsafe(no_mangle)]
pub extern "system" fn ActivateActCtx(_context: usize, cookie: usize) -> i32 {
    if cookie == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let value = COOKIE.fetch_add(1, Ordering::Relaxed);
    unsafe { (cookie as *mut usize).write_unaligned(value) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DeactivateActCtx(_flags: u32, _cookie: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn AddRefActCtx(_context: usize) {}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseActCtx(_context: usize) {}
/// The process default context is active (NULL).
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentActCtx(context: usize) -> i32 {
    if context == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    unsafe { (context as *mut usize).write_unaligned(0) };
    1
}
/// `QueryActCtxW`: ActivationContextBasicInformation (1: the handle, no
/// flags) and RunlevelInformationInActivationContext (5: asInvoker, no UI
/// access).
#[unsafe(no_mangle)]
pub extern "system" fn QueryActCtxW(
    _flags: u32,
    context: usize,
    _subinstance: usize,
    class: u32,
    buffer: usize,
    size: usize,
    written: usize,
) -> i32 {
    let pointer = core::mem::size_of::<usize>();
    let mut out = [0u8; 16];
    let bytes = match class {
        1 => {
            out[..pointer].copy_from_slice(&context.to_le_bytes()[..pointer]);
            2 * pointer
        }
        5 => {
            out[4..8].copy_from_slice(&1u32.to_le_bytes());
            12
        }
        _ => return fail(ERROR_INVALID_PARAMETER),
    };
    if written != 0 {
        unsafe { (written as *mut usize).write_unaligned(bytes) };
    }
    if buffer == 0 || size < bytes {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), buffer as *mut u8, bytes) };
    1
}
/// No assembly is redirected: no section holds the string.
#[unsafe(no_mangle)]
pub extern "system" fn FindActCtxSectionStringW(
    _flags: u32,
    _guid: usize,
    _section: u32,
    _string: usize,
    _data: usize,
) -> i32 {
    fail(ERROR_SXS_KEY_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindActCtxSectionGuid(
    _flags: u32,
    _extension: usize,
    _section: u32,
    _guid: usize,
    _data: usize,
) -> i32 {
    fail(ERROR_SXS_KEY_NOT_FOUND)
}
