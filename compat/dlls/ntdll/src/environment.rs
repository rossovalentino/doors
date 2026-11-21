//! The process environment block (PEB.ProcessParameters.Environment):
//! the session's variables plus those the launcher passed, behind
//! kernel32's Get/Set/ExpandEnvironmentStrings and GetEnvironmentStrings.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_windows_abi::{self as abi, environment::Block};

struct Shared {
    held: AtomicBool,
    block: UnsafeCell<Block>,
}
// SAFETY: `held` serialises every access to `block`.
unsafe impl Sync for Shared {}
static ENVIRONMENT: Shared = Shared {
    held: AtomicBool::new(false),
    block: UnsafeCell::new(Block::new()),
};
fn with<R>(f: impl FnOnce(&mut Block) -> R) -> R {
    while ENVIRONMENT.held.swap(true, Ordering::Acquire) {
        let _ = door_sys::yield_now();
    }
    // SAFETY: the flag gives this closure the only access.
    let r = f(unsafe { &mut *ENVIRONMENT.block.get() });
    ENVIRONMENT.held.store(false, Ordering::Release);
    r
}
const ENVVAR_NOT_FOUND: u32 = 203;
const INVALID_PARAMETER: u32 = 87;
const NOT_ENOUGH_MEMORY: u32 = 8;
const FILENAME_EXCED_RANGE: u32 = 206;
/// The longest string argument these functions take (in UTF-16 units).
const ARGUMENT_UNITS: usize = 4096;

/// Builds the block: the session's variables, then the launcher's (native
/// paths such as PWD=/users/... stay native-only).
pub(super) fn initialize(native: &[u8]) {
    let user = native
        .split(|b| *b == 0)
        .find_map(|e| e.strip_prefix(b"USERNAME="))
        .and_then(|v| core::str::from_utf8(v).ok())
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .unwrap_or("User");
    let mut name = [0u8; 64];
    let mut length = 0u32;
    let computer = if super::files::DoorGetComputerName(
        name.as_mut_ptr() as u64,
        64,
        &mut length as *mut u32 as u64,
    ) == 0
    {
        core::str::from_utf8(&name[..(length as usize).min(64)]).unwrap_or("DOORS")
    } else {
        "DOORS"
    };
    // The profile folder storage gives the account: C:\Users\<uid>.
    let mut identity = [0u8; door_abi::platform::IDENTITY_BYTES];
    let _ = super::guest::DoorGetIdentity(identity.as_mut_ptr() as u64);
    let uid = door_abi::platform::Identity::decode(&identity).map_or(0, |i| i.uid);
    let mut folder = [0u8; 20];
    folder[..9].copy_from_slice(b"C:\\Users\\");
    let mut digits = [0u8; 10];
    let mut count = 0;
    let mut n = uid;
    loop {
        digits[count] = b'0' + (n % 10) as u8;
        n /= 10;
        count += 1;
        if n == 0 {
            break;
        }
    }
    for i in 0..count {
        folder[9 + i] = digits[count - 1 - i];
    }
    let profile = core::str::from_utf8(&folder[..9 + count]).unwrap_or("C:\\Users");
    // The service account (a Windows service): LocalSystem's profile, and
    // USERNAME the computer account's name, as Windows sets them.
    let mut machine = [0u8; 66];
    let (user, profile) = if uid == door_abi::platform::SERVICE_ACCOUNT {
        let n = computer.len().min(64);
        machine[..n].copy_from_slice(&computer.as_bytes()[..n]);
        machine[n] = b'$';
        (
            core::str::from_utf8(&machine[..n + 1]).unwrap_or("SYSTEM"),
            "C:\\Windows\\system32\\config\\systemprofile",
        )
    } else {
        (user, profile)
    };
    with(|b| {
        let _ = abi::environment::session_defaults(b, user, profile, computer, "1");
        for entry in native.split(|b| *b == 0) {
            let Ok(text) = core::str::from_utf8(entry) else {
                continue;
            };
            let Some((name, value)) = text.split_once('=') else {
                continue;
            };
            if name.is_empty()
                || value.starts_with('/')
                || matches!(name, "PWD" | "USER" | "HOME")
                // The console a Terminal tab gave the program (Init's, not
                // the program's environment).
                || name == "DOOR_CONSOLE"
                || name == "DOOR_IMAGE"
                || text == door_service_protocol::application::LONG_ARGUMENTS_ENVIRONMENT
            {
                continue;
            }
            let _ = b.set_text(name, &[value]);
        }
        width_view(b);
    });
    let pointer = with(|b| b.as_ptr() as u64);
    // PEB.ProcessParameters.Environment (offset 0x80).
    let _ = super::guest::copy(
        abi::PARAMETERS_ADDRESS + 0x80,
        &pointer as *const u64 as u64,
        8,
    );
    // A 32-bit process: its PEB32's parameters too (P17).
    #[cfg(target_arch = "x86")]
    {
        let low = pointer as u32;
        let _ = super::guest::copy(
            abi::PARAMETERS32_ADDRESS + abi::x86::PARAMETERS32_ENVIRONMENT as u64,
            &low as *const u32 as u64,
            4,
        );
    }
}
/// The launcher's environment block (CreateProcess lpEnvironment, as
/// NUL-separated UTF-8 entries) replaces the session's, as on Windows.
pub(super) fn replace(entries: &[u8]) {
    with(|b| {
        *b = Block::new();
        for entry in entries.split(|c| *c == 0) {
            let Ok(text) = core::str::from_utf8(entry) else {
                continue;
            };
            // "=C:=C:\x": the name may start with '='.
            let Some(at) = text
                .char_indices()
                .skip(1)
                .find(|(_, c)| *c == '=')
                .map(|(i, _)| i)
            else {
                continue;
            };
            let mut name = [0u16; 256];
            let mut value = [0u16; 2048];
            let mut n = 0;
            let mut v = 0;
            let mut fits = true;
            for unit in text[..at].encode_utf16() {
                match name.get_mut(n) {
                    Some(slot) => *slot = unit,
                    None => fits = false,
                }
                n += 1;
            }
            for unit in text[at + 1..].encode_utf16() {
                match value.get_mut(v) {
                    Some(slot) => *slot = unit,
                    None => fits = false,
                }
                v += 1;
            }
            if fits {
                let _ = b.set(&name[..n], Some(&value[..v]));
            }
        }
        width_view(b);
    });
}
/// The variables WOW64 gives a process of this build's width (P17): a
/// 32-bit process sees ProgramFiles as C:\Program Files (x86), x86 and
/// PROCESSOR_ARCHITEW6432; a 64-bit one a 32-bit parent started gets the
/// 64-bit values back.
fn width_view(b: &mut Block) {
    let _ = abi::wow64::environment(b, cfg!(target_arch = "x86"));
}
/// A NUL-terminated wide string argument into `out`; its length.
fn argument(pointer: u64, out: &mut [u16]) -> Result<usize, u32> {
    if pointer == 0 {
        return Err(INVALID_PARAMETER);
    }
    let mut length = 0;
    loop {
        let at = pointer + 2 * length as u64;
        // Never read past the page the string may end on.
        let n = ((4096 - (at % 4096)) as usize / 2).clamp(1, 256);
        let mut chunk = [0u16; 256];
        super::guest::copy(chunk.as_mut_ptr() as u64, at, 2 * n).map_err(|_| INVALID_PARAMETER)?;
        for &u in &chunk[..n] {
            if u == 0 {
                return Ok(length);
            }
            *out.get_mut(length).ok_or(FILENAME_EXCED_RANGE)? = u;
            length += 1;
        }
    }
}
/// `units` to the caller's buffer at `pointer` (any length: the bulk copy
/// splits it; `guest::copy` takes at most 1 KiB per call).
fn put(pointer: u64, units: &[u16]) -> Result<(), u32> {
    super::guest::copy_bulk(pointer, units.as_ptr() as u64, 2 * units.len())
        .map_err(|_| INVALID_PARAMETER)
}
fn finish(result: Result<u32, u32>) -> u32 {
    let (value, code) = match result {
        Ok(v) => (v, 0),
        Err(e) => (0, e),
    };
    super::guest::RtlSetLastWin32Error(code);
    value
}
/// GetEnvironmentVariableW: the value's length; the size needed (with the
/// NUL) when `size` is too small; 0 and ERROR_ENVVAR_NOT_FOUND.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32 {
    let mut n = [0u16; 512];
    finish((|| {
        let length = argument(name, &mut n)?;
        with(|b| {
            let v = b.get(&n[..length]).ok_or(ENVVAR_NOT_FOUND)?;
            if v.len() + 1 > size as usize || buffer == 0 {
                return Ok(v.len() as u32 + 1);
            }
            put(buffer, v)?;
            put(buffer + 2 * v.len() as u64, &[0])?;
            Ok(v.len() as u32)
        })
    })())
}
/// SetEnvironmentVariableW: a null value removes the variable.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetEnvironmentVariableW(name: u64, value: u64) -> i32 {
    let mut n = [0u16; 512];
    let mut v = [0u16; ARGUMENT_UNITS];
    finish((|| {
        let length = argument(name, &mut n)?;
        if !abi::environment::valid_name(&n[..length]) {
            return Err(INVALID_PARAMETER);
        }
        let value = if value == 0 {
            None
        } else {
            let length = argument(value, &mut v)?;
            Some(&v[..length])
        };
        with(|b| b.set(&n[..length], value)).map_err(|_| NOT_ENOUGH_MEMORY)?;
        Ok(1)
    })()) as i32
}
/// ExpandEnvironmentStringsW: the size the result needs, with its NUL
/// (written only when it fits `size`).
#[unsafe(no_mangle)]
pub extern "system" fn DoorExpandEnvironmentStringsW(
    source: u64,
    destination: u64,
    size: u32,
) -> u32 {
    let mut s = [0u16; ARGUMENT_UNITS];
    let mut out = [0u16; ARGUMENT_UNITS];
    finish((|| {
        let length = argument(source, &mut s)?;
        let mut at = 0;
        let count = with(|b| {
            b.expand(&s[..length], |u| {
                if let Some(slot) = out.get_mut(at) {
                    *slot = u;
                }
                at += 1;
            })
        });
        if count > out.len() {
            return Err(FILENAME_EXCED_RANGE);
        }
        if destination != 0 && count < size as usize {
            put(destination, &out[..count])?;
            put(destination + 2 * count as u64, &[0])?;
        }
        Ok(count as u32 + 1)
    })())
}
/// GetEnvironmentStringsW: a process-heap copy of the block.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetEnvironmentStringsW() -> u64 {
    door_windows_abi::widen(with(|b| {
        let units = b.strings();
        let heap = super::heap::DoorGetProcessHeap();
        let p = super::heap::RtlAllocateHeap(door_windows_abi::narrow(heap), 0, 2 * units.len());
        if p != 0 && put(door_windows_abi::widen(p), units).is_err() {
            super::heap::RtlFreeHeap(door_windows_abi::narrow(heap), 0, p);
            return 0;
        }
        p
    }))
}
