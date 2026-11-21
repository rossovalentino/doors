//! The window-message side of Winsock: WSAAsyncSelect (network events posted
//! to a window, one message per event) and the WSAAsyncGet* database
//! lookups (done on a worker thread, the result copied into the caller's
//! buffer, completion posted). PostMessageW comes from the process's own
//! user32, found at run time as ws2_32 itself does on Windows. Also the
//! Winsock 1.1 blocking-hook calls, which Winsock 2 refuses.
use crate::error::*;
use crate::guest::*;
use core::sync::atomic::{AtomicU64, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetModuleHandleW(name: u64) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
}

/// PostMessageW(HWND, UINT, WPARAM, LPARAM) at pointer width (P17).
type PostMessage = extern "system" fn(usize, u32, usize, isize) -> i32;
static POST: AtomicU64 = AtomicU64::new(0);
/// user32's PostMessageW, when user32 is in this process.
fn post_message() -> Option<PostMessage> {
    let mut address = POST.load(Ordering::Acquire);
    if address == 0 {
        let name: alloc::vec::Vec<u16> = "user32.dll".encode_utf16().chain([0]).collect();
        let module = unsafe { DoorGetModuleHandleW(name.as_ptr() as u64) };
        if module == 0 {
            return None;
        }
        address = unsafe { DoorGetProcAddress(module, c"PostMessageW".as_ptr() as u64) };
        if address == 0 {
            return None;
        }
        POST.store(address, Ordering::Release);
    }
    Some(unsafe { core::mem::transmute::<usize, PostMessage>(address as usize) })
}
pub(crate) fn post(window: u64, message: u32, w: u64, l: u64) -> bool {
    post_message().is_some_and(|post| post(window as usize, message, w as usize, l as isize) != 0)
}

/// WSAAsyncSelect: events in `mask` are posted to `window` as `message`
/// (wParam the socket, lParam WSAMAKESELECTREPLY(event, error)). It
/// cancels any WSAEventSelect on the socket and makes it nonblocking.
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncSelect(
    socket: usize,
    window: usize,
    message: u32,
    mask: i32,
) -> i32 {
    let socket = door_windows_abi::widen(socket);
    let window = door_windows_abi::widen(window);
    {
        if mask != 0 && post_message().is_none() {
            return fail(WSAEINVAL);
        }
        let result = with_entry(socket, |e| {
            e.window = window;
            e.message = message;
            e.event = 0;
            e.mask = mask as u32;
            e.reported = 0;
            e.pending = 0;
            if mask != 0 {
                e.nonblocking = true;
            }
            Ok(())
        });
        if result.is_ok() && mask != 0 && !crate::poll::start_watcher() {
            return fail(WSAENOBUFS);
        }
        result.map_or_else(fail, |()| ok())
    }
}

// ---- WSAAsyncGet* -----------------------------------------------------------
static NEXT_TASK: AtomicU64 = AtomicU64::new(0x4000);
/// Cancelled task handles (a small ring; a late completion of a cancelled
/// request is not posted).
static CANCELLED: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
fn cancelled(task: u64) -> bool {
    CANCELLED.iter().any(|c| c.load(Ordering::Acquire) == task)
}
#[derive(Clone, Copy)]
enum Lookup {
    HostByName,
    HostByAddr,
    ServByName,
    ServByPort,
    ProtoByName,
    ProtoByNumber,
}
struct Request {
    lookup: Lookup,
    window: u64,
    message: u32,
    task: u64,
    /// Copies of the caller's arguments (names are read before returning).
    text: [u8; 256],
    text2: [u8; 32],
    number: i32,
    address: [u8; 16],
    address_length: i32,
    buffer: u64,
    length: i32,
}
/// The database entry the synchronous call returns lives in a fixed
/// static block: pointers into it are rebased onto the caller's buffer.
fn relocate(entry: u64, block: usize, buffer: u64, length: i32) -> Result<u32, (u32, u32)> {
    let size = block as u32;
    if length < 0 || (length as u32) < size {
        return Err((WSAENOBUFS, size));
    }
    let mut bytes = alloc::vec![0u8; block];
    unsafe { core::ptr::copy_nonoverlapping(entry as *const u8, bytes.as_mut_ptr(), block) };
    // The entry's pointers, a pointer-sized aligned word each (P17).
    for at in (0..=block - POINTER).step_by(POINTER) {
        let mut w = [0u8; 8];
        w[..POINTER].copy_from_slice(&bytes[at..at + POINTER]);
        let word = u64::from_le_bytes(w);
        if word >= entry && word < entry + block as u64 {
            bytes[at..at + POINTER]
                .copy_from_slice(&(word - entry + buffer).to_le_bytes()[..POINTER]);
        }
    }
    write(buffer, &bytes).map_err(|e| (e, 0))?;
    Ok(size)
}
extern "system" fn lookup(argument: usize) -> u32 {
    let argument = door_windows_abi::widen(argument);
    {
        let r = unsafe { alloc::boxed::Box::from_raw(argument as *mut Request) };
        let text = r.text.as_ptr() as u64;
        let (entry, block) = match r.lookup {
            Lookup::HostByName => (
                crate::names::gethostbyname(door_windows_abi::narrow(text)),
                crate::names::HOSTENT_BYTES,
            ),
            Lookup::HostByAddr => (
                crate::names::gethostbyaddr(
                    door_windows_abi::narrow(r.address.as_ptr() as u64),
                    r.address_length,
                    r.number,
                ),
                crate::names::HOSTENT_BYTES,
            ),
            Lookup::ServByName => (
                crate::names::getservbyname(
                    door_windows_abi::narrow(text),
                    if r.text2[0] == 0 {
                        0
                    } else {
                        door_windows_abi::narrow(r.text2.as_ptr() as u64)
                    },
                ),
                crate::names::SERVENT_BYTES,
            ),
            Lookup::ServByPort => (
                crate::names::getservbyport(
                    r.number,
                    if r.text2[0] == 0 {
                        0
                    } else {
                        door_windows_abi::narrow(r.text2.as_ptr() as u64)
                    },
                ),
                crate::names::SERVENT_BYTES,
            ),
            Lookup::ProtoByName => (
                crate::names::getprotobyname(door_windows_abi::narrow(text)),
                crate::names::PROTOENT_BYTES,
            ),
            Lookup::ProtoByNumber => (
                crate::names::getprotobynumber(r.number),
                crate::names::PROTOENT_BYTES,
            ),
        };
        // WSAMAKEASYNCREPLY(buflen, error).
        let reply = if entry == 0 {
            (WSAGetLastError() as u32) << 16
        } else {
            match relocate(door_windows_abi::widen(entry), block, r.buffer, r.length) {
                Ok(size) => size,
                Err((error, size)) => error << 16 | size,
            }
        };
        if !cancelled(r.task) {
            post(r.window, r.message, r.task, u64::from(reply));
        }
        0
    }
}
fn start(request: Request) -> u64 {
    if let Err(e) = started() {
        fail(e);
        return 0;
    }
    if post_message().is_none() {
        fail(WSAEINVAL);
        return 0;
    }
    let task = request.task;
    let boxed = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(request));
    let thread = unsafe { DoorCreateThreadEx(lookup as *const () as u64, boxed as u64, 0) };
    if thread == 0 {
        drop(unsafe { alloc::boxed::Box::from_raw(boxed) });
        fail(WSAENOBUFS);
        return 0;
    }
    unsafe { DoorCloseHandle(thread) };
    task
}
fn copy_text(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    for i in 0..out.len() - 1 {
        let mut b = [0u8; 1];
        read(pointer + i as u64, &mut b)?;
        out[i] = b[0];
        if b[0] == 0 {
            return Ok(());
        }
    }
    Err(WSAEFAULT)
}
fn request(lookup: Lookup, window: u64, message: u32, buffer: u64, length: i32) -> Request {
    Request {
        lookup,
        window,
        message,
        task: NEXT_TASK.fetch_add(1, Ordering::Relaxed),
        text: [0; 256],
        text2: [0; 32],
        number: 0,
        address: [0; 16],
        address_length: 0,
        buffer,
        length,
    }
}
macro_rules! named {
    ($export:ident, $lookup:expr) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $export(
            window: usize,
            message: u32,
            name: usize,
            buffer: usize,
            length: i32,
        ) -> usize {
            let window = door_windows_abi::widen(window);
            let name = door_windows_abi::widen(name);
            let buffer = door_windows_abi::widen(buffer);
            door_windows_abi::narrow((move || -> u64 {
                let mut r = request($lookup, window, message, buffer, length);
                if let Err(e) = copy_text(name, &mut r.text) {
                    fail(e);
                    return 0;
                }
                start(r)
            })())
        }
    };
}
named!(WSAAsyncGetHostByName, Lookup::HostByName);
named!(WSAAsyncGetProtoByName, Lookup::ProtoByName);
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncGetHostByAddr(
    window: usize,
    message: u32,
    address: usize,
    address_length: i32,
    family: i32,
    buffer: usize,
    length: i32,
) -> usize {
    let window = door_windows_abi::widen(window);
    let address = door_windows_abi::widen(address);
    let buffer = door_windows_abi::widen(buffer);
    door_windows_abi::narrow((move || -> u64 {
        let mut r = request(Lookup::HostByAddr, window, message, buffer, length);
        let n = (address_length.clamp(0, 16)) as usize;
        if read(address, &mut r.address[..n]).is_err() {
            fail(WSAEFAULT);
            return 0;
        }
        r.address_length = address_length;
        r.number = family;
        start(r)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncGetServByName(
    window: usize,
    message: u32,
    name: usize,
    protocol: usize,
    buffer: usize,
    length: i32,
) -> usize {
    let window = door_windows_abi::widen(window);
    let name = door_windows_abi::widen(name);
    let protocol = door_windows_abi::widen(protocol);
    let buffer = door_windows_abi::widen(buffer);
    door_windows_abi::narrow((move || -> u64 {
        let mut r = request(Lookup::ServByName, window, message, buffer, length);
        if copy_text(name, &mut r.text)
            .and_then(|()| copy_text(protocol, &mut r.text2))
            .is_err()
        {
            fail(WSAEFAULT);
            return 0;
        }
        start(r)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncGetServByPort(
    window: usize,
    message: u32,
    port: i32,
    protocol: usize,
    buffer: usize,
    length: i32,
) -> usize {
    let window = door_windows_abi::widen(window);
    let protocol = door_windows_abi::widen(protocol);
    let buffer = door_windows_abi::widen(buffer);
    door_windows_abi::narrow((move || -> u64 {
        let mut r = request(Lookup::ServByPort, window, message, buffer, length);
        if copy_text(protocol, &mut r.text2).is_err() {
            fail(WSAEFAULT);
            return 0;
        }
        r.number = port;
        start(r)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAAsyncGetProtoByNumber(
    window: usize,
    message: u32,
    number: i32,
    buffer: usize,
    length: i32,
) -> usize {
    let window = door_windows_abi::widen(window);
    let buffer = door_windows_abi::widen(buffer);
    door_windows_abi::narrow((move || -> u64 {
        let mut r = request(Lookup::ProtoByNumber, window, message, buffer, length);
        r.number = number;
        start(r)
    })())
}
static NEXT_CANCEL: AtomicU64 = AtomicU64::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn WSACancelAsyncRequest(task: usize) -> i32 {
    let task = door_windows_abi::widen(task);
    {
        if let Err(e) = started() {
            return fail(e);
        }
        if task < 0x4000 || task >= NEXT_TASK.load(Ordering::Acquire) {
            return fail(WSAEINVAL);
        }
        let slot = NEXT_CANCEL.fetch_add(1, Ordering::Relaxed) as usize % CANCELLED.len();
        CANCELLED[slot].store(task, Ordering::Release);
        ok()
    }
}

// ---- Winsock 1.1 blocking hooks (refused by Winsock 2) -----------------------------
#[unsafe(no_mangle)]
pub extern "system" fn WSASetBlockingHook(_hook: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        fail(WSAEOPNOTSUPP);
        0
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn WSAUnhookBlockingHook() -> i32 {
    fail(WSAEOPNOTSUPP)
}
#[unsafe(no_mangle)]
pub extern "system" fn WSACancelBlockingCall() -> i32 {
    fail(WSAEOPNOTSUPP)
}
/// Winsock 1.1's exit routine: nothing to do.
#[unsafe(no_mangle)]
pub extern "system" fn WEP() -> i32 {
    0
}
