//! advapi32.dll exports: registry functions over the storage service's
//! hives, token and SID functions over the process identity, privileges,
//! and RtlGenRandom (SystemFunction036).
use crate::{Sid, Token, error, hkey, sam};
use alloc::{string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_service_client::{Error, RegistryRoot, RegistryValue, RegistryView, Storage};
use door_service_protocol::Code;
mod crypt;
mod etw;
mod eventlog;
mod safer;
mod sandbox;
mod scm;
mod security;

struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure the only access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorGetInheritedHandle(index: u32) -> u64;
    fn DoorGetIdentity(out: u64) -> u32;
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
    fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32;
    fn DoorCreateEventW(attributes: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlSetLastWin32Error(code: u32);
    fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorGetNativePath(path: u64, out: u64, capacity: u32) -> u32;
    fn DoorGetFileSecurity(handle: u64, out: u64) -> i32;
    fn DoorSetFileSecurity(handle: u64, record: u64) -> i32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorCreateDirectoryW(path: u64) -> i32;
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorDeleteFileW(path: u64) -> i32;
    fn RtlGetLastWin32Error() -> u32;
    fn DoorReadFile(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorWriteFile(handle: u64, source: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
}

// ---- application memory -------------------------------------------------------
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 && !out.is_empty() {
        return Err(error::INVALID_PARAMETER);
    }
    for (i, chunk) in out.chunks_mut(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                chunk.as_mut_ptr() as u64,
                pointer + (i * COPY_CHUNK) as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(error::INVALID_PARAMETER);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 && !data.is_empty() {
        return Err(error::INVALID_PARAMETER);
    }
    for (i, chunk) in data.chunks(COPY_CHUNK).enumerate() {
        if unsafe {
            DoorUserCopy(
                pointer + (i * COPY_CHUNK) as u64,
                chunk.as_ptr() as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(error::INVALID_PARAMETER);
        }
    }
    Ok(())
}
fn read_u32(p: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(p, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn write_u32(p: u64, v: u32) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
/// A handle, pointer or other pointer-sized value an out-parameter receives,
/// at the program's width (P17: 8 bytes on x64, 4 on x86).
fn write_pointer(p: u64, v: u64) -> Result<(), u32> {
    write(p, &v.to_le_bytes()[..POINTER])
}
/// A pointer-sized value the program stored at `p`, widened.
fn read_pointer(p: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(p, &mut b[..POINTER])?;
    Ok(if POINTER == 4 {
        i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as u64
    } else {
        u64::from_le_bytes(b)
    })
}
/// A genuine 64-bit value (LUID, FILETIME): 8 bytes in either build.
fn write_qword(p: u64, v: u64) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
const POINTER: usize = door_windows_abi::layout::POINTER;
/// A NUL-terminated string argument (wide or ANSI) as UTF-16 units.
fn text(p: u64, wide: bool) -> Result<Vec<u16>, u32> {
    let mut out = Vec::new();
    if p == 0 {
        return Ok(out);
    }
    for i in 0..32768u64 {
        let u = if wide {
            let mut b = [0u8; 2];
            read(p + 2 * i, &mut b)?;
            u16::from_le_bytes(b)
        } else {
            let mut b = [0u8; 1];
            read(p + i, &mut b)?;
            u16::from(b[0])
        };
        if u == 0 {
            return Ok(out);
        }
        out.push(u);
    }
    Err(error::INVALID_PARAMETER)
}
/// Writes `units` and a NUL into a buffer of `capacity` characters.
fn put_text(out: u64, capacity: u32, units: &[u16], wide: bool) -> Result<(), u32> {
    if units.len() + 1 > capacity as usize {
        return Err(error::MORE_DATA);
    }
    let bytes: Vec<u8> = if wide {
        units
            .iter()
            .chain([&0])
            .flat_map(|u| u.to_le_bytes())
            .collect()
    } else {
        units
            .iter()
            .chain([&0])
            .map(|u| if *u < 0x100 { *u as u8 } else { b'?' })
            .collect()
    };
    write(out, &bytes)
}
fn last_error(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    i32::from(code == error::SUCCESS)
}

// ---- the token --------------------------------------------------------------------
fn identity() -> Token {
    let mut b = [0u8; door_abi::platform::IDENTITY_BYTES];
    unsafe { DoorGetIdentity(b.as_mut_ptr() as u64) };
    use door_abi::platform::group;
    match door_abi::platform::Identity::decode(&b) {
        Ok(id) => Token::primary(
            id.uid,
            id.session as u32,
            id.groups & group::ADMINISTRATORS != 0,
            id.groups & group::ADMINISTRATOR_ACCOUNT != 0,
        ),
        Err(_) => Token::primary(0, 0, false, false),
    }
}
/// The signed-in account's name (the session's USERNAME).
fn user_name() -> String {
    // LocalSystem's name (USERNAME is the computer account's).
    if identity().uid == door_abi::platform::SERVICE_ACCOUNT {
        return String::from("SYSTEM");
    }
    let name: Vec<u16> = "USERNAME".encode_utf16().chain([0]).collect();
    let mut out = [0u16; 65];
    let n =
        unsafe { DoorGetEnvironmentVariableW(name.as_ptr() as u64, out.as_mut_ptr() as u64, 65) };
    if n == 0 || n > 64 {
        return String::from("User");
    }
    String::from_utf16_lossy(&out[..n as usize])
}
/// The caller's account for account names: (uid, name).
fn me() -> (u32, String) {
    (identity().uid, user_name())
}
/// A token object: which of the account's tokens it is and its privileges'
/// current attributes (AdjustTokenPrivileges changes the object, which
/// every handle to it sees).
struct TokenObject {
    token: Token,
    privileges: Vec<(u32, u32)>,
}
/// Token objects (index 0: the process token) and handles: a kernel event
/// per handle (so CloseHandle accepts it) mapped to its object.
static OBJECTS: Lock<Vec<TokenObject>> = Lock::new(Vec::new());
static TOKENS: Lock<Vec<(u64, usize)>> = Lock::new(Vec::new());
/// Impersonation: (thread id, token object).
static THREADS: Lock<Vec<(u32, usize)>> = Lock::new(Vec::new());
fn process_object() -> usize {
    OBJECTS.with(|o| {
        if o.is_empty() {
            let token = identity();
            o.push(TokenObject {
                token,
                privileges: token.privileges(),
            });
        }
    });
    0
}
fn new_object(token: Token, privileges: Vec<(u32, u32)>) -> usize {
    OBJECTS.with(|o| {
        o.push(TokenObject { token, privileges });
        o.len() - 1
    })
}
fn new_handle(object: usize) -> Result<u64, u32> {
    let handle = unsafe { DoorCreateEventW(0, 1, 0, 0) };
    if handle == 0 || handle == u64::MAX {
        return Err(error::NOT_ENOUGH_MEMORY);
    }
    TOKENS.with(|t| t.push((handle, object)));
    Ok(handle)
}
fn current_thread() -> u32 {
    unsafe { DoorGetCurrentThreadId() }
}
/// The object behind `handle`. GetCurrentProcessToken's pseudo handle (-4)
/// is the process token; GetCurrentThreadToken (-5) and
/// GetCurrentThreadEffectiveToken (-6) the thread's (-6 falls back to the
/// process token).
fn object_of(handle: u64) -> Result<usize, u32> {
    match handle {
        0xFFFF_FFFF_FFFF_FFFC => return Ok(process_object()),
        0xFFFF_FFFF_FFFF_FFFB | 0xFFFF_FFFF_FFFF_FFFA => {
            let thread = THREADS.with(|t| {
                t.iter()
                    .find(|(id, _)| *id == current_thread())
                    .map(|(_, o)| *o)
            });
            return match thread {
                Some(o) => Ok(o),
                None if handle == 0xFFFF_FFFF_FFFF_FFFA => Ok(process_object()),
                None => Err(error::NO_TOKEN),
            };
        }
        _ => (),
    }
    TOKENS
        .with(|t| t.iter().find(|(h, _)| *h == handle).map(|(_, o)| *o))
        .ok_or(error::INVALID_HANDLE)
}
fn token_of(handle: u64) -> Result<Token, u32> {
    let o = object_of(handle)?;
    Ok(OBJECTS.with(|objects| objects[o].token))
}
/// A copy of `object` as a new object of `kind` at impersonation `level`.
fn duplicate(object: usize, kind: u8, level: u8) -> usize {
    let (token, privileges) = OBJECTS.with(|o| (o[object].token, o[object].privileges.clone()));
    new_object(
        Token {
            kind,
            level: if kind == 2 { level } else { 0 },
            ..token
        },
        privileges,
    )
}
const CURRENT_PROCESS: u64 = u64::MAX;
const CURRENT_THREAD: u64 = u64::MAX - 1;
#[unsafe(no_mangle)]
pub extern "system" fn OpenProcessToken(process: usize, _access: u32, out: usize) -> i32 {
    let process = door_windows_abi::widen(process);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if process != CURRENT_PROCESS && process != 0xFFFF_FFFF {
                return Err(error::ACCESS_DENIED);
            }
            let handle = new_handle(process_object())?;
            write_pointer(out, handle)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenThreadToken(thread: usize, _access: u32, _self: i32, out: usize) -> i32 {
    let thread = door_windows_abi::widen(thread);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if thread != CURRENT_THREAD {
                return Err(error::INVALID_HANDLE);
            }
            let object = THREADS.with(|t| {
                t.iter()
                    .find(|(id, _)| *id == current_thread())
                    .map(|(_, o)| *o)
            });
            // A thread that does not impersonate has no token.
            let Some(object) = object else {
                write_pointer(out, 0)?;
                return Err(error::NO_TOKEN);
            };
            write_pointer(out, new_handle(object)?)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// The current thread impersonates `object` (None: reverts).
fn impersonate(object: Option<usize>) {
    let id = current_thread();
    THREADS.with(|t| {
        t.retain(|(thread, _)| *thread != id);
        if let Some(o) = object {
            t.push((id, o));
        }
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateSelf(level: u32) -> i32 {
    if level > 3 {
        return last_error(crate::security::code::BAD_IMPERSONATION_LEVEL);
    }
    let object = duplicate(process_object(), 2, level as u8);
    impersonate(Some(object));
    last_error(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RevertToSelf() -> i32 {
    impersonate(None);
    last_error(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadToken(thread: usize, token: usize) -> i32 {
    let thread = door_windows_abi::widen(thread);
    let token = door_windows_abi::widen(token);
    {
        let r = (|| {
            if thread != 0 && read_pointer(thread)? != CURRENT_THREAD {
                return Err(error::INVALID_HANDLE);
            }
            if token == 0 {
                impersonate(None);
                return Ok(());
            }
            let object = object_of(token)?;
            if OBJECTS.with(|o| o[object].token.kind) != 2 {
                return Err(crate::security::code::BAD_TOKEN_TYPE);
            }
            impersonate(Some(object));
            Ok(())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateLoggedOnUser(token: usize) -> i32 {
    let token = door_windows_abi::widen(token);
    {
        let r = (|| {
            let object = object_of(token)?;
            let object = if OBJECTS.with(|o| o[object].token.kind) == 2 {
                object
            } else {
                duplicate(object, 2, 2)
            };
            impersonate(Some(object));
            Ok(())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
fn privileges_of(object: usize) -> Vec<(u32, u32)> {
    OBJECTS.with(|o| o[object].privileges.clone())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTokenInformation(
    handle: usize,
    class: u32,
    out: usize,
    length: u32,
    needed: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    {
        let r = (|| {
            let object = object_of(handle)?;
            let token = OBJECTS.with(|o| o[object].token);
            if class == crate::class::LINKED_TOKEN {
                // TOKEN_LINKED_TOKEN: exactly one handle (ERROR_BAD_LENGTH
                // otherwise, as Windows): an identification token of the
                // administrator's other token. A standard user has none.
                write_u32(needed, 8)?;
                if length != 8 {
                    return Err(crate::security::code::BAD_LENGTH);
                }
                if !token.administrator {
                    return Err(error::NO_SUCH_LOGON_SESSION);
                }
                let other = Token {
                    elevated: !token.elevated,
                    kind: 2,
                    level: 1,
                    ..token
                };
                let linked = new_object(other, other.privileges());
                return write_pointer(out, new_handle(linked)?);
            }
            let record = if class == crate::class::PRIVILEGES {
                crate::privileges_record(&privileges_of(object))
            } else {
                crate::token_information(&token, class, out, core::mem::size_of::<usize>())?
            };
            write_u32(needed, record.len() as u32)?;
            if (length as usize) < record.len() {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &record)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DuplicateToken(handle: usize, level: u32, out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if level > 3 {
                return Err(crate::security::code::BAD_IMPERSONATION_LEVEL);
            }
            let object = duplicate(object_of(handle)?, 2, level as u8);
            write_pointer(out, new_handle(object)?)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DuplicateTokenEx(
    handle: usize,
    _access: u32,
    _attributes: usize,
    level: u32,
    kind: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if !(1..=2).contains(&kind) || level > 3 {
                return Err(error::INVALID_PARAMETER);
            }
            let object = duplicate(object_of(handle)?, kind as u8, level as u8);
            write_pointer(out, new_handle(object)?)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
fn read_sid(p: u64) -> Result<Sid, u32> {
    let mut head = [0u8; 8];
    read(p, &mut head)?;
    let mut b = vec![0u8; 8 + 4 * usize::from(head[1].min(15))];
    read(p, &mut b)?;
    Sid::decode(&b).ok_or(error::INVALID_SID)
}
/// The token an access check uses: `handle` must be an impersonation token;
/// none means the thread's, else the process token.
fn checking_token(handle: u64) -> Result<Token, u32> {
    if handle == 0 {
        return token_of(0xFFFF_FFFF_FFFF_FFFA);
    }
    let token = token_of(handle)?;
    if token.kind != 2 {
        return Err(crate::security::code::NO_IMPERSONATION_TOKEN);
    }
    if token.level < 1 {
        return Err(crate::security::code::BAD_IMPERSONATION_LEVEL);
    }
    Ok(token)
}
#[unsafe(no_mangle)]
pub extern "system" fn CheckTokenMembership(handle: usize, sid: usize, out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let sid = door_windows_abi::widen(sid);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let token = checking_token(handle)?;
            let sid = read_sid(sid)?;
            write_u32(out, u32::from(token.member(&sid)))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn AdjustTokenPrivileges(
    handle: usize,
    disable_all: i32,
    new: usize,
    length: u32,
    previous: usize,
    needed: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let new = door_windows_abi::widen(new);
    let previous = door_windows_abi::widen(previous);
    let needed = door_windows_abi::widen(needed);
    {
        const ENABLED: u32 = crate::PRIVILEGE_ENABLED;
        let r = (|| {
            let object = object_of(handle)?;
            let mut privileges = privileges_of(object);
            // (luid, wanted attributes) for each privilege the call names.
            let mut wanted: Vec<(u32, u32)> = Vec::new();
            if disable_all != 0 {
                wanted.extend(privileges.iter().map(|(l, _)| (*l, 0)));
            } else {
                if new == 0 {
                    return Err(error::INVALID_PARAMETER);
                }
                let count = read_u32(new)?;
                for i in 0..u64::from(count.min(64)) {
                    let luid = read_u32(new + 4 + i * 12)?;
                    let attributes = read_u32(new + 12 + i * 12)?;
                    wanted.push((luid, attributes & ENABLED));
                }
            }
            let mut changed: Vec<(u32, u32)> = Vec::new();
            let mut all = true;
            for (luid, on) in &wanted {
                match privileges.iter_mut().find(|(l, _)| l == luid) {
                    Some((_, attributes)) => {
                        if *attributes & ENABLED != *on {
                            changed.push((*luid, *attributes));
                            *attributes = (*attributes & !ENABLED) | on;
                        }
                    }
                    None => all = false,
                }
            }
            // PreviousState: the changed privileges' former attributes.
            let record = crate::privileges_record(&changed);
            if needed != 0 {
                write_u32(needed, record.len() as u32)?;
            }
            if previous != 0 {
                if (length as usize) < record.len() {
                    return Err(error::INSUFFICIENT_BUFFER);
                }
                write(previous, &record)?;
            }
            OBJECTS.with(|o| o[object].privileges = privileges);
            // Privileges the token lacks are not assigned; the call still
            // succeeds, as Windows'.
            Ok(if all {
                error::SUCCESS
            } else {
                error::NOT_ALL_ASSIGNED
            })
        })();
        match r {
            Ok(code) => {
                unsafe { RtlSetLastWin32Error(code) };
                1
            }
            Err(e) => last_error(e),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupPrivilegeValueW(_system: usize, name: usize, luid: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let luid = door_windows_abi::widen(luid);
    lookup_privilege(name, luid, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupPrivilegeValueA(_system: usize, name: usize, luid: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let luid = door_windows_abi::widen(luid);
    lookup_privilege(name, luid, false)
}
fn lookup_privilege(name: u64, luid: u64, wide: bool) -> i32 {
    let r = (|| {
        let name = String::from_utf16_lossy(&text(name, wide)?);
        let value = crate::privilege_luid(&name).ok_or(error::NO_SUCH_PRIVILEGE)?;
        write_qword(luid, u64::from(value))
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupPrivilegeNameW(
    _system: usize,
    luid: usize,
    out: usize,
    length: usize,
) -> i32 {
    let luid = door_windows_abi::widen(luid);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let mut b = [0u8; 8];
            read(luid, &mut b)?;
            let name = crate::privilege_name(u32::from_le_bytes(b[..4].try_into().unwrap()))
                .ok_or(error::NO_SUCH_PRIVILEGE)?;
            let units: Vec<u16> = name.encode_utf16().collect();
            let capacity = read_u32(length)?;
            if units.len() + 1 > capacity as usize {
                write_u32(length, units.len() as u32 + 1)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            put_text(out, capacity, &units, true)?;
            write_u32(length, units.len() as u32)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}

// ---- SIDs ----------------------------------------------------------------------------
/// A SID in process-heap memory (FreeSid and LocalFree release it).
fn alloc_sid(sid: &Sid) -> Result<u64, u32> {
    let bytes = sid.encode();
    let p = unsafe {
        RtlAllocateHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            0,
            bytes.len(),
        )
    };
    if p == 0 {
        return Err(error::NOT_ENOUGH_MEMORY);
    }
    write(door_windows_abi::widen(p), &bytes)?;
    Ok(door_windows_abi::widen(p))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn AllocateAndInitializeSid(
    authority: usize,
    count: u8,
    s0: u32,
    s1: u32,
    s2: u32,
    s3: u32,
    s4: u32,
    s5: u32,
    s6: u32,
    s7: u32,
    out: usize,
) -> i32 {
    let authority = door_windows_abi::widen(authority);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if count == 0 || count > 8 {
                return Err(error::INVALID_PARAMETER);
            }
            let mut a = [0u8; 8];
            read(authority, &mut a[2..])?;
            let sub = [s0, s1, s2, s3, s4, s5, s6, s7];
            let p = alloc_sid(&Sid::new(u64::from_be_bytes(a), &sub[..usize::from(count)]))?;
            write_pointer(out, p)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeSid(sid: usize) -> usize {
    let sid = door_windows_abi::widen(sid);
    door_windows_abi::narrow((move || -> u64 {
        if sid != 0 {
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(DoorGetProcessHeap()),
                    0,
                    door_windows_abi::narrow(sid),
                )
            };
        }
        0
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidSid(sid: usize) -> i32 {
    let sid = door_windows_abi::widen(sid);
    i32::from(read_sid(sid).is_ok())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLengthSid(sid: usize) -> u32 {
    let sid = door_windows_abi::widen(sid);
    read_sid(sid).map_or(0, |s| s.length() as u32)
}
#[unsafe(no_mangle)]
pub extern "system" fn EqualSid(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        match (read_sid(a), read_sid(b)) {
            (Ok(a), Ok(b)) => {
                unsafe { RtlSetLastWin32Error(error::SUCCESS) };
                i32::from(a == b)
            }
            _ => last_error(error::INVALID_SID),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CopySid(length: u32, out: usize, sid: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    let sid = door_windows_abi::widen(sid);
    {
        let r = read_sid(sid).and_then(|s| {
            if (length as usize) < s.length() {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &s.encode())
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthorityCount(sid: usize) -> usize {
    let sid = door_windows_abi::widen(sid);
    door_windows_abi::narrow((move || -> u64 { sid + 1 })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSidSubAuthority(sid: usize, index: u32) -> usize {
    let sid = door_windows_abi::widen(sid);
    door_windows_abi::narrow((move || -> u64 { sid + 8 + 4 * u64::from(index) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSidIdentifierAuthority(sid: usize) -> usize {
    let sid = door_windows_abi::widen(sid);
    door_windows_abi::narrow((move || -> u64 { sid + 2 })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertSidToStringSidW(sid: usize, out: usize) -> i32 {
    let sid = door_windows_abi::widen(sid);
    let out = door_windows_abi::widen(out);
    sid_to_string(sid, out, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertSidToStringSidA(sid: usize, out: usize) -> i32 {
    let sid = door_windows_abi::widen(sid);
    let out = door_windows_abi::widen(out);
    sid_to_string(sid, out, false)
}
fn sid_to_string(sid: u64, out: u64, wide: bool) -> i32 {
    let r = (|| {
        let units: Vec<u16> = read_sid(sid)?.text().encode_utf16().collect();
        let bytes = (units.len() + 1) * if wide { 2 } else { 1 };
        let p =
            unsafe { RtlAllocateHeap(door_windows_abi::narrow(DoorGetProcessHeap()), 0, bytes) };
        if p == 0 {
            return Err(error::NOT_ENOUGH_MEMORY);
        }
        put_text(
            door_windows_abi::widen(p),
            units.len() as u32 + 1,
            &units,
            wide,
        )?;
        write_pointer(out, door_windows_abi::widen(p))
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertStringSidToSidW(text_sid: usize, out: usize) -> i32 {
    let text_sid = door_windows_abi::widen(text_sid);
    let out = door_windows_abi::widen(out);
    string_to_sid(text_sid, out, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn ConvertStringSidToSidA(text_sid: usize, out: usize) -> i32 {
    let text_sid = door_windows_abi::widen(text_sid);
    let out = door_windows_abi::widen(out);
    string_to_sid(text_sid, out, false)
}
fn string_to_sid(text_sid: u64, out: u64, wide: bool) -> i32 {
    let r = (|| {
        let s = String::from_utf16_lossy(&text(text_sid, wide)?);
        // "S-1-..." or an SDDL alias ("BA", "LA", ...), as Windows accepts.
        let sid = crate::security::parse_sid(&s).ok_or(error::INVALID_SID)?;
        let p = alloc_sid(&sid)?;
        write_pointer(out, p)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
fn computer_name() -> Vec<u16> {
    let mut bytes = [0u8; 64];
    let mut length = 0u32;
    if unsafe {
        DoorGetComputerName(
            bytes.as_mut_ptr() as u64,
            64,
            &mut length as *mut u32 as u64,
        )
    } != 0
    {
        return "DOORS".encode_utf16().collect();
    }
    bytes[..(length as usize).min(64)]
        .iter()
        .map(|b| u16::from(*b))
        .collect()
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupAccountSidW(
    system: usize,
    sid: usize,
    name: usize,
    name_len: usize,
    domain: usize,
    domain_len: usize,
    use_out: usize,
) -> i32 {
    let system = door_windows_abi::widen(system);
    let sid = door_windows_abi::widen(sid);
    let name = door_windows_abi::widen(name);
    let name_len = door_windows_abi::widen(name_len);
    let domain = door_windows_abi::widen(domain);
    let domain_len = door_windows_abi::widen(domain_len);
    let use_out = door_windows_abi::widen(use_out);
    {
        lookup_account_sid(
            system, sid, name, name_len, domain, domain_len, use_out, true,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LookupAccountSidA(
    system: usize,
    sid: usize,
    name: usize,
    name_len: usize,
    domain: usize,
    domain_len: usize,
    use_out: usize,
) -> i32 {
    let system = door_windows_abi::widen(system);
    let sid = door_windows_abi::widen(sid);
    let name = door_windows_abi::widen(name);
    let name_len = door_windows_abi::widen(name_len);
    let domain = door_windows_abi::widen(domain);
    let domain_len = door_windows_abi::widen(domain_len);
    let use_out = door_windows_abi::widen(use_out);
    {
        lookup_account_sid(
            system, sid, name, name_len, domain, domain_len, use_out, false,
        )
    }
}
#[allow(clippy::too_many_arguments)]
fn lookup_account_sid(
    _system: u64,
    sid: u64,
    name: u64,
    name_len: u64,
    domain: u64,
    domain_len: u64,
    use_out: u64,
    wide: bool,
) -> i32 {
    let r = (|| {
        let sid = read_sid(sid)?;
        let (uid, own) = me();
        let (dom, account, kind) = sid.account((uid, &own)).ok_or(error::NONE_MAPPED)?;
        let account: Vec<u16> = account.encode_utf16().collect();
        let dom: Vec<u16> = if dom == "DOORS" {
            computer_name()
        } else {
            dom.encode_utf16().collect()
        };
        let (have_name, have_domain) = (read_u32(name_len)?, read_u32(domain_len)?);
        if (have_name as usize) < account.len() + 1 || (have_domain as usize) < dom.len() + 1 {
            write_u32(name_len, account.len() as u32 + 1)?;
            write_u32(domain_len, dom.len() as u32 + 1)?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        put_text(name, have_name, &account, wide)?;
        put_text(domain, have_domain, &dom, wide)?;
        write_u32(name_len, account.len() as u32)?;
        write_u32(domain_len, dom.len() as u32)?;
        write_u32(use_out, kind)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserNameW(out: usize, length: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    user_name_into(out, length, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserNameA(out: usize, length: usize) -> i32 {
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    user_name_into(out, length, false)
}
fn user_name_into(out: u64, length: u64, wide: bool) -> i32 {
    let r = (|| {
        let units: Vec<u16> = user_name().encode_utf16().collect();
        let capacity = read_u32(length)?;
        write_u32(length, units.len() as u32 + 1)?;
        if (capacity as usize) < units.len() + 1 {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        put_text(out, capacity, &units, wide)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
/// RtlGenRandom: the kernel entropy source.
#[unsafe(no_mangle)]
pub extern "system" fn SystemFunction036(buffer: usize, length: u32) -> u8 {
    let buffer = door_windows_abi::widen(buffer);
    u8::from(unsafe { DoorGenRandom(buffer, length) } == 0)
}

// ---- the registry ---------------------------------------------------------------
static STORAGE: Lock<Option<Storage>> = Lock::new(None);
fn storage<R>(f: impl FnOnce(&mut Storage) -> Result<R, u32>) -> Result<R, u32> {
    STORAGE.with(|s| {
        if s.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *s = Some(Storage::connect(registry).map_err(|_| error::ACCESS_DENIED)?);
        }
        let result = f(s.as_mut().unwrap());
        if s.as_ref().is_some_and(Storage::needs_reconnect) {
            *s = None;
        }
        result
    })
}
fn win32(e: Error) -> u32 {
    match e {
        Error::Service(Code::NotFound) => error::FILE_NOT_FOUND,
        Error::Service(Code::Denied) => error::ACCESS_DENIED,
        Error::Service(Code::InvalidRequest) => error::INVALID_PARAMETER,
        Error::Service(Code::Exists) => error::CANTWRITE,
        Error::Service(Code::Resources) => error::NOT_ENOUGH_MEMORY,
        Error::Service(Code::InvalidHandle) => error::INVALID_HANDLE,
        _ => error::BADKEY,
    }
}
/// An open key: its hive, the path the program named (below the hive),
/// whether it is in the 32-bit view (P17: the WOW6432Node mapping applies
/// when storage is asked), and the access it was opened for.
#[derive(Clone)]
struct Open {
    root: RegistryRoot,
    view32: bool,
    path: Vec<u16>,
    access: u16,
}
/// This build is a 32-bit (WOW64) one: its keys default to the 32-bit view.
const WOW64: bool = cfg!(target_arch = "x86");
/// The key as storage holds it: the 32-bit view's keys live below
/// WOW6432Node (`door_windows_abi::wow64::registry_path`), in the one tree.
fn physical(open: &Open) -> Result<Vec<u16>, u32> {
    if !open.view32 {
        return Ok(open.path.clone());
    }
    let hive = match open.root {
        RegistryRoot::LocalMachine => 0,
        RegistryRoot::CurrentUser => 1,
        _ => 2,
    };
    let mut out = alloc::vec![0u16; open.path.len() + 2 * door_windows_abi::wow64::NODE.len() + 2];
    let n = door_windows_abi::wow64::registry_path(hive, &open.path, &mut out)
        .ok_or(error::BAD_PATHNAME)?;
    out.truncate(n);
    Ok(out)
}
static KEYS: Lock<Vec<Option<Open>>> = Lock::new(Vec::new());
const KEY_BASE: u64 = 0x0000_0000_0000_4000;
fn key_handle(open: Open) -> u64 {
    KEYS.with(|k| {
        let index = k.iter().position(Option::is_none).unwrap_or_else(|| {
            k.push(None);
            k.len() - 1
        });
        k[index] = Some(open);
        KEY_BASE + 4 * index as u64
    })
}
fn root_of(n: u8) -> RegistryRoot {
    match n {
        0 => RegistryRoot::LocalMachine,
        1 => RegistryRoot::CurrentUser,
        _ => RegistryRoot::ClassesRoot,
    }
}
/// The key an HKEY names, with `sub` joined.
fn resolve(key: u64, sub: &[u16], desired: u32) -> Result<Open, u32> {
    let explicit = desired & (sam::WOW64_32KEY | sam::WOW64_64KEY) != 0;
    let view32 = door_windows_abi::wow64::view32(desired, WOW64);
    if key & 0xFFFF_FFFF_8000_0000 == 0x8000_0000
        || key & 0xFFFF_FFFF_0000_0000 == 0xFFFF_FFFF_0000_0000
    {
        let sid = Sid::user(identity().uid).text();
        let (root, path) =
            crate::predefined(key & 0xFFFF_FFFF, &sid, sub).ok_or(error::FILE_NOT_FOUND)?;
        return Ok(Open {
            root: root_of(root),
            view32,
            path: crate::join(&[], &path),
            access: crate::storage_access(desired),
        });
    }
    let base = KEYS.with(|k| {
        key.checked_sub(KEY_BASE)
            .filter(|o| o % 4 == 0)
            .and_then(|o| k.get((o / 4) as usize).cloned().flatten())
    });
    let base = base.ok_or(error::INVALID_HANDLE)?;
    // A subkey stays in its parent's view unless the call names one.
    Ok(Open {
        path: crate::join(&base.path, sub),
        access: crate::storage_access(desired),
        view32: if explicit { view32 } else { base.view32 },
        ..base
    })
}
fn with_key<R>(
    open: &Open,
    create: bool,
    f: impl FnOnce(&mut Storage, &door_service_client::Key) -> Result<R, u32>,
) -> Result<R, u32> {
    storage(|s| {
        let path = physical(open)?;
        if path.len() > door_service_protocol::REGISTRY_PATH_UNITS {
            return Err(error::BAD_PATHNAME);
        }
        let key = s
            .registry_open(open.root, RegistryView::Bits64, &path, open.access, create)
            .map_err(win32)?;
        let result = f(s, &key);
        let _ = s.registry_close(key);
        result
    })
}
fn reg_open(
    key: u64,
    sub: u64,
    desired: u32,
    out: u64,
    wide: bool,
    create: bool,
    disposition: u64,
) -> u32 {
    let r = (|| {
        let sub = text(sub, wide)?;
        let open = resolve(key, &sub, desired)?;
        let existed = with_key(
            &Open {
                access: 1,
                ..open.clone()
            },
            false,
            |_, _| Ok(()),
        )
        .is_ok();
        if !existed && !create {
            return Err(error::FILE_NOT_FOUND);
        }
        // Check the requested access by opening with it.
        with_key(&open, create, |_, _| Ok(()))?;
        if disposition != 0 {
            write_u32(disposition, if existed { 2 } else { 1 })?;
        }
        write_pointer(out, key_handle(open))
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegOpenKeyExW(
    key: usize,
    sub: usize,
    _options: u32,
    desired: u32,
    out: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, desired, out, true, false, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegOpenKeyExA(
    key: usize,
    sub: usize,
    _options: u32,
    desired: u32,
    out: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, desired, out, false, false, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegOpenKeyW(key: usize, sub: usize, out: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, 0xF_003F, out, true, false, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegOpenKeyA(key: usize, sub: usize, out: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, 0xF_003F, out, false, false, 0)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegCreateKeyExW(
    key: usize,
    sub: usize,
    _reserved: u32,
    _class: usize,
    _options: u32,
    desired: u32,
    _security: usize,
    out: usize,
    disposition: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    let disposition = door_windows_abi::widen(disposition);
    {
        reg_open(
            key,
            sub,
            desired | sam::CREATE_SUB_KEY,
            out,
            true,
            true,
            disposition,
        )
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegCreateKeyExA(
    key: usize,
    sub: usize,
    _reserved: u32,
    _class: usize,
    _options: u32,
    desired: u32,
    _security: usize,
    out: usize,
    disposition: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    let disposition = door_windows_abi::widen(disposition);
    {
        reg_open(
            key,
            sub,
            desired | sam::CREATE_SUB_KEY,
            out,
            false,
            true,
            disposition,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegCreateKeyW(key: usize, sub: usize, out: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, 0xF_003F, out, true, true, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegCreateKeyA(key: usize, sub: usize, out: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let out = door_windows_abi::widen(out);
    reg_open(key, sub, 0xF_003F, out, false, true, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegCloseKey(key: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    {
        if key & 0xFFFF_FFFF_8000_0000 == 0x8000_0000 {
            return error::SUCCESS;
        }
        KEYS.with(|k| {
            match key
                .checked_sub(KEY_BASE)
                .filter(|o| o % 4 == 0)
                .and_then(|o| k.get_mut((o / 4) as usize))
            {
                Some(slot @ Some(_)) => {
                    *slot = None;
                    error::SUCCESS
                }
                _ => error::INVALID_HANDLE,
            }
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegFlushKey(key: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    {
        let r = resolve(key, &[], 0x2_0019).and_then(|_| storage(|s| s.flush().map_err(win32)));
        r.err().unwrap_or(error::SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegOpenCurrentUser(_desired: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        write_pointer(out, hkey::CURRENT_USER)
            .err()
            .unwrap_or(error::SUCCESS)
    }
}
/// A value as the caller asked for it: ANSI string types are converted.
fn value_bytes(kind: u32, data: &[u8], wide: bool) -> Vec<u8> {
    if !wide
        && matches!(
            kind,
            crate::kind::SZ | crate::kind::EXPAND_SZ | crate::kind::MULTI_SZ
        )
    {
        crate::wide_to_ansi(data)
    } else {
        data.to_vec()
    }
}
fn reg_query(
    key: u64,
    sub: &[u16],
    name: u64,
    kind_out: u64,
    data: u64,
    size: u64,
    wide: bool,
) -> u32 {
    let r = (|| {
        let name = text(name, wide)?;
        let open = resolve(key, sub, sam::QUERY_VALUE)?;
        let value = with_key(&open, false, |s, k| {
            s.registry_query(k, &name).map_err(win32)
        })?;
        let kind = u32::from(value.kind);
        let bytes = value_bytes(kind, value.data(), wide);
        if kind_out != 0 {
            write_u32(kind_out, kind)?;
        }
        if size == 0 {
            return if data == 0 {
                Ok(())
            } else {
                Err(error::INVALID_PARAMETER)
            };
        }
        let capacity = read_u32(size)?;
        write_u32(size, bytes.len() as u32)?;
        if data == 0 {
            return Ok(());
        }
        if (capacity as usize) < bytes.len() {
            return Err(error::MORE_DATA);
        }
        write(data, &bytes)
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegQueryValueExW(
    key: usize,
    name: usize,
    _reserved: usize,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_query(key, &[], name, kind, data, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegQueryValueExA(
    key: usize,
    name: usize,
    _reserved: usize,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_query(key, &[], name, kind, data, size, false)
}
/// RRF_RT_* filter bits of RegGetValue.
fn type_allowed(flags: u32, kind: u32) -> bool {
    let bit = match kind {
        crate::kind::NONE => 0x1,
        crate::kind::SZ => 0x2,
        crate::kind::EXPAND_SZ => 0x4,
        crate::kind::BINARY => 0x8,
        crate::kind::DWORD => 0x10,
        crate::kind::MULTI_SZ => 0x20,
        crate::kind::QWORD => 0x40,
        _ => 0,
    };
    flags & 0xFFFF == 0xFFFF || flags & bit != 0
}
#[allow(clippy::too_many_arguments)]
fn reg_get(
    key: u64,
    sub: u64,
    name: u64,
    flags: u32,
    kind_out: u64,
    data: u64,
    size: u64,
    wide: bool,
) -> u32 {
    let r = (|| {
        let sub = text(sub, wide)?;
        let name_units = text(name, wide)?;
        let open = resolve(key, &sub, sam::QUERY_VALUE | (flags & sam::WOW64_32KEY))?;
        let value = with_key(&open, false, |s, k| {
            s.registry_query(k, &name_units).map_err(win32)
        })?;
        let kind = u32::from(value.kind);
        if !type_allowed(flags, kind) {
            return Err(error::NOT_SUPPORTED);
        }
        if kind_out != 0 {
            write_u32(kind_out, kind)?;
        }
        let bytes = value_bytes(kind, value.data(), wide);
        if size != 0 {
            let capacity = read_u32(size)?;
            write_u32(size, bytes.len() as u32)?;
            if data != 0 {
                if (capacity as usize) < bytes.len() {
                    return Err(error::MORE_DATA);
                }
                write(data, &bytes)?;
            }
        }
        Ok(())
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegGetValueW(
    key: usize,
    sub: usize,
    name: usize,
    flags: u32,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let name = door_windows_abi::widen(name);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_get(key, sub, name, flags, kind, data, size, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegGetValueA(
    key: usize,
    sub: usize,
    name: usize,
    flags: u32,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let name = door_windows_abi::widen(name);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_get(key, sub, name, flags, kind, data, size, false)
}
fn reg_set(key: u64, sub: &[u16], name: u64, kind: u32, data: u64, size: u32, wide: bool) -> u32 {
    let r = (|| {
        let name = text(name, wide)?;
        let mut raw = vec![0u8; size as usize];
        read(data, &mut raw)?;
        if !wide
            && matches!(
                kind,
                crate::kind::SZ | crate::kind::EXPAND_SZ | crate::kind::MULTI_SZ
            )
        {
            raw = crate::ansi_to_wide(&raw);
        }
        let stored = crate::stored(kind, &raw)?;
        let value =
            RegistryValue::new(kind as u8, &stored).map_err(|_| error::INVALID_PARAMETER)?;
        let open = resolve(key, sub, sam::SET_VALUE)?;
        with_key(&open, !sub.is_empty(), |s, k| {
            s.registry_set(k, &name, value).map_err(win32)
        })
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegSetValueExW(
    key: usize,
    name: usize,
    _reserved: u32,
    kind: u32,
    data: usize,
    size: u32,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    reg_set(key, &[], name, kind, data, size, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegSetValueExA(
    key: usize,
    name: usize,
    _reserved: u32,
    kind: u32,
    data: usize,
    size: u32,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    reg_set(key, &[], name, kind, data, size, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegSetKeyValueW(
    key: usize,
    sub: usize,
    name: usize,
    kind: u32,
    data: usize,
    size: u32,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    {
        match text(sub, true) {
            Ok(sub) => reg_set(key, &sub, name, kind, data, size, true),
            Err(e) => e,
        }
    }
}
fn reg_delete_value(key: u64, name: u64, wide: bool) -> u32 {
    let r = (|| {
        let name = text(name, wide)?;
        let open = resolve(key, &[], sam::SET_VALUE)?;
        with_key(&open, false, |s, k| {
            s.registry_delete_value(k, &name).map_err(win32)
        })
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteValueW(key: usize, name: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    reg_delete_value(key, name, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteValueA(key: usize, name: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let name = door_windows_abi::widen(name);
    reg_delete_value(key, name, false)
}
fn reg_delete_key(key: u64, sub: u64, desired: u32, wide: bool) -> u32 {
    let r = (|| {
        let sub = text(sub, wide)?;
        if sub.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        let open = resolve(key, &sub, sam::DELETE | (desired & sam::WOW64_32KEY))?;
        with_key(&open, false, |s, k| s.registry_delete_key(k).map_err(win32))
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteKeyW(key: usize, sub: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    reg_delete_key(key, sub, 0, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteKeyA(key: usize, sub: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    reg_delete_key(key, sub, 0, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteKeyExW(
    key: usize,
    sub: usize,
    desired: u32,
    _reserved: u32,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    reg_delete_key(key, sub, desired, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteKeyExA(
    key: usize,
    sub: usize,
    desired: u32,
    _reserved: u32,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    reg_delete_key(key, sub, desired, false)
}
/// Deletes `sub` (or the key's contents when null) and every key below it.
fn delete_tree(open: &Open) -> Result<(), u32> {
    loop {
        let child = with_key(
            &Open {
                access: 1,
                ..open.clone()
            },
            false,
            |s, k| {
                let version = s.registry_version(k).map_err(win32)?;
                Ok(s.registry_enumerate_key(k, 0, version)
                    .ok()
                    .map(|n| n.units().to_vec()))
            },
        )?;
        let Some(child) = child else { break };
        let path = crate::join(&open.path, &child);
        delete_tree(&Open {
            path: path.clone(),
            access: 1 | 2 | 4,
            ..open.clone()
        })?;
        with_key(
            &Open {
                path,
                access: 1 | 2 | 4,
                ..open.clone()
            },
            false,
            |s, k| s.registry_delete_key(k).map_err(win32),
        )?;
    }
    // The key's own values.
    loop {
        let value = with_key(
            &Open {
                access: 1,
                ..open.clone()
            },
            false,
            |s, k| {
                let version = s.registry_version(k).map_err(win32)?;
                Ok(s.registry_enumerate_value(k, 0, version)
                    .ok()
                    .map(|e| e.name.units().to_vec()))
            },
        )?;
        let Some(name) = value else { break };
        with_key(
            &Open {
                access: 1 | 2,
                ..open.clone()
            },
            false,
            |s, k| s.registry_delete_value(k, &name).map_err(win32),
        )?;
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn RegDeleteTreeW(key: usize, sub: usize) -> u32 {
    let key = door_windows_abi::widen(key);
    let sub = door_windows_abi::widen(sub);
    {
        let r = (|| {
            let sub = text(sub, true)?;
            let open = resolve(key, &sub, 0xF_003F)?;
            delete_tree(&open)?;
            if !sub.is_empty() {
                with_key(
                    &Open {
                        access: 1 | 2 | 4,
                        ..open
                    },
                    false,
                    |s, k| s.registry_delete_key(k).map_err(win32),
                )?;
            }
            Ok(())
        })();
        r.err().unwrap_or(error::SUCCESS)
    }
}
fn put_name(units: &[u16], out: u64, length: u64, wide: bool) -> Result<(), u32> {
    let capacity = read_u32(length)?;
    if (capacity as usize) < units.len() + 1 {
        return Err(error::MORE_DATA);
    }
    put_text(out, capacity, units, wide)?;
    write_u32(length, units.len() as u32)
}
fn reg_enum_key(key: u64, index: u32, out: u64, length: u64, wide: bool) -> u32 {
    let r = (|| {
        let open = resolve(key, &[], sam::ENUMERATE_SUB_KEYS)?;
        let name = with_key(&open, false, |s, k| {
            let version = s.registry_version(k).map_err(win32)?;
            s.registry_enumerate_key(k, index, version)
                .map_err(|e| match e {
                    Error::Service(Code::NotFound) => error::NO_MORE_ITEMS,
                    e => win32(e),
                })
        })?;
        put_name(name.units(), out, length, wide)
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegEnumKeyExW(
    key: usize,
    index: u32,
    out: usize,
    length: usize,
    _reserved: usize,
    _class: usize,
    _class_len: usize,
    _time: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    reg_enum_key(key, index, out, length, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegEnumKeyExA(
    key: usize,
    index: u32,
    out: usize,
    length: usize,
    _reserved: usize,
    _class: usize,
    _class_len: usize,
    _time: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    reg_enum_key(key, index, out, length, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegEnumKeyW(key: usize, index: u32, out: usize, capacity: u32) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    {
        let mut length = capacity;
        reg_enum_key(key, index, out, &mut length as *mut u32 as u64, true)
    }
}
#[allow(clippy::too_many_arguments)]
fn reg_enum_value(
    key: u64,
    index: u32,
    out: u64,
    length: u64,
    kind: u64,
    data: u64,
    size: u64,
    wide: bool,
) -> u32 {
    let r = (|| {
        let open = resolve(key, &[], sam::QUERY_VALUE)?;
        let entry = with_key(&open, false, |s, k| {
            let version = s.registry_version(k).map_err(win32)?;
            s.registry_enumerate_value(k, index, version)
                .map_err(|e| match e {
                    Error::Service(Code::NotFound) => error::NO_MORE_ITEMS,
                    e => win32(e),
                })
        })?;
        put_name(entry.name.units(), out, length, wide)?;
        let value_kind = u32::from(entry.value.kind);
        if kind != 0 {
            write_u32(kind, value_kind)?;
        }
        let bytes = value_bytes(value_kind, entry.value.data(), wide);
        if size != 0 {
            let capacity = read_u32(size)?;
            write_u32(size, bytes.len() as u32)?;
            if data != 0 {
                if (capacity as usize) < bytes.len() {
                    return Err(error::MORE_DATA);
                }
                write(data, &bytes)?;
            }
        }
        Ok(())
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegEnumValueW(
    key: usize,
    index: u32,
    out: usize,
    length: usize,
    _reserved: usize,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_enum_value(key, index, out, length, kind, data, size, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegEnumValueA(
    key: usize,
    index: u32,
    out: usize,
    length: usize,
    _reserved: usize,
    kind: usize,
    data: usize,
    size: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    let kind = door_windows_abi::widen(kind);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    reg_enum_value(key, index, out, length, kind, data, size, false)
}
#[allow(clippy::too_many_arguments)]
fn reg_query_info(
    key: u64,
    subkeys: u64,
    max_subkey: u64,
    values: u64,
    max_name: u64,
    max_data: u64,
    time: u64,
) -> u32 {
    let r = (|| {
        let open = resolve(key, &[], sam::QUERY_VALUE | sam::ENUMERATE_SUB_KEYS)?;
        let (k_count, k_max, v_count, v_max, d_max) = with_key(&open, false, |s, k| {
            let version = s.registry_version(k).map_err(win32)?;
            let (mut kc, mut km) = (0u32, 0u32);
            while let Ok(n) = s.registry_enumerate_key(k, kc, version) {
                km = km.max(n.units().len() as u32);
                kc += 1;
            }
            let (mut vc, mut vm, mut dm) = (0u32, 0u32, 0u32);
            while let Ok(e) = s.registry_enumerate_value(k, vc, version) {
                vm = vm.max(e.name.units().len() as u32);
                dm = dm.max(e.value.data().len() as u32);
                vc += 1;
            }
            Ok((kc, km, vc, vm, dm))
        })?;
        for (p, v) in [
            (subkeys, k_count),
            (max_subkey, k_max),
            (values, v_count),
            (max_name, v_max),
            (max_data, d_max),
        ] {
            if p != 0 {
                write_u32(p, v)?;
            }
        }
        if time != 0 {
            write_qword(time, 0)?;
        }
        Ok(())
    })();
    r.err().unwrap_or(error::SUCCESS)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegQueryInfoKeyW(
    key: usize,
    class: usize,
    class_len: usize,
    _reserved: usize,
    subkeys: usize,
    max_subkey: usize,
    max_class: usize,
    values: usize,
    max_name: usize,
    max_data: usize,
    _security: usize,
    time: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let class = door_windows_abi::widen(class);
    let class_len = door_windows_abi::widen(class_len);
    let subkeys = door_windows_abi::widen(subkeys);
    let max_subkey = door_windows_abi::widen(max_subkey);
    let max_class = door_windows_abi::widen(max_class);
    let values = door_windows_abi::widen(values);
    let max_name = door_windows_abi::widen(max_name);
    let max_data = door_windows_abi::widen(max_data);
    let time = door_windows_abi::widen(time);
    {
        if class_len != 0 {
            let _ = write_u32(class_len, 0);
            let _ = (class != 0).then(|| write(class, &[0, 0]));
        }
        if max_class != 0 {
            let _ = write_u32(max_class, 0);
        }
        reg_query_info(key, subkeys, max_subkey, values, max_name, max_data, time)
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn RegQueryInfoKeyA(
    key: usize,
    class: usize,
    class_len: usize,
    _reserved: usize,
    subkeys: usize,
    max_subkey: usize,
    max_class: usize,
    values: usize,
    max_name: usize,
    max_data: usize,
    _security: usize,
    time: usize,
) -> u32 {
    let key = door_windows_abi::widen(key);
    let class = door_windows_abi::widen(class);
    let class_len = door_windows_abi::widen(class_len);
    let subkeys = door_windows_abi::widen(subkeys);
    let max_subkey = door_windows_abi::widen(max_subkey);
    let max_class = door_windows_abi::widen(max_class);
    let values = door_windows_abi::widen(values);
    let max_name = door_windows_abi::widen(max_name);
    let max_data = door_windows_abi::widen(max_data);
    let time = door_windows_abi::widen(time);
    {
        if class_len != 0 {
            let _ = write_u32(class_len, 0);
            let _ = (class != 0).then(|| write(class, &[0]));
        }
        if max_class != 0 {
            let _ = write_u32(max_class, 0);
        }
        reg_query_info(key, subkeys, max_subkey, values, max_name, max_data, time)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegNotifyChangeKeyValue(
    _key: usize,
    _subtree: i32,
    _filter: u32,
    _event: usize,
    _asynchronous: i32,
) -> u32 {
    {
        // The hives report no change notifications yet.
        error::CALL_NOT_IMPLEMENTED
    }
}
