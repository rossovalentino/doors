//! Event Tracing for Windows, provider side (P18: CoreCLR and EventSource
//! register providers at startup). Doors has no trace sessions, so no
//! provider is ever enabled: registration succeeds, the enable callback is
//! never called, EventEnabled is false and a written event is dropped with
//! ERROR_SUCCESS, as Windows does with no listener. Activity IDs are kept
//! per thread, as EventActivityIdControl defines them.
use super::*;

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_SUPPORTED: u32 = 50;
/// A provider: its GUID, enable callback and context (kept for when a
/// session can enable it).
#[derive(Clone, Copy)]
struct Provider {
    #[allow(dead_code)]
    guid: [u8; 16],
    #[allow(dead_code)]
    callback: u64,
    #[allow(dead_code)]
    context: u64,
}
static PROVIDERS: Lock<Vec<Option<Provider>>> = Lock::new(Vec::new());
/// REGHANDLEs are these plus the slot (never 0).
const HANDLE_BASE: u64 = 0x0000_E700_0000_0000;
fn provider_index(handle: u64) -> Option<usize> {
    let index = usize::try_from(handle.checked_sub(HANDLE_BASE)?).ok()?;
    PROVIDERS.with(|p| p.get(index).is_some_and(Option::is_some).then_some(index))
}
/// EventRegister(LPCGUID, PENABLECALLBACK, PVOID, PREGHANDLE).
#[unsafe(no_mangle)]
pub extern "system" fn EventRegister(
    guid: usize,
    callback: usize,
    context: usize,
    out: usize,
) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    let mut g = [0u8; 16];
    if out == 0 || read(guid, &mut g).is_err() {
        return error::INVALID_PARAMETER;
    }
    let provider = Provider {
        guid: g,
        callback: door_windows_abi::widen(callback),
        context: door_windows_abi::widen(context),
    };
    let index = PROVIDERS.with(|p| {
        let i = p.iter().position(Option::is_none).unwrap_or_else(|| {
            p.push(None);
            p.len() - 1
        });
        p[i] = Some(provider);
        i
    });
    let handle = HANDLE_BASE + index as u64;
    if write(out, &handle.to_le_bytes()).is_err() {
        PROVIDERS.with(|p| p[index] = None);
        return error::INVALID_PARAMETER;
    }
    error::SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn EventUnregister(handle: u64) -> u32 {
    match provider_index(handle) {
        Some(i) => {
            PROVIDERS.with(|p| p[i] = None);
            error::SUCCESS
        }
        None => ERROR_INVALID_HANDLE,
    }
}
/// EventSetInformation(REGHANDLE, class, data, length): provider traits
/// (class 2) and the decode GUID (0) are accepted; nothing reads them.
#[unsafe(no_mangle)]
pub extern "system" fn EventSetInformation(
    handle: u64,
    class: u32,
    _data: usize,
    _length: u32,
) -> u32 {
    if provider_index(handle).is_none() {
        return ERROR_INVALID_HANDLE;
    }
    match class {
        0..=3 => error::SUCCESS,
        _ => ERROR_NOT_SUPPORTED,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EventEnabled(_handle: u64, _descriptor: usize) -> u8 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn EventProviderEnabled(_handle: u64, _level: u8, _keyword: u64) -> u8 {
    0
}
fn written(handle: u64) -> u32 {
    // A handle of 0 (registration failed or skipped) writes nothing too.
    if handle == 0 || provider_index(handle).is_some() {
        error::SUCCESS
    } else {
        ERROR_INVALID_HANDLE
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EventWrite(
    handle: u64,
    _descriptor: usize,
    _count: u32,
    _data: usize,
) -> u32 {
    written(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn EventWriteTransfer(
    handle: u64,
    _descriptor: usize,
    _activity: usize,
    _related: usize,
    _count: u32,
    _data: usize,
) -> u32 {
    written(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn EventWriteEx(
    handle: u64,
    _descriptor: usize,
    _filter: u64,
    _flags: u32,
    _activity: usize,
    _related: usize,
    _count: u32,
    _data: usize,
) -> u32 {
    written(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn EventWriteString(
    handle: u64,
    _level: u8,
    _keyword: u64,
    _text: usize,
) -> u32 {
    written(handle)
}

/// Each thread's activity ID (absent = GUID_NULL).
static ACTIVITIES: Lock<Vec<(u32, [u8; 16])>> = Lock::new(Vec::new());
fn activity() -> [u8; 16] {
    let id = unsafe { DoorGetCurrentThreadId() };
    ACTIVITIES.with(|a| {
        a.iter()
            .find(|(t, _)| *t == id)
            .map_or([0; 16], |(_, g)| *g)
    })
}
fn set_activity(g: [u8; 16]) {
    let id = unsafe { DoorGetCurrentThreadId() };
    ACTIVITIES.with(|a| {
        a.retain(|(t, _)| *t != id);
        if g != [0; 16] {
            a.push((id, g));
        }
    });
}
/// A random (version 4) GUID.
fn new_activity() -> [u8; 16] {
    let mut g = [0u8; 16];
    unsafe { DoorGenRandom(g.as_mut_ptr() as u64, 16) };
    g[7] = (g[7] & 0x0F) | 0x40;
    g[8] = (g[8] & 0x3F) | 0x80;
    g
}
/// EventActivityIdControl(code, LPGUID): GET 1, SET 2, CREATE 3,
/// GET_SET 4, CREATE_SET 5.
#[unsafe(no_mangle)]
pub extern "system" fn EventActivityIdControl(code: u32, guid: usize) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let mut given = [0u8; 16];
    if guid == 0 || (matches!(code, 2 | 4) && read(guid, &mut given).is_err()) {
        return error::INVALID_PARAMETER;
    }
    let out = match code {
        1 => activity(),
        2 => {
            set_activity(given);
            return error::SUCCESS;
        }
        3 => new_activity(),
        4 => {
            let previous = activity();
            set_activity(given);
            previous
        }
        5 => {
            let previous = activity();
            set_activity(new_activity());
            previous
        }
        _ => return error::INVALID_PARAMETER,
    };
    if write(guid, &out).is_err() {
        return error::INVALID_PARAMETER;
    }
    error::SUCCESS
}
