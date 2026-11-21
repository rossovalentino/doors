//! MMDevApi.dll's COM objects: the class factory, MMDeviceEnumerator,
//! endpoint collections, the two endpoints (IMMDevice with IMMEndpoint) and
//! their read-only property stores. What an endpoint activates is in
//! `client` (IAudioClient and its services, IAudioEndpointVolume, the
//! worker that feeds door.audio and watches the device).
use crate::*;
use alloc::{boxed::Box, string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

mod activate;
mod client;
use door_service_client::audio::Audio;

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
        // SAFETY: `held` gives this closure exclusive access.
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
    fn DoorReserveSectionRange(bytes: u64) -> u64;
    fn DoorReleaseSectionRange(address: u64, bytes: u64) -> i32;
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorGetTickCount64() -> u64;
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, size: u32) -> u32;
}
#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> usize;
    fn CoTaskMemFree(pointer: usize);
}

// ---- application memory --------------------------------------------------
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
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
            return Err(hr::E_POINTER);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
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
            return Err(hr::E_POINTER);
        }
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn write_u32(pointer: u64, value: u32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
fn write_u64(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
/// The program's pointer width (P17: 4 for a 32-bit program).
pub(crate) const POINTER: usize = door_windows_abi::layout::POINTER;
/// A pointer-sized program value, sign-extended on x86.
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
/// An interface, buffer or task-memory pointer into an out-parameter.
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
fn write_f32(pointer: u64, value: f32) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes())
}
fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(hr::S_OK)
}
/// A copy in COM task memory (the caller frees it with CoTaskMemFree).
fn task_copy(data: &[u8]) -> Result<u64, u32> {
    let p = unsafe { CoTaskMemAlloc(data.len().max(1)) };
    if p == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    if let Err(e) = write(door_windows_abi::widen(p), data) {
        unsafe { CoTaskMemFree(p) };
        return Err(e);
    }
    Ok(door_windows_abi::widen(p))
}
/// A NUL-terminated wide string in COM task memory.
fn task_wide(text: &str) -> Result<u64, u32> {
    let bytes: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    task_copy(&bytes)
}
/// The wide string at `p` (at most 512 units).
fn read_wide(p: u64) -> Result<String, u32> {
    let mut units = Vec::new();
    for i in 0..512u64 {
        let mut b = [0u8; 2];
        read(p + 2 * i, &mut b)?;
        let u = u16::from_le_bytes(b);
        if u == 0 {
            return Ok(String::from_utf16_lossy(&units));
        }
        units.push(u);
    }
    Err(hr::E_INVALIDARG)
}
fn now() -> u64 {
    unsafe { DoorGetTickCount64() }
}

// ---- GUIDs ---------------------------------------------------------------
type Guid = [u8; 16];
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}
fn read_guid(pointer: u64) -> Result<Guid, u32> {
    let mut g = [0u8; 16];
    read(pointer, &mut g)?;
    Ok(g)
}
const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ICLASSFACTORY: Guid = guid(1, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const CLSID_MMDEVICEENUMERATOR: Guid = guid(
    0xBCDE_0395,
    0xE52F,
    0x467C,
    [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
);
const IID_IMMDEVICEENUMERATOR: Guid = guid(
    0xA956_64D2,
    0x9614,
    0x4F35,
    [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
);
const IID_IMMDEVICECOLLECTION: Guid = guid(
    0x0BD7_A1BE,
    0x7A1A,
    0x44DB,
    [0x83, 0x97, 0xCC, 0x53, 0x92, 0x38, 0x7B, 0x5E],
);
const IID_IMMDEVICE: Guid = guid(
    0xD666_063F,
    0x1587,
    0x4E43,
    [0x81, 0xF1, 0xB9, 0x48, 0xE8, 0x07, 0x36, 0x3F],
);
const IID_IMMENDPOINT: Guid = guid(
    0x1BE0_9788,
    0x6894,
    0x4089,
    [0x85, 0x86, 0x9A, 0x2A, 0x6C, 0x26, 0x5A, 0xC5],
);
const IID_IPROPERTYSTORE: Guid = guid(
    0x886D_8EEB,
    0x8CF2,
    0x4446,
    [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99],
);
/// The Doors speaker's and microphone's endpoint GUIDs (dsound's device
/// GUIDs).
const SPEAKER: Guid = guid(
    0xD002_5D01,
    0x5EA4,
    0x4A6B,
    [0x9C, 0x11, 0x2B, 0x6E, 0x15, 0x50, 0x00, 0x01],
);
const MICROPHONE: Guid = guid(
    0xD002_5D01,
    0x5EA4,
    0x4A6B,
    [0x9C, 0x11, 0x2B, 0x6E, 0x15, 0x50, 0x00, 0x02],
);

// ---- door.audio ------------------------------------------------------------
static AUDIO: Lock<Option<Audio>> = Lock::new(None);
fn audio<R>(f: impl FnOnce(&mut Audio) -> R) -> Option<R> {
    AUDIO.with(|a| {
        if a.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *a = Audio::connect(registry).ok();
        }
        a.as_mut().map(f)
    })
}

// ---- endpoints -------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Endpoint {
    Render,
    Capture,
}
impl Endpoint {
    fn guid(self) -> &'static Guid {
        match self {
            Self::Render => &SPEAKER,
            Self::Capture => &MICROPHONE,
        }
    }
    fn id(self) -> String {
        endpoint_id(self == Self::Capture, self.guid())
            .iter()
            .map(|&b| b as char)
            .collect()
    }
    fn flow(self) -> u32 {
        self as u32
    }
    fn name(self) -> &'static str {
        match self {
            Self::Render => "Speakers (VirtIO Sound)",
            Self::Capture => "Microphone (VirtIO Sound)",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Render => "Speakers",
            Self::Capture => "Microphone",
        }
    }
    /// door.audio has the device (and, for the microphone, an input).
    fn present(self) -> bool {
        audio(|a| a.status().ok())
            .flatten()
            .is_some_and(|s| s.present && (self == Self::Render || s.capture))
    }
    fn of_id(id: &str) -> Option<Self> {
        [Self::Render, Self::Capture]
            .into_iter()
            .find(|e| e.id().eq_ignore_ascii_case(id))
    }
}
/// EDataFlow: eRender 0, eCapture 1, eAll 2.
fn flows(flow: u32) -> Result<&'static [Endpoint], u32> {
    match flow {
        0 => Ok(&[Endpoint::Render]),
        1 => Ok(&[Endpoint::Capture]),
        2 => Ok(&[Endpoint::Render, Endpoint::Capture]),
        _ => Err(hr::E_INVALIDARG),
    }
}

// ---- objects -----------------------------------------------------------------
/// Every object: its interface table(s), references and what it is. A
/// device's IMMEndpoint is the second table, 8 bytes in.
#[repr(C)]
struct Object {
    vtbl: u64,
    second: u64,
    refs: AtomicU32,
    data: Data,
}
enum Data {
    Enumerator,
    Collection(Vec<Endpoint>),
    Device(Endpoint),
    Store(Endpoint),
    /// IAudioClient: its key in `client`'s table.
    Client(u32),
    /// A service of a client: the client object it holds, and its key.
    Service(u64, u32),
    Volume(Endpoint),
    /// An ActivateAudioInterfaceAsync operation: its result and interface.
    Operation(u32, u64),
}
fn new_object(vtbl: u64, second: u64, data: Data) -> u64 {
    Box::into_raw(Box::new(Object {
        vtbl,
        second,
        refs: AtomicU32::new(1),
        data,
    })) as u64
}
fn object(this: u64) -> &'static Object {
    // SAFETY: only our objects reach our tables.
    unsafe { &*(this as *const Object) }
}
fn add_ref(this: u64) -> u32 {
    object(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
fn release(this: u64) -> u32 {
    let left = object(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference; the object came from Box::into_raw.
        let o = unsafe { Box::from_raw(this as *mut Object) };
        match o.data {
            Data::Client(id) => client::drop_client(id),
            Data::Service(parent, _) => {
                release(parent);
            }
            Data::Operation(_, iface) if iface != 0 => {
                activate::com_call1(iface, 2);
            }
            _ => {}
        }
    }
    left
}
/// QueryInterface for an object whose interfaces are `iids` (the first
/// table) and `second` (the second table).
fn query(this: u64, iid: u64, out: u64, iids: &[Guid], second: &[Guid]) -> u32 {
    let _ = write_pointer(out, 0);
    let Ok(g) = read_guid(iid) else {
        return hr::E_POINTER;
    };
    if out == 0 {
        return hr::E_POINTER;
    }
    let pointer = if g == IID_IUNKNOWN || iids.contains(&g) {
        this
    } else if second.contains(&g) {
        this + 8
    } else {
        return hr::E_NOINTERFACE;
    };
    add_ref(this);
    result(write_pointer(out, pointer))
}
type Fn1 = extern "system" fn(usize) -> u32;
type Fn2 = extern "system" fn(usize, usize) -> u32;
type Fn3 = extern "system" fn(usize, usize, usize) -> u32;
type Fn4 = extern "system" fn(usize, usize, usize, usize) -> u32;
type Fn5 = extern "system" fn(usize, usize, usize, usize, usize) -> u32;
extern "system" fn obj_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    add_ref(this)
}
extern "system" fn obj_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    release(this)
}

// ---- the class factory -----------------------------------------------------------
#[repr(C)]
struct FactoryVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    create: Fn4,
    lock: Fn2,
}
static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
    query: f_query,
    add_ref: f_count,
    release: f_count,
    create: f_create,
    lock: f_lock,
};
#[repr(C)]
struct Factory {
    vtbl: &'static FactoryVtbl,
}
static FACTORY: Factory = Factory {
    vtbl: &FACTORY_VTBL,
};
static HELD: AtomicU32 = AtomicU32::new(0);
extern "system" fn f_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        match read_guid(iid) {
            Ok(g) if g == IID_IUNKNOWN || g == IID_ICLASSFACTORY => {
                HELD.fetch_add(1, Ordering::AcqRel);
                result(write_pointer(out, this))
            }
            Ok(_) => hr::E_NOINTERFACE,
            Err(e) => e,
        }
    }
}
extern "system" fn f_count(_this: usize) -> u32 {
    1
}
extern "system" fn f_create(_this: usize, outer: usize, iid: usize, out: usize) -> u32 {
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        if outer != 0 {
            return hr::CLASS_E_NOAGGREGATION;
        }
        let e = new_object(&ENUMERATOR_VTBL as *const _ as u64, 0, Data::Enumerator);
        let r = enumerator_query(
            door_windows_abi::narrow(e),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        );
        release(e);
        r
    }
}
extern "system" fn f_lock(_this: usize, lock: usize) -> u32 {
    let lock = door_windows_abi::widen(lock);
    {
        if lock as i32 != 0 {
            HELD.fetch_add(1, Ordering::AcqRel);
        } else {
            HELD.fetch_sub(1, Ordering::AcqRel);
        }
        hr::S_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(clsid: usize, iid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        match read_guid(clsid) {
            Ok(g) if g == CLSID_MMDEVICEENUMERATOR => f_query(
                door_windows_abi::narrow(&FACTORY as *const Factory as u64),
                door_windows_abi::narrow(iid),
                door_windows_abi::narrow(out),
            ),
            Ok(_) => hr::CLASS_E_CLASSNOTAVAILABLE,
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> u32 {
    // Objects and the worker live in this DLL for the process's life.
    hr::S_FALSE
}

// ---- IMMDeviceEnumerator --------------------------------------------------------------
#[repr(C)]
struct EnumeratorVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    enum_endpoints: Fn4,
    default_endpoint: Fn4,
    get_device: Fn3,
    register: Fn2,
    unregister: Fn2,
}
static ENUMERATOR_VTBL: EnumeratorVtbl = EnumeratorVtbl {
    query: enumerator_query,
    add_ref: obj_add_ref,
    release: obj_release,
    enum_endpoints,
    default_endpoint,
    get_device,
    register: client::register_notification,
    unregister: client::unregister_notification,
};
extern "system" fn enumerator_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IMMDEVICEENUMERATOR], &[])
}
fn new_device(e: Endpoint) -> u64 {
    new_object(
        &DEVICE_VTBL as *const _ as u64,
        &ENDPOINT_VTBL as *const _ as u64,
        Data::Device(e),
    )
}
extern "system" fn enum_endpoints(_this: usize, flow: usize, mask: usize, out: usize) -> u32 {
    let flow = door_windows_abi::widen(flow);
    let mask = door_windows_abi::widen(mask);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let mask = mask as u32;
        let endpoints = match flows(flow as u32) {
            Ok(e) => e,
            Err(e) => return e,
        };
        if mask == 0 || mask & !DEVICE_STATEMASK_ALL != 0 {
            return hr::E_INVALIDARG;
        }
        let list = endpoints
            .iter()
            .copied()
            .filter(|e| {
                let state = if e.present() {
                    DEVICE_STATE_ACTIVE
                } else {
                    DEVICE_STATE_NOTPRESENT
                };
                mask & state != 0
            })
            .collect();
        let c = new_object(
            &COLLECTION_VTBL as *const _ as u64,
            0,
            Data::Collection(list),
        );
        result(write_pointer(out, c))
    }
}
extern "system" fn default_endpoint(_this: usize, flow: usize, role: usize, out: usize) -> u32 {
    let flow = door_windows_abi::widen(flow);
    let role = door_windows_abi::widen(role);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        // eRender or eCapture; eConsole, eMultimedia or eCommunications.
        let e = match flow as u32 {
            0 => Endpoint::Render,
            1 => Endpoint::Capture,
            _ => return hr::E_INVALIDARG,
        };
        if role as u32 > 2 {
            return hr::E_INVALIDARG;
        }
        if !e.present() {
            return hr::E_NOTFOUND;
        }
        result(write_pointer(out, new_device(e)))
    }
}
extern "system" fn get_device(_this: usize, id: usize, out: usize) -> u32 {
    let id = door_windows_abi::widen(id);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 || id == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let Ok(text) = read_wide(id) else {
            return hr::E_INVALIDARG;
        };
        match Endpoint::of_id(&text) {
            Some(e) => result(write_pointer(out, new_device(e))),
            None => hr::E_NOTFOUND,
        }
    }
}

// ---- IMMDeviceCollection ----------------------------------------------------------------
#[repr(C)]
struct CollectionVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    count: Fn2,
    item: Fn3,
}
static COLLECTION_VTBL: CollectionVtbl = CollectionVtbl {
    query: collection_query,
    add_ref: obj_add_ref,
    release: obj_release,
    count: collection_count,
    item: collection_item,
};
extern "system" fn collection_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IMMDEVICECOLLECTION], &[])
}
fn collection(this: u64) -> &'static [Endpoint] {
    match &object(this).data {
        Data::Collection(list) => list,
        _ => &[],
    }
}
extern "system" fn collection_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, collection(this).len() as u32))
}
extern "system" fn collection_item(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        match collection(this).get(index as u32 as usize) {
            Some(&e) => result(write_pointer(out, new_device(e))),
            None => hr::E_INVALIDARG,
        }
    }
}

// ---- IMMDevice and IMMEndpoint -------------------------------------------------------------
#[repr(C)]
struct DeviceVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    activate: Fn5,
    open_store: Fn3,
    get_id: Fn2,
    get_state: Fn2,
}
static DEVICE_VTBL: DeviceVtbl = DeviceVtbl {
    query: device_query,
    add_ref: obj_add_ref,
    release: obj_release,
    activate: client::activate,
    open_store,
    get_id: device_id,
    get_state: device_state,
};
#[repr(C)]
struct EndpointVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    data_flow: Fn2,
}
static ENDPOINT_VTBL: EndpointVtbl = EndpointVtbl {
    query: endpoint_query,
    add_ref: endpoint_add_ref,
    release: endpoint_release,
    data_flow: endpoint_flow,
};
extern "system" fn device_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IMMDEVICE], &[IID_IMMENDPOINT])
}
extern "system" fn endpoint_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        device_query(
            door_windows_abi::narrow(this - 8),
            door_windows_abi::narrow(iid),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn endpoint_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    add_ref(this - 8)
}
extern "system" fn endpoint_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    release(this - 8)
}
fn device(this: u64) -> Endpoint {
    match object(this).data {
        Data::Device(e) | Data::Store(e) | Data::Volume(e) => e,
        _ => Endpoint::Render,
    }
}
extern "system" fn endpoint_flow(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, device(this - 8).flow()))
}
extern "system" fn device_id(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match task_wide(&device(this).id()) {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
extern "system" fn device_state(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let state = if device(this).present() {
            DEVICE_STATE_ACTIVE
        } else {
            DEVICE_STATE_NOTPRESENT
        };
        result(write_u32(out, state))
    }
}

// ---- IPropertyStore -----------------------------------------------------------------------------
#[repr(C)]
struct StoreVtbl {
    query: Fn3,
    add_ref: Fn1,
    release: Fn1,
    count: Fn2,
    get_at: Fn3,
    get_value: Fn3,
    set_value: Fn3,
    commit: Fn1,
}
static STORE_VTBL: StoreVtbl = StoreVtbl {
    query: store_query,
    add_ref: obj_add_ref,
    release: obj_release,
    count: store_count,
    get_at: store_get_at,
    get_value: store_get_value,
    set_value: store_set_value,
    commit: store_commit,
};
/// PROPERTYKEYs: (fmtid, pid).
const DEVICE_KEYS: Guid = guid(
    0xA45C_254E,
    0xDF1C,
    0x4EFD,
    [0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0],
);
const INTERFACE_KEYS: Guid = guid(
    0x026E_516E,
    0xB814,
    0x414B,
    [0x83, 0xCD, 0x85, 0x6D, 0x6F, 0xEF, 0x48, 0x22],
);
const ENDPOINT_KEYS: Guid = guid(
    0x1DA5_D803,
    0xD492,
    0x4EDD,
    [0x8C, 0x23, 0xE0, 0xC0, 0xFF, 0xEE, 0x7F, 0x0E],
);
const ENGINE_KEYS: Guid = guid(
    0xF19F_064D,
    0x082C,
    0x4E27,
    [0xBC, 0x73, 0x68, 0x82, 0xA1, 0xBB, 0x8E, 0x4C],
);
const OEM_KEYS: Guid = guid(
    0xE487_0E26,
    0x3CC5,
    0x4CD2,
    [0xBA, 0x46, 0xCA, 0x0A, 0x9A, 0x70, 0xED, 0x04],
);
const KEYS: [(Guid, u32); 7] = [
    (DEVICE_KEYS, 14),   // PKEY_Device_FriendlyName
    (DEVICE_KEYS, 2),    // PKEY_Device_DeviceDesc
    (INTERFACE_KEYS, 2), // PKEY_DeviceInterface_FriendlyName
    (ENDPOINT_KEYS, 0),  // PKEY_AudioEndpoint_FormFactor
    (ENDPOINT_KEYS, 4),  // PKEY_AudioEndpoint_GUID
    (ENGINE_KEYS, 0),    // PKEY_AudioEngine_DeviceFormat
    (OEM_KEYS, 3),       // PKEY_AudioEngine_OEMFormat
];
const VT_UI4: u16 = 19;
const VT_LPWSTR: u16 = 31;
const VT_BLOB: u16 = 65;
extern "system" fn open_store(this: usize, mode: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let mode = door_windows_abi::widen(mode);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        // STGM_READ, STGM_WRITE or STGM_READWRITE.
        if mode as u32 > 2 {
            return hr::E_INVALIDARG;
        }
        let s = new_object(&STORE_VTBL as *const _ as u64, 0, Data::Store(device(this)));
        result(write_pointer(out, s))
    }
}
extern "system" fn store_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out, &[IID_IPROPERTYSTORE], &[])
}
extern "system" fn store_count(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_u32(out, KEYS.len() as u32))
}
extern "system" fn store_get_at(_this: usize, index: usize, out: usize) -> u32 {
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        let Some((fmtid, pid)) = KEYS.get(index as u32 as usize) else {
            return hr::E_INVALIDARG;
        };
        let mut key = [0u8; 20];
        key[..16].copy_from_slice(fmtid);
        key[16..].copy_from_slice(&pid.to_le_bytes());
        result(write(out, &key))
    }
}
/// GetValue: a PROPVARIANT (24 bytes on x64, 16 on x86) of the endpoint's
/// property; an
/// unknown key is VT_EMPTY, as Windows answers.
extern "system" fn store_get_value(this: usize, key: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    {
        let e = device(this);
        let r = (|| -> Result<(), u32> {
            let fmtid = read_guid(key)?;
            let pid = read_u32(key + 16)?;
            let mut v = [0u8; 24];
            let mut text = |s: &str| -> Result<(), u32> {
                v[..2].copy_from_slice(&VT_LPWSTR.to_le_bytes());
                v[8..8 + POINTER].copy_from_slice(&task_wide(s)?.to_le_bytes()[..POINTER]);
                Ok(())
            };
            match (fmtid, pid) {
                (DEVICE_KEYS, 14) => text(e.name())?,
                (DEVICE_KEYS, 2) => text(e.description())?,
                (INTERFACE_KEYS, 2) => text("VirtIO Sound")?,
                (ENDPOINT_KEYS, 4) => {
                    let g: String = guid_text(e.guid()).iter().map(|&b| b as char).collect();
                    text(&alloc::format!("{{{g}}}"))?
                }
                (ENDPOINT_KEYS, 0) => {
                    // FormFactor: Speakers 1, Microphone 4.
                    v[..2].copy_from_slice(&VT_UI4.to_le_bytes());
                    let form: u32 = if e == Endpoint::Render { 1 } else { 4 };
                    v[8..12].copy_from_slice(&form.to_le_bytes());
                }
                (ENGINE_KEYS, 0) | (OEM_KEYS, 3) => {
                    let f = device_format();
                    v[..2].copy_from_slice(&VT_BLOB.to_le_bytes());
                    v[8..12].copy_from_slice(&(f.len() as u32).to_le_bytes());
                    // BLOB { cbSize, pBlobData }.
                    v[8 + POINTER..8 + 2 * POINTER]
                        .copy_from_slice(&task_copy(&f)?.to_le_bytes()[..POINTER]);
                }
                _ => {}
            }
            write(out, &v[..8 + 2 * POINTER])
        })();
        result(r)
    }
}
extern "system" fn store_set_value(_this: usize, _key: usize, _value: usize) -> u32 {
    {
        // Endpoint properties are the system's.
        hr::E_ACCESSDENIED
    }
}
extern "system" fn store_commit(_this: usize) -> u32 {
    hr::S_OK
}
