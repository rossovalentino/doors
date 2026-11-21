//! Standard marshaling between apartments and processes (M6).
//!
//! An exporting apartment hands out OBJREFs naming its process (the OXID is
//! the process ID), the object (OID) and the interface (IPID); calls reach
//! it as ORPC messages (`door_com_wire::message`) over links Init carries
//! to the endpoint `OLE<pid>` (the ncalrpc transport: a channel end cannot
//! move between processes, so Init relays the packets). Each exported interface
//! other than IUnknown and IClassFactory gets the stub its IPSFactoryBuffer
//! makes (HKCR\Interface\{iid}\ProxyStubClsid32: oleaut32's automation
//! marshaler); an importing apartment gets one proxy manager per object
//! (COM identity) with an interface proxy per IID connected to an
//! IRpcChannelBuffer of ours.
//!
//! Calls run on the apartment that owns the object: an STA serves them
//! from its message loop (a message-only "OleMainThreadWndClass" window
//! whose timer polls the endpoint, and a posted message for calls from the
//! same process) or while it waits for a call of its own (reentrancy); the
//! MTA has one dispatcher thread. A proxy used from another apartment
//! fails with RPC_E_WRONG_THREAD; a server that ends mid-call fails it
//! with RPC_S_CALL_FAILED and later calls with RPC_S_SERVER_UNAVAILABLE.
//! LocalServer32 activation asks the broker whether the class's server is
//! up, starts it with `-Embedding` when the broker says so, and waits for
//! its class endpoint.
use super::*;
use alloc::string::String;
use door_com_wire::message::{self, Call, ErrorInfo, Joiner, QueryInterface, Reply, kind};
use door_com_wire::objref::{ObjRef, SORF_NOPING, StdObjRef};
use door_service_client::ComBroker;
use door_service_protocol::com as wire;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetCurrentProcessId() -> u32;
    fn DoorGetInheritedHandle(index: u32) -> u64;
    fn DoorCreateProcessW(
        application: u64,
        command_line: u64,
        directory: u64,
        flags: u32,
        info: u64,
        environment: u64,
    ) -> i32;
    fn DoorGetExitCodeProcess(handle: u64, out: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorGetTickCount64() -> u64;
    fn DoorCreateThread(entry: u64, argument: u64) -> u64;
}
#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: usize) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: usize,
        title: usize,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        instance: usize,
        param: usize,
    ) -> usize;
    fn DefWindowProcW(hwnd: usize, message: u32, w: usize, l: isize) -> isize;
    fn SetTimer(hwnd: usize, id: usize, elapse: u32, procedure: usize) -> usize;
    fn PostMessageW(hwnd: usize, message: u32, w: usize, l: isize) -> i32;
}

pub(super) const IID_ICLASSFACTORY: Guid = [1, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_IDISPATCH: Guid = [0, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// {D5F56B60-593B-101A-B569-08002B2DBF7A}
const IID_IRPCCHANNELBUFFER: Guid = [
    0x60, 0x6B, 0xF5, 0xD5, 0x3B, 0x59, 0x1A, 0x10, 0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A,
];
/// {D5F569D0-593B-101A-B569-08002B2DBF7A}
const IID_IPSFACTORYBUFFER: Guid = [
    0xD0, 0x69, 0xF5, 0xD5, 0x3B, 0x59, 0x1A, 0x10, 0xB5, 0x69, 0x08, 0x00, 0x2B, 0x2D, 0xBF, 0x7A,
];
/// PSOAInterface {00020424-0000-0000-C000-000000000046} and PSDispatch
/// {00020420-...}: oleaut32's automation marshalers.
const PS_OA_INTERFACE: Guid = [0x24, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const PS_DISPATCH: Guid = [0x20, 4, 2, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

pub(super) mod rpc {
    pub const E_DISCONNECTED: u32 = 0x8001_0108;
    pub const E_WRONG_THREAD: u32 = 0x8001_010E;
    pub const E_INVALID_OBJREF: u32 = 0x8001_0116;
    /// RPC_S_SERVER_UNAVAILABLE and RPC_S_CALL_FAILED as HRESULTs.
    pub const S_SERVER_UNAVAILABLE: u32 = 0x8007_06BA;
    pub const S_CALL_FAILED: u32 = 0x8007_06BE;
}
const MSHLFLAGS_TABLEWEAK: u32 = 2;
const WM_TIMER: u32 = 0x113;
const WM_OLE_WAKE: u32 = 0x0400 + 0x1A0;
const HWND_MESSAGE: u64 = (-3i64) as u64;

/// A serial line for the guest record (`DOOR_P16 COM ...`).
fn trace(text: &str) {
    let _ = door_sys::write(alloc::format!("DOOR_P16 COM {text}\n").as_bytes());
}
fn pid() -> u64 {
    u64::from(unsafe { DoorGetCurrentProcessId() })
}
fn now() -> u64 {
    unsafe { DoorGetTickCount64() }
}
fn endpoint_of(oxid: u64) -> String {
    let (name, n) = wire::exporter_endpoint(oxid);
    name[..n].iter().map(|&b| b as char).collect()
}
/// The calling thread's apartment: its thread ID in an STA, 0 in the MTA.
pub(super) fn apartment() -> Option<u32> {
    match current() {
        Some((Apartment::Sta, _)) => Some(me()),
        Some((Apartment::Mta, _)) => Some(0),
        None => None,
    }
}

// ---- the broker ------------------------------------------------------------------------
static BROKER: Lock<Option<ComBroker>> = Lock::new(None);
fn broker<R>(f: impl FnOnce(&mut ComBroker) -> Result<R, u32>) -> Result<R, u32> {
    BROKER.with(|b| {
        if b.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *b = Some(ComBroker::connect(registry).map_err(|_| rpc::S_SERVER_UNAVAILABLE)?);
        }
        let r = f(b.as_mut().unwrap_or_else(|| unreachable!()));
        if r.is_err() && b.as_ref().is_some_and(|x| x.needs_reconnect()) {
            *b = None;
        }
        r
    })
}

// ---- links ----------------------------------------------------------------------------------
// A channel end cannot leave the process that made it, so Init carries
// every packet (`door_service_protocol::com`): one ORPC part per packet.
const _: () = assert!(message::PAYLOAD_BYTES <= wire::PAYLOAD_BYTES);
/// Sends one message on `link` as packets (a full mailbox: take in what
/// arrived meanwhile, so two senders cannot wait for each other, and try
/// again).
fn send(link: u32, kind: u8, call: u32, body: &[u8]) -> Result<(), u32> {
    for part in message::split(kind, call, body) {
        let deadline = now() + 30_000;
        loop {
            match broker(|b| b.send(link, &part).map_err(|_| rpc::S_SERVER_UNAVAILABLE)) {
                Ok(true) => break,
                Ok(false) if now() < deadline => {
                    drain();
                    unsafe { DoorSleep(1) }
                }
                _ => return Err(rpc::S_SERVER_UNAVAILABLE),
            }
        }
    }
    Ok(())
}
fn close_link(link: u32) {
    let _ = broker(|b| b.close(link).map_err(|_| hr::E_FAIL));
}

// ---- the exporter ------------------------------------------------------------------------
struct Iface {
    ipid: Guid,
    iid: Guid,
    pointer: u64,
    /// IRpcStubBuffer (0: IUnknown and IClassFactory, served here).
    stub: u64,
    refs: u32,
}
struct Object {
    oid: u64,
    identity: u64,
    apartment: u32,
    interfaces: Vec<Iface>,
    /// A class object registered for LOCAL_SERVER: kept while registered.
    pinned: bool,
}
struct Exporter {
    registered: bool,
    objects: Vec<Object>,
    next_oid: u64,
}
static EXPORTER: Lock<Exporter> = Lock::new(Exporter {
    registered: false,
    objects: Vec::new(),
    next_oid: 1,
});
/// Class objects served to other processes: (cookie, CLSID, apartment,
/// single use, endpoint registered).
struct ServedClass {
    cookie: u32,
    clsid: Guid,
    object: u64,
    apartment: u32,
    single_use: bool,
}
static CLASSES: Lock<Vec<ServedClass>> = Lock::new(Vec::new());
fn ipid_for(oid: u64, index: u32) -> Guid {
    let mut g = [0u8; 16];
    g[..8].copy_from_slice(&oid.to_le_bytes());
    g[8..12].copy_from_slice(&index.to_le_bytes());
    g[12..].copy_from_slice(&(pid() as u32).to_le_bytes());
    g
}
/// The endpoint of this process is registered and an apartment polls it.
fn ensure_exporting(apartment: u32) -> Result<(), u32> {
    let first = EXPORTER.with(|e| !core::mem::replace(&mut e.registered, true));
    if first {
        let (name, n) = wire::exporter_endpoint(pid());
        let r = broker(|b| {
            b.register(&name[..n])
                .map_err(|_| rpc::S_SERVER_UNAVAILABLE)
        });
        trace(&alloc::format!(
            "EXPORTER endpoint={} result={:#x}",
            core::str::from_utf8(&name[..n]).unwrap_or("?"),
            r.err().unwrap_or(0)
        ));
        if let Err(e) = r {
            EXPORTER.with(|x| x.registered = false);
            return Err(e);
        }
    }
    serve_apartment(apartment);
    Ok(())
}
/// The stub `iid` needs, from its IPSFactoryBuffer.
fn create_stub(iid: &Guid, server: u64) -> Result<u64, u32> {
    let factory = ps_factory(iid)?;
    let mut stub = 0u64;
    // SAFETY: IPSFactoryBuffer::CreateStub is method 4.
    let create: extern "system" fn(usize, usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(factory, 4) as usize) };
    let r = create(
        door_windows_abi::narrow(factory),
        door_windows_abi::narrow(iid.as_ptr() as u64),
        door_windows_abi::narrow(server),
        door_windows_abi::narrow(&mut stub as *mut u64 as u64),
    );
    release(factory);
    if r != hr::S_OK {
        return Err(r);
    }
    Ok(stub)
}
/// Exports `iid` of the object whose IUnknown is `identity` (held by the
/// caller) with `refs` references: its STDOBJREF.
fn export(identity: u64, iid: &Guid, apartment: u32, refs: u32) -> Result<StdObjRef, u32> {
    let known = EXPORTER.with(|e| {
        let o = e.objects.iter_mut().find(|o| o.identity == identity)?;
        let oid = o.oid;
        let f = o.interfaces.iter_mut().find(|f| f.iid == *iid)?;
        f.refs += refs;
        Some((oid, f.ipid))
    });
    if let Some((oid, ipid)) = known {
        return Ok(StdObjRef {
            flags: SORF_NOPING,
            refs,
            oxid: pid(),
            oid,
            ipid,
        });
    }
    let mut pointer = 0u64;
    let r = query_interface(identity, iid, &mut pointer as *mut u64 as u64);
    if r != hr::S_OK {
        return Err(r);
    }
    let stub = if *iid == IID_IUNKNOWN || *iid == IID_ICLASSFACTORY {
        0
    } else {
        match create_stub(iid, pointer) {
            Ok(s) => s,
            Err(e) => {
                release(pointer);
                return Err(e);
            }
        }
    };
    ensure_exporting(apartment)?;
    let (oid, ipid, extra) = EXPORTER.with(|e| {
        let at = match e.objects.iter().position(|o| o.identity == identity) {
            Some(i) => i,
            None => {
                // OIDs (and the IPIDs made from them) are random, as
                // Windows' are, so another process cannot guess one.
                let mut oid = 0u64;
                for _ in 0..4 {
                    let mut b = [0u8; 8];
                    if unsafe { super::DoorGenRandom(b.as_mut_ptr() as u64, 8) } == 0 {
                        let candidate = u64::from_le_bytes(b);
                        if candidate != 0 && !e.objects.iter().any(|o| o.oid == candidate) {
                            oid = candidate;
                            break;
                        }
                    }
                }
                if oid == 0 {
                    oid = e.next_oid;
                    e.next_oid += 1;
                }
                add_ref(identity);
                e.objects.push(Object {
                    oid,
                    identity,
                    apartment,
                    interfaces: Vec::new(),
                    pinned: false,
                });
                e.objects.len() - 1
            }
        };
        let o = &mut e.objects[at];
        // Another thread exported it meanwhile: keep that one.
        if let Some(f) = o.interfaces.iter_mut().find(|f| f.iid == *iid) {
            f.refs += refs;
            return (o.oid, f.ipid, Some((pointer, stub)));
        }
        let ipid = ipid_for(o.oid, o.interfaces.len() as u32);
        o.interfaces.push(Iface {
            ipid,
            iid: *iid,
            pointer,
            stub,
            refs,
        });
        (o.oid, ipid, None)
    });
    if let Some((p, s)) = extra {
        if s != 0 {
            release(s);
        }
        release(p);
    }
    Ok(StdObjRef {
        flags: SORF_NOPING,
        refs,
        oxid: pid(),
        oid,
        ipid,
    })
}
/// Releases `refs` of interface `ipid`; an object without references
/// (and not a served class object) goes.
fn unexport(ipid: &Guid, refs: u32) {
    let gone = EXPORTER.with(|e| {
        let at = e
            .objects
            .iter()
            .position(|o| o.interfaces.iter().any(|f| f.ipid == *ipid))?;
        let o = &mut e.objects[at];
        if let Some(f) = o.interfaces.iter_mut().find(|f| f.ipid == *ipid) {
            f.refs = f.refs.saturating_sub(refs);
        }
        if o.pinned || o.interfaces.iter().any(|f| f.refs > 0) {
            return None;
        }
        Some(e.objects.remove(at))
    });
    if let Some(o) = gone {
        drop_object(o);
    }
}
fn drop_object(o: Object) {
    for f in o.interfaces {
        if f.stub != 0 {
            // SAFETY: IRpcStubBuffer::Disconnect is method 4.
            let disconnect: extern "system" fn(usize) -> u32 =
                unsafe { core::mem::transmute(method(f.stub, 4) as usize) };
            disconnect(door_windows_abi::narrow(f.stub));
            release(f.stub);
        }
        release(f.pointer);
    }
    release(o.identity);
}
/// (object identity, interface pointer, stub, IID, apartment) of `ipid`.
fn lookup(ipid: &Guid) -> Option<(u64, u64, u64, Guid, u32)> {
    EXPORTER.with(|e| {
        e.objects.iter().find_map(|o| {
            o.interfaces
                .iter()
                .find(|f| f.ipid == *ipid)
                .map(|f| (o.identity, f.pointer, f.stub, f.iid, o.apartment))
        })
    })
}

// ---- inbound calls ------------------------------------------------------------------------
enum Back {
    Link(u32),
    Local,
}
struct Pending {
    apartment: u32,
    kind: u8,
    call: u32,
    body: Vec<u8>,
    back: Back,
    /// A request on a class endpoint (the class object it serves).
    class: Option<u32>,
}
/// A link a client opened to one of this process's endpoints.
struct Inbound {
    link: u32,
    joiner: Joiner,
    /// The served class whose endpoint the link reached (`GONE`: a class
    /// revoked meanwhile).
    class: Option<u32>,
}
const GONE: u32 = u32::MAX;
static INBOUND: Lock<Vec<Inbound>> = Lock::new(Vec::new());
static PENDING: Lock<Vec<Pending>> = Lock::new(Vec::new());
static LOCAL_REPLIES: Lock<Vec<(u32, Vec<u8>)>> = Lock::new(Vec::new());
/// Replies that came on outbound links: (link, call, body).
static REPLIES: Lock<Vec<(u32, u32, Vec<u8>)>> = Lock::new(Vec::new());
static DRAINING: AtomicBool = AtomicBool::new(false);
static LAST_DRAIN: AtomicU64 = AtomicU64::new(0);
use core::sync::atomic::AtomicU64;
use door_service_client::ComEvent;

/// The apartment a message targets (None: no such interface).
fn target(kind: u8, body: &[u8], class: Option<u32>) -> Option<u32> {
    if let Some(cookie) = class {
        return CLASSES.with(|c| c.iter().find(|x| x.cookie == cookie).map(|x| x.apartment));
    }
    let ipid: Guid = body.get(..16)?.try_into().ok()?;
    match kind {
        kind::CALL | kind::REM_QUERY_INTERFACE => lookup(&ipid).map(|l| l.4),
        // Reference counts can be settled anywhere.
        _ => Some(u32::MAX),
    }
}
/// Takes in what arrived, then runs the requests that belong to
/// `apartment` (0: the MTA) or to any.
pub(super) fn pump(apartment: u32) {
    drain();
    loop {
        let next = PENDING.with(|p| {
            p.iter()
                .position(|x| x.apartment == apartment || x.apartment == u32::MAX)
                .map(|i| p.remove(i))
        });
        let Some(item) = next else {
            return;
        };
        let reply = execute(item.kind, &item.body, item.class);
        match item.back {
            Back::Link(link) => {
                let _ = send(link, kind::REPLY, item.call, &reply);
            }
            Back::Local => LOCAL_REPLIES.with(|r| r.push((item.call, reply))),
        }
    }
}
/// Takes what Init holds for this process: links clients opened to its
/// endpoints, packets (requests queue for their apartment, replies for
/// their caller) and links whose other side went. One thread at a time,
/// so the parts of a message are joined in order.
fn drain() {
    let active = EXPORTER.with(|e| e.registered)
        || CLASSES.with(|c| !c.is_empty())
        || OUTBOUND.with(|o| o.iter().any(|c| !c.dead));
    let t = now();
    if !active
        || LAST_DRAIN.swap(t, Ordering::Relaxed) == t
        || DRAINING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    for _ in 0..32 {
        match broker(|b| b.poll().map_err(|_| hr::E_FAIL)) {
            Ok(Some(event)) => take(event),
            _ => break,
        }
    }
    DRAINING.store(false, Ordering::Release);
}
fn take(event: ComEvent) {
    let link = event.link;
    match event.kind {
        wire::event::NEW => {
            let name = event.data();
            trace(&alloc::format!(
                "LINK new {link} {}",
                core::str::from_utf8(name).unwrap_or("?")
            ));
            let class = name.starts_with(b"OLECLASS-").then(|| {
                CLASSES
                    .with(|c| {
                        c.iter()
                            .find(|x| wire::class_endpoint(&x.clsid)[..] == *name)
                            .map(|x| x.cookie)
                    })
                    .unwrap_or(GONE)
            });
            INBOUND.with(|i| {
                i.push(Inbound {
                    link,
                    joiner: Joiner::default(),
                    class,
                })
            });
        }
        wire::event::PACKET => {
            let inbound = INBOUND.with(|i| {
                let c = i.iter_mut().find(|c| c.link == link)?;
                Some((c.class, c.joiner.push(event.data())))
            });
            match inbound {
                Some((class, Ok(Some((k, call, body))))) => {
                    // Requests for objects that are gone get their failure
                    // from whichever apartment runs first.
                    let apartment = target(k, &body, class).unwrap_or(u32::MAX);
                    PENDING.with(|p| {
                        p.push(Pending {
                            apartment,
                            kind: k,
                            call,
                            body,
                            back: Back::Link(link),
                            class,
                        })
                    });
                    wake(apartment);
                }
                Some((_, Ok(None))) => {}
                Some((_, Err(_))) => {
                    INBOUND.with(|i| i.retain(|c| c.link != link));
                    close_link(link);
                }
                None => {
                    let got = OUTBOUND.with(|o| {
                        let c = o.iter_mut().find(|c| c.link == link)?;
                        Some(c.joiner.push(event.data()))
                    });
                    match got {
                        Some(Ok(Some((kind::REPLY, call, body)))) => {
                            REPLIES.with(|r| r.push((link, call, body)))
                        }
                        Some(Err(_)) => mark_dead(link),
                        _ => {}
                    }
                }
            }
        }
        _ => {
            // The other side went (Init has closed ours).
            let inbound = INBOUND.with(|i| {
                let n = i.len();
                i.retain(|c| c.link != link);
                n != i.len()
            });
            if !inbound {
                OUTBOUND.with(|o| {
                    if let Some(c) = o.iter_mut().find(|c| c.link == link) {
                        c.dead = true;
                    }
                });
            }
        }
    }
}
/// The thread's error object, taken (oleaut32's GetErrorInfo).
fn take_error_info() -> Option<ErrorInfo> {
    let get = automation("GetErrorInfo")?;
    let mut info = 0u64;
    // SAFETY: oleaut32's GetErrorInfo(DWORD, IErrorInfo**).
    let f: extern "system" fn(u32, usize) -> u32 = unsafe { core::mem::transmute(get as usize) };
    if f(0, door_windows_abi::narrow(&mut info as *mut u64 as u64)) != hr::S_OK || info == 0 {
        return None;
    }
    let mut e = ErrorInfo::default();
    let text = |slot: usize| -> Vec<u16> {
        let mut b = 0u64;
        // SAFETY: IErrorInfo's getters (BSTR out).
        let get: extern "system" fn(usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(info, slot) as usize) };
        if get(
            door_windows_abi::narrow(info),
            door_windows_abi::narrow(&mut b as *mut u64 as u64),
        ) != hr::S_OK
            || b == 0
        {
            return Vec::new();
        }
        let units = bstr_text(b);
        free_bstr(b);
        units
    };
    // SAFETY: IErrorInfo::GetGUID (3), GetHelpContext (7).
    let guid: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(info, 3) as usize) };
    guid(
        door_windows_abi::narrow(info),
        door_windows_abi::narrow(e.guid.as_mut_ptr() as u64),
    );
    e.source = text(4);
    e.description = text(5);
    e.help_file = text(6);
    let context: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(info, 7) as usize) };
    context(
        door_windows_abi::narrow(info),
        door_windows_abi::narrow(&mut e.help_context as *mut u32 as u64),
    );
    release(info);
    Some(e)
}
/// Makes `e` the thread's error object (oleaut32's CreateErrorInfo and
/// SetErrorInfo).
fn set_error_info(e: &ErrorInfo) {
    let (Some(create), Some(set)) = (automation("CreateErrorInfo"), automation("SetErrorInfo"))
    else {
        return;
    };
    let mut c = 0u64;
    // SAFETY: CreateErrorInfo(ICreateErrorInfo**).
    let create: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(create as usize) };
    if create(door_windows_abi::narrow(&mut c as *mut u64 as u64)) != hr::S_OK || c == 0 {
        return;
    }
    let put = |slot: usize, units: &[u16]| {
        let z: Vec<u16> = units.iter().copied().chain([0]).collect();
        // SAFETY: ICreateErrorInfo setters (LPCOLESTR).
        let f: extern "system" fn(usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(c, slot) as usize) };
        f(
            door_windows_abi::narrow(c),
            door_windows_abi::narrow(z.as_ptr() as u64),
        );
    };
    // SAFETY: ICreateErrorInfo::SetGUID (3), SetHelpContext (7).
    let guid: extern "system" fn(usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(c, 3) as usize) };
    guid(
        door_windows_abi::narrow(c),
        door_windows_abi::narrow(e.guid.as_ptr() as u64),
    );
    put(4, &e.source);
    put(5, &e.description);
    put(6, &e.help_file);
    let context: extern "system" fn(usize, u32) -> u32 =
        unsafe { core::mem::transmute(method(c, 7) as usize) };
    context(door_windows_abi::narrow(c), e.help_context);
    let mut info = 0u64;
    const IID_IERRORINFO: Guid = [
        0x20, 0xB1, 0xF2, 0x1C, 0x7D, 0x54, 0x1B, 0x10, 0x8E, 0x65, 0x08, 0x00, 0x2B, 0x2B, 0xD1,
        0x19,
    ];
    if query_interface(c, &IID_IERRORINFO, &mut info as *mut u64 as u64) == hr::S_OK {
        // SAFETY: SetErrorInfo(DWORD, IErrorInfo*).
        let set: extern "system" fn(u32, usize) -> u32 =
            unsafe { core::mem::transmute(set as usize) };
        set(0, door_windows_abi::narrow(info));
        release(info);
    }
    release(c);
}
/// An oleaut32 export, when oleaut32 is in the process.
fn automation(name: &str) -> Option<u64> {
    let module: Vec<u16> = "oleaut32.dll".encode_utf16().chain([0]).collect();
    let m = unsafe { DoorGetModuleHandleW(module.as_ptr() as u64) };
    let m = if m == 0 {
        unsafe { DoorLoadLibraryW(module.as_ptr() as u64) }
    } else {
        m
    };
    if m == 0 {
        return None;
    }
    let n: Vec<u8> = name.bytes().chain([0]).collect();
    let p = unsafe { DoorGetProcAddress(m, n.as_ptr() as u64) };
    (p != 0).then_some(p)
}
fn bstr_text(b: u64) -> Vec<u16> {
    let Ok(bytes) = read_u32(b - 4) else {
        return Vec::new();
    };
    let mut units = vec![0u16; bytes as usize / 2];
    let _ = read(b, unsafe {
        core::slice::from_raw_parts_mut(units.as_mut_ptr() as *mut u8, units.len() * 2)
    });
    units
}
fn free_bstr(b: u64) {
    if let Some(f) = automation("SysFreeString") {
        // SAFETY: SysFreeString(BSTR).
        let f: extern "system" fn(usize) = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(b));
    }
}
/// Runs one request in this apartment: the reply body.
fn execute(k: u8, body: &[u8], class: Option<u32>) -> Vec<u8> {
    match k {
        kind::CALL => {
            let Ok(c) = Call::decode(body) else {
                return Reply::failed(hr::E_INVALIDARG).encode();
            };
            let Some((_, pointer, stub, iid, _)) = lookup(&c.ipid) else {
                return Reply::failed(rpc::E_DISCONNECTED).encode();
            };
            let r = if iid == IID_ICLASSFACTORY {
                class_factory_call(pointer, c.method, &c.data)
            } else if stub != 0 {
                stub_call(stub, &iid, c.method, &c.data)
            } else {
                Err(hr::E_NOTIMPL)
            };
            match r {
                Ok(data) => Reply {
                    status: 0,
                    data,
                    error: take_error_info(),
                }
                .encode(),
                Err(e) => Reply::failed(e).encode(),
            }
        }
        kind::REM_QUERY_INTERFACE => {
            let Ok(q) = QueryInterface::decode(body) else {
                return Reply::failed(hr::E_INVALIDARG).encode();
            };
            // A class endpoint serves its class object; otherwise the
            // object one of whose interfaces `ipid` is.
            let (identity, apartment) = match class {
                Some(cookie) => {
                    let Some((object, apartment, single)) = CLASSES.with(|c| {
                        c.iter()
                            .find(|x| x.cookie == cookie)
                            .map(|x| (x.object, x.apartment, x.single_use))
                    }) else {
                        return Reply::failed(rpc::E_DISCONNECTED).encode();
                    };
                    let mut identity = 0u64;
                    if query_interface(object, &IID_IUNKNOWN, &mut identity as *mut u64 as u64)
                        != hr::S_OK
                    {
                        return Reply::failed(hr::E_NOINTERFACE).encode();
                    }
                    release(identity);
                    if single {
                        unpublish_class(cookie);
                    }
                    (identity, apartment)
                }
                None => match lookup(&q.ipid) {
                    Some((identity, _, _, _, apartment)) => (identity, apartment),
                    None => return Reply::failed(rpc::E_DISCONNECTED).encode(),
                },
            };
            let results: Vec<(u32, Option<StdObjRef>)> = q
                .iids
                .iter()
                .map(
                    |iid| match export(identity, iid, apartment, q.refs.max(1)) {
                        Ok(s) => (hr::S_OK, Some(s)),
                        Err(e) => (e, None),
                    },
                )
                .collect();
            Reply {
                status: 0,
                data: message::encode_qi_results(&results),
                error: None,
            }
            .encode()
        }
        kind::REM_ADD_REF | kind::REM_RELEASE => {
            if let Ok(entries) = message::decode_refs(body) {
                for (ipid, refs) in entries {
                    if k == kind::REM_RELEASE {
                        unexport(&ipid, refs);
                    } else {
                        EXPORTER.with(|e| {
                            if let Some(f) = e
                                .objects
                                .iter_mut()
                                .flat_map(|o| o.interfaces.iter_mut())
                                .find(|f| f.ipid == ipid)
                            {
                                f.refs += refs;
                            }
                        });
                    }
                }
            }
            Reply {
                status: 0,
                data: Vec::new(),
                error: None,
            }
            .encode()
        }
        _ => Reply::failed(hr::E_INVALIDARG).encode(),
    }
}
/// IClassFactory over the wire: CreateInstance (3) `[iid]` replies
/// `[hr][OBJREF blob]`; LockServer (4) `[lock u32]` replies `[hr]`.
fn class_factory_call(factory: u64, method_index: u32, data: &[u8]) -> Result<Vec<u8>, u32> {
    let mut w = door_com_wire::value::Writer::new();
    match method_index {
        3 => {
            let iid: Guid = data
                .get(..16)
                .and_then(|b| b.try_into().ok())
                .ok_or(hr::E_INVALIDARG)?;
            let mut object = 0u64;
            // SAFETY: IClassFactory::CreateInstance is method 3.
            let create: extern "system" fn(usize, usize, usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(factory, 3) as usize) };
            let r = create(
                door_windows_abi::narrow(factory),
                0,
                door_windows_abi::narrow(iid.as_ptr() as u64),
                door_windows_abi::narrow(&mut object as *mut u64 as u64),
            );
            if r != hr::S_OK {
                w.u32(r).blob(&[]);
                return Ok(w.bytes);
            }
            let marshaled = marshal(object, &iid, 0);
            release(object);
            match marshaled {
                Ok(bytes) => w.u32(hr::S_OK).blob(&bytes),
                Err(e) => w.u32(e).blob(&[]),
            };
            Ok(w.bytes)
        }
        4 => {
            let lock = data
                .get(..4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            // SAFETY: IClassFactory::LockServer is method 4.
            let f: extern "system" fn(usize, i32) -> u32 =
                unsafe { core::mem::transmute(method(factory, 4) as usize) };
            w.u32(f(door_windows_abi::narrow(factory), i32::from(lock != 0)));
            Ok(w.bytes)
        }
        _ => Err(hr::E_NOTIMPL),
    }
}
/// RPCOLEMESSAGE (x64): reserved1, dataRepresentation @8, Buffer @16,
/// cbBuffer @24, iMethod @28, reserved2[5] @32, rpcFlags @72.
const MESSAGE_BYTES: usize = 80;
fn stub_call(stub: u64, _iid: &Guid, method_index: u32, data: &[u8]) -> Result<Vec<u8>, u32> {
    let buffer = alloc_bytes(data)?;
    let mut msg = [0u8; MESSAGE_BYTES];
    msg[8..12].copy_from_slice(&0x10u32.to_le_bytes());
    msg[16..24].copy_from_slice(&buffer.to_le_bytes());
    msg[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
    msg[28..32].copy_from_slice(&method_index.to_le_bytes());
    let channel = server_channel();
    // SAFETY: IRpcStubBuffer::Invoke is method 5.
    let invoke: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(stub, 5) as usize) };
    let r = invoke(
        door_windows_abi::narrow(stub),
        door_windows_abi::narrow(msg.as_mut_ptr() as u64),
        door_windows_abi::narrow(channel),
    );
    let out = u64::from_le_bytes(msg[16..24].try_into().unwrap_or([0; 8]));
    let n = u32::from_le_bytes(msg[24..28].try_into().unwrap_or([0; 4])) as usize;
    let reply = if r == hr::S_OK && out != 0 {
        let mut v = vec![0u8; n];
        read(out, &mut v).map(|_| v)
    } else {
        Err(if r == hr::S_OK { hr::E_FAIL } else { r })
    };
    if out != buffer && out != 0 {
        free_bytes(out);
    }
    free_bytes(buffer);
    reply
}
fn alloc_bytes(data: &[u8]) -> Result<u64, u32> {
    let p = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0x8, data.len().max(8)) };
    if p == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    write(door_windows_abi::widen(p), data)?;
    Ok(door_windows_abi::widen(p))
}
fn free_bytes(p: u64) {
    if p != 0 {
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(p),
            )
        };
    }
}
/// Wakes the apartment that has a request waiting.
fn wake(apartment: u32) {
    if apartment == u32::MAX {
        return;
    }
    if let Some(hwnd) = WINDOWS.with(|w| w.iter().find(|x| x.0 == apartment).map(|x| x.1)) {
        unsafe { PostMessageW(door_windows_abi::narrow(hwnd), WM_OLE_WAKE, 0, 0) };
    }
}

// ---- serving apartments ------------------------------------------------------------------
/// STA threads' message-only windows: (thread, window).
static WINDOWS: Lock<Vec<(u32, u64)>> = Lock::new(Vec::new());
static MTA_DISPATCHER: AtomicBool = AtomicBool::new(false);
extern "system" fn ole_window(hwnd: usize, message: u32, w: usize, l: isize) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let w = door_windows_abi::widen(w);
    let l = l as i64;
    ((move || -> i64 {
        if message == WM_TIMER || message == WM_OLE_WAKE {
            if let Some(a) = apartment() {
                pump(a);
            }
            return 0;
        }
        unsafe {
            (DefWindowProcW(
                door_windows_abi::narrow(hwnd),
                message,
                door_windows_abi::narrow(w),
                (l) as isize,
            )) as i64
        }
    })()) as isize
}
extern "system" fn mta_dispatcher(_argument: usize) -> u32 {
    {
        let _ = CoInitializeEx(0, coinit::MULTITHREADED);
        loop {
            pump(0);
            unsafe { DoorSleep(2) };
        }
    }
}
/// Makes sure calls for `apartment` get run: an STA's window (with a
/// timer that polls the endpoint) or the MTA's dispatcher thread.
fn serve_apartment(apartment: u32) {
    if apartment == 0 {
        if !MTA_DISPATCHER.swap(true, Ordering::AcqRel) {
            unsafe { DoorCreateThread(mta_dispatcher as *const () as u64, 0) };
        }
        return;
    }
    if apartment != me() || WINDOWS.with(|w| w.iter().any(|x| x.0 == apartment)) {
        return;
    }
    let class: Vec<u16> = "OleMainThreadWndClass".encode_utf16().chain([0]).collect();
    let title: Vec<u16> = "OleMainThreadWndName".encode_utf16().chain([0]).collect();
    let mut wc = [0u8; 80];
    wc[..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(ole_window as *const () as u64).to_le_bytes());
    wc[64..72].copy_from_slice(&(class.as_ptr() as u64).to_le_bytes());
    unsafe { RegisterClassExW(door_windows_abi::narrow(wc.as_ptr() as u64)) };
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            door_windows_abi::narrow(class.as_ptr() as u64),
            door_windows_abi::narrow(title.as_ptr() as u64),
            0,
            0,
            0,
            0,
            0,
            door_windows_abi::narrow(HWND_MESSAGE),
            0,
            0,
            0,
        )
    };
    let timer = if hwnd != 0 {
        unsafe { SetTimer(hwnd, 1, 10, 0) }
    } else {
        0
    };
    trace(&alloc::format!(
        "STA window thread={apartment} hwnd={hwnd:#x} timer={timer}"
    ));
    if hwnd != 0 {
        WINDOWS.with(|w| w.push((apartment, door_windows_abi::widen(hwnd))));
    }
}

// ---- outbound calls ----------------------------------------------------------------------
struct Outbound {
    /// The exporter endpoint (empty: a class endpoint asked once).
    endpoint: String,
    thread: u32,
    link: u32,
    joiner: Joiner,
    dead: bool,
}
static OUTBOUND: Lock<Vec<Outbound>> = Lock::new(Vec::new());
static NEXT_CALL: AtomicU32 = AtomicU32::new(1);
/// The calling thread's link to `endpoint`.
fn connection(endpoint: &str) -> Result<u32, u32> {
    let t = me();
    if let Some((link, dead)) = OUTBOUND.with(|o| {
        o.iter()
            .find(|c| c.thread == t && c.endpoint == endpoint)
            .map(|c| (c.link, c.dead))
    }) {
        return if dead {
            Err(rpc::S_SERVER_UNAVAILABLE)
        } else {
            Ok(link)
        };
    }
    let link = broker(|b| {
        b.connect_endpoint(endpoint.as_bytes())
            .map_err(|_| rpc::S_SERVER_UNAVAILABLE)
    })?;
    OUTBOUND.with(|o| {
        o.push(Outbound {
            endpoint: endpoint.into(),
            thread: t,
            link,
            joiner: Joiner::default(),
            dead: false,
        })
    });
    Ok(link)
}
/// A link that failed: later calls on it fail at once.
fn mark_dead(link: u32) {
    let was = OUTBOUND.with(|o| {
        let c = o.iter_mut().find(|c| c.link == link && !c.dead)?;
        c.dead = true;
        Some(())
    });
    if was.is_some() {
        close_link(link);
    }
}
/// Sends a request to process `oxid` and waits for its reply, serving the
/// caller's apartment meanwhile (calls back into it run).
fn request(oxid: u64, k: u8, body: &[u8]) -> Result<Vec<u8>, u32> {
    if oxid == pid() {
        // Another apartment of this process.
        let call = NEXT_CALL.fetch_add(1, Ordering::Relaxed);
        let here = apartment().unwrap_or(0);
        let target = target(k, body, None).ok_or(rpc::E_DISCONNECTED)?;
        PENDING.with(|p| {
            p.push(Pending {
                apartment: target,
                kind: k,
                call,
                body: body.to_vec(),
                back: Back::Local,
                class: None,
            })
        });
        wake(target);
        if target == 0 {
            serve_apartment(0);
        }
        loop {
            if let Some(r) =
                LOCAL_REPLIES.with(|r| r.iter().position(|x| x.0 == call).map(|i| r.remove(i).1))
            {
                return Ok(r);
            }
            pump(here);
            unsafe { DoorSleep(1) };
        }
    }
    let link = connection(&endpoint_of(oxid))?;
    call_link(link, k, body, None)
}
/// Sends a request on `link` and waits for its reply (until `deadline`,
/// if any), serving the caller's apartment meanwhile.
fn call_link(link: u32, k: u8, body: &[u8], deadline: Option<u64>) -> Result<Vec<u8>, u32> {
    let call = NEXT_CALL.fetch_add(1, Ordering::Relaxed);
    let here = apartment().unwrap_or(0);
    if let Err(e) = send(link, k, call, body) {
        mark_dead(link);
        return Err(e);
    }
    loop {
        pump(here);
        if let Some(r) = REPLIES.with(|r| {
            r.iter()
                .position(|x| x.0 == link && x.1 == call)
                .map(|i| r.remove(i).2)
        }) {
            return Ok(r);
        }
        // Init hands over a link's packets before saying it went, so a
        // reply that came is taken above.
        if OUTBOUND.with(|o| o.iter().find(|c| c.link == link).is_none_or(|c| c.dead)) {
            return Err(rpc::S_CALL_FAILED);
        }
        if deadline.is_some_and(|d| now() >= d) {
            mark_dead(link);
            return Err(rpc::S_CALL_FAILED);
        }
        unsafe { DoorSleep(1) };
    }
}
/// A call through a proxy: the reply's data, or the call's failure.
fn invoke_remote(
    manager: &Manager,
    ipid: &Guid,
    method_index: u32,
    data: &[u8],
) -> Result<Vec<u8>, u32> {
    if !manager.callable() {
        return Err(rpc::E_WRONG_THREAD);
    }
    let body = Call {
        ipid: *ipid,
        method: method_index,
        data: data.to_vec(),
    }
    .encode();
    let reply = Reply::decode(&request(manager.oxid, kind::CALL, &body)?)
        .map_err(|_| rpc::S_CALL_FAILED)?;
    if reply.status != 0 {
        return Err(reply.status);
    }
    if let Some(e) = &reply.error {
        set_error_info(e);
    }
    Ok(reply.data)
}

// ---- proxies -------------------------------------------------------------------------------
#[repr(C)]
struct UnknownVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
}
struct Proxy {
    iid: Guid,
    ipid: Guid,
    /// The interface pointer handed out.
    pointer: u64,
    /// IRpcProxyBuffer (0: the built-in IClassFactory proxy).
    buffer: u64,
    /// The channel object it was connected to.
    channel: u64,
    refs: u32,
}
/// One imported object in one apartment: its identity (IUnknown).
#[repr(C)]
struct Manager {
    vtbl: &'static UnknownVtbl,
    references: AtomicU32,
    oxid: u64,
    oid: u64,
    apartment: u32,
    proxies: Lock<Vec<Proxy>>,
}
impl Manager {
    fn callable(&self) -> bool {
        apartment() == Some(self.apartment)
    }
    fn any_ipid(&self) -> Option<Guid> {
        self.proxies.with(|p| p.first().map(|x| x.ipid))
    }
}
static MANAGER_VTBL: UnknownVtbl = UnknownVtbl {
    query: manager_query,
    add_ref: manager_add_ref,
    release: manager_release,
};
/// (oxid, oid, apartment, manager): one manager per object and apartment.
static MANAGERS: Lock<Vec<(u64, u64, u32, u64)>> = Lock::new(Vec::new());
fn manager(p: u64) -> &'static Manager {
    // SAFETY: MANAGER_VTBL's methods are only called on Managers.
    unsafe { &*(p as *const Manager) }
}
/// The Manager behind an interface pointer of ours (a proxy or the
/// manager itself), found through its IUnknown.
fn manager_of(unknown: u64) -> Option<u64> {
    let mut identity = 0u64;
    if query_interface(unknown, &IID_IUNKNOWN, &mut identity as *mut u64 as u64) != hr::S_OK {
        return None;
    }
    release(identity);
    // SAFETY: the first word of an interface pointer is its vtable.
    let vtbl = unsafe { *(identity as *const usize) };
    (vtbl == &MANAGER_VTBL as *const UnknownVtbl as usize).then_some(identity)
}
extern "system" fn manager_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    manager(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn manager_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let m = manager(this);
        let left = m.references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 {
            MANAGERS.with(|list| list.retain(|e| e.3 != this));
            let proxies = m.proxies.with(core::mem::take);
            let refs: Vec<(Guid, u32)> = proxies
                .iter()
                .filter(|p| p.refs > 0)
                .map(|p| (p.ipid, p.refs))
                .collect();
            for p in &proxies {
                if p.buffer != 0 {
                    // SAFETY: IRpcProxyBuffer::Disconnect is method 4.
                    let disconnect: extern "system" fn(usize) -> u32 =
                        unsafe { core::mem::transmute(method(p.buffer, 4) as usize) };
                    disconnect(door_windows_abi::narrow(p.buffer));
                    release(p.buffer);
                } else if p.iid == IID_ICLASSFACTORY {
                    // SAFETY: the built-in proxy came from Box::into_raw.
                    drop(unsafe { Box::from_raw(p.pointer as *mut FactoryProxy) });
                }
                if p.channel != 0 {
                    release(p.channel);
                }
            }
            if !refs.is_empty() && m.callable() {
                let _ = request(m.oxid, kind::REM_RELEASE, &message::encode_refs(&refs));
            }
            // SAFETY: the last reference; the manager came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut Manager) });
        }
        left
    }
}
extern "system" fn manager_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_guid(iid) else {
            return hr::E_POINTER;
        };
        let _ = write_pointer(out, 0);
        if g == IID_IUNKNOWN {
            manager_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let m = manager(this);
        if let Some(p) = m
            .proxies
            .with(|p| p.iter().find(|x| x.iid == g).map(|x| x.pointer))
        {
            manager_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, p));
        }
        if !m.callable() {
            return rpc::E_WRONG_THREAD;
        }
        let Some(any) = m.any_ipid() else {
            return hr::E_NOINTERFACE;
        };
        let body = QueryInterface {
            ipid: any,
            refs: 1,
            iids: vec![g],
        }
        .encode();
        let reply = match request(m.oxid, kind::REM_QUERY_INTERFACE, &body)
            .and_then(|b| Reply::decode(&b).map_err(|_| rpc::S_CALL_FAILED))
        {
            Ok(r) if r.status == 0 => r,
            Ok(r) => return r.status,
            Err(e) => return e,
        };
        let Ok(results) = message::decode_qi_results(&reply.data) else {
            return rpc::S_CALL_FAILED;
        };
        match results.first() {
            Some((code, Some(std))) if *code == hr::S_OK => match add_proxy(this, &g, std) {
                Ok(p) => {
                    manager_add_ref(door_windows_abi::narrow(this));
                    result(write_pointer(out, p))
                }
                Err(e) => e,
            },
            Some((code, _)) => *code,
            None => hr::E_NOINTERFACE,
        }
    }
}
/// The interface pointer for `iid` of manager `m`, created and connected
/// when new; `std.refs` are the manager's from then on.
fn add_proxy(m: u64, iid: &Guid, std: &StdObjRef) -> Result<u64, u32> {
    let mgr = manager(m);
    if let Some(p) = mgr.proxies.with(|list| {
        list.iter_mut().find(|x| x.iid == *iid).map(|x| {
            x.refs += std.refs;
            x.pointer
        })
    }) {
        return Ok(p);
    }
    let (pointer, buffer, channel) = if *iid == IID_IUNKNOWN {
        (m, 0, 0)
    } else if *iid == IID_ICLASSFACTORY {
        let p = Box::into_raw(Box::new(FactoryProxy {
            vtbl: &FACTORY_VTBL,
            manager: m,
            ipid: std.ipid,
        })) as u64;
        (p, 0, 0)
    } else {
        let factory = ps_factory(iid)?;
        let mut buffer = 0u64;
        let mut pointer = 0u64;
        // SAFETY: IPSFactoryBuffer::CreateProxy is method 3.
        let create: extern "system" fn(usize, usize, usize, usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(factory, 3) as usize) };
        let r = create(
            door_windows_abi::narrow(factory),
            door_windows_abi::narrow(m),
            door_windows_abi::narrow(iid.as_ptr() as u64),
            door_windows_abi::narrow(&mut buffer as *mut u64 as u64),
            door_windows_abi::narrow(&mut pointer as *mut u64 as u64),
        );
        release(factory);
        if r != hr::S_OK {
            return Err(r);
        }
        let channel = Box::into_raw(Box::new(ChannelBuffer {
            vtbl: &CHANNEL_VTBL,
            references: AtomicU32::new(1),
            manager: m,
            ipid: std.ipid,
            iid: *iid,
        })) as u64;
        // SAFETY: IRpcProxyBuffer::Connect is method 3.
        let connect: extern "system" fn(usize, usize) -> u32 =
            unsafe { core::mem::transmute(method(buffer, 3) as usize) };
        let r = connect(
            door_windows_abi::narrow(buffer),
            door_windows_abi::narrow(channel),
        );
        if r != hr::S_OK {
            release(channel);
            release(buffer);
            return Err(r);
        }
        // CreateProxy's interface pointer holds a reference on the outer
        // unknown (the manager), which the manager's own count covers.
        manager_release(door_windows_abi::narrow(m));
        (pointer, buffer, channel)
    };
    mgr.proxies.with(|list| {
        list.push(Proxy {
            iid: *iid,
            ipid: std.ipid,
            pointer,
            buffer,
            channel,
            refs: std.refs,
        })
    });
    Ok(pointer)
}
/// The manager for (oxid, oid) in this apartment, new or existing (a new
/// reference either way).
fn import(std: &StdObjRef) -> Result<u64, u32> {
    let here = apartment().ok_or(hr::CO_E_NOTINITIALIZED)?;
    if let Some(m) = MANAGERS.with(|list| {
        list.iter()
            .find(|e| e.0 == std.oxid && e.1 == std.oid && e.2 == here)
            .map(|e| e.3)
    }) {
        manager_add_ref(door_windows_abi::narrow(m));
        return Ok(m);
    }
    let m = Box::into_raw(Box::new(Manager {
        vtbl: &MANAGER_VTBL,
        references: AtomicU32::new(1),
        oxid: std.oxid,
        oid: std.oid,
        apartment: here,
        proxies: Lock::new(Vec::new()),
    })) as u64;
    MANAGERS.with(|list| list.push((std.oxid, std.oid, here, m)));
    Ok(m)
}

// ---- the built-in IClassFactory proxy ------------------------------------------------------
#[repr(C)]
struct FactoryVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    create_instance: extern "system" fn(usize, usize, usize, usize) -> u32,
    lock_server: extern "system" fn(usize, i32) -> u32,
}
#[repr(C)]
struct FactoryProxy {
    vtbl: &'static FactoryVtbl,
    manager: u64,
    ipid: Guid,
}
static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query: factory_query,
    add_ref: factory_add_ref,
    release: factory_release,
    create_instance: factory_create,
    lock_server: factory_lock,
};
fn factory(p: u64) -> &'static FactoryProxy {
    // SAFETY: FACTORY_VTBL's methods are only called on FactoryProxies.
    unsafe { &*(p as *const FactoryProxy) }
}
extern "system" fn factory_create(this: usize, outer: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        if outer != 0 {
            return hr::CLASS_E_NOAGGREGATION;
        }
        let Ok(g) = read_guid(iid) else {
            return hr::E_INVALIDARG;
        };
        let f = factory(this);
        let data = match invoke_remote(manager(f.manager), &f.ipid, 3, &g) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut r = door_com_wire::value::Reader::new(&data);
        let (Ok(code), Ok(objref)) = (r.u32(), r.blob()) else {
            return rpc::S_CALL_FAILED;
        };
        if code != hr::S_OK {
            return code;
        }
        match unmarshal_bytes(objref, &g) {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
extern "system" fn factory_lock(this: usize, lock: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let f = factory(this);
        match invoke_remote(
            manager(f.manager),
            &f.ipid,
            4,
            &u32::from(lock != 0).to_le_bytes(),
        ) {
            Ok(d) if d.len() >= 4 => u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
            Ok(_) => rpc::S_CALL_FAILED,
            Err(e) => e,
        }
    }
}

// ---- IRpcChannelBuffer ---------------------------------------------------------------------
#[repr(C)]
struct ChannelVtbl {
    query: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    get_buffer: extern "system" fn(usize, usize, usize) -> u32,
    send_receive: extern "system" fn(usize, usize, usize) -> u32,
    free_buffer: extern "system" fn(usize, usize) -> u32,
    get_dest_ctx: extern "system" fn(usize, usize, usize) -> u32,
    is_connected: extern "system" fn(usize) -> u32,
}
/// A proxy's channel (to one interface of one imported object), or with
/// `manager` 0 the channel stubs reply through.
#[repr(C)]
struct ChannelBuffer {
    vtbl: &'static ChannelVtbl,
    references: AtomicU32,
    manager: u64,
    ipid: Guid,
    iid: Guid,
}
static CHANNEL_VTBL: ChannelVtbl = ChannelVtbl {
    query: channel_query,
    add_ref: channel_add_ref,
    release: channel_release,
    get_buffer: channel_get_buffer,
    send_receive: channel_send_receive,
    free_buffer: channel_free_buffer,
    get_dest_ctx: channel_dest_ctx,
    is_connected: channel_is_connected,
};
extern "system" fn channel_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    channel(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn channel_is_connected(_this: usize) -> u32 {
    hr::S_OK
}
/// MSHCTX_LOCAL: another process on this computer.
extern "system" fn channel_dest_ctx(_this: usize, ctx: usize, pv: usize) -> u32 {
    let ctx = door_windows_abi::widen(ctx);
    let pv = door_windows_abi::widen(pv);
    {
        let _ = write_u32(ctx, 1);
        if pv != 0 {
            let _ = write_pointer(pv, 0);
        }
        hr::S_OK
    }
}
extern "system" fn factory_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        manager_query(
            door_windows_abi::narrow(factory(this).manager),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn factory_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    manager_add_ref(door_windows_abi::narrow(factory(this).manager))
}
extern "system" fn factory_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    manager_release(door_windows_abi::narrow(factory(this).manager))
}
fn channel(p: u64) -> &'static ChannelBuffer {
    // SAFETY: CHANNEL_VTBL's methods are only called on ChannelBuffers.
    unsafe { &*(p as *const ChannelBuffer) }
}
static SERVER_CHANNEL: AtomicU64 = AtomicU64::new(0);
fn server_channel() -> u64 {
    let p = SERVER_CHANNEL.load(Ordering::Acquire);
    if p != 0 {
        return p;
    }
    let c = Box::into_raw(Box::new(ChannelBuffer {
        vtbl: &CHANNEL_VTBL,
        references: AtomicU32::new(1),
        manager: 0,
        ipid: [0; 16],
        iid: [0; 16],
    })) as u64;
    match SERVER_CHANNEL.compare_exchange(0, c, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => c,
        Err(existing) => existing,
    }
}
extern "system" fn channel_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        match read_guid(iid) {
            Ok(g) if g == IID_IUNKNOWN || g == IID_IRPCCHANNELBUFFER => {
                channel(this).references.fetch_add(1, Ordering::AcqRel);
                result(write_pointer(out, this))
            }
            Ok(_) => {
                let _ = write_pointer(out, 0);
                hr::E_NOINTERFACE
            }
            Err(e) => e,
        }
    }
}
extern "system" fn channel_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let c = channel(this);
        let left = c.references.fetch_sub(1, Ordering::AcqRel) - 1;
        if left == 0 && c.manager != 0 {
            // SAFETY: the last reference; it came from Box::into_raw.
            drop(unsafe { Box::from_raw(this as *mut ChannelBuffer) });
        }
        left
    }
}
/// RPCOLEMESSAGE: reserved1, dataRepresentation, Buffer (16 on x64, 8 on
/// x86), cbBuffer, iMethod.
const MSG_BUFFER: u64 = 2 * POINTER as u64;
const MSG_LENGTH: u64 = MSG_BUFFER + POINTER as u64;
const MSG_METHOD: u64 = MSG_LENGTH + 4;
extern "system" fn channel_get_buffer(_this: usize, msg: usize, _iid: usize) -> u32 {
    let msg = door_windows_abi::widen(msg);
    {
        let r = (|| {
            let n = read_u32(msg + MSG_LENGTH)? as usize;
            let p = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0x8, n.max(8)) };
            if p == 0 {
                return Err(hr::E_OUTOFMEMORY);
            }
            write_pointer(msg + MSG_BUFFER, door_windows_abi::widen(p))
        })();
        result(r)
    }
}
extern "system" fn channel_free_buffer(_this: usize, msg: usize) -> u32 {
    let msg = door_windows_abi::widen(msg);
    {
        if let Ok(p) = read_pointer(msg + MSG_BUFFER) {
            free_bytes(p);
            let _ = write_pointer(msg + MSG_BUFFER, 0);
        }
        hr::S_OK
    }
}
/// SendReceive: the request in the message's buffer goes out; the reply
/// replaces it.
extern "system" fn channel_send_receive(this: usize, msg: usize, status: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let msg = door_windows_abi::widen(msg);
    let status = door_windows_abi::widen(status);
    {
        let c = channel(this);
        if c.manager == 0 {
            return hr::E_UNEXPECTED;
        }
        let r = (|| {
            let buffer = read_pointer(msg + MSG_BUFFER)?;
            let n = read_u32(msg + MSG_LENGTH)? as usize;
            let method_index = read_u32(msg + MSG_METHOD)?;
            let mut data = vec![0u8; n];
            read(buffer, &mut data)?;
            let reply = invoke_remote(manager(c.manager), &c.ipid, method_index, &data)?;
            let out = alloc_bytes(&reply)?;
            free_bytes(buffer);
            write_pointer(msg + MSG_BUFFER, out)?;
            write_u32(msg + MSG_LENGTH, reply.len() as u32)
        })();
        if status != 0 {
            let _ = write_u32(status, r.err().unwrap_or(0));
        }
        let _ = c.iid;
        result(r)
    }
}

// ---- IPSFactoryBuffer --------------------------------------------------------------------
fn interface_value(iid: &Guid, sub: &str) -> Option<Vec<u16>> {
    let mut path = ascii("Interface\\");
    path.extend(guid_units(iid));
    path.push(b'\\' as u16);
    path.extend(ascii(sub));
    class_value(&path, None)
}
/// The IPSFactoryBuffer of `iid`'s ProxyStubClsid32 (IDispatch: PSDispatch).
fn ps_factory(iid: &Guid) -> Result<u64, u32> {
    let clsid = match interface_value(iid, "ProxyStubClsid32") {
        Some(text) => {
            let mut z = text.clone();
            z.push(0);
            let mut g = [0u8; 16];
            if CLSIDFromString(
                door_windows_abi::narrow(z.as_ptr() as u64),
                door_windows_abi::narrow(g.as_mut_ptr() as u64),
            ) != hr::S_OK
            {
                return Err(hr::REGDB_E_IIDNOTREG);
            }
            g
        }
        None if *iid == IID_IDISPATCH => PS_DISPATCH,
        None => return Err(hr::REGDB_E_IIDNOTREG),
    };
    let mut factory = 0u64;
    let r = class_object(
        &clsid,
        clsctx::INPROC_SERVER,
        &IID_IPSFACTORYBUFFER,
        &mut factory as *mut u64 as u64,
    );
    if r == hr::S_OK {
        return Ok(factory);
    }
    if clsid == PS_OA_INTERFACE || clsid == PS_DISPATCH {
        // The automation marshalers are oleaut32's, registered or not.
        let get = automation("DllGetClassObject").ok_or(r)?;
        // SAFETY: DllGetClassObject's documented signature.
        let get: extern "system" fn(usize, usize, usize) -> u32 =
            unsafe { core::mem::transmute(get as usize) };
        let r = get(
            door_windows_abi::narrow(clsid.as_ptr() as u64),
            door_windows_abi::narrow(IID_IPSFACTORYBUFFER.as_ptr() as u64),
            door_windows_abi::narrow(&mut factory as *mut u64 as u64),
        );
        return if r == hr::S_OK { Ok(factory) } else { Err(r) };
    }
    Err(r)
}

// ---- marshaling --------------------------------------------------------------------------
/// The OBJREF for `iid` of `unknown` (a NORMAL marshal: one reference;
/// TABLESTRONG keeps the object until CoReleaseMarshalData).
pub(super) fn marshal(unknown: u64, iid: &Guid, flags: u32) -> Result<Vec<u8>, u32> {
    let here = apartment().ok_or(hr::CO_E_NOTINITIALIZED)?;
    // A proxy forwards its object: the new holder gets a reference the
    // manager adds for it.
    if let Some(m) = manager_of(unknown) {
        let mgr = manager(m);
        let mut p = 0u64;
        let r = manager_query(
            door_windows_abi::narrow(m),
            door_windows_abi::narrow(iid.as_ptr() as u64),
            door_windows_abi::narrow(&mut p as *mut u64 as u64),
        );
        if r != hr::S_OK {
            return Err(r);
        }
        release(p);
        let ipid = mgr
            .proxies
            .with(|list| list.iter().find(|x| x.iid == *iid).map(|x| x.ipid))
            .ok_or(hr::E_NOINTERFACE)?;
        request(
            mgr.oxid,
            kind::REM_ADD_REF,
            &message::encode_refs(&[(ipid, 1)]),
        )?;
        return Ok(ObjRef::Standard {
            iid: *iid,
            std: StdObjRef {
                flags: SORF_NOPING,
                refs: 1,
                oxid: mgr.oxid,
                oid: mgr.oid,
                ipid,
            },
            endpoint: endpoint_of(mgr.oxid),
        }
        .encode());
    }
    let mut identity = 0u64;
    let r = query_interface(unknown, &IID_IUNKNOWN, &mut identity as *mut u64 as u64);
    if r != hr::S_OK {
        return Err(r);
    }
    let std = export(
        identity,
        iid,
        here,
        if flags == MSHLFLAGS_TABLEWEAK { 0 } else { 1 },
    );
    release(identity);
    let std = std?;
    Ok(ObjRef::Standard {
        iid: *iid,
        std,
        endpoint: endpoint_of(pid()),
    }
    .encode())
}
/// The interface `iid` an OBJREF names, in this apartment.
pub(super) fn unmarshal_bytes(bytes: &[u8], iid: &Guid) -> Result<u64, u32> {
    let (objref, _) = ObjRef::decode(bytes).ok_or(rpc::E_INVALID_OBJREF)?;
    unmarshal(&objref, iid)
}
fn unmarshal(objref: &ObjRef, iid: &Guid) -> Result<u64, u32> {
    let ObjRef::Standard {
        iid: marshaled,
        std,
        ..
    } = objref
    else {
        return Err(hr::E_NOTIMPL);
    };
    let here = apartment().ok_or(hr::CO_E_NOTINITIALIZED)?;
    if std.oxid == pid()
        && let Some((identity, pointer, _, _, owner)) = lookup(&std.ipid)
        && owner == here
    {
        // Back in its own apartment: the object itself.
        let mut out = 0u64;
        let r = if *iid == *marshaled {
            add_ref(pointer);
            out = pointer;
            hr::S_OK
        } else {
            query_interface(identity, iid, &mut out as *mut u64 as u64)
        };
        unexport(&std.ipid, std.refs);
        return if r == hr::S_OK { Ok(out) } else { Err(r) };
    }
    let m = import(std)?;
    if let Err(e) = add_proxy(m, marshaled, std) {
        manager_release(door_windows_abi::narrow(m));
        return Err(e);
    }
    let mut out = 0u64;
    let r = manager_query(
        door_windows_abi::narrow(m),
        door_windows_abi::narrow(iid.as_ptr() as u64),
        door_windows_abi::narrow(&mut out as *mut u64 as u64),
    );
    manager_release(door_windows_abi::narrow(m));
    if r == hr::S_OK { Ok(out) } else { Err(r) }
}
/// IStream::Read of exactly `n` bytes.
fn stream_read(stream: u64, n: usize) -> Result<Vec<u8>, u32> {
    let mut b = vec![0u8; n];
    let mut got = 0u32;
    // SAFETY: IStream::Read is method 3.
    let f: extern "system" fn(usize, usize, u32, usize) -> u32 =
        unsafe { core::mem::transmute(method(stream, 3) as usize) };
    let r = f(
        door_windows_abi::narrow(stream),
        door_windows_abi::narrow(b.as_mut_ptr() as u64),
        n as u32,
        door_windows_abi::narrow(&mut got as *mut u32 as u64),
    );
    if r != hr::S_OK || got as usize != n {
        return Err(if r == hr::S_OK {
            rpc::E_INVALID_OBJREF
        } else {
            r
        });
    }
    Ok(b)
}
fn stream_write(stream: u64, data: &[u8]) -> Result<(), u32> {
    let mut put = 0u32;
    // SAFETY: IStream::Write is method 4.
    let f: extern "system" fn(usize, usize, u32, usize) -> u32 =
        unsafe { core::mem::transmute(method(stream, 4) as usize) };
    let r = f(
        door_windows_abi::narrow(stream),
        door_windows_abi::narrow(data.as_ptr() as u64),
        data.len() as u32,
        door_windows_abi::narrow(&mut put as *mut u32 as u64),
    );
    if r != hr::S_OK { Err(r) } else { Ok(()) }
}
/// An OBJREF read off `stream`.
fn read_objref(stream: u64) -> Result<ObjRef, u32> {
    let mut bytes = stream_read(stream, 24)?;
    let flags = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    match flags {
        door_com_wire::objref::FLAGS_STANDARD => {
            bytes.extend(stream_read(stream, 40 + 4)?);
            let n = bytes.len();
            let entries = u16::from_le_bytes([bytes[n - 4], bytes[n - 3]]) as usize;
            bytes.extend(stream_read(stream, entries * 2)?);
        }
        door_com_wire::objref::FLAGS_CUSTOM => {
            bytes.extend(stream_read(stream, 24)?);
            let n = bytes.len();
            let size = u32::from_le_bytes([bytes[n - 4], bytes[n - 3], bytes[n - 2], bytes[n - 1]])
                as usize;
            bytes.extend(stream_read(stream, size)?);
        }
        _ => return Err(rpc::E_INVALID_OBJREF),
    }
    ObjRef::decode(&bytes)
        .map(|o| o.0)
        .ok_or(rpc::E_INVALID_OBJREF)
}

#[unsafe(no_mangle)]
pub extern "system" fn CoMarshalInterface(
    stream: usize,
    iid: usize,
    unknown: usize,
    _context: u32,
    _reserved: usize,
    flags: u32,
) -> u32 {
    let stream = door_windows_abi::widen(stream);
    let iid = door_windows_abi::widen(iid);
    let unknown = door_windows_abi::widen(unknown);
    {
        if stream == 0 || unknown == 0 {
            return hr::E_INVALIDARG;
        }
        let Ok(g) = read_guid(iid) else {
            return hr::E_INVALIDARG;
        };
        let r = marshal(unknown, &g, flags).and_then(|b| stream_write(stream, &b));
        result(r)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoUnmarshalInterface(stream: usize, iid: usize, out: usize) -> u32 {
    let stream = door_windows_abi::widen(stream);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let r = (|| {
            let objref = read_objref(stream)?;
            let wanted = if iid == 0 {
                *objref.iid()
            } else {
                read_guid(iid)?
            };
            unmarshal(&objref, &wanted)
        })();
        match r {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoReleaseMarshalData(stream: usize) -> u32 {
    let stream = door_windows_abi::widen(stream);
    {
        let r = (|| {
            let ObjRef::Standard { std, .. } = read_objref(stream)? else {
                return Ok(());
            };
            if std.oxid == pid() {
                unexport(&std.ipid, std.refs.max(1));
                Ok(())
            } else {
                request(
                    std.oxid,
                    kind::REM_RELEASE,
                    &message::encode_refs(&[(std.ipid, std.refs)]),
                )
                .map(|_| ())
            }
        })();
        result(r)
    }
}
/// What our OBJREFs take at most.
#[unsafe(no_mangle)]
pub extern "system" fn CoGetMarshalSizeMax(
    size: usize,
    _iid: usize,
    _unknown: usize,
    _context: u32,
    _reserved: usize,
    _flags: u32,
) -> u32 {
    let size = door_windows_abi::widen(size);
    result(write_u32(size, 256))
}
#[unsafe(no_mangle)]
pub extern "system" fn CoMarshalInterThreadInterfaceInStream(
    iid: usize,
    unknown: usize,
    out: usize,
) -> u32 {
    let iid = door_windows_abi::widen(iid);
    let unknown = door_windows_abi::widen(unknown);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let mut stream = 0u64;
        let r = CreateStreamOnHGlobal(
            0,
            1,
            door_windows_abi::narrow(&mut stream as *mut u64 as u64),
        );
        if r != hr::S_OK {
            return r;
        }
        // MSHCTX_INPROC (3), MSHLFLAGS_NORMAL.
        let r = CoMarshalInterface(
            door_windows_abi::narrow(stream),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(unknown),
            3,
            0,
            0,
        );
        if r != hr::S_OK {
            release(stream);
            return r;
        }
        // SAFETY: IStream::Seek is method 5 (to the start).
        let seek: extern "system" fn(usize, isize, u32, usize) -> u32 =
            unsafe { core::mem::transmute(method(stream, 5) as usize) };
        seek(door_windows_abi::narrow(stream), 0, 0, 0);
        result(write_pointer(out, stream))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoGetInterfaceAndReleaseStream(
    stream: usize,
    iid: usize,
    out: usize,
) -> u32 {
    let stream = door_windows_abi::widen(stream);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let r = CoUnmarshalInterface(
            door_windows_abi::narrow(stream),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        );
        release(stream);
        r
    }
}
/// Disconnects every remote connection to `unknown`'s object: its stubs
/// go and later calls fail with RPC_E_DISCONNECTED.
#[unsafe(no_mangle)]
pub extern "system" fn CoDisconnectObject(unknown: usize, _reserved: u32) -> u32 {
    let unknown = door_windows_abi::widen(unknown);
    {
        let mut identity = 0u64;
        if query_interface(unknown, &IID_IUNKNOWN, &mut identity as *mut u64 as u64) != hr::S_OK {
            return hr::E_INVALIDARG;
        }
        release(identity);
        let gone = EXPORTER.with(|e| {
            e.objects
                .iter()
                .position(|o| o.identity == identity)
                .map(|i| e.objects.remove(i))
        });
        if let Some(o) = gone {
            drop_object(o);
        }
        hr::S_OK
    }
}

// ---- class objects for other processes -------------------------------------------------
/// CoRegisterClassObject with CLSCTX_LOCAL_SERVER: its class endpoint.
pub(super) fn publish_class(
    cookie: u32,
    clsid: &Guid,
    object: u64,
    single_use: bool,
) -> Result<(), u32> {
    let here = apartment().ok_or(hr::CO_E_NOTINITIALIZED)?;
    let name = wire::class_endpoint(clsid);
    let r = broker(|b| b.register(&name).map_err(|_| hr::CO_E_OBJNOTREG));
    trace(&alloc::format!(
        "CLASS publish endpoint={} pid={} result={:#x}",
        core::str::from_utf8(&name).unwrap_or("?"),
        pid(),
        r.err().unwrap_or(0)
    ));
    r?;
    add_ref(object);
    CLASSES.with(|c| {
        c.push(ServedClass {
            cookie,
            clsid: *clsid,
            object,
            apartment: here,
            single_use,
        })
    });
    serve_apartment(here);
    Ok(())
}
pub(super) fn unpublish_class(cookie: u32) {
    let gone = CLASSES.with(|c| {
        c.iter()
            .position(|x| x.cookie == cookie)
            .map(|i| c.remove(i))
    });
    if let Some(x) = gone {
        let name = wire::class_endpoint(&x.clsid);
        let _ = broker(|b| b.unregister(&name).map_err(|_| hr::E_FAIL));
        release(x.object);
    }
}
/// LocalServer32 activation: the class object `iid` of `clsid` from its
/// running server, which is started first when the broker says so.
pub(super) fn local_class_object(clsid: &Guid, iid: &Guid, out: u64) -> u32 {
    let deadline = now() + wire::LAUNCH_TICKS;
    let mut launched: Option<u64> = None;
    let state = loop {
        let state = match broker(|b| b.activate(clsid).map_err(|_| hr::CO_E_SERVER_EXEC_FAILURE)) {
            Ok(s) => s,
            Err(e) => return e,
        };
        match state {
            wire::READY => break wire::READY,
            wire::LAUNCH if launched.is_none() => match start_server(clsid) {
                Ok(process) => {
                    trace("ACTIVATE launched LocalServer32 -Embedding");
                    launched = Some(process)
                }
                Err(e) => {
                    trace(&alloc::format!("ACTIVATE launch failed {e:#x}"));
                    let _ = broker(|b| b.launch_failed(clsid).map_err(|_| hr::E_FAIL));
                    return e;
                }
            },
            _ => {}
        }
        // A server that ended before registering has failed.
        if let Some(process) = launched {
            let mut code = 0u32;
            if unsafe { DoorGetExitCodeProcess(process, &mut code as *mut u32 as u64) } != 0
                && code != 259
            {
                trace(&alloc::format!(
                    "ACTIVATE server ended before registering exit={code}"
                ));
                let _ = broker(|b| b.launch_failed(clsid).map_err(|_| hr::E_FAIL));
                unsafe { DoorCloseHandle(process) };
                return hr::CO_E_SERVER_EXEC_FAILURE;
            }
        }
        if now() >= deadline {
            trace("ACTIVATE timed out");
            let _ = broker(|b| b.launch_failed(clsid).map_err(|_| hr::E_FAIL));
            break 0xFFFF;
        }
        if let Some(a) = apartment() {
            pump(a);
        }
        unsafe { DoorSleep(20) };
    };
    if let Some(process) = launched {
        unsafe { DoorCloseHandle(process) };
    }
    if state != wire::READY {
        return hr::CO_E_SERVER_EXEC_FAILURE;
    }
    // Ask the class endpoint for its class object.
    let name = wire::class_endpoint(clsid);
    let r = (|| -> Result<u64, u32> {
        let link = broker(|b| {
            b.connect_endpoint(&name)
                .map_err(|_| hr::CO_E_SERVER_EXEC_FAILURE)
        })?;
        OUTBOUND.with(|o| {
            o.push(Outbound {
                endpoint: String::new(),
                thread: me(),
                link,
                joiner: Joiner::default(),
                dead: false,
            })
        });
        let body = QueryInterface {
            ipid: [0; 16],
            refs: 1,
            iids: vec![*iid],
        }
        .encode();
        let reply = call_link(
            link,
            kind::REM_QUERY_INTERFACE,
            &body,
            Some(now() + wire::LAUNCH_TICKS),
        )
        .map_err(|_| hr::CO_E_SERVER_EXEC_FAILURE);
        let open = OUTBOUND.with(|o| {
            let open = o.iter().any(|c| c.link == link && !c.dead);
            o.retain(|c| c.link != link);
            open
        });
        if open {
            close_link(link);
        }
        trace(&alloc::format!(
            "ACTIVATE class object reply={}",
            match &reply {
                Ok(b) => alloc::format!("{} bytes", b.len()),
                Err(e) => alloc::format!("{e:#x}"),
            }
        ));
        let reply = Reply::decode(&reply?).map_err(|_| hr::CO_E_SERVER_EXEC_FAILURE)?;
        if reply.status != 0 {
            return Err(reply.status);
        }
        let results =
            message::decode_qi_results(&reply.data).map_err(|_| hr::CO_E_SERVER_EXEC_FAILURE)?;
        let std = match results.first() {
            Some((code, Some(std))) if *code == hr::S_OK => *std,
            Some((code, _)) => return Err(*code),
            None => return Err(hr::E_NOINTERFACE),
        };
        unmarshal(
            &ObjRef::Standard {
                iid: *iid,
                std,
                endpoint: endpoint_of(std.oxid),
            },
            iid,
        )
    })();
    match r {
        Ok(p) => result(write_pointer(out, p)),
        Err(e) => e,
    }
}
/// Starts the LocalServer32 command with `-Embedding`: its process handle.
fn start_server(clsid: &Guid) -> Result<u64, u32> {
    let command = super::local_server(clsid, 0).ok_or(hr::REGDB_E_CLASSNOTREG)?;
    let mut expanded = vec![0u16; 1024];
    let mut source = command.clone();
    source.push(0);
    let n = unsafe {
        DoorExpandEnvironmentStringsW(source.as_ptr() as u64, expanded.as_mut_ptr() as u64, 1024)
    };
    let mut line: Vec<u16> = if n > 0 && (n as usize) <= expanded.len() {
        expanded[..n as usize - 1].to_vec()
    } else {
        command
    };
    line.extend(" -Embedding".encode_utf16());
    line.push(0);
    let mut info = [0u8; 24];
    if unsafe { DoorCreateProcessW(0, line.as_ptr() as u64, 0, 0, info.as_mut_ptr() as u64, 0) }
        == 0
    {
        return Err(hr::CO_E_SERVER_EXEC_FAILURE);
    }
    // PROCESS_INFORMATION { hProcess, hThread, ... } at this build's width
    // (ntdll writes it as the program's).
    use door_windows_abi::layout::{self, Field::*};
    const PROCESS_INFORMATION: [layout::Field; 4] = [Pointer, Pointer, U32, U32];
    let thread = layout::get(&info, &PROCESS_INFORMATION, 1);
    if thread != 0 {
        unsafe { DoorCloseHandle(thread) };
    }
    Ok(layout::get(&info, &PROCESS_INFORMATION, 0))
}
