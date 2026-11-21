//! rpcrt4 exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const RPC_S_OK: u32 = 0;
const RPC_S_INVALID_STRING_UUID: u32 = 1705;
const RPC_S_OUT_OF_MEMORY: u32 = 14;
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> usize;
    fn HeapAlloc(heap: usize, flags: u32, bytes: usize) -> usize;
    fn HeapFree(heap: usize, flags: u32, block: usize) -> i32;
}
fn read_ansi(p: usize) -> Option<[u8; 36]> {
    let mut out = [0u8; 36];
    for (i, b) in out.iter_mut().enumerate() {
        // SAFETY: the caller's NUL-terminated string, read up to 37 bytes.
        *b = unsafe { (p as *const u8).add(i).read() };
        if *b == 0 {
            return None;
        }
    }
    (unsafe { (p as *const u8).add(36).read() } == 0).then_some(out)
}
/// UuidCreate: a random (version 4) UUID.
#[unsafe(no_mangle)]
pub extern "system" fn UuidCreate(out: usize) -> u32 {
    let mut b = [0u8; 16];
    if unsafe { DoorGenRandom(b.as_mut_ptr() as u64, 16) } != 0 {
        return RPC_S_OUT_OF_MEMORY;
    }
    crate::version4(&mut b);
    // SAFETY: the caller's UUID out-parameter.
    unsafe { (out as *mut [u8; 16]).write_unaligned(b) };
    RPC_S_OK
}
/// UuidCreateSequential: Doors exposes no MAC address, so this is random
/// too (RPC_S_UUID_LOCAL_ONLY would be Windows' answer without one).
#[unsafe(no_mangle)]
pub extern "system" fn UuidCreateSequential(out: usize) -> u32 {
    let r = UuidCreate(out);
    if r == RPC_S_OK { 1824 } else { r }
}
#[unsafe(no_mangle)]
pub extern "system" fn UuidCreateNil(out: usize) -> u32 {
    // SAFETY: the caller's UUID out-parameter.
    unsafe { (out as *mut [u8; 16]).write_unaligned([0; 16]) };
    RPC_S_OK
}
/// UuidFromStringA: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" (NULL is the
/// nil UUID).
#[unsafe(no_mangle)]
pub extern "system" fn UuidFromStringA(text: usize, out: usize) -> u32 {
    if text == 0 {
        return UuidCreateNil(out);
    }
    match read_ansi(text).and_then(|t| crate::parse(&t)) {
        Some(b) => {
            // SAFETY: the caller's UUID out-parameter.
            unsafe { (out as *mut [u8; 16]).write_unaligned(b) };
            RPC_S_OK
        }
        None => RPC_S_INVALID_STRING_UUID,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn UuidFromStringW(text: usize, out: usize) -> u32 {
    if text == 0 {
        return UuidCreateNil(out);
    }
    let mut a = [0u8; 37];
    for (i, b) in a.iter_mut().enumerate() {
        // SAFETY: the caller's NUL-terminated string, read up to 37 units.
        let u = unsafe { (text as *const u16).add(i).read_unaligned() };
        if u > 0x7F {
            return RPC_S_INVALID_STRING_UUID;
        }
        *b = u as u8;
        if u == 0 {
            break;
        }
    }
    UuidFromStringA(a.as_ptr() as usize, out)
}
/// UuidToStringA/W: lower-case text in an RpcStringFree block.
fn to_string(uuid: usize, out: usize, wide: bool) -> u32 {
    if uuid == 0 || out == 0 {
        return RPC_S_INVALID_STRING_UUID;
    }
    // SAFETY: the caller's UUID.
    let b = unsafe { (uuid as *const [u8; 16]).read_unaligned() };
    let text = crate::format(&b);
    let unit = if wide { 2 } else { 1 };
    let block = unsafe { HeapAlloc(GetProcessHeap(), 8, 37 * unit) };
    if block == 0 {
        return RPC_S_OUT_OF_MEMORY;
    }
    for (i, c) in text.iter().enumerate() {
        // SAFETY: a fresh zeroed block of 37 units.
        unsafe {
            if wide {
                (block as *mut u16).add(i).write_unaligned(u16::from(*c));
            } else {
                (block as *mut u8).add(i).write(*c);
            }
        }
    }
    clear(out);
    // SAFETY: the caller's string out-pointer.
    unsafe { (out as *mut usize).write_unaligned(block) };
    RPC_S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn UuidToStringA(uuid: usize, out: usize) -> u32 {
    to_string(uuid, out, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn UuidToStringW(uuid: usize, out: usize) -> u32 {
    to_string(uuid, out, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RpcStringFreeA(string: usize) -> u32 {
    if string != 0 {
        // SAFETY: the caller's pointer to an RPC string.
        let block = unsafe { (string as *const usize).read_unaligned() };
        if block != 0 {
            unsafe { HeapFree(GetProcessHeap(), 0, block) };
        }
        clear(string);
    }
    RPC_S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn RpcStringFreeW(string: usize) -> u32 {
    RpcStringFreeA(string)
}
