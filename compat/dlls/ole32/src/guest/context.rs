//! Contexts, initialize spies and the free-threaded marshaler (P18, for
//! CoreCLR's COM interop). Every object lives in this process and is
//! called directly, so a context is an apartment's identity: the MTA has
//! one, each STA thread its own. IContextCallback::ContextCallback calls
//! the function on the caller's thread (there is no other apartment's
//! thread to switch to while methods are called directly).
use super::*;

const IID_ICONTEXTCALLBACK: Guid = guid(0x1da, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMARSHAL: Guid = guid(3, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IAGILEOBJECT: Guid = guid(
    0x94EA_2B94,
    0xE9CC,
    0x49E0,
    [0xC0, 0xFF, 0xEE, 0x64, 0xCA, 0x8F, 0x5B, 0x90],
);
const IID_IINITIALIZESPY: Guid = guid(0x34, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

// ---- contexts ------------------------------------------------------------------------
#[repr(C)]
struct ContextVtbl {
    query_interface: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    context_callback: extern "system" fn(usize, usize, usize, usize, i32, usize) -> u32,
}
static CONTEXT_VTBL: ContextVtbl = ContextVtbl {
    query_interface: c_query_interface,
    add_ref: c_add_ref,
    release: c_release,
    context_callback: c_context_callback,
};
/// A context: its table pointer and the apartment's thread (0 = the MTA).
#[repr(C)]
struct Context {
    vtbl: &'static ContextVtbl,
    thread: u32,
}
static MTA_CONTEXT: Context = Context {
    vtbl: &CONTEXT_VTBL,
    thread: 0,
};
/// Each STA thread's context. A context is never freed: a token is an
/// identity callers keep after the apartment is gone.
static STA_CONTEXTS: Lock<Vec<(u32, u64)>> = Lock::new(Vec::new());
/// The calling thread's context, or CO_E_NOTINITIALIZED.
fn context() -> Result<u64, u32> {
    match current() {
        None => Err(hr::CO_E_NOTINITIALIZED),
        Some((Apartment::Mta, _)) => Ok(&MTA_CONTEXT as *const Context as u64),
        Some((Apartment::Sta, _)) => {
            let id = me();
            Ok(STA_CONTEXTS.with(|list| {
                if let Some((_, c)) = list.iter().find(|(t, _)| *t == id) {
                    return *c;
                }
                let c = Box::into_raw(Box::new(Context {
                    vtbl: &CONTEXT_VTBL,
                    thread: id,
                })) as u64;
                list.push((id, c));
                c
            }))
        }
    }
}
/// A thread that exits forgets its STA context (the object stays valid).
pub(super) fn thread_detach(id: u32) {
    STA_CONTEXTS.with(|list| list.retain(|(t, _)| *t != id));
    SPIES.with(|s| s.list.retain(|spy| spy.thread != id));
}
extern "system" fn c_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    let Ok(iid) = read_guid(iid) else {
        return hr::E_POINTER;
    };
    if [IID_IUNKNOWN, IID_ICONTEXTCALLBACK, IID_IAGILEOBJECT].contains(&iid) {
        return result(write_pointer(out, this));
    }
    let _ = write_pointer(out, 0);
    hr::E_NOINTERFACE
}
extern "system" fn c_add_ref(_this: usize) -> u32 {
    2
}
extern "system" fn c_release(_this: usize) -> u32 {
    1
}
/// ContextCallback(pfnCallback, ComCallData*, riid, iMethod, pUnk): the
/// callback's HRESULT.
extern "system" fn c_context_callback(
    _this: usize,
    callback: usize,
    data: usize,
    _iid: usize,
    _method: i32,
    _unknown: usize,
) -> u32 {
    if callback == 0 {
        return hr::E_INVALIDARG;
    }
    // SAFETY: the caller passes a PFNCONTEXTCALL.
    let f: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(callback) };
    f(data)
}
/// CoGetContextToken(ULONG_PTR*): the current context's identity.
#[unsafe(no_mangle)]
pub extern "system" fn CoGetContextToken(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return hr::E_POINTER;
    }
    match context() {
        Ok(c) => result(write_pointer(out, c)),
        Err(e) => e,
    }
}
/// CoGetObjectContext(riid, ppv): the current context as `riid`.
#[unsafe(no_mangle)]
pub extern "system" fn CoGetObjectContext(iid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return hr::E_POINTER;
    }
    match context() {
        Ok(c) => c_query_interface(
            door_windows_abi::narrow(c),
            iid,
            door_windows_abi::narrow(out),
        ),
        Err(e) => {
            let _ = write_pointer(out, 0);
            e
        }
    }
}

// ---- the free-threaded marshaler ---------------------------------------------------------
#[repr(C)]
struct InnerVtbl {
    query_interface: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
}
#[repr(C)]
struct MarshalVtbl {
    query_interface: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    get_unmarshal_class: extern "system" fn(usize, usize, usize, u32, usize, u32, usize) -> u32,
    get_marshal_size_max: extern "system" fn(usize, usize, usize, u32, usize, u32, usize) -> u32,
    marshal_interface: extern "system" fn(usize, usize, usize, usize, u32, usize, u32) -> u32,
    unmarshal_interface: extern "system" fn(usize, usize, usize, usize) -> u32,
    release_marshal_data: extern "system" fn(usize, usize) -> u32,
    disconnect_object: extern "system" fn(usize, u32) -> u32,
}
static INNER_VTBL: InnerVtbl = InnerVtbl {
    query_interface: f_inner_query_interface,
    add_ref: f_inner_add_ref,
    release: f_inner_release,
};
static MARSHAL_VTBL: MarshalVtbl = MarshalVtbl {
    query_interface: f_query_interface,
    add_ref: f_add_ref,
    release: f_release,
    get_unmarshal_class: f_not_implemented6,
    get_marshal_size_max: f_not_implemented6,
    marshal_interface: f_marshal_interface,
    unmarshal_interface: f_unmarshal_interface,
    release_marshal_data: f_release_marshal_data,
    disconnect_object: f_disconnect_object,
};
/// The marshaler: its non-delegating IUnknown, its IMarshal (delegating
/// to `outer`, itself when not aggregated) and a count.
#[repr(C)]
struct FreeThreaded {
    inner: &'static InnerVtbl,
    marshal: &'static MarshalVtbl,
    outer: u64,
    references: AtomicU32,
}
const MARSHAL_OFFSET: usize = core::mem::size_of::<usize>();
fn from_marshal(this: u64) -> u64 {
    this - MARSHAL_OFFSET as u64
}
#[unsafe(no_mangle)]
pub extern "system" fn CoCreateFreeThreadedMarshaler(outer: usize, out: usize) -> u32 {
    let outer = door_windows_abi::widen(outer);
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return hr::E_INVALIDARG;
    }
    let object = Box::into_raw(Box::new(FreeThreaded {
        inner: &INNER_VTBL,
        marshal: &MARSHAL_VTBL,
        outer: 0,
        references: AtomicU32::new(1),
    })) as u64;
    // SAFETY: just allocated; not yet shared.
    unsafe {
        (*(object as *mut FreeThreaded)).outer = if outer == 0 { object } else { outer };
    }
    let r = result(write_pointer(out, object));
    if r != hr::S_OK {
        f_inner_release(door_windows_abi::narrow(object));
    }
    r
}
extern "system" fn f_inner_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    let Ok(iid) = read_guid(iid) else {
        return hr::E_POINTER;
    };
    let pointer = if iid == IID_IUNKNOWN {
        this
    } else if iid == IID_IMARSHAL {
        this + MARSHAL_OFFSET as u64
    } else {
        let _ = write_pointer(out, 0);
        return hr::E_NOINTERFACE;
    };
    f_inner_add_ref(door_windows_abi::narrow(this));
    result(write_pointer(out, pointer))
}
extern "system" fn f_inner_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    // SAFETY: `this` is a live marshaler.
    let object = unsafe { &*(this as *const FreeThreaded) };
    object.references.fetch_add(1, Ordering::Relaxed) + 1
}
extern "system" fn f_inner_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    // SAFETY: `this` is a live marshaler.
    let left = unsafe { &*(this as *const FreeThreaded) }
        .references
        .fetch_sub(1, Ordering::AcqRel)
        - 1;
    if left == 0 {
        // SAFETY: the last reference; allocated by Box above.
        drop(unsafe { Box::from_raw(this as *mut FreeThreaded) });
    }
    left
}
fn outer_of(this: usize) -> u64 {
    let object = from_marshal(door_windows_abi::widen(this));
    // SAFETY: `this` is the IMarshal of a live marshaler.
    unsafe { (*(object as *const FreeThreaded)).outer }
}
extern "system" fn f_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let outer = outer_of(this);
    let iid = door_windows_abi::widen(iid);
    let Ok(iid) = read_guid(iid) else {
        return hr::E_POINTER;
    };
    query_interface(outer, &iid, door_windows_abi::widen(out))
}
extern "system" fn f_add_ref(this: usize) -> u32 {
    add_ref(outer_of(this))
}
extern "system" fn f_release(this: usize) -> u32 {
    release(outer_of(this))
}
extern "system" fn f_not_implemented6(
    _this: usize,
    _iid: usize,
    _object: usize,
    _context: u32,
    _reserved: usize,
    _flags: u32,
    _out: usize,
) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn f_marshal_interface(
    _this: usize,
    _stream: usize,
    _iid: usize,
    _object: usize,
    _context: u32,
    _reserved: usize,
    _flags: u32,
) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn f_unmarshal_interface(
    _this: usize,
    _stream: usize,
    _iid: usize,
    _out: usize,
) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn f_release_marshal_data(_this: usize, _stream: usize) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn f_disconnect_object(_this: usize, _reserved: u32) -> u32 {
    hr::S_OK
}

// ---- initialize spies ------------------------------------------------------------------
struct Spy {
    thread: u32,
    cookie: u64,
    spy: u64,
}
struct Spies {
    list: Vec<Spy>,
    next: u64,
}
static SPIES: Lock<Spies> = Lock::new(Spies {
    list: Vec::new(),
    next: 1,
});
/// CoRegisterInitializeSpy(IInitializeSpy*, ULARGE_INTEGER*): notified of
/// the calling thread's CoInitialize(Ex) and CoUninitialize calls.
#[unsafe(no_mangle)]
pub extern "system" fn CoRegisterInitializeSpy(spy: usize, cookie: usize) -> u32 {
    let spy = door_windows_abi::widen(spy);
    let cookie = door_windows_abi::widen(cookie);
    if spy == 0 || cookie == 0 {
        return hr::E_INVALIDARG;
    }
    let mut held = 0u64;
    let r = query_interface(spy, &IID_IINITIALIZESPY, &mut held as *mut u64 as u64);
    if r != hr::S_OK {
        return r;
    }
    // A 32-bit program's pointer is the low half of `held`.
    let held = held & if POINTER == 4 { 0xFFFF_FFFF } else { u64::MAX };
    let id = me();
    let value = SPIES.with(|s| {
        let value = s.next;
        s.next += 1;
        s.list.push(Spy {
            thread: id,
            cookie: value,
            spy: held,
        });
        value
    });
    if let Err(e) = write_u64(cookie, value) {
        let _ = CoRevokeInitializeSpy(value);
        return e;
    }
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn CoRevokeInitializeSpy(cookie: u64) -> u32 {
    let id = me();
    let removed = SPIES.with(|s| {
        let at = s
            .list
            .iter()
            .position(|spy| spy.cookie == cookie && spy.thread == id)?;
        Some(s.list.remove(at).spy)
    });
    match removed {
        Some(spy) => {
            release(spy);
            hr::S_OK
        }
        None => hr::E_INVALIDARG,
    }
}
/// The calling thread's spies, each held while it is called.
fn thread_spies() -> Vec<u64> {
    let id = me();
    let spies: Vec<u64> = SPIES.with(|s| {
        s.list
            .iter()
            .filter(|spy| spy.thread == id)
            .map(|spy| spy.spy)
            .collect()
    });
    for &spy in &spies {
        add_ref(spy);
    }
    spies
}
fn notify(spies: &[u64], index: usize, call: impl Fn(u64, u64) -> u32) {
    for &spy in spies {
        // SAFETY: a held IInitializeSpy.
        call(spy, unsafe { method(spy, index) });
    }
}
/// IInitializeSpy::PreInitialize(dwCoInit, dwCurThreadAptRefs).
pub(super) fn pre_initialize(flags: u32, references: u32) -> Vec<u64> {
    let spies = thread_spies();
    notify(&spies, 3, |spy, f| {
        let f: extern "system" fn(usize, u32, u32) -> u32 =
            unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(spy), flags, references)
    });
    spies
}
/// IInitializeSpy::PostInitialize(hrCoInit, dwCoInit, dwNewThreadAptRefs):
/// each spy may change the result.
pub(super) fn post_initialize(spies: Vec<u64>, mut hr: u32, flags: u32, references: u32) -> u32 {
    for &spy in &spies {
        // SAFETY: a held IInitializeSpy.
        let f: extern "system" fn(usize, u32, u32, u32) -> u32 =
            unsafe { core::mem::transmute(method(spy, 4) as usize) };
        hr = f(door_windows_abi::narrow(spy), hr, flags, references);
        release(spy);
    }
    hr
}
/// IInitializeSpy::PreUninitialize(dwCurThreadAptRefs).
pub(super) fn pre_uninitialize(references: u32) -> Vec<u64> {
    let spies = thread_spies();
    notify(&spies, 5, |spy, f| {
        let f: extern "system" fn(usize, u32) -> u32 = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(spy), references)
    });
    spies
}
/// IInitializeSpy::PostUninitialize(dwNewThreadAptRefs).
pub(super) fn post_uninitialize(spies: Vec<u64>, references: u32) {
    notify(&spies, 6, |spy, f| {
        let f: extern "system" fn(usize, u32) -> u32 = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(spy), references)
    });
    for spy in spies {
        release(spy);
    }
}
