//! winhttp.dll exports. Handles are entries of a process-wide table;
//! requests run their network work on the calling thread (synchronous
//! sessions) or on one worker thread per operation (WINHTTP_FLAG_ASYNC),
//! reporting through the status callback as WinHTTP does.
use crate::{Jar, error};
use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_http::{Headers, Url};
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_inet::{
    Failure, Tolerance,
    exchange::{Cookies, Event, Exchange, Settings, Stop},
    guest::now_unix,
    sync::Lock,
};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(value: u32);
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, size: usize) -> usize;
}

// ---- application memory ------------------------------------------------
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if out.is_empty() {
        return Ok(());
    }
    if pointer == 0 || unsafe { DoorUserCopy(out.as_mut_ptr() as u64, pointer, out.len()) } != 0 {
        return Err(error::INVALID_PARAMETER);
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if data.is_empty() {
        return Ok(());
    }
    if pointer == 0 || unsafe { DoorUserCopy(pointer, data.as_ptr() as u64, data.len()) } != 0 {
        return Err(error::INVALID_PARAMETER);
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
/// The program's pointer width (P17: 4 for a 32-bit program).
const POINTER: usize = layout::POINTER;
/// A pointer-sized program value (pointers, DWORD_PTR), sign-extended on
/// x86.
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(layout::get(&b, &[Pointer], 0))
}
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// URL_COMPONENTS: dwStructSize, lpszScheme, dwSchemeLength, nScheme,
/// lpszHostName, dwHostNameLength, nPort, lpszUserName, dwUserNameLength,
/// lpszPassword, dwPasswordLength, lpszUrlPath, dwUrlPathLength,
/// lpszExtraInfo, dwExtraInfoLength (104 bytes on x64, 60 on x86).
const URL_COMPONENTS: [Field; 15] = [
    U32, Pointer, U32, U32, Pointer, U32, U16, Pointer, U32, Pointer, U32, Pointer, U32, Pointer,
    U32,
];
fn url_field(index: usize) -> u64 {
    layout::offset(&URL_COMPONENTS, index) as u64
}
/// WINHTTP_PROXY_INFO: dwAccessType, lpszProxy, lpszProxyBypass.
const PROXY_INFO: [Field; 3] = [U32, Pointer, Pointer];
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// A UTF-16 string of `length` units, or NUL-terminated when `length` is
/// u32::MAX (-1L) or 0 for the "length unknown" conventions.
fn wide(pointer: u64, length: u32) -> Result<String, u32> {
    if pointer == 0 {
        return Ok(String::new());
    }
    let mut units = Vec::new();
    let limit = if length == u32::MAX || length == 0 {
        65536
    } else {
        length as usize
    };
    for i in 0..limit {
        let mut b = [0u8; 2];
        read(pointer + (i * 2) as u64, &mut b)?;
        let u = u16::from_le_bytes(b);
        if u == 0 && (length == u32::MAX || length == 0) {
            break;
        }
        units.push(u);
    }
    String::from_utf16(&units).map_err(|_| error::INVALID_PARAMETER)
}
fn fail<T>(code: u32, value: T) -> T {
    unsafe { RtlSetLastWin32Error(code) };
    value
}
fn done(result: Result<(), u32>) -> i32 {
    match result {
        Ok(()) => {
            unsafe { RtlSetLastWin32Error(0) };
            1
        }
        Err(e) => fail(e, 0),
    }
}

// ---- handles -----------------------------------------------------------------------
const TYPE_SESSION: u32 = 1;
const TYPE_CONNECT: u32 = 2;
const TYPE_REQUEST: u32 = 3;
/// What every handle carries: its callback, context and parent.
#[derive(Clone, Copy, Default)]
struct Common {
    callback: u64,
    notify: u32,
    context: u64,
    parent: u64,
}
struct Session {
    agent: String,
    access: u32,
    proxy: String,
    bypass: String,
    asynchronous: bool,
    /// Resolve, connect, send, receive (ms; 0 = infinite).
    timeouts: [u32; 4],
    redirect_policy: u32,
    max_redirects: u32,
    disabled: u32,
    jar: Jar,
    /// WINHTTP_OPTION_DECOMPRESSION (WINHTTP_DECOMPRESSION_FLAG_*).
    decompression: u32,
}
struct Connect {
    host: String,
    port: u16,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Open,
    Sent,
    Received,
}
struct Request {
    exchange: Exchange,
    stage: Stage,
    security_flags: u32,
    /// WINHTTP_OPTION_DECOMPRESSION set on the request itself.
    decompression: Option<u32>,
    /// An asynchronous operation is in flight.
    busy: bool,
}
enum Object {
    Session(Arc<Lock<Session>>),
    Connect(Arc<Lock<Connect>>),
    Request(Arc<Lock<Request>>),
}
struct Slot {
    common: Common,
    object: Object,
    closed: bool,
    /// Set by WinHttpCloseHandle; in-flight work ends with
    /// ERROR_WINHTTP_OPERATION_CANCELLED.
    cancel: Arc<AtomicBool>,
}
static TABLE: Lock<Vec<Option<Slot>>> = Lock::new(Vec::new());
/// Handle values: 0x5150_0000 + index * 16 + type, never 0 or small.
fn handle_value(index: usize, kind: u32) -> u64 {
    0x5150_0000 + index as u64 * 16 + u64::from(kind)
}
fn index_of(handle: u64) -> Option<usize> {
    handle.checked_sub(0x5150_0000).map(|v| (v / 16) as usize)
}
fn insert(common: Common, object: Object) -> u64 {
    insert_with(common, object, Arc::new(AtomicBool::new(false)))
}
fn insert_with(common: Common, object: Object, cancel: Arc<AtomicBool>) -> u64 {
    let kind = match object {
        Object::Session(_) => TYPE_SESSION,
        Object::Connect(_) => TYPE_CONNECT,
        Object::Request(_) => TYPE_REQUEST,
    };
    TABLE.with(|t| {
        let index = t.iter().position(Option::is_none).unwrap_or_else(|| {
            t.push(None);
            t.len() - 1
        });
        t[index] = Some(Slot {
            common,
            object,
            closed: false,
            cancel,
        });
        handle_value(index, kind)
    })
}
fn common(handle: u64) -> Option<Common> {
    let index = index_of(handle)?;
    TABLE.with(|t| {
        t.get(index)
            .and_then(|s| s.as_ref())
            .filter(|s| !s.closed && handle_value(index, kind_of(&s.object)) == handle)
            .map(|s| s.common)
    })
}
fn kind_of(object: &Object) -> u32 {
    match object {
        Object::Session(_) => TYPE_SESSION,
        Object::Connect(_) => TYPE_CONNECT,
        Object::Request(_) => TYPE_REQUEST,
    }
}
fn set_common(handle: u64, f: impl FnOnce(&mut Common)) -> bool {
    let Some(index) = index_of(handle) else {
        return false;
    };
    TABLE.with(|t| match t.get_mut(index).and_then(|s| s.as_mut()) {
        Some(slot) if !slot.closed => {
            f(&mut slot.common);
            true
        }
        _ => false,
    })
}
fn session(handle: u64) -> Result<Arc<Lock<Session>>, u32> {
    let index = index_of(handle).ok_or(error::INVALID_HANDLE)?;
    TABLE.with(|t| match t.get(index).and_then(|s| s.as_ref()) {
        Some(Slot {
            object: Object::Session(s),
            closed: false,
            ..
        }) if handle == handle_value(index, TYPE_SESSION) => Ok(s.clone()),
        Some(_) => Err(error::INCORRECT_HANDLE_TYPE),
        None => Err(error::INVALID_HANDLE),
    })
}
fn connect(handle: u64) -> Result<Arc<Lock<Connect>>, u32> {
    let index = index_of(handle).ok_or(error::INVALID_HANDLE)?;
    TABLE.with(|t| match t.get(index).and_then(|s| s.as_ref()) {
        Some(Slot {
            object: Object::Connect(c),
            closed: false,
            ..
        }) if handle == handle_value(index, TYPE_CONNECT) => Ok(c.clone()),
        Some(_) => Err(error::INCORRECT_HANDLE_TYPE),
        None => Err(error::INVALID_HANDLE),
    })
}
fn request(handle: u64) -> Result<Arc<Lock<Request>>, u32> {
    let index = index_of(handle).ok_or(error::INVALID_HANDLE)?;
    TABLE.with(|t| match t.get(index).and_then(|s| s.as_ref()) {
        Some(Slot {
            object: Object::Request(r),
            closed: false,
            ..
        }) if handle == handle_value(index, TYPE_REQUEST) => Ok(r.clone()),
        Some(_) => Err(error::INCORRECT_HANDLE_TYPE),
        None => Err(error::INVALID_HANDLE),
    })
}
/// The session of a connect or request handle.
fn session_of(handle: u64) -> Result<(u64, Arc<Lock<Session>>), u32> {
    let mut h = handle;
    for _ in 0..3 {
        if let Ok(s) = session(h) {
            return Ok((h, s));
        }
        h = common(h).ok_or(error::INVALID_HANDLE)?.parent;
    }
    Err(error::INVALID_HANDLE)
}

// ---- status callbacks --------------------------------------------------------------
pub mod status {
    pub const RESOLVING_NAME: u32 = 0x1;
    pub const NAME_RESOLVED: u32 = 0x2;
    pub const CONNECTING_TO_SERVER: u32 = 0x4;
    pub const CONNECTED_TO_SERVER: u32 = 0x8;
    pub const SENDING_REQUEST: u32 = 0x10;
    pub const REQUEST_SENT: u32 = 0x20;
    pub const RECEIVING_RESPONSE: u32 = 0x40;
    pub const RESPONSE_RECEIVED: u32 = 0x80;
    pub const CLOSING_CONNECTION: u32 = 0x100;
    pub const CONNECTION_CLOSED: u32 = 0x200;
    pub const HANDLE_CREATED: u32 = 0x400;
    pub const HANDLE_CLOSING: u32 = 0x800;
    pub const REDIRECT: u32 = 0x4000;
    pub const SECURE_FAILURE: u32 = 0x10000;
    pub const HEADERS_AVAILABLE: u32 = 0x20000;
    pub const DATA_AVAILABLE: u32 = 0x40000;
    pub const READ_COMPLETE: u32 = 0x80000;
    pub const WRITE_COMPLETE: u32 = 0x100000;
    pub const REQUEST_ERROR: u32 = 0x200000;
    pub const SENDREQUEST_COMPLETE: u32 = 0x400000;
}
/// WINHTTP_ASYNC_RESULT.dwResult: which API failed.
mod api {
    pub const RECEIVE_RESPONSE: u64 = 1;
    pub const QUERY_DATA_AVAILABLE: u64 = 2;
    pub const READ_DATA: u64 = 3;
    pub const WRITE_DATA: u64 = 4;
    pub const SEND_REQUEST: u64 = 5;
}
/// WINHTTP_STATUS_CALLBACK(hInternet, dwContext, dwInternetStatus,
/// lpvStatusInformation, dwStatusInformationLength) at pointer width.
type Callback = extern "system" fn(usize, usize, u32, usize, u32);
fn notify(handle: u64, code: u32, info: u64, length: u32) {
    let Some(c) = common(handle) else { return };
    if c.callback != 0 && c.notify & code != 0 {
        let call: Callback =
            unsafe { core::mem::transmute::<usize, Callback>(c.callback as usize) };
        call(
            handle as usize,
            c.context as usize,
            code,
            info as usize,
            length,
        );
    }
}
fn notify_text(handle: u64, code: u32, text: &str) {
    let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
    notify(
        handle,
        code,
        units.as_ptr() as u64,
        (units.len() - 1) as u32,
    );
}
fn notify_error(handle: u64, which: u64, error: u32) {
    // WINHTTP_ASYNC_RESULT { dwResult (DWORD_PTR), dwError }: 16 bytes on
    // x64, 8 on x86.
    const ASYNC_RESULT: [Field; 2] = [Pointer, U32];
    let mut result = [0u8; layout::size(&ASYNC_RESULT)];
    layout::set(&mut result, &ASYNC_RESULT, 0, which);
    layout::set(&mut result, &ASYNC_RESULT, 1, u64::from(error));
    notify(
        handle,
        status::REQUEST_ERROR,
        result.as_ptr() as u64,
        result.len() as u32,
    );
}

// ---- WinHttpOpen / Connect / OpenRequest / CloseHandle -----------------------------------
const ACCESS_DEFAULT_PROXY: u32 = 0;
const ACCESS_NO_PROXY: u32 = 1;
const ACCESS_NAMED_PROXY: u32 = 3;
const ACCESS_AUTOMATIC_PROXY: u32 = 4;
const FLAG_ASYNC: u32 = 0x1000_0000;
const FLAG_SECURE: u32 = 0x0080_0000;

#[unsafe(no_mangle)]
pub extern "system" fn WinHttpOpen(
    agent: usize,
    access: u32,
    proxy: usize,
    bypass: usize,
    flags: u32,
) -> usize {
    let agent = door_windows_abi::widen(agent);
    let proxy = door_windows_abi::widen(proxy);
    let bypass = door_windows_abi::widen(bypass);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| {
            if !matches!(
                access,
                ACCESS_DEFAULT_PROXY
                    | ACCESS_NO_PROXY
                    | ACCESS_NAMED_PROXY
                    | ACCESS_AUTOMATIC_PROXY
            ) {
                return Err(error::INVALID_PARAMETER);
            }
            let proxy = wide(proxy, u32::MAX)?;
            if access == ACCESS_NAMED_PROXY && proxy.is_empty() {
                return Err(error::INVALID_PARAMETER);
            }
            Ok(Session {
                agent: wide(agent, u32::MAX)?,
                access,
                proxy,
                bypass: wide(bypass, u32::MAX)?,
                asynchronous: flags & FLAG_ASYNC != 0,
                timeouts: [0, 60_000, 30_000, 30_000],
                redirect_policy: 1,
                max_redirects: 10,
                disabled: 0,
                jar: Jar::default(),
                decompression: 0,
            })
        })();
        match result {
            Ok(s) => {
                let h = insert(Common::default(), Object::Session(Arc::new(Lock::new(s))));
                unsafe { RtlSetLastWin32Error(0) };
                h
            }
            Err(e) => fail(e, 0),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpConnect(
    parent: usize,
    server: usize,
    port: u16,
    reserved: u32,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let server = door_windows_abi::widen(server);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| {
            session(parent)?;
            if reserved != 0 {
                return Err(error::INVALID_PARAMETER);
            }
            let host = wide(server, u32::MAX)?;
            if host.is_empty() {
                return Err(error::INVALID_PARAMETER);
            }
            let inherited = common(parent).ok_or(error::INVALID_HANDLE)?;
            Ok((Connect { host, port }, inherited))
        })();
        match result {
            Ok((c, inherited)) => {
                let h = insert(
                    Common {
                        parent,
                        context: 0,
                        ..inherited
                    },
                    Object::Connect(Arc::new(Lock::new(c))),
                );
                notify(h, status::HANDLE_CREATED, (&h as *const u64) as u64, 8);
                h
            }
            Err(e) => fail(e, 0),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpOpenRequest(
    parent: usize,
    verb: usize,
    object: usize,
    version: usize,
    referrer: usize,
    accept: usize,
    flags: u32,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let verb = door_windows_abi::widen(verb);
    let object = door_windows_abi::widen(object);
    let version = door_windows_abi::widen(version);
    let referrer = door_windows_abi::widen(referrer);
    let accept = door_windows_abi::widen(accept);
    door_windows_abi::narrow((move || -> u64 {
        let result = (|| {
            let c = connect(parent)?;
            let (host, port) = c.with(|c| (c.host.clone(), c.port));
            let verb = if verb == 0 {
                String::from("GET")
            } else {
                wide(verb, u32::MAX)?
            };
            let mut object = if object == 0 {
                String::from("/")
            } else {
                wide(object, u32::MAX)?
            };
            if !object.starts_with('/') && !object.contains("://") {
                object.insert(0, '/');
            }
            let version = if version == 0 {
                String::from("HTTP/1.1")
            } else {
                wide(version, u32::MAX)?
            };
            let mut headers = Headers::default();
            let referrer = wide(referrer, u32::MAX)?;
            if !referrer.is_empty() {
                headers.set("Referer", &referrer);
            }
            // WINHTTP_DEFAULT_ACCEPT_TYPES: a NULL-terminated array of strings.
            if accept != 0 {
                let mut types = Vec::new();
                for i in 0..64u64 {
                    let p = read_pointer(accept + i * POINTER as u64)?;
                    if p == 0 {
                        break;
                    }
                    types.push(wide(p, u32::MAX)?);
                }
                if !types.is_empty() {
                    headers.set("Accept", &types.join(", "));
                }
            }
            let secure = flags & FLAG_SECURE != 0;
            let port = if port == 0 {
                if secure { 443 } else { 80 }
            } else {
                port
            };
            let scheme = if secure {
                door_http::Scheme::Https
            } else {
                door_http::Scheme::Http
            };
            let url = Url {
                scheme,
                user: String::new(),
                password: String::new(),
                host,
                port,
                path: object,
                fragment: String::new(),
            };
            Ok((verb, url, version, headers))
        })();
        match result {
            Ok((verb, url, version, headers)) => {
                let cancel = Arc::new(AtomicBool::new(false));
                let r = Request {
                    exchange: Exchange::new(verb, url, version, headers, cancel.clone()),
                    stage: Stage::Open,
                    security_flags: 0,
                    decompression: None,
                    busy: false,
                };
                let inherited = common(parent).unwrap_or_default();
                let h = insert_with(
                    Common {
                        parent,
                        context: 0,
                        ..inherited
                    },
                    Object::Request(Arc::new(Lock::new(r))),
                    cancel,
                );
                notify(h, status::HANDLE_CREATED, (&h as *const u64) as u64, 8);
                h
            }
            Err(e) => fail(e, 0),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCloseHandle(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let Some(index) = index_of(handle) else {
            return fail(error::INVALID_HANDLE, 0);
        };
        if common(handle).is_none() {
            return fail(error::INVALID_HANDLE, 0);
        }
        // In-flight work sees the cancellation and ends with
        // ERROR_WINHTTP_OPERATION_CANCELLED; the connection closes when the
        // last reference to the request goes.
        TABLE.with(|t| {
            if let Some(slot) = t.get(index).and_then(|s| s.as_ref()) {
                slot.cancel.store(true, Ordering::Release);
            }
        });
        notify(
            handle,
            status::HANDLE_CLOSING,
            (&handle as *const u64) as u64,
            8,
        );
        TABLE.with(|t| {
            if let Some(slot) = t.get_mut(index) {
                *slot = None;
            }
        });
        done(Ok(()))
    }
}

// ---- options -----------------------------------------------------------------------
mod option {
    pub const CALLBACK: u32 = 1;
    pub const RESOLVE_TIMEOUT: u32 = 2;
    pub const CONNECT_TIMEOUT: u32 = 3;
    pub const CONNECT_RETRIES: u32 = 4;
    pub const SEND_TIMEOUT: u32 = 5;
    pub const RECEIVE_TIMEOUT: u32 = 6;
    pub const RECEIVE_RESPONSE_TIMEOUT: u32 = 7;
    pub const HANDLE_TYPE: u32 = 9;
    pub const READ_BUFFER_SIZE: u32 = 12;
    pub const WRITE_BUFFER_SIZE: u32 = 13;
    pub const PARENT_HANDLE: u32 = 21;
    pub const EXTENDED_ERROR: u32 = 24;
    pub const SECURITY_FLAGS: u32 = 31;
    pub const URL: u32 = 34;
    pub const SECURITY_KEY_BITNESS: u32 = 36;
    pub const PROXY: u32 = 38;
    pub const USER_AGENT: u32 = 41;
    pub const CONTEXT_VALUE: u32 = 45;
    pub const CLIENT_CERT_CONTEXT: u32 = 47;
    pub const DISABLE_FEATURE: u32 = 63;
    pub const SECURE_PROTOCOLS: u32 = 84;
    pub const ENABLE_FEATURE: u32 = 79;
    pub const REDIRECT_POLICY: u32 = 88;
    pub const MAX_HTTP_AUTOMATIC_REDIRECTS: u32 = 89;
    pub const DECOMPRESSION: u32 = 118;
    pub const ENABLE_HTTP_PROTOCOL: u32 = 133;
    pub const HTTP_PROTOCOL_USED: u32 = 134;
}
fn dword_option(buffer: u64, length: u32) -> Result<u32, u32> {
    if length < 4 {
        return Err(error::INSUFFICIENT_BUFFER);
    }
    read_u32(buffer)
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetOption(
    handle: usize,
    which: u32,
    buffer: usize,
    length: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    {
        let result = (|| {
            if common(handle).is_none() {
                return Err(error::INVALID_HANDLE);
            }
            match which {
                option::CONTEXT_VALUE => {
                    if (length as usize) < POINTER {
                        return Err(error::INSUFFICIENT_BUFFER);
                    }
                    let value = read_pointer(buffer)?;
                    set_common(handle, |c| c.context = value);
                }
                option::RESOLVE_TIMEOUT
                | option::CONNECT_TIMEOUT
                | option::SEND_TIMEOUT
                | option::RECEIVE_TIMEOUT
                | option::RECEIVE_RESPONSE_TIMEOUT => {
                    let value = dword_option(buffer, length)?;
                    let slot = match which {
                        option::RESOLVE_TIMEOUT => 0,
                        option::CONNECT_TIMEOUT => 1,
                        option::SEND_TIMEOUT => 2,
                        _ => 3,
                    };
                    session_of(handle)?.1.with(|s| s.timeouts[slot] = value);
                }
                option::CONNECT_RETRIES
                | option::READ_BUFFER_SIZE
                | option::WRITE_BUFFER_SIZE
                | option::ENABLE_HTTP_PROTOCOL => {
                    dword_option(buffer, length)?;
                }
                option::DECOMPRESSION => {
                    // WINHTTP_DECOMPRESSION_FLAG_GZIP (1) and _DEFLATE (2).
                    let value = dword_option(buffer, length)?;
                    if value & !3 != 0 {
                        return Err(error::INVALID_PARAMETER);
                    }
                    match request(handle) {
                        Ok(r) => r.with(|r| r.decompression = Some(value)),
                        Err(_) => session_of(handle)?.1.with(|s| s.decompression = value),
                    }
                }
                option::SECURITY_FLAGS => {
                    let value = dword_option(buffer, length)?;
                    request(handle)?.with(|r| r.security_flags = value);
                }
                option::REDIRECT_POLICY => {
                    let value = dword_option(buffer, length)?;
                    if value > 2 {
                        return Err(error::INVALID_PARAMETER);
                    }
                    session_of(handle)?.1.with(|s| s.redirect_policy = value);
                }
                option::MAX_HTTP_AUTOMATIC_REDIRECTS => {
                    let value = dword_option(buffer, length)?;
                    session_of(handle)?.1.with(|s| s.max_redirects = value);
                }
                option::DISABLE_FEATURE => {
                    let value = dword_option(buffer, length)?;
                    session_of(handle)?.1.with(|s| s.disabled |= value);
                }
                option::SECURE_PROTOCOLS => {
                    // TLS 1.3 is the only protocol this client speaks: a
                    // selection without it cannot connect.
                    let value = dword_option(buffer, length)?;
                    if value & 0x2000 == 0 && value & 0x800 == 0 {
                        return Err(error::INVALID_PARAMETER);
                    }
                }
                option::USER_AGENT => {
                    let agent = wide(buffer, length)?;
                    session_of(handle)?.1.with(|s| s.agent = agent);
                }
                option::PROXY => {
                    // WINHTTP_PROXY_INFO { dwAccessType, lpszProxy, lpszProxyBypass }.
                    let access = read_u32(buffer)?;
                    let at = |index| buffer + layout::offset(&PROXY_INFO, index) as u64;
                    let proxy = wide(read_pointer(at(1))?, u32::MAX)?;
                    let bypass = wide(read_pointer(at(2))?, u32::MAX)?;
                    session_of(handle)?.1.with(|s| {
                        s.access = access;
                        s.proxy = proxy;
                        s.bypass = bypass;
                    });
                }
                option::ENABLE_FEATURE => {
                    // SSL revocation checking is not implemented: refusing the
                    // request is truthful, accepting it would not be.
                    dword_option(buffer, length)?;
                    return Err(error::INVALID_PARAMETER);
                }
                option::CLIENT_CERT_CONTEXT => {
                    // Only WINHTTP_NO_CLIENT_CERT_CONTEXT (NULL): no client
                    // certificates.
                    if buffer != 0 {
                        return Err(error::INVALID_PARAMETER);
                    }
                }
                option::HANDLE_TYPE
                | option::PARENT_HANDLE
                | option::URL
                | option::HTTP_PROTOCOL_USED
                | option::SECURITY_KEY_BITNESS
                | option::EXTENDED_ERROR => {
                    return Err(error::OPTION_NOT_SETTABLE);
                }
                _ => return Err(error::INVALID_OPTION),
            }
            Ok(())
        })();
        done(result)
    }
}
/// Copies an option value out with WinHTTP's size protocol.
fn deliver(buffer: u64, length: u64, data: &[u8]) -> Result<(), u32> {
    let capacity = read_u32(length)?;
    if buffer == 0 || (capacity as usize) < data.len() {
        write_u32(length, data.len() as u32)?;
        return Err(error::INSUFFICIENT_BUFFER);
    }
    write(buffer, data)?;
    write_u32(length, data.len() as u32)
}
fn wide_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpQueryOption(
    handle: usize,
    which: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let result = (|| {
            let c = common(handle).ok_or(error::INVALID_HANDLE)?;
            let data: Vec<u8> = match which {
                option::CONTEXT_VALUE => c.context.to_le_bytes()[..POINTER].to_vec(),
                option::PARENT_HANDLE => c.parent.to_le_bytes()[..POINTER].to_vec(),
                option::CALLBACK => c.callback.to_le_bytes()[..POINTER].to_vec(),
                option::HANDLE_TYPE => {
                    let kind = if session(handle).is_ok() {
                        1
                    } else if connect(handle).is_ok() {
                        2
                    } else {
                        3
                    };
                    u32::to_le_bytes(kind).to_vec()
                }
                option::RESOLVE_TIMEOUT
                | option::CONNECT_TIMEOUT
                | option::SEND_TIMEOUT
                | option::RECEIVE_TIMEOUT
                | option::RECEIVE_RESPONSE_TIMEOUT => {
                    let slot = match which {
                        option::RESOLVE_TIMEOUT => 0,
                        option::CONNECT_TIMEOUT => 1,
                        option::SEND_TIMEOUT => 2,
                        _ => 3,
                    };
                    session_of(handle)?
                        .1
                        .with(|s| s.timeouts[slot])
                        .to_le_bytes()
                        .to_vec()
                }
                option::SECURITY_FLAGS => request(handle)?
                    .with(|r| {
                        let mut flags = r.security_flags;
                        // SECURITY_FLAG_SECURE | STRENGTH_STRONG once connected.
                        if r.exchange.connected_securely() {
                            flags |= 0x1 | 0x2000_0000;
                        }
                        flags
                    })
                    .to_le_bytes()
                    .to_vec(),
                option::REDIRECT_POLICY => session_of(handle)?
                    .1
                    .with(|s| s.redirect_policy)
                    .to_le_bytes()
                    .to_vec(),
                option::MAX_HTTP_AUTOMATIC_REDIRECTS => session_of(handle)?
                    .1
                    .with(|s| s.max_redirects)
                    .to_le_bytes()
                    .to_vec(),
                option::USER_AGENT => wide_bytes(&session_of(handle)?.1.with(|s| s.agent.clone())),
                option::URL => wide_bytes(&request(handle)?.with(|r| r.exchange.url.to_text())),
                option::SECURE_PROTOCOLS => 0x2000u32.to_le_bytes().to_vec(),
                option::HTTP_PROTOCOL_USED => 0u32.to_le_bytes().to_vec(),
                option::SECURITY_KEY_BITNESS => 256u32.to_le_bytes().to_vec(),
                option::EXTENDED_ERROR => 0u32.to_le_bytes().to_vec(),
                _ => return Err(error::INVALID_OPTION),
            };
            // String options report their size in bytes without the NUL.
            let string = matches!(which, option::USER_AGENT | option::URL);
            let capacity = read_u32(length)?;
            if buffer == 0 || (capacity as usize) < data.len() {
                write_u32(length, data.len() as u32)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(buffer, &data)?;
            write_u32(length, data.len() as u32 - if string { 2 } else { 0 })
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetTimeouts(
    handle: usize,
    resolve: i32,
    connect: i32,
    send: i32,
    receive: i32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let result = (|| {
            if [resolve, connect, send, receive].iter().any(|t| *t < -1) {
                return Err(error::INVALID_PARAMETER);
            }
            let as_ms = |t: i32| if t == -1 { 0 } else { t as u32 };
            session_of(handle)?.1.with(|s| {
                s.timeouts = [as_ms(resolve), as_ms(connect), as_ms(send), as_ms(receive)]
            });
            Ok(())
        })();
        done(result)
    }
}
/// Returns the previous callback; WINHTTP_INVALID_STATUS_CALLBACK (-1) on
/// failure.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetStatusCallback(
    handle: usize,
    callback: usize,
    notify: u32,
    _reserved: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let callback = door_windows_abi::widen(callback);
    door_windows_abi::narrow((move || -> u64 {
        let mut previous = 0;
        if !set_common(handle, |c| {
            previous = c.callback;
            c.callback = callback;
            c.notify = notify;
        }) {
            return fail(error::INVALID_HANDLE, u64::MAX);
        }
        unsafe { RtlSetLastWin32Error(0) };
        previous
    })())
}

// ---- request headers ------------------------------------------------------------------
const ADD_REPLACE: u32 = 0x8000_0000;
const ADD_IF_NEW: u32 = 0x1000_0000;
const ADD: u32 = 0x2000_0000;
const COALESCE_WITH_COMMA: u32 = 0x4000_0000;
const COALESCE_WITH_SEMICOLON: u32 = 0x0100_0000;
const REPLACE: u32 = 0x8000_0000;
fn add_headers(r: &mut Request, text: &str, modifiers: u32) -> Result<(), u32> {
    let headers = &mut r.exchange.headers;
    let lines = door_http::parse_header_lines(text).map_err(|_| error::INVALID_PARAMETER)?;
    for (name, value) in lines {
        let exists = headers.get(&name).is_some();
        if modifiers & ADD_IF_NEW != 0 && exists {
            return Err(error::HEADER_ALREADY_EXISTS);
        }
        if modifiers & COALESCE_WITH_COMMA != 0 && exists {
            headers.coalesce(&name, &value, ", ");
        } else if modifiers & COALESCE_WITH_SEMICOLON != 0 && exists {
            headers.coalesce(&name, &value, "; ");
        } else if modifiers & (REPLACE | ADD_REPLACE) != 0 {
            if value.is_empty() {
                headers.remove(&name);
            } else if exists || modifiers & ADD != 0 {
                headers.set(&name, &value);
            } else {
                return Err(error::HEADER_NOT_FOUND);
            }
        } else {
            headers.add(&name, &value);
        }
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpAddRequestHeaders(
    handle: usize,
    headers: usize,
    length: u32,
    modifiers: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    {
        let result = (|| {
            let r = request(handle)?;
            let text = wide(headers, length)?;
            r.with(|r| {
                if r.stage != Stage::Open {
                    return Err(error::INCORRECT_HANDLE_STATE);
                }
                add_headers(r, &text, modifiers)
            })
        })();
        done(result)
    }
}

// ---- the exchange -------------------------------------------------------------------
fn failure_code(f: Failure) -> u32 {
    match f {
        Failure::NameNotResolved => error::NAME_NOT_RESOLVED,
        Failure::CannotConnect => error::CANNOT_CONNECT,
        Failure::Timeout => error::TIMEOUT,
        Failure::ConnectionError => error::CONNECTION_ERROR,
        Failure::Secure(_) => error::SECURE_FAILURE,
        Failure::InvalidResponse => error::INVALID_SERVER_RESPONSE,
        Failure::Cancelled => error::OPERATION_CANCELLED,
        Failure::OutOfMemory => error::NOT_ENOUGH_MEMORY,
    }
}
fn stop_code(s: Stop) -> u32 {
    match s {
        Stop::Failed(f) => failure_code(f),
        Stop::RedirectFailed => error::REDIRECT_FAILED,
    }
}
/// The settings an exchange runs under, from its session and request.
fn settings(handle: u64, r: &Request) -> Result<Settings, u32> {
    let (_, s) = session_of(handle)?;
    Ok(s.with(|s| {
        let proxy = match s.access {
            ACCESS_NAMED_PROXY if !door_windows_inet::bypassed(&r.exchange.url.host, &s.bypass) => {
                door_windows_inet::parse_proxy(&s.proxy)
            }
            _ => None,
        };
        Settings {
            timeouts: s.timeouts,
            proxy,
            agent: s.agent.clone(),
            // WINHTTP_OPTION_REDIRECT_POLICY: never (0), not https -> http
            // (1, the default), always (2); WINHTTP_DISABLE_REDIRECTS.
            follow_redirects: s.redirect_policy != 0 && s.disabled & 0x2 == 0,
            allow_downgrade: s.redirect_policy == 2,
            max_redirects: s.max_redirects,
            tolerance: Tolerance::from_flags(r.security_flags),
            keep_alive: s.disabled & 0x4 == 0,
            decode: r.decompression.unwrap_or(s.decompression) != 0,
        }
    }))
}
/// The session's cookie jar unless WINHTTP_DISABLE_COOKIES is set.
struct SessionCookies(Arc<Lock<Session>>);
impl Cookies for SessionCookies {
    fn header(&mut self, url: &Url) -> Option<String> {
        let now = now_unix();
        self.0.with(|s| {
            if s.disabled & 0x1 != 0 {
                None
            } else {
                s.jar.header(
                    &url.host,
                    &url.path,
                    url.scheme == door_http::Scheme::Https,
                    now,
                )
            }
        })
    }
    fn store(&mut self, url: &Url, set_cookie: &str, now: i64) {
        self.0.with(|s| {
            if s.disabled & 0x1 == 0 {
                s.jar.store(set_cookie, &url.host, &url.path, now);
            }
        });
    }
}
fn cookies(handle: u64) -> Result<SessionCookies, u32> {
    Ok(SessionCookies(session_of(handle)?.1))
}
/// WinHTTP's status codes for the engine's progress events.
fn event(handle: u64, e: Event<'_>) {
    match e {
        Event::Resolving(h) => notify_text(handle, status::RESOLVING_NAME, h),
        Event::Resolved(a) => notify_text(handle, status::NAME_RESOLVED, a),
        Event::Connecting(a) => notify_text(handle, status::CONNECTING_TO_SERVER, a),
        Event::Connected(a) => notify_text(handle, status::CONNECTED_TO_SERVER, a),
        Event::Sending => notify(handle, status::SENDING_REQUEST, 0, 0),
        Event::Sent(n) => notify(handle, status::REQUEST_SENT, (&n as *const u32) as u64, 4),
        Event::Receiving => notify(handle, status::RECEIVING_RESPONSE, 0, 0),
        Event::Received(n) => notify(
            handle,
            status::RESPONSE_RECEIVED,
            (&n as *const u32) as u64,
            4,
        ),
        Event::Redirect(url) => notify_text(handle, status::REDIRECT, url),
        Event::SecureFailure(flags) => notify(
            handle,
            status::SECURE_FAILURE,
            (&flags as *const u32) as u64,
            4,
        ),
        Event::ClosingConnection => notify(handle, status::CLOSING_CONNECTION, 0, 0),
        Event::ConnectionClosed => notify(handle, status::CONNECTION_CLOSED, 0, 0),
    }
}

// ---- operations ---------------------------------------------------------------------
enum Work {
    Send(Vec<u8>),
    Receive,
    Available(u64),
    Read(u64, u32, u64),
    Write(Vec<u8>, u64),
}
/// What a finished operation reports in asynchronous mode.
enum Completion {
    Sent,
    Headers,
    Available(u32),
    Read(u64, u32),
    Written(u32),
}
fn api_of(work: &Work) -> u64 {
    match work {
        Work::Send(_) => api::SEND_REQUEST,
        Work::Receive => api::RECEIVE_RESPONSE,
        Work::Available(_) => api::QUERY_DATA_AVAILABLE,
        Work::Read(..) => api::READ_DATA,
        Work::Write(..) => api::WRITE_DATA,
    }
}
/// Performs one operation with the request locked; status callbacks for
/// progress fire during it, the completion after it.
fn perform(handle: u64, r: &Arc<Lock<Request>>, work: &Work) -> Result<Completion, u32> {
    let mut jar = cookies(handle)?;
    r.with(|r| {
        let s = settings(handle, r)?;
        match work {
            Work::Send(optional) => {
                r.exchange
                    .send(&s, &mut jar, optional, &mut |e| event(handle, e))
                    .map_err(failure_code)?;
                r.stage = Stage::Sent;
                Ok(Completion::Sent)
            }
            Work::Receive => {
                r.exchange
                    .receive(&s, &mut jar, &mut |e| event(handle, e))
                    .map_err(stop_code)?;
                r.stage = Stage::Received;
                Ok(Completion::Headers)
            }
            Work::Available(out) => {
                let n = r.exchange.available(s.timeouts[3]).map_err(failure_code)? as u32;
                if *out != 0 {
                    write_u32(*out, n)?;
                }
                Ok(Completion::Available(n))
            }
            Work::Read(buffer, length, read_out) => {
                let bytes = r
                    .exchange
                    .read(*length as usize, false, s.timeouts[3])
                    .map_err(failure_code)?;
                write(*buffer, &bytes)?;
                if *read_out != 0 {
                    write_u32(*read_out, bytes.len() as u32)?;
                }
                Ok(Completion::Read(*buffer, bytes.len() as u32))
            }
            Work::Write(data, written_out) => {
                r.exchange.write(data, &s).map_err(failure_code)?;
                if *written_out != 0 {
                    write_u32(*written_out, data.len() as u32)?;
                }
                Ok(Completion::Written(data.len() as u32))
            }
        }
    })
}
fn complete(handle: u64, completion: Completion) {
    match completion {
        Completion::Sent => notify(handle, status::SENDREQUEST_COMPLETE, 0, 0),
        Completion::Headers => notify(handle, status::HEADERS_AVAILABLE, 0, 0),
        Completion::Available(n) => {
            notify(handle, status::DATA_AVAILABLE, (&n as *const u32) as u64, 4)
        }
        Completion::Read(buffer, n) => notify(handle, status::READ_COMPLETE, buffer, n),
        Completion::Written(n) => {
            notify(handle, status::WRITE_COMPLETE, (&n as *const u32) as u64, 4)
        }
    }
}
struct Job {
    handle: u64,
    request: Arc<Lock<Request>>,
    work: Work,
}
extern "system" fn worker(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let job = unsafe { Box::from_raw(argument as *mut Job) };
        let outcome = perform(job.handle, &job.request, &job.work);
        // The request is free again before its completion is reported: the
        // callback may start the next operation.
        job.request.with(|r| r.busy = false);
        match outcome {
            Ok(c) => complete(job.handle, c),
            Err(e) => notify_error(job.handle, api_of(&job.work), e),
        }
        0
    }
}
/// Runs `work` now (synchronous session: the result is the return value)
/// or on a worker thread (asynchronous: TRUE now, the completion later).
fn submit(handle: u64, work: Work) -> i32 {
    let r = match request(handle) {
        Ok(r) => r,
        Err(e) => return fail(e, 0),
    };
    let Ok((_, s)) = session_of(handle) else {
        return fail(error::INVALID_HANDLE, 0);
    };
    if !s.with(|s| s.asynchronous) {
        return done(perform(handle, &r, &work).map(|_| ()));
    }
    if r.with(|r| core::mem::replace(&mut r.busy, true)) {
        return fail(error::INCORRECT_HANDLE_STATE, 0);
    }
    let boxed = Box::into_raw(Box::new(Job {
        handle,
        request: r,
        work,
    }));
    let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, boxed as u64, 0) };
    if thread == 0 {
        let job = unsafe { Box::from_raw(boxed) };
        job.request.with(|r| r.busy = false);
        return fail(error::NOT_ENOUGH_MEMORY, 0);
    }
    unsafe { DoorCloseHandle(thread) };
    done(Ok(()))
}

#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSendRequest(
    handle: usize,
    headers: usize,
    length: u32,
    optional: usize,
    optional_length: u32,
    total: u32,
    context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    let optional = door_windows_abi::widen(optional);
    let context = door_windows_abi::widen(context);
    {
        let prepared = (|| {
            let r = request(handle)?;
            let text = wide(headers, length)?;
            let mut body = vec![0u8; optional_length as usize];
            read(optional, &mut body)?;
            r.with(|r| -> Result<(), u32> {
                if !text.is_empty() {
                    add_headers(r, &text, ADD | REPLACE)?;
                }
                r.exchange.total = u64::from(total.max(optional_length));
                r.exchange.redirects = 0;
                Ok(())
            })?;
            set_common(handle, |c| c.context = context);
            Ok(body)
        })();
        match prepared {
            Ok(body) => submit(handle, Work::Send(body)),
            Err(e) => fail(e, 0),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpWriteData(
    handle: usize,
    buffer: usize,
    length: u32,
    written: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let written = door_windows_abi::widen(written);
    {
        let mut data = vec![0u8; length as usize];
        if read(buffer, &mut data).is_err() {
            return fail(error::INVALID_PARAMETER, 0);
        }
        match request(handle).map(|r| r.with(|r| r.stage)) {
            Ok(Stage::Sent) => submit(handle, Work::Write(data, written)),
            Ok(_) => fail(error::INCORRECT_HANDLE_STATE, 0),
            Err(e) => fail(e, 0),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpReceiveResponse(handle: usize, reserved: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let reserved = door_windows_abi::widen(reserved);
    {
        if reserved != 0 {
            return fail(error::INVALID_PARAMETER, 0);
        }
        match request(handle).map(|r| r.with(|r| r.stage)) {
            Ok(Stage::Open) => fail(error::INCORRECT_HANDLE_STATE, 0),
            Ok(_) => submit(handle, Work::Receive),
            Err(e) => fail(e, 0),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpQueryDataAvailable(handle: usize, available_out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let available_out = door_windows_abi::widen(available_out);
    {
        match request(handle).map(|r| r.with(|r| r.stage)) {
            Ok(Stage::Received) => submit(handle, Work::Available(available_out)),
            Ok(_) => fail(error::INCORRECT_HANDLE_STATE, 0),
            Err(e) => fail(e, 0),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpReadData(
    handle: usize,
    buffer: usize,
    length: u32,
    read_out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let read_out = door_windows_abi::widen(read_out);
    {
        match request(handle).map(|r| r.with(|r| r.stage)) {
            Ok(Stage::Received) => submit(handle, Work::Read(buffer, length, read_out)),
            Ok(_) => fail(error::INCORRECT_HANDLE_STATE, 0),
            Err(e) => fail(e, 0),
        }
    }
}

// ---- response headers -------------------------------------------------------------------
const QUERY_FLAG_REQUEST_HEADERS: u32 = 0x8000_0000;
const QUERY_FLAG_SYSTEMTIME: u32 = 0x4000_0000;
const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
const QUERY_FLAG_NUMBER64: u32 = 0x0800_0000;
const QUERY_CUSTOM: u32 = 65535;
const QUERY_VERSION: u32 = 18;
const QUERY_STATUS_CODE: u32 = 19;
const QUERY_STATUS_TEXT: u32 = 20;
const QUERY_RAW_HEADERS: u32 = 21;
const QUERY_RAW_HEADERS_CRLF: u32 = 22;
const QUERY_REQUEST_METHOD: u32 = 45;
/// WINHTTP_QUERY_* levels that name one header.
fn header_name(level: u32) -> Option<&'static str> {
    Some(match level {
        0 => "Mime-Version",
        1 => "Content-Type",
        2 => "Content-Transfer-Encoding",
        3 => "Content-ID",
        4 => "Content-Description",
        5 => "Content-Length",
        6 => "Content-Language",
        7 => "Allow",
        8 => "Public",
        9 => "Date",
        10 => "Expires",
        11 => "Last-Modified",
        12 => "Message-ID",
        13 => "URI",
        14 => "Derived-From",
        15 => "Cost",
        16 => "Link",
        17 => "Pragma",
        23 => "Connection",
        24 => "Accept",
        25 => "Accept-Charset",
        26 => "Accept-Encoding",
        27 => "Accept-Language",
        28 => "Authorization",
        29 => "Content-Encoding",
        30 => "Forwarded",
        31 => "From",
        32 => "If-Modified-Since",
        33 => "Location",
        34 => "Orig-URI",
        35 => "Referer",
        36 => "Retry-After",
        37 => "Server",
        38 => "Title",
        39 => "User-Agent",
        40 => "WWW-Authenticate",
        41 => "Proxy-Authenticate",
        42 => "Accept-Ranges",
        43 => "Set-Cookie",
        44 => "Cookie",
        46 => "Refresh",
        47 => "Content-Disposition",
        48 => "Age",
        49 => "Cache-Control",
        50 => "Content-Base",
        51 => "Content-Location",
        52 => "Content-MD5",
        53 => "Content-Range",
        54 => "ETag",
        55 => "Host",
        56 => "If-Match",
        57 => "If-None-Match",
        58 => "If-Range",
        59 => "If-Unmodified-Since",
        60 => "Max-Forwards",
        61 => "Proxy-Authorization",
        62 => "Range",
        63 => "Transfer-Encoding",
        64 => "Upgrade",
        65 => "Vary",
        66 => "Via",
        67 => "Warning",
        68 => "Expect",
        69 => "Proxy-Connection",
        70 => "Unless-Modified-Since",
        75 => "Proxy-Support",
        76 => "Authentication-Info",
        _ => return None,
    })
}
/// SYSTEMTIME of Unix seconds.
fn system_time(unix: i64) -> [u8; 16] {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    let (year, month, day) = door_http::civil(days);
    let weekday = (days + 4).rem_euclid(7);
    let fields = [
        year as u16,
        month as u16,
        weekday as u16,
        day as u16,
        (seconds / 3600) as u16,
        (seconds / 60 % 60) as u16,
        (seconds % 60) as u16,
        0,
    ];
    let mut b = [0u8; 16];
    for (i, f) in fields.iter().enumerate() {
        b[i * 2..i * 2 + 2].copy_from_slice(&f.to_le_bytes());
    }
    b
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpQueryHeaders(
    handle: usize,
    info: u32,
    name: usize,
    buffer: usize,
    length: usize,
    index: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    let index = door_windows_abi::widen(index);
    {
        let result = (|| {
            let r = request(handle)?;
            let level = info & 0xffff;
            let modifiers = info & 0xffff_0000;
            let requested = modifiers & QUERY_FLAG_REQUEST_HEADERS != 0;
            let (response, sent, verb) = r.with(|r| {
                (
                    r.exchange.response().cloned(),
                    r.exchange.sent.clone(),
                    r.exchange.verb.clone(),
                )
            });
            if !requested && response.is_none() {
                return Err(error::INCORRECT_HANDLE_STATE);
            }
            let which = if index != 0 {
                read_u32(index)? as usize
            } else {
                0
            };
            let headers = if requested {
                &sent
            } else {
                &response.as_ref().unwrap().headers
            };
            let value: String = match level {
                QUERY_CUSTOM => {
                    let n = wide(name, u32::MAX)?;
                    headers
                        .nth(&n, which)
                        .map(String::from)
                        .ok_or(error::HEADER_NOT_FOUND)?
                }
                QUERY_STATUS_CODE if !requested => {
                    alloc::format!("{}", response.as_ref().unwrap().status)
                }
                QUERY_STATUS_TEXT if !requested => response.as_ref().unwrap().reason.clone(),
                QUERY_VERSION if !requested => response.as_ref().unwrap().version.clone(),
                QUERY_REQUEST_METHOD => verb,
                QUERY_RAW_HEADERS_CRLF if !requested => response.as_ref().unwrap().raw.clone(),
                QUERY_RAW_HEADERS if !requested => {
                    let raw = &response.as_ref().unwrap().raw;
                    let mut s: String = raw
                        .trim_end_matches("\r\n")
                        .split("\r\n")
                        .collect::<Vec<_>>()
                        .join("\0");
                    s.push('\0');
                    s
                }
                level => {
                    let header = header_name(level).ok_or(error::INVALID_QUERY_REQUEST)?;
                    headers
                        .nth(header, which)
                        .map(String::from)
                        .ok_or(error::HEADER_NOT_FOUND)?
                }
            };
            if index != 0 {
                write_u32(index, which as u32 + 1)?;
            }
            let data: Vec<u8> = if modifiers & QUERY_FLAG_NUMBER != 0 {
                value
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| error::INVALID_QUERY_REQUEST)?
                    .to_le_bytes()
                    .to_vec()
            } else if modifiers & QUERY_FLAG_NUMBER64 != 0 {
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| error::INVALID_QUERY_REQUEST)?
                    .to_le_bytes()
                    .to_vec()
            } else if modifiers & QUERY_FLAG_SYSTEMTIME != 0 {
                system_time(door_http::parse_http_date(&value).ok_or(error::INVALID_QUERY_REQUEST)?)
                    .to_vec()
            } else {
                let mut d = wide_bytes(&value);
                let capacity = read_u32(length)?;
                if buffer == 0 || (capacity as usize) < d.len() {
                    write_u32(length, d.len() as u32)?;
                    return Err(error::INSUFFICIENT_BUFFER);
                }
                write(buffer, &d)?;
                d.truncate(d.len() - 2);
                return write_u32(length, d.len() as u32);
            };
            deliver(buffer, length, &data)
        })();
        done(result)
    }
}

// ---- authentication -------------------------------------------------------------------
const AUTH_SCHEME_BASIC: u32 = 0x1;
const AUTH_SCHEME_NTLM: u32 = 0x2;
const AUTH_SCHEME_PASSPORT: u32 = 0x4;
const AUTH_SCHEME_DIGEST: u32 = 0x8;
const AUTH_SCHEME_NEGOTIATE: u32 = 0x10;
const AUTH_TARGET_SERVER: u32 = 0;
const AUTH_TARGET_PROXY: u32 = 1;
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpQueryAuthSchemes(
    handle: usize,
    supported: usize,
    first: usize,
    target: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let supported = door_windows_abi::widen(supported);
    let first = door_windows_abi::widen(first);
    let target = door_windows_abi::widen(target);
    {
        let result = (|| {
            let r = request(handle)?;
            let response = r
                .with(|r| r.exchange.response().cloned())
                .ok_or(error::INCORRECT_HANDLE_STATE)?;
            let (header, who) = match response.status {
                401 => ("WWW-Authenticate", AUTH_TARGET_SERVER),
                407 => ("Proxy-Authenticate", AUTH_TARGET_PROXY),
                _ => return Err(INVALID_OPERATION),
            };
            let mut schemes = 0u32;
            let mut first_scheme = 0u32;
            for (name, value) in &response.headers.0 {
                if !name.eq_ignore_ascii_case(header) {
                    continue;
                }
                let bit = match value
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "basic" => AUTH_SCHEME_BASIC,
                    "ntlm" => AUTH_SCHEME_NTLM,
                    "passport1.4" => AUTH_SCHEME_PASSPORT,
                    "digest" => AUTH_SCHEME_DIGEST,
                    "negotiate" => AUTH_SCHEME_NEGOTIATE,
                    _ => 0,
                };
                if first_scheme == 0 {
                    first_scheme = bit;
                }
                schemes |= bit;
            }
            if schemes == 0 {
                return Err(INVALID_OPERATION);
            }
            write_u32(supported, schemes)?;
            write_u32(first, first_scheme)?;
            write_u32(target, who)
        })();
        done(result)
    }
}
mod error_more {
    pub const INVALID_OPERATION: u32 = 4317;
}
use error_more::INVALID_OPERATION;
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetCredentials(
    handle: usize,
    target: u32,
    scheme: u32,
    user: usize,
    password: usize,
    _reserved: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let user = door_windows_abi::widen(user);
    let password = door_windows_abi::widen(password);
    {
        let result = (|| {
            let r = request(handle)?;
            let target = match target {
                AUTH_TARGET_SERVER => door_windows_inet::exchange::Target::Server,
                AUTH_TARGET_PROXY => door_windows_inet::exchange::Target::Proxy,
                _ => return Err(error::INVALID_PARAMETER),
            };
            // One scheme per call; Passport needs a Microsoft account service.
            if !matches!(
                scheme,
                AUTH_SCHEME_BASIC | AUTH_SCHEME_NTLM | AUTH_SCHEME_DIGEST | AUTH_SCHEME_NEGOTIATE
            ) {
                return Err(error::INVALID_PARAMETER);
            }
            // NULL credentials name the logged-on user: Doors has no domain
            // identity to offer, so only explicit credentials are accepted.
            if user == 0 {
                return Err(error::INVALID_PARAMETER);
            }
            let user = wide(user, u32::MAX)?;
            let password = if password == 0 {
                String::new()
            } else {
                wide(password, u32::MAX)?
            };
            r.with(|r| r.exchange.set_credentials(target, user, password, scheme));
            Ok(())
        })();
        done(result)
    }
}

// ---- URLs and times ---------------------------------------------------------------------
const ICU_ESCAPE: u32 = 0x8000_0000;
const ICU_DECODE: u32 = 0x1000_0000;
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCrackUrl(
    url: usize,
    length: u32,
    flags: u32,
    components: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let components = door_windows_abi::widen(components);
    {
        let result = (|| {
            let size = layout::size(&URL_COMPONENTS) as u32;
            if url == 0 || components == 0 || read_u32(components)? != size {
                return Err(error::INVALID_PARAMETER);
            }
            let text = wide(url, length)?;
            let units: Vec<u16> = text.encode_utf16().collect();
            let c = crate::crack(&units)?;
            // (pointer field, length field, span) of each component.
            let fields = [
                (url_field(1), url_field(2), c.scheme),
                (url_field(4), url_field(5), c.host),
                (url_field(7), url_field(8), c.user),
                (url_field(9), url_field(10), c.password),
                (url_field(11), url_field(12), c.path),
                (url_field(13), url_field(14), c.extra),
            ];
            for (pointer_at, length_at, (start, span)) in fields {
                let pointer = read_pointer(components + pointer_at)?;
                let capacity = read_u32(components + length_at)?;
                if capacity == 0 {
                    continue;
                }
                if pointer == 0 {
                    // Decoding or escaping needs the caller's buffers.
                    if flags & (ICU_DECODE | ICU_ESCAPE) != 0 {
                        return Err(error::INVALID_PARAMETER);
                    }
                    write_pointer(components + pointer_at, url + (start * 2) as u64)?;
                    write_u32(components + length_at, span as u32)?;
                } else {
                    if (capacity as usize) <= span {
                        write_u32(components + length_at, span as u32 + 1)?;
                        return Err(error::INSUFFICIENT_BUFFER);
                    }
                    let mut bytes: Vec<u8> = units[start..start + span]
                        .iter()
                        .chain([&0u16])
                        .flat_map(|u| u.to_le_bytes())
                        .collect();
                    if flags & ICU_DECODE != 0 {
                        let decoded =
                            percent_decode(&String::from_utf16_lossy(&units[start..start + span]));
                        bytes = wide_bytes(&decoded);
                    }
                    write(pointer, &bytes)?;
                    write_u32(components + length_at, (bytes.len() / 2 - 1) as u32)?;
                }
            }
            write(components + url_field(3), &c.scheme_number.to_le_bytes())?;
            write(components + url_field(6), &c.port.to_le_bytes())
        })();
        done(result)
    }
}
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(
                core::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"),
                16,
            )
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCreateUrl(
    components: usize,
    _flags: u32,
    url: usize,
    length: usize,
) -> i32 {
    let components = door_windows_abi::widen(components);
    let url = door_windows_abi::widen(url);
    let length = door_windows_abi::widen(length);
    {
        let result = (|| {
            if components == 0
                || length == 0
                || read_u32(components)? != layout::size(&URL_COMPONENTS) as u32
            {
                return Err(error::INVALID_PARAMETER);
            }
            let field = |pointer_at: u64, length_at: u64| -> Result<String, u32> {
                let p = read_pointer(components + pointer_at)?;
                let n = read_u32(components + length_at)?;
                wide(p, if n == 0 { u32::MAX } else { n })
            };
            let mut scheme_number = [0u8; 4];
            read(components + url_field(3), &mut scheme_number)?;
            let number = i32::from_le_bytes(scheme_number);
            let mut scheme_name = field(url_field(1), url_field(2))?;
            if scheme_name.is_empty() {
                scheme_name = String::from(match number {
                    crate::scheme::HTTPS => "https",
                    crate::scheme::FTP => "ftp",
                    _ => "http",
                });
            }
            let mut port = [0u8; 2];
            read(components + url_field(6), &mut port)?;
            let (host, user, password, path, extra) = (
                field(url_field(4), url_field(5))?,
                field(url_field(7), url_field(8))?,
                field(url_field(9), url_field(10))?,
                field(url_field(11), url_field(12))?,
                field(url_field(13), url_field(14))?,
            );
            let text = crate::create(&crate::Parts {
                scheme: &scheme_name,
                number,
                host: &host,
                port: u16::from_le_bytes(port),
                user: &user,
                password: &password,
                path: &path,
                extra: &extra,
            });
            let units: Vec<u16> = text.encode_utf16().collect();
            let capacity = read_u32(length)?;
            if url == 0 || (capacity as usize) <= units.len() {
                write_u32(length, units.len() as u32 + 1)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(url, &wide_bytes(&text))?;
            write_u32(length, units.len() as u32)
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpTimeFromSystemTime(time: usize, out: usize) -> i32 {
    let time = door_windows_abi::widen(time);
    let out = door_windows_abi::widen(out);
    {
        let result = (|| {
            let mut b = [0u8; 16];
            read(time, &mut b)?;
            let f = |i: usize| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]);
            let days =
                door_http::days_from_civil(i64::from(f(0)), u32::from(f(1)), u32::from(f(3)));
            let unix =
                days * 86_400 + i64::from(f(4)) * 3600 + i64::from(f(5)) * 60 + i64::from(f(6));
            write(out, &wide_bytes(&door_http::http_date(unix)))
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpTimeToSystemTime(text: usize, time: usize) -> i32 {
    let text = door_windows_abi::widen(text);
    let time = door_windows_abi::widen(time);
    {
        let result = (|| {
            let unix = door_http::parse_http_date(&wide(text, u32::MAX)?)
                .ok_or(error::INVALID_PARAMETER)?;
            write(time, &system_time(unix))
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpCheckPlatform() -> i32 {
    1
}

// ---- proxy configuration ------------------------------------------------------------------
fn global_wide(text: &str) -> Result<u64, u32> {
    if text.is_empty() {
        return Ok(0);
    }
    let bytes = wide_bytes(text);
    // GlobalFree-compatible: the process heap.
    let memory = unsafe {
        RtlAllocateHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            0,
            bytes.len(),
        )
    };
    if memory == 0 {
        return Err(error::NOT_ENOUGH_MEMORY);
    }
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), memory as *mut u8, bytes.len()) };
    Ok(door_windows_abi::widen(memory))
}
/// The machine's WinHTTP proxy (netsh winhttp): direct access.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpGetDefaultProxyConfiguration(info: usize) -> i32 {
    let info = door_windows_abi::widen(info);
    {
        let mut b = [0u8; layout::size(&PROXY_INFO)];
        b[..4].copy_from_slice(&ACCESS_NO_PROXY.to_le_bytes());
        done(write(info, &b))
    }
}
/// The user's proxy settings (Settings > Network & internet > Proxy):
/// WINHTTP_CURRENT_USER_IE_PROXY_CONFIG { fAutoDetect, lpszAutoConfigUrl,
/// lpszProxy, lpszProxyBypass }, the strings GlobalAlloc'ed (NULL when
/// unset).
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpGetIEProxyConfigForCurrentUser(config: usize) -> i32 {
    let config = door_windows_abi::widen(config);
    {
        let result = (|| {
            let user = door_windows_inet::autoproxy::user_proxy();
            let string = |s: &Option<String>| -> Result<u64, u32> {
                match s {
                    Some(text) => global_wide(text),
                    None => Ok(0),
                }
            };
            // 32 bytes on x64, 16 on x86.
            const IE_PROXY_CONFIG: [Field; 4] = [U32, Pointer, Pointer, Pointer];
            let mut b = [0u8; layout::size(&IE_PROXY_CONFIG)];
            for (index, value) in [
                u64::from(user.auto_detect),
                string(&user.config_url)?,
                string(&user.server)?,
                string(&user.bypass)?,
            ]
            .into_iter()
            .enumerate()
            {
                layout::set(&mut b, &IE_PROXY_CONFIG, index, value);
            }
            write(config, &b)
        })();
        done(result)
    }
}
/// WINHTTP_AUTOPROXY_* flags.
const AUTOPROXY_AUTO_DETECT: u32 = 0x1;
const AUTOPROXY_CONFIG_URL: u32 = 0x2;
fn autoproxy_error(f: door_windows_inet::autoproxy::AutoFailure) -> u32 {
    use door_windows_inet::autoproxy::AutoFailure;
    match f {
        AutoFailure::NotDetected => error::AUTODETECTION_FAILED,
        AutoFailure::Download => error::UNABLE_TO_DOWNLOAD_SCRIPT,
        AutoFailure::Script => error::BAD_AUTO_PROXY_SCRIPT,
    }
}
/// The proxy for `url` from a proxy auto-config script: the one
/// WINHTTP_AUTOPROXY_OPTIONS names (CONFIG_URL), else one WPAD finds
/// (AUTO_DETECT). WINHTTP_PROXY_INFO gets the proxies before the script's
/// first DIRECT (NAMED_PROXY), or NO_PROXY, with GlobalAlloc'ed strings.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpGetProxyForUrl(
    session_handle: usize,
    url: usize,
    options: usize,
    info: usize,
) -> i32 {
    let session_handle = door_windows_abi::widen(session_handle);
    let url = door_windows_abi::widen(url);
    let options = door_windows_abi::widen(options);
    let info = door_windows_abi::widen(info);
    {
        let result = (|| {
            session(session_handle)?;
            let url = wide(url, u32::MAX)?;
            if url.is_empty() || options == 0 || info == 0 {
                return Err(error::INVALID_PARAMETER);
            }
            if !url.contains("://") {
                return Err(error::UNRECOGNIZED_SCHEME);
            }
            let flags = read_u32(options)?;
            let script_url = if flags & AUTOPROXY_CONFIG_URL != 0 {
                wide(read_pointer(options + 8)?, u32::MAX)?
            } else if flags & AUTOPROXY_AUTO_DETECT != 0 {
                door_windows_inet::autoproxy::detect(None).map_err(autoproxy_error)?
            } else {
                return Err(error::INVALID_PARAMETER);
            };
            let script =
                door_windows_inet::autoproxy::script(&script_url).map_err(autoproxy_error)?;
            let answer =
                door_windows_inet::autoproxy::evaluate(&script, &url).map_err(autoproxy_error)?;
            let mut b = [0u8; layout::size(&PROXY_INFO)];
            match door_windows_inet::proxyconfig::proxy_list(&answer) {
                Some(list) => {
                    b[..4].copy_from_slice(&ACCESS_NAMED_PROXY.to_le_bytes());
                    layout::set(&mut b, &PROXY_INFO, 1, global_wide(&list)?);
                }
                None => b[..4].copy_from_slice(&ACCESS_NO_PROXY.to_le_bytes()),
            }
            write(info, &b)
        })();
        done(result)
    }
}
/// WPAD: the script URL DNS discovery finds (`flags`: DHCP 1, DNS_A 2).
/// Doors' adapter gets no domain from DHCP, so this fails as Windows does
/// on a network without WPAD.
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpDetectAutoProxyConfigUrl(flags: u32, url: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    {
        let result = (|| {
            if flags & 0x3 == 0 || url == 0 {
                return Err(error::INVALID_PARAMETER);
            }
            let found = door_windows_inet::autoproxy::detect(None).map_err(autoproxy_error)?;
            write_pointer(url, global_wide(&found)?)
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WinHttpSetDefaultProxyConfiguration(_info: usize) -> i32 {
    {
        // Changing the machine proxy needs an administrator (ERROR_ACCESS_DENIED).
        fail(5, 0)
    }
}
