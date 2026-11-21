//! wininet.dll exports, Unicode and ANSI. Handles live in a process-wide
//! table; a request's exchange runs on the calling thread, or on a worker
//! thread for INTERNET_FLAG_ASYNC sessions (ERROR_IO_PENDING, then
//! INTERNET_STATUS_REQUEST_COMPLETE through the status callback).
use crate::{error, from_ansi, to_ansi};
mod ftp;
mod urlcache;
use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_http::{Headers, Url};
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_inet::{
    Failure, Tolerance,
    exchange::{Cookies, Event, Exchange, NoCookies, Settings, Stop, Target},
    sync::Lock,
};
/// Basic, NTLM, Digest and Negotiate.
const ALL_SCHEMES: u32 = door_http::auth::BASIC
    | door_http::auth::NTLM
    | door_http::auth::DIGEST
    | door_http::auth::NEGOTIATE;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(value: u32);
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorNetStatus(out: u64) -> u32;
}

// ---- application memory and text -------------------------------------------
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
pub(crate) const POINTER: usize = layout::POINTER;
/// A pointer-sized program value (pointers, DWORD_PTR), sign-extended on
/// x86.
pub(crate) fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(layout::get(&b, &[Pointer], 0))
}
pub(crate) fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
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
/// INTERNET_ASYNC_RESULT { dwResult (DWORD_PTR), dwError }: 16 bytes on
/// x64, 8 on x86.
const ASYNC_RESULT: [Field; 2] = [Pointer, U32];
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// Unicode or ANSI entry points.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Text {
    Wide,
    Ansi,
}
/// A string argument; `length` u32::MAX (-1L) or 0 means NUL-terminated.
fn text(pointer: u64, length: u32, kind: Text) -> Result<String, u32> {
    if pointer == 0 {
        return Ok(String::new());
    }
    let terminated = length == u32::MAX || length == 0;
    let limit = if terminated { 65536 } else { length as usize };
    match kind {
        Text::Wide => {
            let mut units = Vec::new();
            for i in 0..limit {
                let mut b = [0u8; 2];
                read(pointer + (i * 2) as u64, &mut b)?;
                let u = u16::from_le_bytes(b);
                if u == 0 && terminated {
                    break;
                }
                units.push(u);
            }
            String::from_utf16(&units).map_err(|_| error::INVALID_PARAMETER)
        }
        Text::Ansi => {
            let mut bytes = Vec::new();
            for i in 0..limit {
                let mut b = [0u8; 1];
                read(pointer + i as u64, &mut b)?;
                if b[0] == 0 && terminated {
                    break;
                }
                bytes.push(b[0]);
            }
            Ok(from_ansi(&bytes))
        }
    }
}
/// `value` NUL-terminated in the entry point's encoding.
fn encode(value: &str, kind: Text) -> Vec<u8> {
    match kind {
        Text::Wide => value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect(),
        Text::Ansi => to_ansi(value).into_iter().chain([0]).collect(),
    }
}
fn unit(kind: Text) -> u32 {
    if kind == Text::Wide { 2 } else { 1 }
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

// ---- handles -------------------------------------------------------------------
const TYPE_INTERNET: u32 = 1;
const TYPE_CONNECT_FTP: u32 = 2;
const TYPE_CONNECT_HTTP: u32 = 4;
const TYPE_FTP_FIND: u32 = 5;
const TYPE_FTP_FILE: u32 = 7;
const TYPE_HTTP_REQUEST: u32 = 13;
#[derive(Clone, Copy, Default)]
struct Common {
    callback: u64,
    context: u64,
    parent: u64,
}
struct Session {
    agent: String,
    access: u32,
    proxy: String,
    bypass: String,
    asynchronous: bool,
    /// Connect, send, receive (ms).
    timeouts: [u32; 3],
    cookies: Arc<Lock<crate::guest::Jar>>,
    /// INTERNET_OPTION_HTTP_DECODING on the session.
    decoding: bool,
}
struct Connect {
    host: String,
    port: u16,
    user: String,
    password: String,
}
struct Request {
    exchange: Exchange,
    flags: u32,
    security_flags: u32,
    /// INTERNET_OPTION_HTTP_DECODING set on the request itself.
    decoding: Option<bool>,
    busy: bool,
    /// The body read so far of a response the URL cache will keep.
    capture: Option<Vec<u8>>,
}
enum Object {
    Session(Arc<Lock<Session>>),
    Connect(Arc<Lock<Connect>>),
    Request(Arc<Lock<Request>>),
    /// An FTP connection, a file open on one, a FtpFindFirstFile search.
    Ftp(Arc<Lock<ftp::FtpSession>>),
    FtpFile(Arc<Lock<ftp::FtpFile>>),
    FtpFind(Arc<Lock<ftp::FtpFind>>),
}
struct Slot {
    common: Common,
    object: Object,
    cancel: Arc<AtomicBool>,
}
static TABLE: Lock<Vec<Option<Slot>>> = Lock::new(Vec::new());
fn kind_of(object: &Object) -> u32 {
    match object {
        Object::Session(_) => TYPE_INTERNET,
        Object::Connect(_) => TYPE_CONNECT_HTTP,
        Object::Request(_) => TYPE_HTTP_REQUEST,
        Object::Ftp(_) => TYPE_CONNECT_FTP,
        Object::FtpFile(_) => TYPE_FTP_FILE,
        Object::FtpFind(_) => TYPE_FTP_FIND,
    }
}
fn handle_value(index: usize) -> u64 {
    0x00CC_0004 + index as u64 * 4
}
fn index_of(handle: u64) -> Option<usize> {
    handle
        .checked_sub(0x00CC_0004)
        .filter(|v| v % 4 == 0)
        .map(|v| (v / 4) as usize)
}
fn insert(common: Common, object: Object, cancel: Arc<AtomicBool>) -> u64 {
    TABLE.with(|t| {
        let index = t.iter().position(Option::is_none).unwrap_or_else(|| {
            t.push(None);
            t.len() - 1
        });
        t[index] = Some(Slot {
            common,
            object,
            cancel,
        });
        handle_value(index)
    })
}
fn slot<R>(handle: u64, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
    let index = index_of(handle)?;
    TABLE.with(|t| t.get_mut(index).and_then(|s| s.as_mut()).map(f))
}
fn session(handle: u64) -> Result<Arc<Lock<Session>>, u32> {
    slot(handle, |s| match &s.object {
        Object::Session(x) => Ok(x.clone()),
        _ => Err(error::INCORRECT_HANDLE_TYPE),
    })
    .unwrap_or(Err(error::INVALID_HANDLE))
}
fn connect(handle: u64) -> Result<Arc<Lock<Connect>>, u32> {
    slot(handle, |s| match &s.object {
        Object::Connect(x) => Ok(x.clone()),
        _ => Err(error::INCORRECT_HANDLE_TYPE),
    })
    .unwrap_or(Err(error::INVALID_HANDLE))
}
fn request(handle: u64) -> Result<Arc<Lock<Request>>, u32> {
    slot(handle, |s| match &s.object {
        Object::Request(x) => Ok(x.clone()),
        _ => Err(error::INCORRECT_HANDLE_TYPE),
    })
    .unwrap_or(Err(error::INVALID_HANDLE))
}
fn common(handle: u64) -> Option<Common> {
    slot(handle, |s| s.common)
}
fn session_of(handle: u64) -> Result<Arc<Lock<Session>>, u32> {
    let mut h = handle;
    for _ in 0..3 {
        if let Ok(s) = session(h) {
            return Ok(s);
        }
        h = common(h).ok_or(error::INVALID_HANDLE)?.parent;
    }
    Err(error::INVALID_HANDLE)
}

// ---- cookies (the process's WinINet cookie store) ------------------------------------
pub use door_windows_inet::cookies::Jar;
static COOKIES: Lock<Option<Arc<Lock<Jar>>>> = Lock::new(None);
fn cookie_store() -> Arc<Lock<Jar>> {
    COOKIES.with(|c| {
        c.get_or_insert_with(|| Arc::new(Lock::new(Jar::default())))
            .clone()
    })
}
struct SessionCookies(Arc<Lock<Jar>>);
impl Cookies for SessionCookies {
    fn header(&mut self, url: &Url) -> Option<String> {
        let now = door_windows_inet::guest::now_unix();
        self.0.with(|j| {
            j.header(
                &url.host,
                &url.path,
                url.scheme == door_http::Scheme::Https,
                now,
            )
        })
    }
    fn store(&mut self, url: &Url, set_cookie: &str, now: i64) {
        self.0
            .with(|j| j.store(set_cookie, &url.host, &url.path, now));
    }
}

// ---- status callbacks --------------------------------------------------------------
mod status {
    pub const RESOLVING_NAME: u32 = 10;
    pub const NAME_RESOLVED: u32 = 11;
    pub const CONNECTING_TO_SERVER: u32 = 20;
    pub const CONNECTED_TO_SERVER: u32 = 21;
    pub const SENDING_REQUEST: u32 = 30;
    pub const REQUEST_SENT: u32 = 31;
    pub const RECEIVING_RESPONSE: u32 = 40;
    pub const RESPONSE_RECEIVED: u32 = 41;
    pub const CLOSING_CONNECTION: u32 = 50;
    pub const CONNECTION_CLOSED: u32 = 51;
    pub const HANDLE_CREATED: u32 = 60;
    pub const HANDLE_CLOSING: u32 = 70;
    pub const REQUEST_COMPLETE: u32 = 100;
    pub const REDIRECT: u32 = 110;
}
/// INTERNET_STATUS_CALLBACK(hInternet, dwContext, dwInternetStatus,
/// lpvStatusInformation, dwStatusInformationLength) at pointer width.
type Callback = extern "system" fn(usize, usize, u32, usize, u32);
fn notify(handle: u64, code: u32, info: u64, length: u32) {
    let Some(c) = common(handle) else { return };
    if c.callback != 0 && c.context != 0 {
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
/// INTERNET_STATUS_HANDLE_CREATED with INTERNET_ASYNC_RESULT { handle, 0 }.
fn handle_created(handle: u64) {
    let mut result = [0u8; layout::size(&ASYNC_RESULT)];
    layout::set(&mut result, &ASYNC_RESULT, 0, handle);
    notify(
        handle,
        status::HANDLE_CREATED,
        result.as_ptr() as u64,
        result.len() as u32,
    );
}
fn notify_text(handle: u64, code: u32, value: &str) {
    // WinINet passes ANSI host names and URLs in its callbacks.
    let bytes: Vec<u8> = to_ansi(value).into_iter().chain([0]).collect();
    notify(handle, code, bytes.as_ptr() as u64, bytes.len() as u32);
}
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
        Event::SecureFailure(_) => (),
        Event::ClosingConnection => notify(handle, status::CLOSING_CONNECTION, 0, 0),
        Event::ConnectionClosed => notify(handle, status::CONNECTION_CLOSED, 0, 0),
    }
}
fn failure_code(f: Failure) -> u32 {
    match f {
        Failure::NameNotResolved => error::NAME_NOT_RESOLVED,
        Failure::CannotConnect => error::CANNOT_CONNECT,
        Failure::Timeout => error::TIMEOUT,
        Failure::ConnectionError => error::CONNECTION_ABORTED,
        Failure::Secure(flags) => {
            use door_windows_inet::secure;
            if flags & secure::INVALID_CA != 0 {
                error::INVALID_CA
            } else if flags & secure::CERT_CN_INVALID != 0 {
                error::SEC_CERT_CN_INVALID
            } else if flags & secure::CERT_DATE_INVALID != 0 {
                error::SEC_CERT_DATE_INVALID
            } else if flags & secure::INVALID_CERT != 0 {
                error::SEC_INVALID_CERT
            } else {
                error::SECURITY_CHANNEL_ERROR
            }
        }
        Failure::InvalidResponse => error::HTTP_INVALID_SERVER_RESPONSE,
        Failure::Cancelled => error::OPERATION_CANCELLED,
        Failure::OutOfMemory => error::NOT_ENOUGH_MEMORY,
    }
}
fn stop_code(s: Stop) -> u32 {
    match s {
        Stop::Failed(f) => failure_code(f),
        Stop::RedirectFailed => error::HTTP_REDIRECT_FAILED,
    }
}

// ---- InternetOpen / Connect / HttpOpenRequest / CloseHandle --------------------------
const OPEN_TYPE_PRECONFIG: u32 = 0;
const OPEN_TYPE_DIRECT: u32 = 1;
const OPEN_TYPE_PROXY: u32 = 3;
const OPEN_TYPE_PRECONFIG_NO_AUTOPROXY: u32 = 4;
const FLAG_ASYNC: u32 = 0x1000_0000;
const FLAG_SECURE: u32 = 0x0080_0000;
const FLAG_NO_AUTO_REDIRECT: u32 = 0x0020_0000;
const FLAG_NO_COOKIES: u32 = 0x0008_0000;
const FLAG_IGNORE_CERT_CN_INVALID: u32 = 0x0000_1000;
const FLAG_IGNORE_CERT_DATE_INVALID: u32 = 0x0000_2000;
const FLAG_IGNORE_REDIRECT_TO_HTTP: u32 = 0x0000_8000;
const SERVICE_FTP: u32 = 1;
const SERVICE_HTTP: u32 = 3;

fn internet_open(agent: u64, access: u32, proxy: u64, bypass: u64, flags: u32, kind: Text) -> u64 {
    let result = (|| {
        if !matches!(
            access,
            OPEN_TYPE_PRECONFIG
                | OPEN_TYPE_DIRECT
                | OPEN_TYPE_PROXY
                | OPEN_TYPE_PRECONFIG_NO_AUTOPROXY
        ) {
            return Err(error::INVALID_PARAMETER);
        }
        Ok(Session {
            agent: text(agent, u32::MAX, kind)?,
            access,
            proxy: text(proxy, u32::MAX, kind)?,
            bypass: text(bypass, u32::MAX, kind)?,
            asynchronous: flags & FLAG_ASYNC != 0,
            timeouts: [60_000, 30_000, 30_000],
            cookies: cookie_store(),
            decoding: false,
        })
    })();
    match result {
        Ok(s) => {
            unsafe { RtlSetLastWin32Error(0) };
            insert(
                Common::default(),
                Object::Session(Arc::new(Lock::new(s))),
                Arc::new(AtomicBool::new(false)),
            )
        }
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetOpenW(
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
        internet_open(agent, access, proxy, bypass, flags, Text::Wide)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetOpenA(
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
        internet_open(agent, access, proxy, bypass, flags, Text::Ansi)
    })())
}
#[allow(clippy::too_many_arguments)]
fn internet_connect(
    parent: u64,
    server: u64,
    port: u16,
    user: u64,
    password: u64,
    service: u32,
    _flags: u32,
    context: u64,
    kind: Text,
) -> u64 {
    // FTP logs in now and answers its own connection handle.
    if service == SERVICE_FTP {
        let opened = (|| {
            let host = text(server, u32::MAX, kind)?;
            let user = text(user, u32::MAX, kind)?;
            let password = text(password, u32::MAX, kind)?;
            if host.is_empty() {
                return Err(error::INVALID_PARAMETER);
            }
            ftp::connect(parent, &host, port, &user, &password, context)
        })();
        return match opened {
            Ok(h) => {
                unsafe { RtlSetLastWin32Error(0) };
                h
            }
            Err(e) => fail(e, 0),
        };
    }
    let result = (|| {
        session(parent)?;
        if service != SERVICE_HTTP {
            // Gopher is not implemented (Windows dropped it too).
            return Err(error::PROTOCOL_NOT_FOUND);
        }
        let host = text(server, u32::MAX, kind)?;
        if host.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        Ok(Connect {
            host,
            port,
            user: text(user, u32::MAX, kind)?,
            password: text(password, u32::MAX, kind)?,
        })
    })();
    match result {
        Ok(c) => {
            let callback = common(parent).map_or(0, |c| c.callback);
            let h = insert(
                Common {
                    callback,
                    context,
                    parent,
                },
                Object::Connect(Arc::new(Lock::new(c))),
                Arc::new(AtomicBool::new(false)),
            );
            handle_created(h);
            h
        }
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetConnectW(
    parent: usize,
    server: usize,
    port: u16,
    user: usize,
    password: usize,
    service: u32,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let server = door_windows_abi::widen(server);
    let user = door_windows_abi::widen(user);
    let password = door_windows_abi::widen(password);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        internet_connect(
            parent,
            server,
            port,
            user,
            password,
            service,
            flags,
            context,
            Text::Wide,
        )
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetConnectA(
    parent: usize,
    server: usize,
    port: u16,
    user: usize,
    password: usize,
    service: u32,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let server = door_windows_abi::widen(server);
    let user = door_windows_abi::widen(user);
    let password = door_windows_abi::widen(password);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        internet_connect(
            parent,
            server,
            port,
            user,
            password,
            service,
            flags,
            context,
            Text::Ansi,
        )
    })())
}
/// What a new request handle starts with.
struct Opened {
    url: Url,
    verb: String,
    version: String,
    headers: Headers,
    credentials: Option<(String, String)>,
}
fn new_request(parent: u64, opened: Opened, flags: u32, context: u64) -> u64 {
    let Opened {
        url,
        verb,
        version,
        headers,
        credentials,
    } = opened;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut exchange = Exchange::new(verb, url, version, headers, cancel.clone());
    // WinINet answers any scheme the server offers with these.
    if let Some((user, password)) = credentials {
        exchange.set_credentials(Target::Server, user, password, ALL_SCHEMES);
    }
    let security_flags = flags & (FLAG_IGNORE_CERT_CN_INVALID | FLAG_IGNORE_CERT_DATE_INVALID);
    let r = Request {
        exchange,
        flags,
        security_flags,
        decoding: None,
        busy: false,
        capture: None,
    };
    let callback = common(parent).map_or(0, |c| c.callback);
    let h = insert(
        Common {
            callback,
            context,
            parent,
        },
        Object::Request(Arc::new(Lock::new(r))),
        cancel,
    );
    handle_created(h);
    h
}
#[allow(clippy::too_many_arguments)]
fn http_open_request(
    parent: u64,
    verb: u64,
    object: u64,
    version: u64,
    referrer: u64,
    accept: u64,
    flags: u32,
    context: u64,
    kind: Text,
) -> u64 {
    let result = (|| {
        let c = connect(parent)?;
        let (host, port, user, password) =
            c.with(|c| (c.host.clone(), c.port, c.user.clone(), c.password.clone()));
        let verb = if verb == 0 {
            String::from("GET")
        } else {
            text(verb, u32::MAX, kind)?
        };
        let mut path = if object == 0 {
            String::from("/")
        } else {
            text(object, u32::MAX, kind)?
        };
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        let version = if version == 0 {
            String::from("HTTP/1.1")
        } else {
            text(version, u32::MAX, kind)?
        };
        let mut headers = Headers::default();
        let referrer = text(referrer, u32::MAX, kind)?;
        if !referrer.is_empty() {
            headers.set("Referer", &referrer);
        }
        if accept != 0 {
            let mut types = Vec::new();
            for i in 0..64u64 {
                let p = read_pointer(accept + i * POINTER as u64)?;
                if p == 0 {
                    break;
                }
                types.push(text(p, u32::MAX, kind)?);
            }
            if !types.is_empty() {
                headers.set("Accept", &types.join(", "));
            }
        }
        let secure = flags & FLAG_SECURE != 0;
        let scheme = if secure {
            door_http::Scheme::Https
        } else {
            door_http::Scheme::Http
        };
        let port = if port == 0 {
            scheme.default_port()
        } else {
            port
        };
        let url = Url {
            scheme,
            user: String::new(),
            password: String::new(),
            host,
            port,
            path,
            fragment: String::new(),
        };
        let credentials = (!user.is_empty()).then_some((user, password));
        Ok((url, verb, version, headers, credentials))
    })();
    match result {
        Ok((url, verb, version, headers, credentials)) => new_request(
            parent,
            Opened {
                url,
                verb,
                version,
                headers,
                credentials,
            },
            flags,
            context,
        ),
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpOpenRequestW(
    parent: usize,
    verb: usize,
    object: usize,
    version: usize,
    referrer: usize,
    accept: usize,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let verb = door_windows_abi::widen(verb);
    let object = door_windows_abi::widen(object);
    let version = door_windows_abi::widen(version);
    let referrer = door_windows_abi::widen(referrer);
    let accept = door_windows_abi::widen(accept);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        http_open_request(
            parent,
            verb,
            object,
            version,
            referrer,
            accept,
            flags,
            context,
            Text::Wide,
        )
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpOpenRequestA(
    parent: usize,
    verb: usize,
    object: usize,
    version: usize,
    referrer: usize,
    accept: usize,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let verb = door_windows_abi::widen(verb);
    let object = door_windows_abi::widen(object);
    let version = door_windows_abi::widen(version);
    let referrer = door_windows_abi::widen(referrer);
    let accept = door_windows_abi::widen(accept);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        http_open_request(
            parent,
            verb,
            object,
            version,
            referrer,
            accept,
            flags,
            context,
            Text::Ansi,
        )
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCloseHandle(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let Some(index) = index_of(handle) else {
            return fail(error::INVALID_HANDLE, 0);
        };
        let Some(cancel) = slot(handle, |s| s.cancel.clone()) else {
            return fail(error::INVALID_HANDLE, 0);
        };
        cancel.store(true, Ordering::Release);
        // An FTP file ends its transfer; a search frees its connection.
        match slot(handle, |s| match &s.object {
            Object::FtpFile(f) => (Some(f.clone()), None),
            Object::FtpFind(f) => (None, Some(f.clone())),
            _ => (None, None),
        }) {
            Some((Some(file), _)) => ftp::close_file(&file),
            Some((_, Some(find))) => ftp::close_find(&find),
            _ => (),
        }
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

// ---- the exchange ---------------------------------------------------------------------
fn settings(handle: u64, r: &Request) -> Result<Settings, u32> {
    let s = session_of(handle)?;
    Ok(s.with(|s| {
        let host = &r.exchange.url.host;
        let proxy = match s.access {
            OPEN_TYPE_PROXY if !door_windows_inet::bypassed(host, &s.bypass) => {
                door_windows_inet::parse_proxy(&s.proxy)
            }
            // The user's settings: a setup script (unless NO_AUTOPROXY),
            // else the manual proxy and its exceptions.
            OPEN_TYPE_PRECONFIG | OPEN_TYPE_PRECONFIG_NO_AUTOPROXY => {
                let user = door_windows_inet::autoproxy::user_proxy();
                let scripted = user
                    .config_url
                    .as_deref()
                    .filter(|_| s.access == OPEN_TYPE_PRECONFIG)
                    .and_then(|script| door_windows_inet::autoproxy::script(script).ok())
                    .and_then(|script| {
                        door_windows_inet::autoproxy::evaluate(&script, &url_text(&r.exchange.url))
                            .ok()
                    });
                match scripted {
                    Some(answer) => {
                        door_windows_inet::proxyconfig::proxy_list(&answer).and_then(|list| {
                            door_windows_inet::parse_proxy(list.split(';').next().unwrap_or(""))
                        })
                    }
                    None => user
                        .server
                        .filter(|_| {
                            !door_windows_inet::bypassed(host, user.bypass.as_deref().unwrap_or(""))
                        })
                        .and_then(|server| door_windows_inet::parse_proxy(&server)),
                }
            }
            _ => None,
        };
        let mut tolerance = Tolerance::from_flags(r.security_flags);
        tolerance.common_name |= r.flags & FLAG_IGNORE_CERT_CN_INVALID != 0;
        tolerance.date |= r.flags & FLAG_IGNORE_CERT_DATE_INVALID != 0;
        Settings {
            timeouts: [s.timeouts[0], s.timeouts[0], s.timeouts[1], s.timeouts[2]],
            proxy,
            agent: s.agent.clone(),
            follow_redirects: r.flags & FLAG_NO_AUTO_REDIRECT == 0,
            allow_downgrade: r.flags & FLAG_IGNORE_REDIRECT_TO_HTTP != 0,
            max_redirects: 100,
            tolerance,
            keep_alive: true,
            decode: r.decoding.unwrap_or(s.decoding)
                && r.exchange.headers.get("Accept-Encoding").is_some(),
        }
    }))
}
fn cookies(handle: u64, r: &Request) -> Box<dyn Cookies> {
    if r.flags & FLAG_NO_COOKIES != 0 {
        return Box::new(NoCookies);
    }
    match session_of(handle) {
        Ok(s) => Box::new(SessionCookies(s.with(|s| s.cookies.clone()))),
        Err(_) => Box::new(NoCookies),
    }
}
/// Send, then read the response head (HttpSendRequest does both).
fn send_and_receive(
    handle: u64,
    r: &Arc<Lock<Request>>,
    optional: &[u8],
    finish: bool,
) -> Result<(), u32> {
    r.with(|r| {
        // INTERNET_FLAG_FROM_CACHE / OFFLINE: the URL cache answers, or
        // ERROR_FILE_NOT_FOUND.
        if r.flags & FLAG_FROM_CACHE != 0 && r.exchange.verb.eq_ignore_ascii_case("GET") {
            let url = url_text(&r.exchange.url);
            let (head, body) = urlcache::load(&url).ok_or(error::FILE_NOT_FOUND)?;
            r.exchange.from_store(&head, &body);
            r.capture = None;
            return Ok(());
        }
        let s = settings(handle, r)?;
        let mut jar = cookies(handle, r);
        r.exchange
            .send(&s, jar.as_mut(), optional, &mut |e| event(handle, e))
            .map_err(failure_code)?;
        if finish {
            r.exchange
                .receive(&s, jar.as_mut(), &mut |e| event(handle, e))
                .map_err(stop_code)?;
            r.capture = cacheable(r).then(Vec::new);
        }
        Ok(())
    })
}
/// INTERNET_FLAG_FROM_CACHE (= OFFLINE) and INTERNET_FLAG_NO_CACHE_WRITE.
const FLAG_FROM_CACHE: u32 = 0x0100_0000;
const FLAG_NO_CACHE_WRITE: u32 = 0x0400_0000;
/// A URL as the cache keys it: scheme://host[:port]/path?query.
fn url_text(url: &Url) -> String {
    let (scheme, default) = match url.scheme {
        door_http::Scheme::Https => ("https", 443),
        door_http::Scheme::Http => ("http", 80),
    };
    if url.port == default {
        alloc::format!("{scheme}://{}{}", url.host, url.path)
    } else {
        alloc::format!("{scheme}://{}:{}{}", url.host, url.port, url.path)
    }
}
/// The response may go to the URL cache as it is read: a GET answered 200
/// that does not forbid storing, from a request that does not.
fn cacheable(r: &Request) -> bool {
    r.flags & FLAG_NO_CACHE_WRITE == 0
        && r.exchange.response().is_some_and(|response| {
            door_windows_inet::cache::storable(
                &r.exchange.verb,
                response.status,
                response.headers.get("Cache-Control"),
            )
        })
}
/// Body bytes the application read: kept for the cache, which takes the
/// response once its end is read.
fn capture(r: &mut Request, bytes: &[u8]) {
    let Some(kept) = r.capture.as_mut() else {
        return;
    };
    if !bytes.is_empty() {
        if kept.len() + bytes.len() > urlcache::MAX_STORED {
            r.capture = None;
        } else {
            kept.extend_from_slice(bytes);
        }
        return;
    }
    if !r.exchange.finished() {
        return;
    }
    let body = r.capture.take().unwrap_or_default();
    if let Some(response) = r.exchange.response() {
        let expires = response
            .headers
            .get("Expires")
            .and_then(door_http::parse_http_date)
            .map(|unix| (unix + 11_644_473_600).max(0) as u64 * 10_000_000);
        urlcache::store(
            &url_text(&r.exchange.url),
            &response.raw,
            &body,
            response.headers.get("Cache-Control"),
            expires,
        );
    }
}
fn end_request(handle: u64, r: &Arc<Lock<Request>>) -> Result<(), u32> {
    r.with(|r| {
        let s = settings(handle, r)?;
        let mut jar = cookies(handle, r);
        r.exchange
            .receive(&s, jar.as_mut(), &mut |e| event(handle, e))
            .map_err(stop_code)
    })
}
fn receive_timeout(handle: u64) -> u32 {
    session_of(handle).map_or(30_000, |s| s.with(|s| s.timeouts[2]))
}

// ---- asynchronous operation ------------------------------------------------------------
enum Work {
    Send(Vec<u8>, bool),
    /// InternetOpenUrl: completes with the request handle.
    OpenUrl,
    End,
    Read(u64, u32, u64),
    Available(u64),
    Write(Vec<u8>, u64),
}
struct Job {
    handle: u64,
    request: Arc<Lock<Request>>,
    work: Work,
}
fn perform(job: &Job) -> Result<u64, u32> {
    let h = job.handle;
    let r = &job.request;
    match &job.work {
        Work::Send(optional, finish) => send_and_receive(h, r, optional, *finish).map(|()| 1),
        Work::End => end_request(h, r).map(|()| 1),
        Work::OpenUrl => send_and_receive(h, r, &[], true).map(|()| h),
        Work::Read(buffer, length, read_out) => {
            let timeout = receive_timeout(h);
            let bytes = r
                .with(|r| {
                    let bytes = r.exchange.read(*length as usize, true, timeout)?;
                    capture(r, &bytes);
                    Ok(bytes)
                })
                .map_err(failure_code)?;
            write(*buffer, &bytes)?;
            if *read_out != 0 {
                write_u32(*read_out, bytes.len() as u32)?;
            }
            Ok(1)
        }
        Work::Available(out) => {
            let timeout = receive_timeout(h);
            let n = r
                .with(|r| r.exchange.available(timeout))
                .map_err(failure_code)?;
            if *out != 0 {
                write_u32(*out, n as u32)?;
            }
            Ok(1)
        }
        Work::Write(data, written) => {
            r.with(|r| {
                let s = settings(h, r)?;
                r.exchange.write(data, &s).map_err(failure_code)
            })?;
            if *written != 0 {
                write_u32(*written, data.len() as u32)?;
            }
            Ok(1)
        }
    }
}
extern "system" fn worker(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let job = unsafe { Box::from_raw(argument as *mut Job) };
        let outcome = perform(&job);
        job.request.with(|r| r.busy = false);
        // INTERNET_ASYNC_RESULT { dwResult, dwError }.
        let mut result = [0u8; layout::size(&ASYNC_RESULT)];
        match outcome {
            Ok(v) => layout::set(&mut result, &ASYNC_RESULT, 0, v),
            Err(e) => layout::set(&mut result, &ASYNC_RESULT, 1, u64::from(e)),
        }
        notify(
            job.handle,
            status::REQUEST_COMPLETE,
            result.as_ptr() as u64,
            result.len() as u32,
        );
        0
    }
}
fn submit(handle: u64, work: Work) -> i32 {
    let r = match request(handle) {
        Ok(r) => r,
        Err(e) => return fail(e, 0),
    };
    let asynchronous = session_of(handle).is_ok_and(|s| s.with(|s| s.asynchronous));
    let job = Job {
        handle,
        request: r.clone(),
        work,
    };
    if !asynchronous {
        return done(perform(&job).map(|_| ()));
    }
    if r.with(|r| core::mem::replace(&mut r.busy, true)) {
        return fail(error::INCORRECT_HANDLE_STATE, 0);
    }
    let boxed = Box::into_raw(Box::new(job));
    let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, boxed as u64, 0) };
    if thread == 0 {
        let job = unsafe { Box::from_raw(boxed) };
        job.request.with(|r| r.busy = false);
        return fail(error::NOT_ENOUGH_MEMORY, 0);
    }
    unsafe { DoorCloseHandle(thread) };
    fail(error::IO_PENDING, 0)
}

// ---- request headers ------------------------------------------------------------------
const ADDREQ_FLAG_ADD_IF_NEW: u32 = 0x1000_0000;
const ADDREQ_FLAG_ADD: u32 = 0x2000_0000;
const ADDREQ_FLAG_COALESCE_WITH_COMMA: u32 = 0x4000_0000;
const ADDREQ_FLAG_COALESCE_WITH_SEMICOLON: u32 = 0x0100_0000;
const ADDREQ_FLAG_REPLACE: u32 = 0x8000_0000;
fn add_headers(headers: &mut Headers, lines: &str, modifiers: u32) -> Result<(), u32> {
    let lines = door_http::parse_header_lines(lines).map_err(|_| error::HTTP_INVALID_HEADER)?;
    for (name, value) in lines {
        let exists = headers.get(&name).is_some();
        if modifiers & ADDREQ_FLAG_ADD_IF_NEW != 0 && exists {
            return Err(error::HTTP_HEADER_ALREADY_EXISTS);
        }
        if modifiers & ADDREQ_FLAG_COALESCE_WITH_COMMA != 0 && exists {
            headers.coalesce(&name, &value, ", ");
        } else if modifiers & ADDREQ_FLAG_COALESCE_WITH_SEMICOLON != 0 && exists {
            headers.coalesce(&name, &value, "; ");
        } else if modifiers & ADDREQ_FLAG_REPLACE != 0 {
            if value.is_empty() {
                headers.remove(&name);
            } else if exists || modifiers & ADDREQ_FLAG_ADD != 0 {
                headers.set(&name, &value);
            } else {
                return Err(error::HTTP_HEADER_NOT_FOUND);
            }
        } else {
            headers.add(&name, &value);
        }
    }
    Ok(())
}
fn http_add_request_headers(
    handle: u64,
    headers: u64,
    length: u32,
    modifiers: u32,
    kind: Text,
) -> i32 {
    let result = (|| {
        let r = request(handle)?;
        let lines = text(headers, length, kind)?;
        r.with(|r| add_headers(&mut r.exchange.headers, &lines, modifiers))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpAddRequestHeadersW(
    handle: usize,
    headers: usize,
    length: u32,
    modifiers: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    http_add_request_headers(handle, headers, length, modifiers, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpAddRequestHeadersA(
    handle: usize,
    headers: usize,
    length: u32,
    modifiers: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    http_add_request_headers(handle, headers, length, modifiers, Text::Ansi)
}
fn http_send_request(
    handle: u64,
    headers: u64,
    length: u32,
    optional: u64,
    optional_length: u32,
    kind: Text,
) -> i32 {
    let prepared = (|| {
        let r = request(handle)?;
        let lines = text(headers, length, kind)?;
        let mut body = vec![0u8; optional_length as usize];
        read(optional, &mut body)?;
        r.with(|r| -> Result<(), u32> {
            if !lines.is_empty() {
                add_headers(
                    &mut r.exchange.headers,
                    &lines,
                    ADDREQ_FLAG_ADD | ADDREQ_FLAG_REPLACE,
                )?;
            }
            r.exchange.total = u64::from(optional_length);
            r.exchange.redirects = 0;
            Ok(())
        })?;
        Ok(body)
    })();
    match prepared {
        Ok(body) => submit(handle, Work::Send(body, true)),
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpSendRequestW(
    handle: usize,
    headers: usize,
    length: u32,
    optional: usize,
    optional_length: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    let optional = door_windows_abi::widen(optional);
    {
        http_send_request(
            handle,
            headers,
            length,
            optional,
            optional_length,
            Text::Wide,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpSendRequestA(
    handle: usize,
    headers: usize,
    length: u32,
    optional: usize,
    optional_length: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let headers = door_windows_abi::widen(headers);
    let optional = door_windows_abi::widen(optional);
    {
        http_send_request(
            handle,
            headers,
            length,
            optional,
            optional_length,
            Text::Ansi,
        )
    }
}
/// INTERNET_BUFFERS: dwStructSize, Next, lpcszHeader, dwHeadersLength,
/// dwHeadersTotal, lpvBuffer, dwBufferLength, dwBufferTotal, dwOffsetLow,
/// dwOffsetHigh (56 bytes on x64, 40 on x86).
const INTERNET_BUFFERS: [Field; 10] =
    [U32, Pointer, Pointer, U32, U32, Pointer, U32, U32, U32, U32];
fn http_send_request_ex(handle: u64, buffers: u64, kind: Text) -> i32 {
    let prepared = (|| {
        let r = request(handle)?;
        let (headers, header_length, total) = if buffers == 0 {
            (0, 0, 0)
        } else {
            let at = |index| buffers + layout::offset(&INTERNET_BUFFERS, index) as u64;
            (read_pointer(at(2))?, read_u32(at(3))?, read_u32(at(9))?)
        };
        let lines = text(headers, header_length, kind)?;
        r.with(|r| -> Result<(), u32> {
            if !lines.is_empty() {
                add_headers(
                    &mut r.exchange.headers,
                    &lines,
                    ADDREQ_FLAG_ADD | ADDREQ_FLAG_REPLACE,
                )?;
            }
            r.exchange.total = u64::from(total);
            Ok(())
        })
    })();
    match prepared {
        Ok(()) => submit(handle, Work::Send(Vec::new(), false)),
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpSendRequestExW(
    handle: usize,
    buffers_in: usize,
    _buffers_out: usize,
    _flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffers_in = door_windows_abi::widen(buffers_in);
    http_send_request_ex(handle, buffers_in, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpSendRequestExA(
    handle: usize,
    buffers_in: usize,
    _buffers_out: usize,
    _flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffers_in = door_windows_abi::widen(buffers_in);
    http_send_request_ex(handle, buffers_in, Text::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpEndRequestW(
    handle: usize,
    _buffers: usize,
    _flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    submit(handle, Work::End)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpEndRequestA(
    handle: usize,
    _buffers: usize,
    _flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    submit(handle, Work::End)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetWriteFile(
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
        if let Some(file) = ftp::file(handle) {
            let result = ftp::write_file(&file, &data).and_then(|()| {
                if written != 0 {
                    write_u32(written, length)?;
                }
                Ok(())
            });
            return done(result);
        }
        submit(handle, Work::Write(data, written))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetReadFile(
    handle: usize,
    buffer: usize,
    length: u32,
    read_out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let read_out = door_windows_abi::widen(read_out);
    {
        if read_out == 0 {
            return fail(error::INVALID_PARAMETER, 0);
        }
        if let Some(file) = ftp::file(handle) {
            let result = ftp::read_file(&file, length as usize).and_then(|bytes| {
                write(buffer, &bytes)?;
                write_u32(read_out, bytes.len() as u32)
            });
            return done(result);
        }
        submit(handle, Work::Read(buffer, length, read_out))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetQueryDataAvailable(
    handle: usize,
    available: usize,
    _flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let available = door_windows_abi::widen(available);
    {
        if ftp::file(handle).is_some() {
            // A data connection gives no count ahead of the read.
            return done(if available != 0 {
                write_u32(available, 1)
            } else {
                Ok(())
            });
        }
        submit(handle, Work::Available(available))
    }
}

// ---- InternetOpenUrl ------------------------------------------------------------------
fn internet_open_url(
    parent: u64,
    url: u64,
    headers: u64,
    length: u32,
    flags: u32,
    context: u64,
    kind: Text,
) -> u64 {
    // ftp:// URLs open the file on a connection of their own.
    if let Ok(text_url) = text(url, u32::MAX, kind)
        && text_url.trim().len() > 6
        && text_url.trim()[..6].eq_ignore_ascii_case("ftp://")
    {
        return match ftp::open_url(parent, text_url.trim(), flags, context) {
            Ok(h) => {
                unsafe { RtlSetLastWin32Error(0) };
                h
            }
            Err(e) => fail(e, 0),
        };
    }
    let prepared = (|| {
        session(parent)?;
        let text_url = text(url, u32::MAX, kind)?;
        let parsed = door_http::parse_url(&text_url).map_err(|e| match e {
            door_http::UrlError::UnrecognizedScheme => error::UNRECOGNIZED_SCHEME,
            door_http::UrlError::Invalid => error::INVALID_URL,
        })?;
        let mut extra = Headers::default();
        let lines = text(headers, length, kind)?;
        if !lines.is_empty() {
            add_headers(&mut extra, &lines, ADDREQ_FLAG_ADD)?;
        }
        let credentials =
            (!parsed.user.is_empty()).then(|| (parsed.user.clone(), parsed.password.clone()));
        let flags = flags
            | if parsed.scheme == door_http::Scheme::Https {
                FLAG_SECURE
            } else {
                0
            };
        Ok((parsed, extra, flags, credentials))
    })();
    let (url, extra, flags, credentials) = match prepared {
        Ok(p) => p,
        Err(e) => return fail(e, 0),
    };
    let h = new_request(
        parent,
        Opened {
            url,
            verb: String::from("GET"),
            version: String::from("HTTP/1.1"),
            headers: extra,
            credentials,
        },
        flags,
        context,
    );
    let Ok(r) = request(h) else {
        return fail(error::INTERNAL_ERROR, 0);
    };
    if session_of(h).is_ok_and(|s| s.with(|s| s.asynchronous)) {
        // The handle is announced through INTERNET_STATUS_HANDLE_CREATED;
        // completion follows with INTERNET_STATUS_REQUEST_COMPLETE.
        let job = Box::into_raw(Box::new(Job {
            handle: h,
            request: r,
            work: Work::OpenUrl,
        }));
        let thread = unsafe { DoorCreateThreadEx(worker as *const () as u64, job as u64, 0) };
        if thread == 0 {
            drop(unsafe { Box::from_raw(job) });
            InternetCloseHandle(door_windows_abi::narrow(h));
            return fail(error::NOT_ENOUGH_MEMORY, 0);
        }
        unsafe { DoorCloseHandle(thread) };
        return fail(error::IO_PENDING, 0);
    }
    match send_and_receive(h, &r, &[], true) {
        Ok(()) => {
            unsafe { RtlSetLastWin32Error(0) };
            h
        }
        Err(e) => {
            InternetCloseHandle(door_windows_abi::narrow(h));
            fail(e, 0)
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetOpenUrlW(
    parent: usize,
    url: usize,
    headers: usize,
    length: u32,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let url = door_windows_abi::widen(url);
    let headers = door_windows_abi::widen(headers);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        internet_open_url(parent, url, headers, length, flags, context, Text::Wide)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetOpenUrlA(
    parent: usize,
    url: usize,
    headers: usize,
    length: u32,
    flags: u32,
    context: usize,
) -> usize {
    let parent = door_windows_abi::widen(parent);
    let url = door_windows_abi::widen(url);
    let headers = door_windows_abi::widen(headers);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        internet_open_url(parent, url, headers, length, flags, context, Text::Ansi)
    })())
}

// ---- response headers -------------------------------------------------------------------
const QUERY_FLAG_REQUEST_HEADERS: u32 = 0x8000_0000;
const QUERY_FLAG_SYSTEMTIME: u32 = 0x4000_0000;
const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;
const QUERY_FLAG_NUMBER64: u32 = 0x0800_0000;
const QUERY_CUSTOM: u32 = 65535;
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
        17 => "Pragma",
        23 => "Connection",
        24 => "Accept",
        26 => "Accept-Encoding",
        27 => "Accept-Language",
        28 => "Authorization",
        29 => "Content-Encoding",
        33 => "Location",
        35 => "Referer",
        36 => "Retry-After",
        37 => "Server",
        39 => "User-Agent",
        40 => "WWW-Authenticate",
        41 => "Proxy-Authenticate",
        42 => "Accept-Ranges",
        43 => "Set-Cookie",
        44 => "Cookie",
        47 => "Content-Disposition",
        48 => "Age",
        49 => "Cache-Control",
        51 => "Content-Location",
        53 => "Content-Range",
        54 => "ETag",
        55 => "Host",
        63 => "Transfer-Encoding",
        64 => "Upgrade",
        65 => "Vary",
        66 => "Via",
        _ => return None,
    })
}
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
fn http_query_info(
    handle: u64,
    info: u32,
    buffer: u64,
    length: u64,
    index: u64,
    kind: Text,
) -> i32 {
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
        let value = match level {
            // HTTP_QUERY_CUSTOM: the name arrives in the buffer.
            QUERY_CUSTOM => {
                let name = text(buffer, u32::MAX, kind)?;
                headers
                    .nth(&name, which)
                    .map(String::from)
                    .ok_or(error::HTTP_HEADER_NOT_FOUND)?
            }
            18 if !requested => response.as_ref().unwrap().version.clone(),
            19 if !requested => alloc::format!("{}", response.as_ref().unwrap().status),
            20 if !requested => response.as_ref().unwrap().reason.clone(),
            22 if !requested => response.as_ref().unwrap().raw.clone(),
            21 if !requested => {
                let raw = &response.as_ref().unwrap().raw;
                let mut s: String = raw
                    .trim_end_matches("\r\n")
                    .split("\r\n")
                    .collect::<Vec<_>>()
                    .join("\0");
                s.push('\0');
                s
            }
            45 => verb,
            level => headers
                .nth(
                    header_name(level).ok_or(error::HTTP_INVALID_QUERY_REQUEST)?,
                    which,
                )
                .map(String::from)
                .ok_or(error::HTTP_HEADER_NOT_FOUND)?,
        };
        let data: Vec<u8> = if modifiers & QUERY_FLAG_NUMBER != 0 {
            value
                .trim()
                .parse::<u32>()
                .map_err(|_| error::HTTP_INVALID_QUERY_REQUEST)?
                .to_le_bytes()
                .to_vec()
        } else if modifiers & QUERY_FLAG_NUMBER64 != 0 {
            value
                .trim()
                .parse::<u64>()
                .map_err(|_| error::HTTP_INVALID_QUERY_REQUEST)?
                .to_le_bytes()
                .to_vec()
        } else if modifiers & QUERY_FLAG_SYSTEMTIME != 0 {
            system_time(
                door_http::parse_http_date(&value).ok_or(error::HTTP_INVALID_QUERY_REQUEST)?,
            )
            .to_vec()
        } else {
            let d = encode(&value, kind);
            let capacity = read_u32(length)?;
            if buffer == 0 || (capacity as usize) < d.len() {
                write_u32(length, d.len() as u32)?;
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(buffer, &d)?;
            if index != 0 {
                write_u32(index, which as u32 + 1)?;
            }
            return write_u32(length, (d.len() as u32) - unit(kind));
        };
        let capacity = read_u32(length)?;
        if buffer == 0 || (capacity as usize) < data.len() {
            write_u32(length, data.len() as u32)?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(buffer, &data)?;
        if index != 0 {
            write_u32(index, which as u32 + 1)?;
        }
        write_u32(length, data.len() as u32)
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpQueryInfoW(
    handle: usize,
    info: u32,
    buffer: usize,
    length: usize,
    index: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    let index = door_windows_abi::widen(index);
    http_query_info(handle, info, buffer, length, index, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn HttpQueryInfoA(
    handle: usize,
    info: u32,
    buffer: usize,
    length: usize,
    index: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    let index = door_windows_abi::widen(index);
    http_query_info(handle, info, buffer, length, index, Text::Ansi)
}

// ---- options and callbacks -----------------------------------------------------------
mod option {
    pub const CALLBACK: u32 = 1;
    pub const CONNECT_TIMEOUT: u32 = 2;
    pub const CONNECT_RETRIES: u32 = 3;
    pub const SEND_TIMEOUT: u32 = 5;
    pub const RECEIVE_TIMEOUT: u32 = 6;
    pub const HANDLE_TYPE: u32 = 9;
    pub const READ_BUFFER_SIZE: u32 = 12;
    pub const WRITE_BUFFER_SIZE: u32 = 13;
    pub const PARENT_HANDLE: u32 = 21;
    pub const USERNAME: u32 = 28;
    pub const PASSWORD: u32 = 29;
    pub const SECURITY_FLAGS: u32 = 31;
    pub const PROXY_USERNAME: u32 = 43;
    pub const PROXY_PASSWORD: u32 = 44;
    pub const URL: u32 = 34;
    pub const REFRESH: u32 = 37;
    pub const PROXY: u32 = 38;
    pub const SETTINGS_CHANGED: u32 = 39;
    pub const USER_AGENT: u32 = 41;
    pub const END_BROWSER_SESSION: u32 = 42;
    pub const CONTEXT_VALUE: u32 = 45;
    pub const HTTP_VERSION: u32 = 59;
    /// INTERNET_OPTION_HTTP_DECODING: undo gzip/deflate the app asked for.
    pub const HTTP_DECODING: u32 = 65;
}
fn internet_set_option(handle: u64, which: u32, buffer: u64, length: u32, kind: Text) -> i32 {
    let result = (|| {
        let dword = || -> Result<u32, u32> {
            if length < 4 {
                return Err(error::BAD_OPTION_LENGTH);
            }
            read_u32(buffer)
        };
        match which {
            // Global notifications apply with a NULL handle.
            option::SETTINGS_CHANGED | option::REFRESH | option::END_BROWSER_SESSION => {
                if which == option::END_BROWSER_SESSION {
                    // Session cookies end with the browser session.
                    cookie_store().with(|j| j.end_session());
                }
                return Ok(());
            }
            _ => (),
        }
        common(handle).ok_or(error::INVALID_HANDLE)?;
        match which {
            option::CONTEXT_VALUE => {
                if (length as usize) < POINTER {
                    return Err(error::BAD_OPTION_LENGTH);
                }
                let value = read_pointer(buffer)?;
                slot(handle, |s| s.common.context = value);
            }
            option::CONNECT_TIMEOUT | option::SEND_TIMEOUT | option::RECEIVE_TIMEOUT => {
                let value = dword()?;
                let i = match which {
                    option::CONNECT_TIMEOUT => 0,
                    option::SEND_TIMEOUT => 1,
                    _ => 2,
                };
                session_of(handle)?.with(|s| s.timeouts[i] = value);
            }
            option::CONNECT_RETRIES
            | option::READ_BUFFER_SIZE
            | option::WRITE_BUFFER_SIZE
            | option::HTTP_VERSION => {
                dword()?;
            }
            option::SECURITY_FLAGS => {
                let value = dword()?;
                request(handle)?.with(|r| r.security_flags = value);
            }
            option::HTTP_DECODING => {
                // A BOOL; WinINet decodes only what the application asked
                // for with its own Accept-Encoding header.
                let on = dword()? != 0;
                match request(handle) {
                    Ok(r) => r.with(|r| r.decoding = Some(on)),
                    Err(_) => session_of(handle)?.with(|s| s.decoding = on),
                }
            }
            option::USER_AGENT => {
                let agent = text(buffer, length, kind)?;
                session_of(handle)?.with(|s| s.agent = agent);
            }
            option::USERNAME
            | option::PASSWORD
            | option::PROXY_USERNAME
            | option::PROXY_PASSWORD => {
                let value = text(buffer, length, kind)?;
                let target = if which == option::USERNAME || which == option::PASSWORD {
                    Target::Server
                } else {
                    Target::Proxy
                };
                request(handle)?.with(|r| {
                    let client = match target {
                        Target::Server => &r.exchange.server,
                        Target::Proxy => &r.exchange.proxy,
                    };
                    let (mut user, mut password) = client
                        .credentials
                        .as_ref()
                        .map(|c| (c.user.clone(), c.password.clone()))
                        .unwrap_or_default();
                    if which == option::USERNAME || which == option::PROXY_USERNAME {
                        user = value;
                    } else {
                        password = value;
                    }
                    r.exchange
                        .set_credentials(target, user, password, ALL_SCHEMES);
                });
            }
            option::PROXY => {
                // INTERNET_PROXY_INFO { dwAccessType, lpszProxy, lpszProxyBypass }.
                let access = read_u32(buffer)?;
                let proxy = text(read_pointer(buffer + POINTER as u64)?, u32::MAX, kind)?;
                let bypass = text(read_pointer(buffer + 2 * POINTER as u64)?, u32::MAX, kind)?;
                session_of(handle)?.with(|s| {
                    s.access = access;
                    s.proxy = proxy;
                    s.bypass = bypass;
                });
            }
            option::HANDLE_TYPE | option::PARENT_HANDLE | option::URL | option::CALLBACK => {
                return Err(error::OPTION_NOT_SETTABLE);
            }
            _ => return Err(error::INVALID_OPTION),
        }
        Ok(())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetOptionW(
    handle: usize,
    which: u32,
    buffer: usize,
    length: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    internet_set_option(handle, which, buffer, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetOptionA(
    handle: usize,
    which: u32,
    buffer: usize,
    length: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    internet_set_option(handle, which, buffer, length, Text::Ansi)
}
fn internet_query_option(handle: u64, which: u32, buffer: u64, length: u64, kind: Text) -> i32 {
    let result = (|| {
        let c = common(handle).ok_or(error::INVALID_HANDLE)?;
        let (data, string): (Vec<u8>, bool) = match which {
            option::CONTEXT_VALUE => (c.context.to_le_bytes()[..POINTER].to_vec(), false),
            option::PARENT_HANDLE => (c.parent.to_le_bytes()[..POINTER].to_vec(), false),
            option::CALLBACK => (c.callback.to_le_bytes()[..POINTER].to_vec(), false),
            option::HANDLE_TYPE => {
                let t = slot(handle, |s| kind_of(&s.object)).unwrap_or(0);
                (t.to_le_bytes().to_vec(), false)
            }
            option::CONNECT_TIMEOUT | option::SEND_TIMEOUT | option::RECEIVE_TIMEOUT => {
                let i = match which {
                    option::CONNECT_TIMEOUT => 0,
                    option::SEND_TIMEOUT => 1,
                    _ => 2,
                };
                (
                    session_of(handle)?
                        .with(|s| s.timeouts[i])
                        .to_le_bytes()
                        .to_vec(),
                    false,
                )
            }
            option::SECURITY_FLAGS => {
                let flags = request(handle)?.with(|r| {
                    let mut f = r.security_flags;
                    if r.exchange.connected_securely() {
                        // SECURITY_FLAG_SECURE | SECURITY_FLAG_STRENGTH_STRONG.
                        f |= 0x1 | 0x2000_0000;
                    }
                    f
                });
                (flags.to_le_bytes().to_vec(), false)
            }
            option::URL => (
                encode(&request(handle)?.with(|r| r.exchange.url.to_text()), kind),
                true,
            ),
            option::USER_AGENT => (
                encode(&session_of(handle)?.with(|s| s.agent.clone()), kind),
                true,
            ),
            option::HTTP_VERSION => {
                let mut v = 1u32.to_le_bytes().to_vec();
                v.extend_from_slice(&1u32.to_le_bytes());
                (v, false)
            }
            _ => return Err(error::INVALID_OPTION),
        };
        let capacity = read_u32(length)?;
        if buffer == 0 || (capacity as usize) < data.len() {
            write_u32(length, data.len() as u32)?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(buffer, &data)?;
        write_u32(
            length,
            data.len() as u32 - if string { unit(kind) } else { 0 },
        )
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetQueryOptionW(
    handle: usize,
    which: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    internet_query_option(handle, which, buffer, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetQueryOptionA(
    handle: usize,
    which: u32,
    buffer: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    internet_query_option(handle, which, buffer, length, Text::Ansi)
}
/// Returns the previous callback; INTERNET_INVALID_STATUS_CALLBACK (-1)
/// for an invalid handle.
fn set_status_callback(handle: u64, callback: u64) -> u64 {
    match slot(handle, |s| {
        core::mem::replace(&mut s.common.callback, callback)
    }) {
        Some(previous) => previous,
        None => fail(error::INVALID_HANDLE, u64::MAX),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetStatusCallbackW(handle: usize, callback: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    let callback = door_windows_abi::widen(callback);
    door_windows_abi::narrow((move || -> u64 { set_status_callback(handle, callback) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetStatusCallbackA(handle: usize, callback: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    let callback = door_windows_abi::widen(callback);
    door_windows_abi::narrow((move || -> u64 { set_status_callback(handle, callback) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetStatusCallback(handle: usize, callback: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    let callback = door_windows_abi::widen(callback);
    door_windows_abi::narrow((move || -> u64 { set_status_callback(handle, callback) })())
}

// ---- connectivity ------------------------------------------------------------------------
fn adapter_up() -> bool {
    let mut b = [0u8; door_service_protocol::net::STATUS_BYTES];
    if unsafe { DoorNetStatus(b.as_mut_ptr() as u64) } != 0 {
        return false;
    }
    door_service_protocol::net::Status::decode(&b)
        .is_some_and(|s| s.present && s.link && s.enabled && s.address != [0; 4])
}
/// INTERNET_CONNECTION_LAN | INTERNET_RAS_INSTALLED |
/// INTERNET_CONNECTION_CONFIGURED when the adapter is up, as Windows
/// reports a wired connection; INTERNET_CONNECTION_OFFLINE otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetConnectedState(flags: usize, _reserved: u32) -> i32 {
    let flags = door_windows_abi::widen(flags);
    {
        let up = adapter_up();
        let value: u32 = if up { 0x02 | 0x10 | 0x40 } else { 0x20 | 0x10 };
        if flags != 0 {
            let _ = write_u32(flags, value);
        }
        i32::from(up)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetConnectedStateExW(
    flags: usize,
    name: usize,
    length: u32,
    reserved: u32,
) -> i32 {
    let flags = door_windows_abi::widen(flags);
    let name = door_windows_abi::widen(name);
    {
        if name != 0 && length > 0 {
            let _ = write(name, &encode("", Text::Wide));
        }
        InternetGetConnectedState(door_windows_abi::narrow(flags), reserved)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetAttemptConnect(_reserved: u32) -> u32 {
    if adapter_up() {
        0
    } else {
        error::CANNOT_CONNECT
    }
}
fn check_connection(url: u64, kind: Text) -> i32 {
    let result = (|| {
        let target = text(url, u32::MAX, kind)?;
        let parsed = door_http::parse_url(&target).map_err(|_| error::INVALID_URL)?;
        let address = door_windows_inet::guest::resolve(
            &parsed.host,
            door_windows_inet::guest::Deadline::after(10_000),
        )
        .map_err(failure_code)?;
        door_windows_inet::guest::Stream::connect(
            address,
            parsed.port,
            door_windows_inet::guest::Deadline::after(10_000),
            &|| false,
        )
        .map_err(failure_code)?;
        Ok(())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCheckConnectionW(url: usize, _flags: u32, _reserved: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    check_connection(url, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCheckConnectionA(url: usize, _flags: u32, _reserved: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    check_connection(url, Text::Ansi)
}

// ---- cookies ---------------------------------------------------------------------------
fn get_cookie(url: u64, name: u64, data: u64, size: u64, kind: Text) -> i32 {
    let result = (|| {
        let parsed = door_http::parse_url(&text(url, u32::MAX, kind)?)
            .map_err(|_| error::INVALID_PARAMETER)?;
        let wanted = text(name, u32::MAX, kind)?;
        let now = door_windows_inet::guest::now_unix();
        let header = cookie_store()
            .with(|j| {
                j.header(
                    &parsed.host,
                    &parsed.path,
                    parsed.scheme == door_http::Scheme::Https,
                    now,
                )
            })
            .ok_or(259u32)?;
        let value = if wanted.is_empty() {
            header
        } else {
            header
                .split("; ")
                .find(|c| c.split_once('=').is_some_and(|(n, _)| n == wanted))
                .map(String::from)
                .ok_or(259u32)?
        };
        let bytes = encode(&value, kind);
        let capacity = read_u32(size)?;
        // The size is in characters for InternetGetCookie.
        let characters = bytes.len() as u32 / unit(kind);
        if data == 0 || capacity < characters {
            write_u32(size, characters)?;
            return if data == 0 {
                Ok(())
            } else {
                Err(error::INSUFFICIENT_BUFFER)
            };
        }
        write(data, &bytes)?;
        write_u32(size, characters)
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetCookieW(
    url: usize,
    name: usize,
    data: usize,
    size: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    get_cookie(url, name, data, size, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetCookieA(
    url: usize,
    name: usize,
    data: usize,
    size: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    let size = door_windows_abi::widen(size);
    get_cookie(url, name, data, size, Text::Ansi)
}
fn set_cookie(url: u64, name: u64, data: u64, kind: Text) -> i32 {
    let result = (|| {
        let parsed = door_http::parse_url(&text(url, u32::MAX, kind)?)
            .map_err(|_| error::INVALID_PARAMETER)?;
        let name = text(name, u32::MAX, kind)?;
        let data = text(data, u32::MAX, kind)?;
        let line = if name.is_empty() {
            data
        } else {
            alloc::format!("{name}={data}")
        };
        let now = door_windows_inet::guest::now_unix();
        cookie_store().with(|j| j.store(&line, &parsed.host, &parsed.path, now));
        Ok(())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetCookieW(url: usize, name: usize, data: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    set_cookie(url, name, data, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetSetCookieA(url: usize, name: usize, data: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    let name = door_windows_abi::widen(name);
    let data = door_windows_abi::widen(data);
    set_cookie(url, name, data, Text::Ansi)
}

// ---- URL helpers ---------------------------------------------------------------------------
const ICU_DECODE: u32 = 0x1000_0000;
const ICU_ESCAPE: u32 = 0x8000_0000;
fn crack_url(url: u64, length: u32, flags: u32, components: u64, kind: Text) -> i32 {
    let result = (|| {
        let size = layout::size(&URL_COMPONENTS) as u32;
        if url == 0 || components == 0 || read_u32(components)? != size {
            return Err(error::INVALID_PARAMETER);
        }
        let value = text(url, length, kind)?;
        let units: Vec<u16> = value.encode_utf16().collect();
        let c = crate::crack(&units)?;
        // Byte offset of a UTF-16 index in the caller's string.
        let offset = |index: usize| -> u64 {
            match kind {
                Text::Wide => (index * 2) as u64,
                Text::Ansi => String::from_utf16_lossy(&units[..index]).chars().count() as u64,
            }
        };
        for (pointer_at, length_at, (start, span)) in [
            (url_field(1), url_field(2), c.scheme),
            (url_field(4), url_field(5), c.host),
            (url_field(7), url_field(8), c.user),
            (url_field(9), url_field(10), c.password),
            (url_field(11), url_field(12), c.path),
            (url_field(13), url_field(14), c.extra),
        ] {
            let pointer = read_pointer(components + pointer_at)?;
            let capacity = read_u32(components + length_at)?;
            if capacity == 0 {
                continue;
            }
            if pointer == 0 {
                if flags & (ICU_DECODE | ICU_ESCAPE) != 0 {
                    return Err(error::INVALID_PARAMETER);
                }
                write_pointer(components + pointer_at, url + offset(start))?;
                write_u32(components + length_at, span as u32)?;
            } else {
                let piece = String::from_utf16_lossy(&units[start..start + span]);
                let bytes = encode(&piece, kind);
                let characters = bytes.len() as u32 / unit(kind);
                if capacity < characters {
                    write_u32(components + length_at, characters)?;
                    return Err(error::INSUFFICIENT_BUFFER);
                }
                write(pointer, &bytes)?;
                write_u32(components + length_at, characters - 1)?;
            }
        }
        write(components + url_field(3), &c.number.to_le_bytes())?;
        write(components + url_field(6), &c.port.to_le_bytes())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCrackUrlW(
    url: usize,
    length: u32,
    flags: u32,
    components: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let components = door_windows_abi::widen(components);
    crack_url(url, length, flags, components, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCrackUrlA(
    url: usize,
    length: u32,
    flags: u32,
    components: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let components = door_windows_abi::widen(components);
    crack_url(url, length, flags, components, Text::Ansi)
}
/// Copies a result string with WinINet's size protocol (characters).
fn deliver_text(value: &str, buffer: u64, length: u64, kind: Text) -> Result<(), u32> {
    let bytes = encode(value, kind);
    let characters = bytes.len() as u32 / unit(kind);
    let capacity = read_u32(length)?;
    if buffer == 0 || capacity < characters {
        write_u32(length, characters)?;
        return Err(error::INSUFFICIENT_BUFFER);
    }
    write(buffer, &bytes)?;
    write_u32(length, characters - 1)
}
fn canonicalize_url(url: u64, buffer: u64, length: u64, kind: Text) -> i32 {
    let result = text(url, u32::MAX, kind)
        .and_then(|u| crate::canonicalize(&u))
        .and_then(|c| deliver_text(&c, buffer, length, kind));
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCanonicalizeUrlW(
    url: usize,
    buffer: usize,
    length: usize,
    _flags: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    canonicalize_url(url, buffer, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCanonicalizeUrlA(
    url: usize,
    buffer: usize,
    length: usize,
    _flags: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    canonicalize_url(url, buffer, length, Text::Ansi)
}
fn combine_url(base: u64, relative: u64, buffer: u64, length: u64, kind: Text) -> i32 {
    let result = (|| {
        let combined = crate::combine(
            &text(base, u32::MAX, kind)?,
            &text(relative, u32::MAX, kind)?,
        )?;
        deliver_text(&combined, buffer, length, kind)
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCombineUrlW(
    base: usize,
    relative: usize,
    buffer: usize,
    length: usize,
    _flags: u32,
) -> i32 {
    let base = door_windows_abi::widen(base);
    let relative = door_windows_abi::widen(relative);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    combine_url(base, relative, buffer, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetCombineUrlA(
    base: usize,
    relative: usize,
    buffer: usize,
    length: usize,
    _flags: u32,
) -> i32 {
    let base = door_windows_abi::widen(base);
    let relative = door_windows_abi::widen(relative);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    combine_url(base, relative, buffer, length, Text::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetTimeFromSystemTimeW(
    time: usize,
    _format: u32,
    out: usize,
    length: u32,
) -> i32 {
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
            let bytes = encode(&door_http::http_date(unix), Text::Wide);
            if (length as usize) < bytes.len() {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            write(out, &bytes)
        })();
        done(result)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetTimeToSystemTimeW(value: usize, time: usize, _reserved: u32) -> i32 {
    let value = door_windows_abi::widen(value);
    let time = door_windows_abi::widen(time);
    {
        let result = (|| {
            let unix = door_http::parse_http_date(&text(value, u32::MAX, Text::Wide)?)
                .ok_or(error::INVALID_PARAMETER)?;
            write(time, &system_time(unix))
        })();
        done(result)
    }
}
