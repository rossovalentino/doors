//! ole32.dll exports. Apartments are per thread (DllMain forgets a thread
//! at DLL_THREAD_DETACH); a thread that never initialized COM runs in the
//! implicit MTA while any thread holds the MTA. In-process activation
//! finds class objects registered with CoRegisterClassObject first, then
//! the server HKEY_CLASSES_ROOT\CLSID\{...}\InprocServer32 names (its
//! DllGetClassObject). Every object lives in this process and its methods
//! are called directly: apartments decide initialization rules, not
//! marshaling, until the broker brings out-of-process servers (M6).
use crate::*;
use alloc::{boxed::Box, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
mod context;
mod remote;
mod transfer;

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
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlReAllocateHeap(heap: usize, flags: u32, pointer: usize, bytes: usize) -> usize;
    fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorGetModuleHandleW(name: u64) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
    fn DoorExpandEnvironmentStringsW(source: u64, destination: u64, size: u32) -> u32;
    fn DoorWaitForMultipleObjectsEx(
        count: u32,
        handles: u64,
        all: i32,
        millis: u32,
        alertable: i32,
    ) -> u32;
}
#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: usize, sub: usize, options: u32, desired: u32, out: usize) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegCloseKey(key: usize) -> u32;
}
const HKEY_CLASSES_ROOT: u64 = 0x8000_0000;
const KEY_READ: u32 = 0x2_0019;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;

// ---- application memory ------------------------------------------------------------
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
fn read_u16(pointer: u64) -> Result<u16, u32> {
    let mut b = [0u8; 2];
    read(pointer, &mut b)?;
    Ok(u16::from_le_bytes(b))
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
/// A pointer-sized program value (interface pointers, task memory),
/// sign-extended on x86.
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(door_windows_abi::layout::get(
        &b,
        &[door_windows_abi::layout::Field::Pointer],
        0,
    ))
}
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
/// PROPVARIANT: vt and three reserved WORDs, then its union at 8 (a
/// pointer, or BLOB/CA* { count, pointer }): 24 bytes on x64, 16 on x86.
const PROPVARIANT_BYTES: usize = 8 + 2 * POINTER;
fn read_guid(pointer: u64) -> Result<Guid, u32> {
    let mut g = [0u8; 16];
    read(pointer, &mut g)?;
    Ok(g)
}
/// A NUL-terminated UTF-16 string (at most `max` units).
fn read_wide(pointer: u64, max: usize) -> Result<Vec<u16>, u32> {
    if pointer == 0 {
        return Err(hr::E_POINTER);
    }
    let mut out = Vec::new();
    while out.len() < max {
        let unit = read_u16(pointer + 2 * out.len() as u64)?;
        if unit == 0 {
            return Ok(out);
        }
        out.push(unit);
    }
    Err(hr::E_INVALIDARG)
}
fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(hr::S_OK)
}

// ---- calling COM objects -------------------------------------------------------
/// The `index`th method of `object`'s first interface table.
///
/// SAFETY: `object` is a live COM interface pointer in this process.
/// Method `index` of a COM object: its first field is the vtable, a table
/// of pointer-sized entries (P17: 4 bytes in a 32-bit process).
unsafe fn method(object: u64, index: usize) -> u64 {
    const POINTER: usize = core::mem::size_of::<usize>();
    unsafe { *((*(object as *const usize) + index * POINTER) as *const usize) as u64 }
}
fn add_ref(object: u64) -> u32 {
    // SAFETY: callers pass interface pointers they hold.
    let f: extern "system" fn(usize) -> u32 =
        unsafe { core::mem::transmute(method(object, 1) as usize) };
    f(door_windows_abi::narrow(object))
}
fn release(object: u64) -> u32 {
    // SAFETY: as add_ref.
    let f: extern "system" fn(usize) -> u32 =
        unsafe { core::mem::transmute(method(object, 2) as usize) };
    f(door_windows_abi::narrow(object))
}
fn query_interface(object: u64, iid: &Guid, out: u64) -> u32 {
    // SAFETY: as add_ref.
    let f: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(object, 0) as usize) };
    f(
        door_windows_abi::narrow(object),
        door_windows_abi::narrow(iid.as_ptr() as u64),
        door_windows_abi::narrow(out),
    )
}

// ---- the task allocator -----------------------------------------------------------
fn heap() -> u64 {
    unsafe { DoorGetProcessHeap() }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoTaskMemAlloc(bytes: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        // A zero-byte request still answers a distinct block, as Windows does.
        unsafe {
            door_windows_abi::widen(RtlAllocateHeap(
                door_windows_abi::narrow(heap()),
                0,
                bytes.max(1),
            ))
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CoTaskMemRealloc(block: usize, bytes: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 {
        if block == 0 {
            return door_windows_abi::widen(CoTaskMemAlloc(bytes));
        }
        if bytes == 0 {
            CoTaskMemFree(door_windows_abi::narrow(block));
            return 0;
        }
        unsafe {
            door_windows_abi::widen(RtlReAllocateHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(block),
                bytes,
            ))
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CoTaskMemFree(block: usize) {
    let block = door_windows_abi::widen(block);
    {
        if block != 0 {
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(block),
                )
            };
        }
    }
}
/// A copy of `units` plus a NUL in task memory.
fn task_string(units: &[u16]) -> Result<u64, u32> {
    let block = CoTaskMemAlloc((units.len() + 1) * 2);
    if block == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    let bytes: Vec<u8> = units
        .iter()
        .chain([&0])
        .flat_map(|u| u.to_le_bytes())
        .collect();
    write(door_windows_abi::widen(block), &bytes).inspect_err(|_| CoTaskMemFree(block))?;
    Ok(door_windows_abi::widen(block))
}

#[repr(C)]
struct MallocVtbl {
    query_interface: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    alloc: extern "system" fn(usize, usize) -> usize,
    realloc: extern "system" fn(usize, usize, usize) -> usize,
    free: extern "system" fn(usize, usize),
    get_size: extern "system" fn(usize, usize) -> usize,
    did_alloc: extern "system" fn(usize, usize) -> i32,
    heap_minimize: extern "system" fn(usize),
}
static MALLOC_VTBL: MallocVtbl = MallocVtbl {
    query_interface: m_query_interface,
    add_ref: m_add_ref,
    release: m_release,
    alloc: m_alloc,
    realloc: m_realloc,
    free: m_free,
    get_size: m_get_size,
    did_alloc: m_did_alloc,
    heap_minimize: m_heap_minimize,
};
/// The process's one IMalloc (its "object" is the table pointer).
static MALLOC: &MallocVtbl = &MALLOC_VTBL;
fn malloc_object() -> u64 {
    &MALLOC as *const &MallocVtbl as u64
}
extern "system" fn m_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(iid) = read_guid(iid) else {
            return hr::E_POINTER;
        };
        if iid == IID_IUNKNOWN || iid == IID_IMALLOC {
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn m_add_ref(_this: usize) -> u32 {
    2
}
extern "system" fn m_release(_this: usize) -> u32 {
    1
}
extern "system" fn m_alloc(_this: usize, bytes: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(CoTaskMemAlloc(bytes))
    })())
}
extern "system" fn m_realloc(_this: usize, block: usize, bytes: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 {
        door_windows_abi::widen(CoTaskMemRealloc(door_windows_abi::narrow(block), bytes))
    })())
}
extern "system" fn m_free(_this: usize, block: usize) {
    let block = door_windows_abi::widen(block);
    CoTaskMemFree(door_windows_abi::narrow(block))
}
extern "system" fn m_get_size(_this: usize, block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    {
        if block == 0 {
            usize::MAX
        } else {
            unsafe {
                RtlSizeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(block),
                )
            }
        }
    }
}
extern "system" fn m_did_alloc(_this: usize, block: usize) -> i32 {
    let block = door_windows_abi::widen(block);
    {
        if block == 0 {
            return -1;
        }
        i32::from(
            unsafe {
                RtlSizeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(block),
                )
            } != usize::MAX,
        )
    }
}
extern "system" fn m_heap_minimize(_this: usize) {
    {}
}
#[unsafe(no_mangle)]
pub extern "system" fn CoGetMalloc(context: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        // MEMCTX_TASK (1) is the only context.
        if context != 1 {
            return hr::E_INVALIDARG;
        }
        result(write_pointer(out, malloc_object()))
    }
}

// ---- apartments --------------------------------------------------------------------
struct Threads {
    list: Vec<(u32, ThreadInit)>,
    /// Threads in the MTA now.
    mta: u32,
    /// The first thread that entered an STA (the main STA) while it is one.
    main_sta: u32,
}
static THREADS: Lock<Threads> = Lock::new(Threads {
    list: Vec::new(),
    mta: 0,
    main_sta: 0,
});
static SECURITY: AtomicBool = AtomicBool::new(false);
fn me() -> u32 {
    unsafe { DoorGetCurrentThreadId() }
}
/// The calling thread's apartment, the implicit MTA counting as MTA.
fn current() -> Option<(Apartment, bool)> {
    let id = me();
    THREADS.with(|t| {
        match t
            .list
            .iter()
            .find(|(tid, _)| *tid == id)
            .and_then(|(_, i)| i.apartment)
        {
            Some(a) => Some((a, false)),
            None if t.mta > 0 => Some((Apartment::Mta, true)),
            None => None,
        }
    })
}
fn initialized() -> Result<(), u32> {
    current().map(|_| ()).ok_or(hr::CO_E_NOTINITIALIZED)
}
#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeEx(reserved: usize, flags: u32) -> u32 {
    let reserved = door_windows_abi::widen(reserved);
    {
        if reserved != 0 {
            return hr::E_INVALIDARG;
        }
        let id = me();
        let spies = context::pre_initialize(flags, apartment_references(id));
        let r = THREADS.with(|t| {
            let at = match t.list.iter().position(|(tid, _)| *tid == id) {
                Some(at) => at,
                None => {
                    t.list.push((id, ThreadInit::default()));
                    t.list.len() - 1
                }
            };
            let before = t.list[at].1.apartment;
            let r = t.list[at].1.initialize(flags);
            if before.is_none() {
                match t.list[at].1.apartment {
                    Some(Apartment::Mta) => t.mta += 1,
                    Some(Apartment::Sta) if t.main_sta == 0 => t.main_sta = id,
                    _ => (),
                }
            }
            r
        });
        context::post_initialize(spies, r, flags, apartment_references(id))
    }
}
/// The calling thread's CoInitialize(Ex) count (what initialize spies see).
fn apartment_references(id: u32) -> u32 {
    THREADS.with(|t| {
        t.list
            .iter()
            .find(|(tid, _)| *tid == id)
            .map_or(0, |(_, i)| if i.apartment.is_some() { i.count } else { 0 })
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CoInitialize(reserved: usize) -> u32 {
    let reserved = door_windows_abi::widen(reserved);
    {
        CoInitializeEx(
            door_windows_abi::narrow(reserved),
            coinit::APARTMENTTHREADED,
        )
    }
}
fn leave(t: &mut Threads, id: u32, apartment: Option<Apartment>) {
    match apartment {
        Some(Apartment::Mta) => t.mta = t.mta.saturating_sub(1),
        Some(Apartment::Sta) if t.main_sta == id => t.main_sta = 0,
        _ => (),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoUninitialize() {
    let id = me();
    let spies = context::pre_uninitialize(apartment_references(id));
    THREADS.with(|t| {
        let Some(at) = t.list.iter().position(|(tid, _)| *tid == id) else {
            return;
        };
        let apartment = t.list[at].1.apartment;
        if t.list[at].1.uninitialize() {
            leave(t, id, apartment);
        }
    });
    context::post_uninitialize(spies, apartment_references(id));
}
/// `RoInitialize` (combase, api-ms-win-core-winrt-l1-1): the Windows
/// Runtime's apartment is COM's: RO_INIT_SINGLETHREADED (0) an STA,
/// RO_INIT_MULTITHREADED (1) the MTA, with CoInitializeEx's answers.
#[unsafe(no_mangle)]
pub extern "system" fn RoInitialize(kind: u32) -> u32 {
    match kind {
        0 => CoInitializeEx(0, coinit::APARTMENTTHREADED),
        1 => CoInitializeEx(0, coinit::MULTITHREADED),
        _ => hr::E_INVALIDARG,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RoUninitialize() {
    CoUninitialize();
}
/// No Windows Runtime classes are registered: activation answers
/// REGDB_E_CLASSNOTREG, as Windows does for an unknown class.
#[unsafe(no_mangle)]
pub extern "system" fn RoGetActivationFactory(_class: usize, _iid: usize, factory: usize) -> u32 {
    let factory = door_windows_abi::widen(factory);
    if factory == 0 {
        return hr::E_INVALIDARG;
    }
    unsafe { (factory as *mut usize).write_unaligned(0) };
    hr::REGDB_E_CLASSNOTREG
}
#[unsafe(no_mangle)]
pub extern "system" fn RoActivateInstance(_class: usize, instance: usize) -> u32 {
    let instance = door_windows_abi::widen(instance);
    if instance == 0 {
        return hr::E_INVALIDARG;
    }
    unsafe { (instance as *mut usize).write_unaligned(0) };
    hr::REGDB_E_CLASSNOTREG
}
#[unsafe(no_mangle)]
pub extern "system" fn OleInitialize(reserved: usize) -> u32 {
    let reserved = door_windows_abi::widen(reserved);
    {
        let r = CoInitializeEx(
            door_windows_abi::narrow(reserved),
            coinit::APARTMENTTHREADED,
        );
        if r == hr::S_OK || r == hr::S_FALSE {
            let id = me();
            THREADS.with(|t| {
                if let Some((_, i)) = t.list.iter_mut().find(|(tid, _)| *tid == id) {
                    i.ole += 1;
                }
            });
        }
        r
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OleUninitialize() {
    let id = me();
    let was = THREADS.with(|t| {
        t.list
            .iter_mut()
            .find(|(tid, _)| *tid == id)
            .is_some_and(|(_, i)| {
                let was = i.ole > 0;
                i.ole = i.ole.saturating_sub(1);
                was
            })
    });
    if was {
        CoUninitialize();
    }
}
/// APTTYPE: STA 0, MTA 1, MAINSTA 3; qualifier IMPLICIT_MTA 1.
#[unsafe(no_mangle)]
pub extern "system" fn CoGetApartmentType(kind: usize, qualifier: usize) -> u32 {
    let kind = door_windows_abi::widen(kind);
    let qualifier = door_windows_abi::widen(qualifier);
    {
        if kind == 0 || qualifier == 0 {
            return hr::E_INVALIDARG;
        }
        let id = me();
        let Some((apartment, implicit)) = current() else {
            let _ = write_u32(kind, u32::MAX);
            let _ = write_u32(qualifier, 0);
            return hr::CO_E_NOTINITIALIZED;
        };
        let value = match apartment {
            Apartment::Mta => 1,
            Apartment::Sta if THREADS.with(|t| t.main_sta == id) => 3,
            Apartment::Sta => 0,
        };
        result(write_u32(kind, value).and_then(|_| write_u32(qualifier, u32::from(implicit))))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoGetCurrentProcess() -> u32 {
    me()
}
#[unsafe(no_mangle)]
pub extern "system" fn CoInitializeSecurity(
    _descriptor: usize,
    _count: i32,
    _services: usize,
    _reserved1: usize,
    _authn: u32,
    _imp: u32,
    _list: usize,
    _capabilities: u32,
    _reserved3: usize,
) -> u32 {
    {
        // Once per process, before any marshaling (there is none yet).
        if SECURITY.swap(true, Ordering::AcqRel) {
            hr::RPC_E_TOO_LATE
        } else {
            hr::S_OK
        }
    }
}
/// DLL entry point: a thread that exits leaves its apartment.
#[unsafe(no_mangle)]
pub extern "system" fn DllMain(_module: usize, reason: u32, _reserved: usize) -> i32 {
    {
        if reason == 3 {
            let id = me();
            THREADS.with(|t| {
                if let Some(at) = t.list.iter().position(|(tid, _)| *tid == id) {
                    let (_, init) = t.list.remove(at);
                    if init.apartment.is_some() {
                        leave(t, id, init.apartment);
                    }
                }
            });
            context::thread_detach(id);
        }
        1
    }
}

// ---- GUIDs ---------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn CoCreateGuid(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let mut g = [0u8; 16];
        if unsafe { DoorGenRandom(g.as_mut_ptr() as u64, 16) } != 0 {
            return hr::E_FAIL;
        }
        // A version 4 (random) GUID, RFC 4122 variant.
        g[7] = (g[7] & 0x0F) | 0x40;
        g[8] = (g[8] & 0x3F) | 0x80;
        result(write(out, &g))
    }
}
fn guid_units(g: &Guid) -> Vec<u16> {
    format_guid(g).iter().map(|&b| u16::from(b)).collect()
}
#[unsafe(no_mangle)]
pub extern "system" fn StringFromGUID2(guid: usize, out: usize, capacity: i32) -> i32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_guid(guid) else { return 0 };
        if capacity < 39 {
            return 0;
        }
        let bytes: Vec<u8> = guid_units(&g)
            .iter()
            .chain([&0])
            .flat_map(|u| u.to_le_bytes())
            .collect();
        if write(out, &bytes).is_err() { 0 } else { 39 }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn StringFromCLSID(clsid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let out = door_windows_abi::widen(out);
    {
        let g = match read_guid(clsid) {
            Ok(g) => g,
            Err(e) => return e,
        };
        match task_string(&guid_units(&g)) {
            Ok(block) => result(
                write_pointer(out, block)
                    .inspect_err(|_| CoTaskMemFree(door_windows_abi::narrow(block))),
            ),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn StringFromIID(iid: usize, out: usize) -> u32 {
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    StringFromCLSID(door_windows_abi::narrow(iid), door_windows_abi::narrow(out))
}
#[unsafe(no_mangle)]
pub extern "system" fn IsEqualGUID(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        match (read_guid(a), read_guid(b)) {
            (Ok(a), Ok(b)) => i32::from(a == b),
            _ => 0,
        }
    }
}
/// A GUID string or, for CLSIDFromString, a ProgID.
#[unsafe(no_mangle)]
pub extern "system" fn CLSIDFromString(text: usize, out: usize) -> u32 {
    let text = door_windows_abi::widen(text);
    let out = door_windows_abi::widen(out);
    {
        if text == 0 {
            return result(write(out, &GUID_NULL));
        }
        let Ok(units) = read_wide(text, 260) else {
            return hr::CO_E_CLASSSTRING;
        };
        let found = parse_guid(&units)
            .ok_or(hr::CO_E_CLASSSTRING)
            .or_else(|_| progid_clsid(&units));
        match found {
            Ok(g) => result(write(out, &g)),
            Err(_) => {
                let _ = write(out, &GUID_NULL);
                hr::CO_E_CLASSSTRING
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IIDFromString(text: usize, out: usize) -> u32 {
    let text = door_windows_abi::widen(text);
    let out = door_windows_abi::widen(out);
    {
        const CO_E_IIDSTRING: u32 = 0x8004_01F4;
        if text == 0 {
            return result(write(out, &GUID_NULL));
        }
        match read_wide(text, 64).ok().and_then(|u| parse_guid(&u)) {
            Some(g) => result(write(out, &g)),
            None => CO_E_IIDSTRING,
        }
    }
}

// ---- the registry ------------------------------------------------------------------------
/// The default (or `name`) string value of HKCR\`path`, environment
/// references expanded for REG_EXPAND_SZ.
fn class_value(path: &[u16], name: Option<&[u16]>) -> Option<Vec<u16>> {
    class_value_in(path, name, 0)
}
/// KEY_WOW64_64KEY and KEY_WOW64_32KEY: the registry view to read.
const KEY_WOW64_64KEY: u32 = 0x100;
const KEY_WOW64_32KEY: u32 = 0x200;
/// CLSCTX_ACTIVATE_32_BIT_SERVER and _64_BIT_SERVER.
const ACTIVATE_32_BIT_SERVER: u32 = 0x4_0000;
const ACTIVATE_64_BIT_SERVER: u32 = 0x8_0000;
/// The LocalServer32 command of `clsid`. A server's bitness need not be
/// the client's: the client's own registry view first, then the other one
/// (Windows' activation finds a 64-bit server for a 32-bit client and the
/// reverse); CLSCTX_ACTIVATE_32_BIT_SERVER or _64_BIT_SERVER names one.
pub(crate) fn local_server(clsid: &Guid, context: u32) -> Option<Vec<u16>> {
    let own = if POINTER == 4 {
        KEY_WOW64_32KEY
    } else {
        KEY_WOW64_64KEY
    };
    let other = own ^ (KEY_WOW64_32KEY | KEY_WOW64_64KEY);
    let views: &[u32] = if context & ACTIVATE_32_BIT_SERVER != 0 {
        &[KEY_WOW64_32KEY]
    } else if context & ACTIVATE_64_BIT_SERVER != 0 {
        &[KEY_WOW64_64KEY]
    } else {
        &[own, other]
    };
    let path = clsid_key(clsid, "LocalServer32");
    views
        .iter()
        .find_map(|&view| class_value_in(&path, None, view))
}
fn class_value_in(path: &[u16], name: Option<&[u16]>, view: u32) -> Option<Vec<u16>> {
    let path: Vec<u16> = path.iter().copied().chain([0]).collect();
    let mut key = [0u8; 8];
    if unsafe {
        RegOpenKeyExW(
            door_windows_abi::narrow(HKEY_CLASSES_ROOT),
            door_windows_abi::narrow(path.as_ptr() as u64),
            0,
            KEY_READ | view,
            door_windows_abi::narrow(key.as_mut_ptr() as u64),
        )
    } != 0
    {
        return None;
    }
    let key = u64::from_le_bytes(key);
    let name: Option<Vec<u16>> = name.map(|n| n.iter().copied().chain([0]).collect());
    let mut data = vec![0u16; 520];
    let mut size = (data.len() * 2) as u32;
    let mut kind = 0u32;
    let r = unsafe {
        RegQueryValueExW(
            door_windows_abi::narrow(key),
            name.as_ref()
                .map_or(0, |n| door_windows_abi::narrow(n.as_ptr() as u64)),
            0,
            door_windows_abi::narrow(&mut kind as *mut u32 as u64),
            door_windows_abi::narrow(data.as_mut_ptr() as u64),
            door_windows_abi::narrow(&mut size as *mut u32 as u64),
        )
    };
    unsafe { RegCloseKey(door_windows_abi::narrow(key)) };
    if r != 0 || !matches!(kind, REG_SZ | REG_EXPAND_SZ) {
        return None;
    }
    data.truncate(size as usize / 2);
    while data.last() == Some(&0) {
        data.pop();
    }
    if kind == REG_EXPAND_SZ {
        let source: Vec<u16> = data.iter().copied().chain([0]).collect();
        let mut out = vec![0u16; 1024];
        let n = unsafe {
            DoorExpandEnvironmentStringsW(
                source.as_ptr() as u64,
                out.as_mut_ptr() as u64,
                out.len() as u32,
            )
        } as usize;
        if n == 0 || n > out.len() {
            return None;
        }
        out.truncate(n - 1);
        return Some(out);
    }
    Some(data)
}
fn ascii(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn clsid_key(clsid: &Guid, sub: &str) -> Vec<u16> {
    let mut path = ascii("CLSID\\");
    path.extend(guid_units(clsid));
    if !sub.is_empty() {
        path.push(u16::from(b'\\'));
        path.extend(ascii(sub));
    }
    path
}
/// HKCR\<progid>\CLSID, following CurVer once when it is missing.
fn progid_clsid(progid: &[u16]) -> Result<Guid, u32> {
    if !valid_progid(progid) {
        return Err(hr::CO_E_CLASSSTRING);
    }
    let mut key = progid.to_vec();
    key.extend(ascii("\\CLSID"));
    if let Some(g) = class_value(&key, None).and_then(|v| parse_guid(&v)) {
        return Ok(g);
    }
    let mut cur = progid.to_vec();
    cur.extend(ascii("\\CurVer"));
    let version = class_value(&cur, None).ok_or(hr::CO_E_CLASSSTRING)?;
    let mut key = version;
    key.extend(ascii("\\CLSID"));
    class_value(&key, None)
        .and_then(|v| parse_guid(&v))
        .ok_or(hr::CO_E_CLASSSTRING)
}
#[unsafe(no_mangle)]
pub extern "system" fn CLSIDFromProgID(progid: usize, out: usize) -> u32 {
    let progid = door_windows_abi::widen(progid);
    let out = door_windows_abi::widen(out);
    {
        if progid == 0 || out == 0 {
            return hr::E_INVALIDARG;
        }
        let Ok(units) = read_wide(progid, 260) else {
            return hr::CO_E_CLASSSTRING;
        };
        match progid_clsid(&units) {
            Ok(g) => result(write(out, &g)),
            Err(e) => {
                let _ = write(out, &GUID_NULL);
                e
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CLSIDFromProgIDEx(progid: usize, out: usize) -> u32 {
    let progid = door_windows_abi::widen(progid);
    let out = door_windows_abi::widen(out);
    {
        CLSIDFromProgID(
            door_windows_abi::narrow(progid),
            door_windows_abi::narrow(out),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ProgIDFromCLSID(clsid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let g = match read_guid(clsid) {
            Ok(g) => g,
            Err(e) => return e,
        };
        let Some(progid) = class_value(&clsid_key(&g, "ProgID"), None) else {
            let _ = write_pointer(out, 0);
            return hr::REGDB_E_CLASSNOTREG;
        };
        match task_string(&progid) {
            Ok(block) => result(
                write_pointer(out, block)
                    .inspect_err(|_| CoTaskMemFree(door_windows_abi::narrow(block))),
            ),
            Err(e) => e,
        }
    }
}

// ---- class objects ---------------------------------------------------------------------------
struct Registration {
    cookie: u32,
    clsid: Guid,
    object: u64,
    context: u32,
    suspended: bool,
    /// REGCLS_SINGLEUSE: one activation from another process.
    single: bool,
}
static REGISTERED: Lock<Vec<Registration>> = Lock::new(Vec::new());
static COOKIE: AtomicU32 = AtomicU32::new(0x100);
/// In-process servers this process loaded for activation (module bases).
static SERVERS: Lock<Vec<u64>> = Lock::new(Vec::new());
#[unsafe(no_mangle)]
pub extern "system" fn CoRegisterClassObject(
    clsid: usize,
    object: usize,
    context: u32,
    flags: u32,
    out: usize,
) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let object = door_windows_abi::widen(object);
    let out = door_windows_abi::widen(out);
    {
        if object == 0 || out == 0 {
            return hr::E_INVALIDARG;
        }
        if let Err(e) = initialized() {
            return e;
        }
        let g = match read_guid(clsid) {
            Ok(g) => g,
            Err(e) => return e,
        };
        let cookie = COOKIE.fetch_add(1, Ordering::Relaxed);
        if let Err(e) = write_u32(out, cookie) {
            return e;
        }
        let suspended = flags & regcls::SUSPENDED != 0;
        // Other processes reach a LOCAL_SERVER class object through its class
        // endpoint (SUSPENDED: once CoResumeClassObjects runs).
        if context & clsctx::LOCAL_SERVER != 0 && !suspended {
            let single = flags & (regcls::MULTIPLEUSE | regcls::MULTI_SEPARATE) == 0;
            if let Err(e) = remote::publish_class(cookie, &g, object, single) {
                return e;
            }
        }
        add_ref(object);
        REGISTERED.with(|r| {
            r.push(Registration {
                cookie,
                clsid: g,
                object,
                context,
                suspended,
                single: flags & (regcls::MULTIPLEUSE | regcls::MULTI_SEPARATE) == 0,
            })
        });
        hr::S_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoRevokeClassObject(cookie: u32) -> u32 {
    let object = REGISTERED.with(|r| {
        r.iter()
            .position(|x| x.cookie == cookie)
            .map(|at| r.remove(at).object)
    });
    match object {
        Some(object) => {
            remote::unpublish_class(cookie);
            release(object);
            hr::S_OK
        }
        None => hr::E_INVALIDARG,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoResumeClassObjects() -> u32 {
    // Suspended LOCAL_SERVER registrations get their class endpoints now.
    let resumed: Vec<(u32, Guid, u64, bool)> = REGISTERED.with(|r| {
        r.iter_mut()
            .filter(|x| x.suspended)
            .filter_map(|x| {
                x.suspended = false;
                (x.context & clsctx::LOCAL_SERVER != 0)
                    .then_some((x.cookie, x.clsid, x.object, x.single))
            })
            .collect()
    });
    for (cookie, clsid, object, single) in resumed {
        if let Err(e) = remote::publish_class(cookie, &clsid, object, single) {
            return e;
        }
    }
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn CoSuspendClassObjects() -> u32 {
    REGISTERED.with(|r| r.iter_mut().for_each(|x| x.suspended = true));
    hr::S_OK
}
static SERVER_REFS: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn CoAddRefServerProcess() -> u32 {
    SERVER_REFS.fetch_add(1, Ordering::AcqRel) + 1
}
#[unsafe(no_mangle)]
pub extern "system" fn CoReleaseServerProcess() -> u32 {
    let left = SERVER_REFS.load(Ordering::Acquire).saturating_sub(1);
    SERVER_REFS.store(left, Ordering::Release);
    if left == 0 {
        // The last reference suspends the class objects (no new activations).
        CoSuspendClassObjects();
    }
    left
}

/// The InprocServer32 module of `clsid`, loaded: its base.
fn inproc_server(clsid: &Guid) -> Result<u64, u32> {
    let value =
        class_value(&clsid_key(clsid, "InprocServer32"), None).ok_or(hr::REGDB_E_CLASSNOTREG)?;
    let path = server_path(&value);
    if path.is_empty() {
        return Err(hr::REGDB_E_CLASSNOTREG);
    }
    // A module already loaded answers by its file name; otherwise the
    // registered file itself, by its full path (never a same-named DLL from
    // the client's folder or current directory).
    let name: Vec<u16> = file_name(&path).iter().copied().chain([0]).collect();
    let mut module = unsafe { DoorGetModuleHandleW(name.as_ptr() as u64) };
    if module == 0 {
        let full: Vec<u16> = path.iter().copied().chain([0]).collect();
        module = unsafe { DoorLoadLibraryW(full.as_ptr() as u64) };
    }
    if module == 0 {
        return Err(hr::CO_E_DLLNOTFOUND);
    }
    SERVERS.with(|s| {
        if !s.contains(&module) {
            s.push(module);
        }
    });
    Ok(module)
}
fn class_object(clsid: &Guid, context: u32, iid: &Guid, out: u64) -> u32 {
    let registered = REGISTERED.with(|r| {
        r.iter()
            .find(|x| x.clsid == *clsid && x.context & context != 0 && !x.suspended)
            .map(|x| {
                add_ref(x.object);
                x.object
            })
    });
    if let Some(object) = registered {
        let r = query_interface(object, iid, out);
        release(object);
        return r;
    }
    if context & (clsctx::INPROC_SERVER | clsctx::INPROC_HANDLER) != 0 {
        match inproc_server(clsid) {
            Ok(module) => {
                let entry =
                    unsafe { DoorGetProcAddress(module, c"DllGetClassObject".as_ptr() as u64) };
                if entry == 0 {
                    return hr::CO_E_ERRORINDLL;
                }
                // SAFETY: DllGetClassObject's documented signature.
                let get: extern "system" fn(usize, usize, usize) -> u32 =
                    unsafe { core::mem::transmute(entry as usize) };
                return get(
                    door_windows_abi::narrow(clsid.as_ptr() as u64),
                    door_windows_abi::narrow(iid.as_ptr() as u64),
                    door_windows_abi::narrow(out),
                );
            }
            Err(hr::REGDB_E_CLASSNOTREG) => (),
            Err(e) => return e,
        }
    }
    if context & clsctx::LOCAL_SERVER != 0 && local_server(clsid, context).is_some() {
        // Out-of-process: the server's class object through the broker.
        return remote::local_class_object(clsid, iid, out);
    }
    hr::REGDB_E_CLASSNOTREG
}
#[unsafe(no_mangle)]
pub extern "system" fn CoGetClassObject(
    clsid: usize,
    context: u32,
    _server: usize,
    iid: usize,
    out: usize,
) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        if let Err(e) = initialized() {
            return e;
        }
        let (Ok(clsid), Ok(iid)) = (read_guid(clsid), read_guid(iid)) else {
            return hr::E_INVALIDARG;
        };
        if context & clsctx::ALL == 0 {
            return hr::E_INVALIDARG;
        }
        class_object(&clsid, context, &iid, out)
    }
}
/// An instance of `clsid` through its class factory: (result, IUnknown or
/// the interface asked for).
fn create_instance(clsid: &Guid, outer: u64, context: u32, iid: &Guid) -> (u32, u64) {
    let mut factory = [0u8; 8];
    let r = class_object(
        clsid,
        context,
        &IID_ICLASSFACTORY,
        factory.as_mut_ptr() as u64,
    );
    if r != hr::S_OK {
        return (r, 0);
    }
    let factory = u64::from_le_bytes(factory);
    let mut object = [0u8; 8];
    // SAFETY: IClassFactory::CreateInstance is method 3.
    let create: extern "system" fn(usize, usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(factory, 3) as usize) };
    let r = create(
        door_windows_abi::narrow(factory),
        door_windows_abi::narrow(outer),
        door_windows_abi::narrow(iid.as_ptr() as u64),
        door_windows_abi::narrow(object.as_mut_ptr() as u64),
    );
    release(factory);
    (
        r,
        if r == hr::S_OK {
            u64::from_le_bytes(object)
        } else {
            0
        },
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn CoCreateInstance(
    clsid: usize,
    outer: usize,
    context: u32,
    iid: usize,
    out: usize,
) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        if let Err(e) = initialized() {
            return e;
        }
        let (Ok(clsid), Ok(iid)) = (read_guid(clsid), read_guid(iid)) else {
            return hr::E_INVALIDARG;
        };
        // Aggregation hands out the inner IUnknown only.
        if outer != 0 && iid != IID_IUNKNOWN {
            return hr::CLASS_E_NOAGGREGATION;
        }
        let (r, object) = create_instance(&clsid, outer, context, &iid);
        if r != hr::S_OK {
            return r;
        }
        result(write_pointer(out, object))
    }
}
/// MULTI_QI { const IID *pIID; IUnknown *pItf; HRESULT hr; } (24 bytes).
#[unsafe(no_mangle)]
pub extern "system" fn CoCreateInstanceEx(
    clsid: usize,
    outer: usize,
    context: u32,
    _server: usize,
    count: u32,
    results: usize,
) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let outer = door_windows_abi::widen(outer);
    let results = door_windows_abi::widen(results);
    {
        const CO_S_NOTALLINTERFACES: u32 = 0x0008_0012;
        if count == 0 || results == 0 {
            return hr::E_INVALIDARG;
        }
        if let Err(e) = initialized() {
            return e;
        }
        let Ok(clsid) = read_guid(clsid) else {
            return hr::E_INVALIDARG;
        };
        // MULTI_QI { pIID, pItf, hr }: 24 bytes on x64, 12 on x86.
        const P: u64 = POINTER as u64;
        let entry = |i: u64| {
            results
                + i * door_windows_abi::layout::size(&{
                    use door_windows_abi::layout::Field::*;
                    [Pointer, Pointer, U32]
                }) as u64
        };
        let mut iids = Vec::new();
        for i in 0..u64::from(count) {
            let at = entry(i);
            let Ok(iid) = read_pointer(at).and_then(read_guid) else {
                return hr::E_INVALIDARG;
            };
            iids.push(iid);
            let _ = write_pointer(at + P, 0);
        }
        let (r, unknown) = create_instance(&clsid, outer, context, &IID_IUNKNOWN);
        if r != hr::S_OK {
            for i in 0..u64::from(count) {
                let _ = write_u32(entry(i) + 2 * P, r);
            }
            return r;
        }
        let mut got = 0;
        for (i, iid) in iids.iter().enumerate() {
            let at = entry(i as u64);
            let r = query_interface(unknown, iid, at + P);
            let _ = write_u32(at + 2 * P, r);
            if r == hr::S_OK {
                got += 1;
            }
        }
        release(unknown);
        match got {
            0 => hr::E_NOINTERFACE,
            n if n == iids.len() => hr::S_OK,
            _ => CO_S_NOTALLINTERFACES,
        }
    }
}
/// Unloads servers whose DllCanUnloadNow says S_OK.
#[unsafe(no_mangle)]
pub extern "system" fn CoFreeUnusedLibrariesEx(_delay: u32, _reserved: u32) {
    let servers = SERVERS.with(|s| s.clone());
    for module in servers {
        let entry = unsafe { DoorGetProcAddress(module, c"DllCanUnloadNow".as_ptr() as u64) };
        if entry == 0 {
            continue;
        }
        // SAFETY: DllCanUnloadNow's documented signature.
        let can: extern "system" fn() -> u32 = unsafe { core::mem::transmute(entry as usize) };
        if can() == hr::S_OK {
            SERVERS.with(|s| s.retain(|m| *m != module));
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CoFreeUnusedLibraries() {
    CoFreeUnusedLibrariesEx(u32::MAX, 0)
}
/// COWAIT_WAITALL 1, COWAIT_ALERTABLE 2.
#[unsafe(no_mangle)]
pub extern "system" fn CoWaitForMultipleHandles(
    flags: u32,
    millis: u32,
    count: u32,
    handles: usize,
    index: usize,
) -> u32 {
    let handles = door_windows_abi::widen(handles);
    let index = door_windows_abi::widen(index);
    {
        const RPC_S_CALLPENDING: u32 = 0x8001_0115;
        if handles == 0 || index == 0 || count == 0 || count > 64 {
            return hr::E_INVALIDARG;
        }
        // The wait serves calls into the caller's apartment as it goes, in
        // 10 ms slices (as an STA's modal loop dispatches them).
        let here = remote::apartment();
        let mut left = millis;
        let r = loop {
            let slice = if here.is_some() { left.min(10) } else { left };
            let r = unsafe {
                DoorWaitForMultipleObjectsEx(
                    count,
                    handles,
                    i32::from(flags & 1 != 0),
                    slice,
                    i32::from(flags & 2 != 0),
                )
            };
            if r != 0x102 || left == 0 {
                break r;
            }
            if let Some(a) = here {
                remote::pump(a);
            }
            if left != u32::MAX {
                left -= slice;
                if left == 0 {
                    break 0x102;
                }
            }
        };
        match r {
            0x102 => RPC_S_CALLPENDING,
            r if r < count || (0x80..0x80 + count).contains(&r) || r == 0xC0 => {
                result(write_u32(index, r & 0x7F))
            }
            _ => hr::E_FAIL,
        }
    }
}

// ---- PROPVARIANT -------------------------------------------------------------------------------
const VT_VECTOR: u16 = 0x1000;
const VT_ARRAY: u16 = 0x2000;
const VT_BYREF: u16 = 0x4000;
/// The bytes of one element of a VT_VECTOR of `vt`, and whether it
/// points to task memory (a string or CLSID) to free.
fn element(vt: u16) -> Option<(u64, bool)> {
    Some(match vt {
        16 | 17 => (1, false),
        2 | 18 | 11 => (2, false),
        3 | 19 | 4 | 10 => (4, false),
        20 | 21 | 5 | 6 | 7 | 64 => (8, false),
        72 => (16, false),
        30 | 31 | 8 => (POINTER as u64, true),
        _ => return None,
    })
}
/// Frees a BSTR: its length prefix sits 4 bytes before it (oleaut32 makes
/// them on the process heap).
fn free_bstr(bstr: u64) {
    if bstr != 0 {
        CoTaskMemFree(door_windows_abi::narrow(bstr - 4));
    }
}
fn clear(pv: u64) -> Result<(), u32> {
    let vt = read_u16(pv)?;
    let data = pv + 8;
    match vt {
        0 | 1 | 2..=7 | 10 | 11 | 14 | 16..=23 | 64 => (),
        8 => free_bstr(read_pointer(data)?),
        9 | 13 | 66 | 67 => {
            let object = read_pointer(data)?;
            if object != 0 {
                release(object);
            }
        }
        30 | 31 | 72 => CoTaskMemFree(door_windows_abi::narrow(read_pointer(data)?)),
        // VT_BLOB { cbSize; pBlobData }
        65 => CoTaskMemFree(door_windows_abi::narrow(read_pointer(
            data + POINTER as u64,
        )?)),
        v if v & VT_VECTOR != 0 && v & !VT_VECTOR != 0 => {
            let (size, owned) = element(v & !VT_VECTOR).ok_or(hr::DISP_E_BADVARTYPE)?;
            let count = u64::from(read_u32(data)?);
            let items = read_pointer(data + POINTER as u64)?;
            if owned {
                for i in 0..count {
                    let item = read_pointer(items + i * size)?;
                    if v & !VT_VECTOR == 8 {
                        free_bstr(item)
                    } else {
                        CoTaskMemFree(door_windows_abi::narrow(item))
                    }
                }
            }
            CoTaskMemFree(door_windows_abi::narrow(items));
        }
        v if v & (VT_ARRAY | VT_BYREF) != 0 => return Err(hr::DISP_E_BADVARTYPE),
        _ => return Err(hr::DISP_E_BADVARTYPE),
    }
    write(pv, &[0u8; PROPVARIANT_BYTES])
}
#[unsafe(no_mangle)]
pub extern "system" fn PropVariantClear(pv: usize) -> u32 {
    let pv = door_windows_abi::widen(pv);
    {
        if pv == 0 {
            return hr::S_OK;
        }
        result(clear(pv))
    }
}
/// A copy of task memory `bytes` long.
fn task_copy(source: u64, bytes: u64) -> Result<u64, u32> {
    if source == 0 {
        return Ok(0);
    }
    let mut data = vec![0u8; bytes as usize];
    read(source, &mut data)?;
    let block = CoTaskMemAlloc(data.len());
    if block == 0 {
        return Err(hr::E_OUTOFMEMORY);
    }
    write(door_windows_abi::widen(block), &data)?;
    Ok(door_windows_abi::widen(block))
}
fn wide_len(pointer: u64) -> Result<u64, u32> {
    Ok(read_wide(pointer, 1 << 20)?.len() as u64)
}
fn narrow_len(pointer: u64) -> Result<u64, u32> {
    let mut n = 0;
    loop {
        let mut b = [0u8];
        read(pointer + n, &mut b)?;
        if b[0] == 0 {
            return Ok(n);
        }
        n += 1;
    }
}
fn copy_bstr(bstr: u64) -> Result<u64, u32> {
    if bstr == 0 {
        return Ok(0);
    }
    let bytes = u64::from(read_u32(bstr - 4)?);
    Ok(task_copy(bstr - 4, bytes + 6)? + 4)
}
fn copy_item(vt: u16, value: u64) -> Result<u64, u32> {
    match vt {
        8 => copy_bstr(value),
        30 => task_copy(value, narrow_len(value)? + 1),
        31 => task_copy(value, (wide_len(value)? + 1) * 2),
        72 => task_copy(value, 16),
        _ => Ok(value),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PropVariantCopy(destination: usize, source: usize) -> u32 {
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    {
        let copy = || -> Result<(), u32> {
            use door_windows_abi::layout::{self, Field::*};
            // The union's pointer at 8; a BLOB's or CA*'s count at 8 and
            // pointer after it.
            const UNION: [layout::Field; 3] = [U64, Pointer, Pointer];
            let mut pv = [0u8; PROPVARIANT_BYTES];
            read(source, &mut pv)?;
            let vt = u16::from_le_bytes([pv[0], pv[1]]);
            let value = layout::get(&pv, &UNION, 1);
            match vt {
                0 | 1 | 2..=7 | 10 | 11 | 14 | 16..=23 | 64 => (),
                8 | 30 | 31 | 72 => layout::set(&mut pv, &UNION, 1, copy_item(vt, value)?),
                9 | 13 | 66 | 67 => {
                    if value != 0 {
                        add_ref(value);
                    }
                }
                65 => {
                    let size = u64::from(u32::from_le_bytes(pv[8..12].try_into().unwrap()));
                    let blob = layout::get(&pv, &UNION, 2);
                    layout::set(&mut pv, &UNION, 2, task_copy(blob, size)?);
                }
                v if v & VT_VECTOR != 0 => {
                    let item = v & !VT_VECTOR;
                    let (size, owned) = element(item).ok_or(hr::DISP_E_BADVARTYPE)?;
                    let count = u64::from(u32::from_le_bytes(pv[8..12].try_into().unwrap()));
                    let items = layout::get(&pv, &UNION, 2);
                    let copied = task_copy(items, count * size)?;
                    if owned {
                        for i in 0..count {
                            let at = copied + i * size;
                            let v = copy_item(item, read_pointer(at)?)?;
                            write_pointer(at, v)?;
                        }
                    }
                    layout::set(&mut pv, &UNION, 2, copied);
                }
                _ => return Err(hr::DISP_E_BADVARTYPE),
            }
            write(destination, &pv)
        };
        result(copy())
    }
}

// ---- IStream over HGLOBAL ---------------------------------------------------------------------
/// One HGLOBAL's contents, shared by a stream and its clones. HGLOBALs are
/// fixed process-heap blocks (kernel32's GlobalAlloc), so the handle is
/// the block's address.
struct Memory {
    handle: u64,
    size: u64,
    delete: bool,
    users: u32,
}
struct Stream {
    object: u64,
    refs: u32,
    memory: usize,
    position: u64,
}
struct Streams {
    memories: Vec<Option<Memory>>,
    streams: Vec<Stream>,
}
static STREAMS: Lock<Streams> = Lock::new(Streams {
    memories: Vec::new(),
    streams: Vec::new(),
});
#[repr(C)]
struct StreamVtbl {
    query_interface: extern "system" fn(usize, usize, usize) -> u32,
    add_ref: extern "system" fn(usize) -> u32,
    release: extern "system" fn(usize) -> u32,
    read: extern "system" fn(usize, usize, u32, usize) -> u32,
    write: extern "system" fn(usize, usize, u32, usize) -> u32,
    seek: extern "system" fn(usize, isize, u32, usize) -> u32,
    set_size: extern "system" fn(usize, usize) -> u32,
    copy_to: extern "system" fn(usize, usize, usize, usize, usize) -> u32,
    commit: extern "system" fn(usize, u32) -> u32,
    revert: extern "system" fn(usize) -> u32,
    lock_region: extern "system" fn(usize, usize, usize, u32) -> u32,
    unlock_region: extern "system" fn(usize, usize, usize, u32) -> u32,
    stat: extern "system" fn(usize, usize, u32) -> u32,
    clone: extern "system" fn(usize, usize) -> u32,
}
static STREAM_VTBL: StreamVtbl = StreamVtbl {
    query_interface: s_query_interface,
    add_ref: s_add_ref,
    release: s_release,
    read: s_read,
    write: s_write,
    seek: s_seek,
    set_size: s_set_size,
    copy_to: s_copy_to,
    commit: s_commit,
    revert: s_revert,
    lock_region: s_lock_region,
    unlock_region: s_unlock_region,
    stat: s_stat,
    clone: s_clone,
};
fn new_stream(s: &mut Streams, memory: usize, position: u64) -> u64 {
    let object = Box::into_raw(Box::new(&STREAM_VTBL as *const StreamVtbl as u64)) as u64;
    s.streams.push(Stream {
        object,
        refs: 1,
        memory,
        position,
    });
    if let Some(m) = s.memories[memory].as_mut() {
        m.users += 1;
    }
    object
}
fn with_stream<R>(
    this: u64,
    f: impl FnOnce(&mut Stream, &mut Memory) -> Result<R, u32>,
) -> Result<R, u32> {
    STREAMS.with(|s| {
        let stream = s
            .streams
            .iter_mut()
            .find(|x| x.object == this)
            .ok_or(hr::E_POINTER)?;
        let memory = s.memories[stream.memory].as_mut().ok_or(hr::E_UNEXPECTED)?;
        f(stream, memory)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateStreamOnHGlobal(
    handle: usize,
    delete_on_release: i32,
    out: usize,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let (handle, size) = if handle == 0 {
            let block = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0, 1) };
            if block == 0 {
                return hr::E_OUTOFMEMORY;
            }
            (door_windows_abi::widen(block), 0)
        } else {
            let size = unsafe {
                RtlSizeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(handle),
                )
            };
            if size == usize::MAX {
                return hr::E_INVALIDARG;
            }
            (handle, size as u64)
        };
        let object = STREAMS.with(|s| {
            s.memories.push(Some(Memory {
                handle,
                size,
                delete: delete_on_release != 0,
                users: 0,
            }));
            let memory = s.memories.len() - 1;
            new_stream(s, memory, 0)
        });
        result(write_pointer(out, object))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetHGlobalFromStream(stream: usize, out: usize) -> u32 {
    let stream = door_windows_abi::widen(stream);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        match with_stream(stream, |_, m| Ok(m.handle)) {
            Ok(h) => result(write_pointer(out, h)),
            Err(_) => hr::E_INVALIDARG,
        }
    }
}
extern "system" fn s_query_interface(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(iid) = read_guid(iid) else {
            return hr::E_POINTER;
        };
        if [IID_IUNKNOWN, IID_ISEQUENTIALSTREAM, IID_ISTREAM].contains(&iid) {
            s_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn s_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        with_stream(this, |s, _| {
            s.refs += 1;
            Ok(s.refs)
        })
        .unwrap_or(0)
    }
}
extern "system" fn s_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let freed = STREAMS.with(|s| {
            let at = s.streams.iter().position(|x| x.object == this)?;
            s.streams[at].refs -= 1;
            if s.streams[at].refs != 0 {
                return Some((s.streams[at].refs, 0));
            }
            let stream = s.streams.remove(at);
            let mut free = 0;
            if let Some(m) = s.memories[stream.memory].as_mut() {
                m.users -= 1;
                if m.users == 0 {
                    if m.delete {
                        free = m.handle;
                    }
                    s.memories[stream.memory] = None;
                }
            }
            Some((0, free))
        });
        match freed {
            Some((left, free)) => {
                if free != 0 {
                    unsafe {
                        RtlFreeHeap(
                            door_windows_abi::narrow(heap()),
                            0,
                            door_windows_abi::narrow(free),
                        )
                    };
                }
                left
            }
            None => 0,
        }
    }
}
/// Grows (or shrinks) the HGLOBAL to `size` bytes.
fn resize(m: &mut Memory, size: u64) -> Result<(), u32> {
    let block = unsafe {
        RtlReAllocateHeap(
            door_windows_abi::narrow(heap()),
            0,
            door_windows_abi::narrow(m.handle),
            size.max(1) as usize,
        )
    };
    if block == 0 {
        return Err(hr::STG_E_MEDIUMFULL);
    }
    m.handle = door_windows_abi::widen(block);
    if size > m.size {
        // New bytes read as zero.
        // SAFETY: the block holds `size` bytes now.
        unsafe {
            core::ptr::write_bytes(
                (block + door_windows_abi::narrow(m.size)) as *mut u8,
                0,
                (size - m.size) as usize,
            )
        };
    }
    m.size = size;
    Ok(())
}
extern "system" fn s_read(this: usize, out: usize, bytes: u32, done: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let done = door_windows_abi::widen(done);
    {
        let r = with_stream(this, |s, m| {
            let n = u64::from(bytes).min(m.size.saturating_sub(s.position));
            if n > 0 {
                // SAFETY: [position, position + n) lies inside the block.
                let data = unsafe {
                    core::slice::from_raw_parts((m.handle + s.position) as *const u8, n as usize)
                };
                write(out, data).map_err(|_| hr::STG_E_INVALIDPOINTER)?;
            }
            s.position += n;
            Ok(n as u32)
        });
        match r {
            Ok(n) => {
                if done != 0 {
                    let _ = write_u32(done, n);
                }
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn s_write(this: usize, data: usize, bytes: u32, done: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let data = door_windows_abi::widen(data);
    let done = door_windows_abi::widen(done);
    {
        let mut buffer = vec![0u8; bytes as usize];
        if read(data, &mut buffer).is_err() && bytes != 0 {
            return hr::STG_E_INVALIDPOINTER;
        }
        let r = with_stream(this, |s, m| {
            let end = s.position + u64::from(bytes);
            if end > m.size {
                resize(m, end)?;
            }
            // SAFETY: the block holds `end` bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buffer.as_ptr(),
                    (m.handle + s.position) as *mut u8,
                    buffer.len(),
                )
            };
            s.position = end;
            Ok(())
        });
        match r {
            Ok(()) => {
                if done != 0 {
                    let _ = write_u32(done, bytes);
                }
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
/// STREAM_SEEK_SET 0, CUR 1, END 2.
extern "system" fn s_seek(this: usize, offset: isize, origin: u32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let offset = offset as i64;
    {
        let r = with_stream(this, |s, m| {
            let base = match origin {
                0 => 0i128,
                1 => i128::from(s.position),
                2 => i128::from(m.size),
                _ => return Err(hr::STG_E_INVALIDFUNCTION),
            };
            let target = base + i128::from(offset);
            if target < 0 {
                return Err(hr::STG_E_INVALIDFUNCTION);
            }
            s.position = target as u64;
            Ok(s.position)
        });
        match r {
            Ok(p) => {
                if out != 0 {
                    // plibNewPosition: a ULARGE_INTEGER.
                    let _ = write_u64(out, p);
                }
                hr::S_OK
            }
            Err(e) => e,
        }
    }
}
extern "system" fn s_set_size(this: usize, size: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let size = door_windows_abi::widen(size);
    result(with_stream(this, |_, m| resize(m, size)))
}
extern "system" fn s_copy_to(
    this: usize,
    target: usize,
    bytes: usize,
    read_out: usize,
    written_out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let target = door_windows_abi::widen(target);
    let bytes = door_windows_abi::widen(bytes);
    let read_out = door_windows_abi::widen(read_out);
    let written_out = door_windows_abi::widen(written_out);
    {
        if target == 0 {
            return hr::STG_E_INVALIDPOINTER;
        }
        let chunk = match with_stream(this, |s, m| {
            let n = bytes.min(m.size.saturating_sub(s.position));
            // SAFETY: [position, position + n) lies inside the block.
            let data = unsafe {
                core::slice::from_raw_parts((m.handle + s.position) as *const u8, n as usize)
            }
            .to_vec();
            s.position += n;
            Ok(data)
        }) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut written = [0u8; 4];
        // SAFETY: ISequentialStream::Write is method 4 of the target.
        let write_fn: extern "system" fn(usize, usize, u32, usize) -> u32 =
            unsafe { core::mem::transmute(method(target, 4) as usize) };
        let r = write_fn(
            door_windows_abi::narrow(target),
            door_windows_abi::narrow(chunk.as_ptr() as u64),
            chunk.len() as u32,
            door_windows_abi::narrow(written.as_mut_ptr() as u64),
        );
        // pcbRead and pcbWritten: ULARGE_INTEGERs.
        if read_out != 0 {
            let _ = write_u64(read_out, chunk.len() as u64);
        }
        if written_out != 0 {
            let _ = write_u64(written_out, u64::from(u32::from_le_bytes(written)));
        }
        r
    }
}
extern "system" fn s_commit(this: usize, _flags: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_stream(this, |_, _| Ok(())))
}
extern "system" fn s_revert(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    result(with_stream(this, |_, _| Ok(())))
}
extern "system" fn s_lock_region(_this: usize, _offset: usize, _bytes: usize, _kind: u32) -> u32 {
    {
        // STG_E_INVALIDFUNCTION: memory streams do not lock ranges.
        hr::STG_E_INVALIDFUNCTION
    }
}
extern "system" fn s_unlock_region(_this: usize, _offset: usize, _bytes: usize, _kind: u32) -> u32 {
    hr::STG_E_INVALIDFUNCTION
}
/// STATSTG: pwcsName, type (STGTY_STREAM 2), cbSize, three FILETIMEs,
/// grfMode, grfLocksSupported, clsid, grfStateBits, reserved (80 bytes on
/// x64, 72 on x86).
const STATSTG: [door_windows_abi::layout::Field; 11] = {
    use door_windows_abi::layout::Field::*;
    [
        Pointer,
        U32,
        U64,
        Bytes(8),
        Bytes(8),
        Bytes(8),
        U32,
        U32,
        Bytes(16),
        U32,
        U32,
    ]
};
extern "system" fn s_stat(this: usize, out: usize, _flags: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let size = match with_stream(this, |_, m| Ok(m.size)) {
            Ok(s) => s,
            Err(e) => return e,
        };
        use door_windows_abi::layout;
        let mut st = [0u8; layout::size(&STATSTG)];
        layout::set(&mut st, &STATSTG, 1, 2);
        layout::set(&mut st, &STATSTG, 2, size);
        // STGM_READWRITE.
        layout::set(&mut st, &STATSTG, 6, 2);
        result(write(out, &st))
    }
}
extern "system" fn s_clone(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::STG_E_INVALIDPOINTER;
        }
        let object = STREAMS.with(|s| {
            let at = s.streams.iter().position(|x| x.object == this)?;
            let (memory, position) = (s.streams[at].memory, s.streams[at].position);
            Some(new_stream(s, memory, position))
        });
        match object {
            Some(o) => result(write_pointer(out, o)),
            None => hr::E_POINTER,
        }
    }
}

// ---- Windows Runtime strings (api-ms-win-core-winrt-string) ---------------------------

/// An HSTRING: a reference count, the length in UTF-16 units, then the
/// units and a terminator, on the process heap. NULL is the empty string.
#[repr(C)]
struct HString {
    references: core::sync::atomic::AtomicU32,
    length: u32,
}
/// A string reference (WindowsCreateStringReference) lives in the caller's
/// HSTRING_HEADER: this mark, the length, then the caller's units.
const REFERENCE: u32 = u32::MAX;
/// The units of a reference string, if `string` is one.
fn reference_units(string: u64) -> Option<u64> {
    // SAFETY: an HSTRING this DLL made or a reference header.
    let header = unsafe { &*(string as *const HString) };
    (header
        .references
        .load(core::sync::atomic::Ordering::Relaxed)
        == REFERENCE)
        .then(|| unsafe { ((string + 8) as *const usize).read_unaligned() } as u64)
}
fn hstring_layout(length: usize) -> Option<core::alloc::Layout> {
    let bytes = core::mem::size_of::<HString>().checked_add((length + 1).checked_mul(2)?)?;
    core::alloc::Layout::from_size_align(bytes, 8).ok()
}
/// `WindowsCreateString(source, length, string)`: a copy of `length` units.
#[unsafe(no_mangle)]
pub extern "system" fn WindowsCreateString(source: usize, length: u32, string: usize) -> u32 {
    let (source, string) = (
        door_windows_abi::widen(source),
        door_windows_abi::widen(string),
    );
    if string == 0 || (source == 0 && length != 0) {
        return hr::E_INVALIDARG;
    }
    if length == 0 {
        unsafe { (string as *mut usize).write_unaligned(0) };
        return hr::S_OK;
    }
    let Some(layout) = hstring_layout(length as usize) else {
        return hr::E_OUTOFMEMORY;
    };
    // SAFETY: a fresh allocation of `layout`, written before it is shared.
    let pointer = unsafe { alloc::alloc::alloc(layout) };
    if pointer.is_null() {
        return hr::E_OUTOFMEMORY;
    }
    unsafe {
        (pointer as *mut HString).write(HString {
            references: core::sync::atomic::AtomicU32::new(1),
            length,
        });
        let units = pointer.add(core::mem::size_of::<HString>()) as *mut u16;
        core::ptr::copy_nonoverlapping(source as *const u16, units, length as usize);
        units.add(length as usize).write(0);
        (string as *mut usize).write_unaligned(pointer as usize);
    }
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowsDuplicateString(string: usize, out: usize) -> u32 {
    let (string, out) = (
        door_windows_abi::widen(string),
        door_windows_abi::widen(out),
    );
    if out == 0 {
        return hr::E_INVALIDARG;
    }
    if string != 0
        && let Some(units) = reference_units(string)
    {
        // A reference outlives no header: the duplicate is a copy.
        let length = WindowsGetStringLen(door_windows_abi::narrow(string));
        return WindowsCreateString(
            door_windows_abi::narrow(units),
            length,
            door_windows_abi::narrow(out),
        );
    }
    if string != 0 {
        // SAFETY: an HSTRING this DLL made.
        unsafe { &*(string as *const HString) }
            .references
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    unsafe { (out as *mut usize).write_unaligned(door_windows_abi::narrow(string)) };
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowsDeleteString(string: usize) -> u32 {
    let string = door_windows_abi::widen(string);
    if string == 0 || reference_units(string).is_some() {
        return hr::S_OK;
    }
    // SAFETY: an HSTRING this DLL made; the last reference frees it.
    let header = unsafe { &*(string as *const HString) };
    if header
        .references
        .fetch_sub(1, core::sync::atomic::Ordering::AcqRel)
        == 1
        && let Some(layout) = hstring_layout(header.length as usize)
    {
        unsafe { alloc::alloc::dealloc(string as *mut u8, layout) };
    }
    hr::S_OK
}
/// `WindowsGetStringRawBuffer(string, length)`: the units ("" for NULL).
#[unsafe(no_mangle)]
pub extern "system" fn WindowsGetStringRawBuffer(string: usize, length: usize) -> usize {
    static EMPTY: u16 = 0;
    let (string, length) = (
        door_windows_abi::widen(string),
        door_windows_abi::widen(length),
    );
    let (units, count) = if string == 0 {
        (&EMPTY as *const u16 as u64, 0)
    } else {
        // SAFETY: an HSTRING this DLL made, or a reference header.
        let header = unsafe { &*(string as *const HString) };
        (
            reference_units(string).unwrap_or(string + core::mem::size_of::<HString>() as u64),
            header.length,
        )
    };
    if length != 0 {
        unsafe { (length as *mut u32).write_unaligned(count) };
    }
    door_windows_abi::narrow(units)
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowsGetStringLen(string: usize) -> u32 {
    let string = door_windows_abi::widen(string);
    if string == 0 {
        0
    } else {
        // SAFETY: an HSTRING this DLL made.
        unsafe { &*(string as *const HString) }.length
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowsIsStringEmpty(string: usize) -> i32 {
    i32::from(WindowsGetStringLen(string) == 0)
}
/// `WindowsCreateStringReference(source, length, header, string)`: a
/// "fast-pass" string over the caller's NUL-terminated units, kept in the
/// caller's HSTRING_HEADER (no copy, nothing to delete).
#[unsafe(no_mangle)]
pub extern "system" fn WindowsCreateStringReference(
    source: usize,
    length: u32,
    header: usize,
    string: usize,
) -> u32 {
    let (source, header, string) = (
        door_windows_abi::widen(source),
        door_windows_abi::widen(header),
        door_windows_abi::widen(string),
    );
    if string == 0 || header == 0 || (source == 0 && length != 0) {
        return hr::E_INVALIDARG;
    }
    if length == 0 {
        unsafe { (string as *mut usize).write_unaligned(0) };
        return hr::S_OK;
    }
    // The units must end in a terminator, as Windows checks.
    if unsafe { (source as *const u16).add(length as usize).read_unaligned() } != 0 {
        return hr::E_INVALIDARG;
    }
    // SAFETY: the caller's HSTRING_HEADER (20 bytes on x86, 24 on x64).
    unsafe {
        (header as *mut u32).write_unaligned(REFERENCE);
        ((header + 4) as *mut u32).write_unaligned(length);
        ((header + 8) as *mut usize).write_unaligned(door_windows_abi::narrow(source));
        (string as *mut usize).write_unaligned(door_windows_abi::narrow(header));
    }
    hr::S_OK
}
