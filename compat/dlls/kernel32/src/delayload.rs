//! Delay-loaded imports (api-ms-win-core-delayload): the resolver an
//! image's delay-load helper calls on the first call through a delayed
//! import, and the system failure hook.
use door_windows_abi::{narrow, widen};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(code: u32);
}
const ERROR_MOD_NOT_FOUND: u32 = 126;
const ERROR_PROC_NOT_FOUND: u32 = 127;

/// What a delayed import that could not be resolved calls instead: it
/// fails with ERROR_PROC_NOT_FOUND and returns 0, as the stub Windows'
/// failure hook hands back.
extern "system" fn delay_load_failed() -> usize {
    unsafe { RtlSetLastWin32Error(ERROR_PROC_NOT_FOUND) };
    0
}
fn read_u32(address: u64) -> u32 {
    unsafe { (address as *const u32).read_unaligned() }
}

/// `ResolveDelayLoadedAPI(base, descriptor, dllHook, systemHook, thunk,
/// flags)`: load the descriptor's DLL (once: its HMODULE slot keeps it),
/// find the function the thunk's import-name entry names (by name or
/// ordinal), store it in the thunk and return it. A DLL or function that
/// is missing gets the failure stub, as the system failure hook gives.
#[unsafe(no_mangle)]
pub extern "system" fn ResolveDelayLoadedAPI(
    base: usize,
    descriptor: usize,
    _dll_hook: usize,
    _system_hook: usize,
    thunk: usize,
    _flags: u32,
) -> usize {
    let (base, descriptor, thunk) = (widen(base), widen(descriptor), widen(thunk));
    // IMAGE_DELAYLOAD_DESCRIPTOR: Attributes, DllNameRVA, ModuleHandleRVA,
    // ImportAddressTableRVA, ImportNameTableRVA, ...
    let rva = |at: u64| base + u64::from(read_u32(descriptor + at));
    let (name, handle_slot, iat, int) = (rva(4), rva(8), rva(12), rva(16));
    let width = core::mem::size_of::<usize>() as u64;
    let index = (thunk - iat) / width;
    let mut module = unsafe { (handle_slot as *const usize).read_unaligned() };
    if module == 0 {
        module = crate::guest::LoadLibraryA(narrow(name));
        if module == 0 {
            unsafe { RtlSetLastWin32Error(ERROR_MOD_NOT_FOUND) };
            return delay_load_failed as *const () as usize;
        }
        unsafe { (handle_slot as *mut usize).write_unaligned(module) };
    }
    let entry = unsafe { ((int + index * width) as *const usize).read_unaligned() } as u64;
    let ordinal_flag = 1u64 << (width * 8 - 1);
    let symbol = if entry & ordinal_flag != 0 {
        entry & 0xffff
    } else {
        // IMAGE_IMPORT_BY_NAME: the hint, then the name.
        base + (entry & 0xffff_ffff) + 2
    };
    let address = crate::guest::GetProcAddress(module, narrow(symbol));
    let address = if address == 0 {
        delay_load_failed as *const () as usize
    } else {
        address
    };
    unsafe { (thunk as *mut usize).write_unaligned(address) };
    address
}
/// `DelayLoadFailureHook(dll, procedure)`: the failure stub.
#[unsafe(no_mangle)]
pub extern "system" fn DelayLoadFailureHook(_dll: usize, _procedure: usize) -> usize {
    delay_load_failed as *const () as usize
}
