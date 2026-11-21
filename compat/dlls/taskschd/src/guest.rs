//! taskschd.dll's COM objects. Every interface is dual: IDispatch goes
//! through this DLL's type library (its TYPELIB resource, the members,
//! DISPIDs and signatures of Windows' TaskScheduler 1.0 library) with
//! oleaut32's DispGetIDsOfNames and DispInvoke on the vtable side.
//!
//! `ITaskService` connects to the scheduler Init hosts; folders are task
//! name prefixes (`Vendor\Updater` is in `\Vendor`); a registered task is
//! its name, read from the scheduler (state, times, last result) and from
//! the XML Init keeps for it. Definitions (`def`) live here until
//! RegisterTaskDefinition plans them onto the scheduler.
use crate::*;
use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use door_service_client::{Error, Tasks};
use door_service_protocol::{Code, tasks as wire};

mod def;

pub(crate) struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
unsafe impl<T: Send> Send for Lock<T> {}
impl<T> Lock<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
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
    fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32;
    fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    fn DoorGenRandom(buffer: u64, length: u32) -> i32;
}
#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> usize;
}
#[link(name = "oleaut32", kind = "dylib")]
unsafe extern "system" {
    fn SysAllocStringLen(text: usize, length: u32) -> usize;
    fn SysStringLen(bstr: usize) -> u32;
    fn LoadTypeLibEx(path: usize, kind: u32, out: usize) -> u32;
    fn DispGetIDsOfNames(info: usize, names: usize, count: u32, ids: usize) -> u32;
    fn DispInvoke(
        this: usize,
        info: usize,
        member: i32,
        flags: u16,
        params: usize,
        result: usize,
        excep: usize,
        argerr: usize,
    ) -> u32;
}

// ---- application memory ----------------------------------------------------------------
const COPY_CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
pub(crate) fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
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
pub(crate) fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
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
pub(crate) fn read_u32(p: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(p, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
pub(crate) fn write_u16(p: u64, v: u16) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
pub(crate) fn write_u32(p: u64, v: u32) -> Result<(), u32> {
    write(p, &v.to_le_bytes())
}
/// The program's pointer width (P17: 4 for a 32-bit program).
pub(crate) const POINTER: usize = door_windows_abi::layout::POINTER;
/// A pointer-sized program value (interfaces, BSTRs), sign-extended on
/// x86.
pub(crate) fn read_pointer(p: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(p, &mut b[..POINTER])?;
    Ok(pointer_at(&b, 0))
}
fn pointer_at(b: &[u8], at: usize) -> u64 {
    door_windows_abi::layout::get(&b[at..], &[door_windows_abi::layout::Field::Pointer], 0)
}
/// An interface, BSTR or task-memory pointer into an out-parameter.
pub(crate) fn write_pointer(p: u64, v: u64) -> Result<(), u32> {
    write(p, &v.to_le_bytes()[..POINTER])
}
/// sizeof(VARIANT): 24 bytes on x64, 16 on x86.
pub(crate) const VARIANT_BYTES: usize = 8 + 2 * POINTER;
/// A VARIANT parameter passed by value (P17): x64 passes the caller's copy
/// by address (as Rust passes this struct too), x86 pushes its 16 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct VariantArg([u8; VARIANT_BYTES]);
impl VariantArg {
    /// The value's address, for `variant`.
    pub(crate) fn at(&self) -> u64 {
        self.0.as_ptr() as u64
    }
}
pub(crate) fn result(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(hr::S_OK)
}
pub(crate) fn read_guid(p: u64) -> Result<Guid, u32> {
    let mut g = [0u8; 16];
    read(p, &mut g)?;
    Ok(g)
}

// ---- BSTR and VARIANT --------------------------------------------------------------------
/// A BSTR argument (None: NULL or empty).
pub(crate) fn bstr_in(b: u64) -> Option<String> {
    if b == 0 {
        return None;
    }
    let n = unsafe { SysStringLen(door_windows_abi::narrow(b)) } as usize;
    let mut bytes = vec![0u8; n * 2];
    read(b, &mut bytes).ok()?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Some(String::from_utf16_lossy(&units)).filter(|s| !s.is_empty())
}
pub(crate) fn bstr(text: &str) -> u64 {
    let units: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        door_windows_abi::widen(SysAllocStringLen(
            door_windows_abi::narrow(units.as_ptr() as u64),
            units.len() as u32,
        ))
    }
}
/// A BSTR out-parameter (NULL for None).
pub(crate) fn bstr_out(out: u64, text: Option<&str>) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    let b = text.map_or(0, bstr);
    result(write_pointer(out, b))
}
#[derive(Debug)]
pub(crate) enum Arg {
    Empty,
    Number(i32),
    Text(String),
}
/// A VARIANT argument (x64 passes one by value as its address).
pub(crate) fn variant(p: u64) -> Result<Arg, u32> {
    if p == 0 {
        return Ok(Arg::Empty);
    }
    let mut v = [0u8; 24];
    read(p, &mut v[..VARIANT_BYTES])?;
    let vt = u16::from_le_bytes([v[0], v[1]]);
    // Numbers come from the low bytes; BSTRs and BYREF targets are pointers.
    let data = pointer_at(&v, 8);
    Ok(match vt {
        0 | 1 | 10 => Arg::Empty,
        2 | 18 => Arg::Number(i32::from(data as i16)),
        3 | 19 | 22 | 23 => Arg::Number(data as i32),
        16 | 17 => Arg::Number(i32::from(data as u8)),
        8 => bstr_in(data).map_or(Arg::Empty, Arg::Text),
        // VT_BYREF | VT_VARIANT, VT_BYREF | VT_I4, VT_BYREF | VT_BSTR.
        0x400C => return variant(data),
        0x4003 => Arg::Number(read_u32(data)? as i32),
        0x4008 => bstr_in(read_pointer(data)?).map_or(Arg::Empty, Arg::Text),
        _ => return Err(hr::E_INVALIDARG),
    })
}
/// A VARIANT out-parameter holding VT_BSTR (or VT_EMPTY for None).
pub(crate) fn variant_out(out: u64, text: Option<&str>) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    let mut v = [0u8; 24];
    if let Some(t) = text {
        v[..2].copy_from_slice(&8u16.to_le_bytes());
        v[8..8 + POINTER].copy_from_slice(&bstr(t).to_le_bytes()[..POINTER]);
    }
    result(write(out, &v[..VARIANT_BYTES]))
}

// ---- GUIDs ---------------------------------------------------------------------------------
pub(crate) type Guid = [u8; 16];
const fn hex(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        _ => c - b'A' + 10,
    }
}
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` as stored.
pub(crate) const fn g(s: &str) -> Guid {
    let b = s.as_bytes();
    const fn byte(b: &[u8], at: usize) -> u8 {
        (hex(b[at]) << 4) | hex(b[at + 1])
    }
    [
        byte(b, 6),
        byte(b, 4),
        byte(b, 2),
        byte(b, 0),
        byte(b, 11),
        byte(b, 9),
        byte(b, 16),
        byte(b, 14),
        byte(b, 19),
        byte(b, 21),
        byte(b, 24),
        byte(b, 26),
        byte(b, 28),
        byte(b, 30),
        byte(b, 32),
        byte(b, 34),
    ]
}
pub(crate) mod iid {
    use super::{Guid, g};
    pub const UNKNOWN: Guid = g("00000000-0000-0000-C000-000000000046");
    pub const DISPATCH: Guid = g("00020400-0000-0000-C000-000000000046");
    pub const CLASS_FACTORY: Guid = g("00000001-0000-0000-C000-000000000046");
    pub const ENUM_VARIANT: Guid = g("00020404-0000-0000-C000-000000000046");
    pub const SERVICE: Guid = g("2faba4c7-4da9-4013-9697-20cc3fd40f85");
    pub const FOLDER: Guid = g("8cfac062-a080-4c15-9a88-aa7c2af80dfc");
    pub const FOLDERS: Guid = g("79184a66-8664-423f-97f1-637356a5d812");
    pub const REGISTERED: Guid = g("9c86f320-dee3-4dd1-b972-a303f26b061e");
    pub const REGISTERED_TASKS: Guid = g("86627eb4-42a7-41e4-a4d9-ac33a72f2d52");
    pub const RUNNING: Guid = g("653758fb-7b9a-4f1e-a471-beeb8e9b834e");
    pub const RUNNING_TASKS: Guid = g("6a67614b-6828-4fec-aa54-6d52e8f1f2db");
    pub const DEFINITION: Guid = g("f5bc8fc5-536d-4f77-b852-fbc1356fdeb6");
    pub const REGISTRATION_INFO: Guid = g("416D8B73-CB41-4ea1-805C-9BE9A5AC4A74");
    pub const TRIGGERS: Guid = g("85df5081-1b24-4f32-878a-d9d14df4cb77");
    pub const TRIGGER: Guid = g("09941815-ea89-4b5b-89e0-2a773801fac3");
    pub const REPETITION: Guid = g("7FB9ACF1-26BE-400e-85B5-294B9C75DFD6");
    pub const ACTIONS: Guid = g("02820E19-7B98-4ed2-B2E8-FDCCCEFF619B");
    pub const ACTION: Guid = g("BAE54997-48B1-4cbe-9965-D6BE263EBEA4");
    pub const EXEC: Guid = g("4c3d624d-fd6b-49a3-b9b7-09cb3cd3f047");
    pub const EXEC2: Guid = g("f2a82542-bda5-4e6b-9143-e2bf4F8987b6");
    pub const COM_HANDLER: Guid = g("6D2FD252-75C5-4f66-90BA-2A7D8CC3039F");
    pub const EMAIL: Guid = g("10F62C64-7E16-4314-A0C2-0C3683F99D40");
    pub const SHOW_MESSAGE: Guid = g("505E9E68-AF89-46b8-A30F-56162A83D537");
    pub const SETTINGS: Guid = g("8FD4711D-2D02-4c8c-87E3-EFF699DE127E");
    pub const SETTINGS2: Guid = g("2C05C3F0-6EED-4c05-A15F-ED7D7A98A369");
    pub const SETTINGS3: Guid = g("0AD9D0D7-0C7F-4EBB-9A5F-D1C648DCA528");
    pub const IDLE: Guid = g("84594461-0053-4342-A8FD-088FABF11F32");
    pub const NETWORK: Guid = g("9F7DEA84-C30B-4245-80B6-00E9F646F1B4");
    pub const MAINTENANCE: Guid = g("A6024FA8-9652-4ADB-A6BF-5CFCD877A7BA");
    pub const PRINCIPAL: Guid = g("D98D51E5-C9B4-496a-A9C1-18980261CF0F");
    pub const PRINCIPAL2: Guid = g("248919AE-E345-4A6D-8AEB-E0D3165C904E");
    pub const PAIRS: Guid = g("B4EF826B-63C3-46e4-A504-EF69E4F7EA4D");
    pub const PAIR: Guid = g("39038068-2B46-4afd-8662-7BB6F868D221");
    pub const IDLE_TRIGGER: Guid = g("d537d2b0-9fb3-4d34-9739-1ff5ce7b1ef3");
    pub const LOGON_TRIGGER: Guid = g("72DADE38-FAE4-4b3e-BAF4-5D009AF02B1C");
    pub const SESSION_TRIGGER: Guid = g("754DA71B-4385-4475-9DD9-598294FA3641");
    pub const EVENT_TRIGGER: Guid = g("d45b0167-9653-4eef-b94f-0732ca7af251");
    pub const TIME_TRIGGER: Guid = g("b45747e0-eba7-4276-9f29-85c5bb300006");
    pub const DAILY_TRIGGER: Guid = g("126c5cd8-b288-41d5-8dbf-e491446adc5c");
    pub const WEEKLY_TRIGGER: Guid = g("5038fc98-82ff-436d-8728-a512a57c9dc1");
    pub const MONTHLY_TRIGGER: Guid = g("97c45ef1-6b02-4a1a-9c0e-1ebfba1500ac");
    pub const MONTHLYDOW_TRIGGER: Guid = g("77d025a3-90fa-43aa-b52e-cda5499b946a");
    pub const BOOT_TRIGGER: Guid = g("2A9C35DA-D357-41f4-BBC1-207AC1B1F3CB");
    pub const REGISTRATION_TRIGGER: Guid = g("4c8fec3a-c218-4e0c-b23d-629024db91a2");
}
const CLSID_TASK_SCHEDULER: Guid = g("0f87369f-a4e5-4cfc-bd3e-73e6154572dd");

// ---- objects -----------------------------------------------------------------------------------
/// A vtable: function pointers of mixed signatures.
pub(crate) struct Table<const N: usize>(pub [*const (); N]);
unsafe impl<const N: usize> Sync for Table<N> {}
/// Every object: its interface tables (`second` is the secondary
/// interface, 8 bytes in), references and what it is.
#[repr(C)]
pub(crate) struct Obj {
    vtbl: *const (),
    second: *const (),
    refs: AtomicU32,
    /// The primary interface (the one IDispatch describes) and the IIDs
    /// QueryInterface answers with the primary and secondary tables.
    pub(crate) iid: Guid,
    primary: &'static [Guid],
    secondary: &'static [Guid],
    second_iid: Guid,
    pub(crate) kind: Kind,
}
pub(crate) enum Kind {
    Service(AtomicBool),
    Folder(String),
    /// A folder, registered task or running task collection: the items'
    /// COM objects (each holding a reference).
    Collection(Vec<u64>),
    Registered(String),
    Running(String, Guid),
    Enum(Vec<u64>, AtomicU64),
    Def(def::Part),
}
pub(crate) fn obj(this: u64) -> &'static Obj {
    // SAFETY: only our objects reach our tables.
    unsafe { &*(this as *const Obj) }
}
pub(crate) fn new_obj<const N: usize>(
    table: &'static Table<N>,
    iid: Guid,
    primary: &'static [Guid],
    kind: Kind,
) -> u64 {
    Box::into_raw(Box::new(Obj {
        vtbl: table.0.as_ptr() as *const (),
        second: core::ptr::null(),
        refs: AtomicU32::new(1),
        iid,
        primary,
        secondary: &[],
        second_iid: [0; 16],
        kind,
    })) as u64
}
pub(crate) fn add_ref(this: u64) -> u32 {
    obj(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
pub(crate) fn release(this: u64) -> u32 {
    if let Some(left) = def::release_item(this) {
        return left;
    }
    let left = obj(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference; the object came from Box::into_raw.
        let o = unsafe { Box::from_raw(this as *mut Obj) };
        match &o.kind {
            Kind::Collection(items) | Kind::Enum(items, _) => {
                for &i in items {
                    com_release(i);
                }
            }
            _ => {}
        }
    }
    left
}
/// Release of any COM pointer (an application's or ours).
pub(crate) fn com_release(p: u64) {
    if p == 0 {
        return;
    }
    if let Ok(table) = read_pointer(p)
        && let Ok(f) = read_pointer(table + 2 * POINTER as u64)
    {
        // SAFETY: IUnknown::Release.
        let f: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(p));
    }
}
pub(crate) fn com_add_ref(p: u64) {
    if let Ok(table) = read_pointer(p)
        && let Ok(f) = read_pointer(table + POINTER as u64)
    {
        // SAFETY: IUnknown::AddRef.
        let f: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(f as usize) };
        f(door_windows_abi::narrow(p));
    }
}
/// Whether `p` is one of this DLL's objects (its first table is ours).
pub(crate) fn ours(p: u64) -> bool {
    p != 0 && read_pointer(p).is_ok_and(|t| def::is_our_table(t) || is_table(t))
}
pub(crate) fn query(this: u64, iid: u64, out: u64) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let Ok(want) = read_guid(iid) else {
        return hr::E_POINTER;
    };
    let o = obj(this);
    let p = if want == iid::UNKNOWN || want == iid::DISPATCH || o.primary.contains(&want) {
        this
    } else if !o.second.is_null() && o.secondary.contains(&want) {
        this + 8
    } else {
        return hr::E_NOINTERFACE;
    };
    add_ref(this);
    result(write_pointer(out, p))
}
pub(crate) extern "system" fn q1(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this, iid, out)
}
pub(crate) extern "system" fn a1(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    add_ref(this)
}
pub(crate) extern "system" fn r1(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    release(this)
}
pub(crate) extern "system" fn q2(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    query(this - 8, iid, out)
}
pub(crate) extern "system" fn a2(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    add_ref(this - 8)
}
pub(crate) extern "system" fn r2(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    release(this - 8)
}

// ---- IDispatch through the type library ------------------------------------------------------
static LIBRARY: AtomicU64 = AtomicU64::new(0);
/// This DLL's ITypeLib (its TYPELIB resource).
fn library() -> Result<u64, u32> {
    let l = LIBRARY.load(Ordering::Acquire);
    if l != 0 {
        return Ok(l);
    }
    let path: Vec<u16> = door_windows_abi::modules::own::SYSTEM_DIRECTORY
        .encode_utf16()
        .chain("taskschd.dll".encode_utf16())
        .chain([0])
        .collect();
    let mut lib = 0u64;
    // REGKIND_NONE.
    let r = unsafe {
        LoadTypeLibEx(
            door_windows_abi::narrow(path.as_ptr() as u64),
            2,
            door_windows_abi::narrow(&mut lib as *mut u64 as u64),
        )
    };
    if r != 0 {
        return Err(r);
    }
    match LIBRARY.compare_exchange(0, lib, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(lib),
        Err(other) => {
            com_release(lib);
            Ok(other)
        }
    }
}
fn method(p: u64, index: u64) -> Result<u64, u32> {
    read_pointer(read_pointer(p)? + POINTER as u64 * index)
}
/// The interface's dispatch-side ITypeInfo (what GetTypeInfo gives) or
/// its vtable side (what DispInvoke calls through).
fn type_info(interface: &Guid, vtable: bool) -> Result<u64, u32> {
    let lib = library()?;
    let mut info = 0u64;
    // SAFETY: ITypeLib::GetTypeInfoOfGuid is method 6.
    let get: extern "system" fn(usize, usize, usize) -> u32 =
        unsafe { core::mem::transmute(method(lib, 6)? as usize) };
    let r = get(
        door_windows_abi::narrow(lib),
        door_windows_abi::narrow(interface.as_ptr() as u64),
        door_windows_abi::narrow(&mut info as *mut u64 as u64),
    );
    if r != 0 {
        return Err(r);
    }
    if !vtable {
        return Ok(info);
    }
    let mut href = 0u32;
    // ITypeInfo::GetRefTypeOfImplType (8) and GetRefTypeInfo (14).
    let of: extern "system" fn(usize, u32, usize) -> u32 =
        unsafe { core::mem::transmute(method(info, 8)? as usize) };
    let r = of(
        door_windows_abi::narrow(info),
        u32::MAX,
        door_windows_abi::narrow(&mut href as *mut u32 as u64),
    );
    if r != 0 {
        com_release(info);
        return Err(r);
    }
    let mut side = 0u64;
    let get: extern "system" fn(usize, u32, usize) -> u32 =
        unsafe { core::mem::transmute(method(info, 14)? as usize) };
    let r = get(
        door_windows_abi::narrow(info),
        href,
        door_windows_abi::narrow(&mut side as *mut u64 as u64),
    );
    com_release(info);
    if r != 0 {
        return Err(r);
    }
    Ok(side)
}
fn interface_of(this: u64, second: bool) -> (u64, Guid) {
    if second {
        (this - 8, obj(this - 8).second_iid)
    } else {
        (this, obj(this).iid)
    }
}
fn count(_this: u64, out: u64) -> u32 {
    result(write_u32(out, 1))
}
fn info(this: u64, second: bool, index: u32, out: u64) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    let _ = write_pointer(out, 0);
    if index != 0 {
        return 0x8002_000B; // DISP_E_BADINDEX
    }
    match type_info(&interface_of(this, second).1, false) {
        Ok(i) => result(write_pointer(out, i)),
        Err(e) => e,
    }
}
fn ids(this: u64, second: bool, names: u64, n: u32, out: u64) -> u32 {
    match type_info(&interface_of(this, second).1, true) {
        Ok(i) => {
            let r = unsafe {
                DispGetIDsOfNames(
                    door_windows_abi::narrow(i),
                    door_windows_abi::narrow(names),
                    n,
                    door_windows_abi::narrow(out),
                )
            };
            com_release(i);
            r
        }
        Err(e) => e,
    }
}
#[allow(clippy::too_many_arguments)]
fn invoke(
    this: u64,
    second: bool,
    member: i32,
    flags: u16,
    params: u64,
    out: u64,
    excep: u64,
    arg: u64,
) -> u32 {
    match type_info(&interface_of(this, second).1, true) {
        Ok(i) => {
            let r = unsafe {
                DispInvoke(
                    door_windows_abi::narrow(this),
                    door_windows_abi::narrow(i),
                    member,
                    flags,
                    door_windows_abi::narrow(params),
                    door_windows_abi::narrow(out),
                    door_windows_abi::narrow(excep),
                    door_windows_abi::narrow(arg),
                )
            };
            com_release(i);
            r
        }
        Err(e) => e,
    }
}
pub(crate) extern "system" fn c1(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    count(this, out)
}
pub(crate) extern "system" fn i1(this: usize, index: u32, _lcid: u32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    info(this, false, index, out)
}
pub(crate) extern "system" fn n1(
    this: usize,
    _riid: usize,
    names: usize,
    n: u32,
    _lcid: u32,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let names = door_windows_abi::widen(names);
    let out = door_windows_abi::widen(out);
    ids(this, false, names, n, out)
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn v1(
    this: usize,
    member: i32,
    _riid: usize,
    _lcid: u32,
    flags: u16,
    params: usize,
    out: usize,
    excep: usize,
    arg: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let params = door_windows_abi::widen(params);
    let out = door_windows_abi::widen(out);
    let excep = door_windows_abi::widen(excep);
    let arg = door_windows_abi::widen(arg);
    invoke(this, false, member, flags, params, out, excep, arg)
}
pub(crate) extern "system" fn c2(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    count(this, out)
}
pub(crate) extern "system" fn i2(this: usize, index: u32, _lcid: u32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    info(this, true, index, out)
}
pub(crate) extern "system" fn n2(
    this: usize,
    _riid: usize,
    names: usize,
    n: u32,
    _lcid: u32,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let names = door_windows_abi::widen(names);
    let out = door_windows_abi::widen(out);
    ids(this, true, names, n, out)
}
#[allow(clippy::too_many_arguments)]
pub(crate) extern "system" fn v2(
    this: usize,
    member: i32,
    _riid: usize,
    _lcid: u32,
    flags: u16,
    params: usize,
    out: usize,
    excep: usize,
    arg: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let params = door_windows_abi::widen(params);
    let out = door_windows_abi::widen(out);
    let excep = door_windows_abi::widen(excep);
    let arg = door_windows_abi::widen(arg);
    invoke(this, true, member, flags, params, out, excep, arg)
}
/// IUnknown and IDispatch, the first seven entries of every table.
macro_rules! dispatch1 {
    ($($rest:expr),* $(,)?) => {
        Table([
            q1 as *const (), a1 as *const (), r1 as *const (), c1 as *const (), i1 as *const (),
            n1 as *const (), v1 as *const (), $($rest as *const ()),*
        ])
    };
}
macro_rules! dispatch2 {
    ($($rest:expr),* $(,)?) => {
        Table([
            q2 as *const (), a2 as *const (), r2 as *const (), c2 as *const (), i2 as *const (),
            n2 as *const (), v2 as *const (), $($rest as *const ()),*
        ])
    };
}
pub(crate) use {dispatch1, dispatch2};

// ---- the scheduler -------------------------------------------------------------------------
static CLIENT: Lock<Option<Tasks>> = Lock::new(None);
pub(crate) fn scheduler<R>(f: impl FnOnce(&mut Tasks) -> Result<R, Error>) -> Result<R, u32> {
    CLIENT.with(|c| {
        if c.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *c = Some(Tasks::connect(registry).map_err(|_| hr::SERVER_UNAVAILABLE)?);
        }
        let client = c.as_mut().ok_or(hr::SERVER_UNAVAILABLE)?;
        let r = f(client);
        if r.is_err() && client.needs_reconnect() {
            *c = None;
        }
        r.map_err(|e| match e {
            Error::Service(Code::NotFound) => hr::FILE_NOT_FOUND,
            Error::Service(Code::Denied) => hr::E_ACCESSDENIED,
            Error::Service(Code::Exists) => hr::ALREADY_EXISTS,
            Error::Service(Code::Resources) => hr::E_OUTOFMEMORY,
            Error::Service(Code::InvalidRequest) => hr::E_INVALIDARG,
            _ => hr::SERVER_UNAVAILABLE,
        })
    })
}
/// Every task the caller sees.
pub(crate) fn all_tasks() -> Result<Vec<wire::Status>, u32> {
    let mut out = Vec::new();
    for i in 0..wire::TASKS as u16 {
        match scheduler(|t| t.query(i))? {
            Some(s) => out.push(s),
            None => break,
        }
    }
    Ok(out)
}
pub(crate) fn find_task(name: &str) -> Result<wire::Status, u32> {
    all_tasks()?
        .into_iter()
        .find(|s| s.task.name().eq_ignore_ascii_case(name))
        .ok_or(hr::FILE_NOT_FOUND)
}
/// The task's XML as Init keeps it (None: a task schtasks made).
pub(crate) fn stored_xml(name: &str) -> Option<String> {
    let mut buffer = vec![0u8; wire::XML_BYTES];
    let n = scheduler(|t| t.get_xml(name, &mut buffer)).ok()?;
    (n > 0).then(|| String::from_utf8_lossy(&buffer[..n]).into_owned())
}
/// The definition of a registered task: its XML, or one made from what
/// the scheduler keeps; the scheduler's enabled state either way.
pub(crate) fn registered_definition(s: &wire::Status) -> Definition {
    let name = s.task.name();
    let mut d = stored_xml(name)
        .and_then(|x| xml::read(&x).ok())
        .unwrap_or_else(|| synthesize(s));
    d.settings.enabled = s.state != wire::Status::DISABLED;
    d
}
/// A definition for a task schtasks created: one Exec action and the
/// trigger its schedule is.
fn synthesize(s: &wire::Status) -> Definition {
    use door_service_protocol::tasks::Schedule;
    let mut d = Definition::from_defaults();
    let command = s.task.command();
    let (path, arguments) = split_command(command);
    let mut a = Action::new(action::EXEC);
    a.path = Some(path);
    a.arguments = arguments;
    d.actions.push(a);
    let start = local_text(s.task.start);
    let t = match s.task.schedule {
        Schedule::Once => {
            let mut t = Trigger::new(trigger::TIME);
            t.start = start;
            t
        }
        Schedule::Minutes(n) | Schedule::Hours(n) => {
            let mut t = Trigger::new(trigger::TIME);
            t.start = start;
            let unit = if matches!(s.task.schedule, Schedule::Minutes(_)) {
                'M'
            } else {
                'H'
            };
            t.repetition.interval = Some(alloc::format!("PT{n}{unit}"));
            t
        }
        Schedule::Days(n) => {
            let mut t = Trigger::new(trigger::DAILY);
            t.start = start;
            t.days_interval = n as i16;
            t
        }
        Schedule::OnLogon => Trigger::new(trigger::LOGON),
    };
    d.triggers.push(t);
    d
}
/// A command line's program (quotes removed) and its arguments.
pub(crate) fn split_command(command: &str) -> (String, Option<String>) {
    let c = command.trim();
    let (path, rest) = if let Some(q) = c.strip_prefix('"') {
        match q.find('"') {
            Some(i) => (&q[..i], &q[i + 1..]),
            None => (q, ""),
        }
    } else {
        match c.find(' ') {
            Some(i) => (&c[..i], &c[i..]),
            None => (c, ""),
        }
    };
    let rest = rest.trim_start();
    (
        String::from(path),
        (!rest.is_empty()).then(|| String::from(rest)),
    )
}

// ---- time ----------------------------------------------------------------------------------
static ZONE: AtomicU32 = AtomicU32::new(u32::MAX);
fn zone() -> &'static door_locale::tz::Zone {
    let mut id = ZONE.load(Ordering::Relaxed);
    if id == u32::MAX {
        let registry =
            unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
        id = door_service_client::Storage::connect(registry)
            .and_then(|mut s| s.get_setting(door_service_client::Setting::TimeZone))
            .unwrap_or(u32::from(door_locale::tz::DEFAULT_ID));
        ZONE.store(id, Ordering::Relaxed);
    }
    door_locale::tz::zone_or_utc(id)
}
const FILETIME_PER_DAY: u64 = 864_000_000_000;
/// A UTC FILETIME as a local DATE (days since 1899-12-30); 0 for none.
pub(crate) fn local_date(utc: u64) -> f64 {
    if utc == 0 {
        return 0.0;
    }
    let (offset, _) = zone().offset_at(utc);
    let local = utc as i64 + i64::from(offset) * 600_000_000;
    local as f64 / FILETIME_PER_DAY as f64 - 109_205.0
}
/// A UTC FILETIME as a boundary in local time (`YYYY-MM-DDTHH:MM:SS`).
fn local_text(utc: u64) -> Option<String> {
    if utc == 0 {
        return None;
    }
    let l = zone().to_local(utc)?.time;
    Some(alloc::format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        l.year,
        l.month,
        l.day,
        l.hour,
        l.minute,
        l.second
    ))
}
/// A boundary as a UTC FILETIME: its own offset, or the time zone's rules.
pub(crate) fn boundary_utc(b: &Boundary) -> Option<u64> {
    let wall = door_abi::clock::WallClock {
        year: b.year,
        month: b.month,
        day: b.day,
        hour: b.hour,
        minute: b.minute,
        second: b.second,
    };
    match b.offset {
        Some(o) => {
            let ft = wall.to_filetime()? as i64 - i64::from(o) * 600_000_000;
            u64::try_from(ft).ok()
        }
        None => match zone().to_utc(wall) {
            door_locale::tz::LocalResult::Unique(t) => Some(t),
            door_locale::tz::LocalResult::Ambiguous(t, _) => Some(t),
            door_locale::tz::LocalResult::Nonexistent => None,
        },
    }
}

// ---- the class factory --------------------------------------------------------------------
static FACTORY: Table<5> = Table([
    f_query as *const (),
    f_count as *const (),
    f_count as *const (),
    f_create as *const (),
    f_lock as *const (),
]);
#[repr(C)]
struct Factory {
    vtbl: *const (),
}
unsafe impl Sync for Factory {}
static THE_FACTORY: Factory = Factory {
    vtbl: FACTORY.0.as_ptr() as *const (),
};
extern "system" fn f_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        match read_guid(iid) {
            Ok(g) if g == iid::UNKNOWN || g == iid::CLASS_FACTORY => {
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
        let s = new_obj(
            &SERVICE,
            iid::SERVICE,
            &[iid::SERVICE],
            Kind::Service(AtomicBool::new(false)),
        );
        let r = query(s, iid, out);
        release(s);
        r
    }
}
extern "system" fn f_lock(_this: usize, _lock: i32) -> u32 {
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(clsid: usize, iid: usize, out: usize) -> u32 {
    let clsid = door_windows_abi::widen(clsid);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        match read_guid(clsid) {
            Ok(g) if g == CLSID_TASK_SCHEDULER => f_query(
                door_windows_abi::narrow(&THE_FACTORY as *const Factory as u64),
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
    hr::S_FALSE
}

// ---- ITaskService --------------------------------------------------------------------------
static SERVICE: Table<16> = dispatch1!(
    service_get_folder,
    service_running,
    service_new_task,
    service_connect,
    service_connected,
    service_server,
    service_user,
    service_domain,
    service_version,
);
fn connected(this: u64) -> bool {
    matches!(&obj(this).kind, Kind::Service(c) if c.load(Ordering::Acquire))
}
extern "system" fn service_get_folder(this: usize, path: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        open_folder(&String::from("\\"), bstr_in(path).as_deref(), out)
    }
}
extern "system" fn service_running(this: usize, flags: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        running_collection(out, |_| true, flags)
    }
}
extern "system" fn service_new_task(_this: usize, flags: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        if flags != 0 {
            return hr::E_INVALIDARG;
        }
        result(write_pointer(
            out,
            def::new_definition(Definition::new_task()),
        ))
    }
}
extern "system" fn service_connect(
    this: usize,
    server: VariantArg,
    user: VariantArg,
    domain: VariantArg,
    password: VariantArg,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let (server, user, domain, password) = (server.at(), user.at(), domain.at(), password.at());
    {
        let local = computer_name();
        match variant(server) {
            Ok(Arg::Empty) => {}
            Ok(Arg::Text(s)) if s.trim_start_matches('\\').eq_ignore_ascii_case(&local) => {}
            Ok(Arg::Text(_)) => return hr::SERVER_UNAVAILABLE,
            _ => return hr::E_INVALIDARG,
        }
        // Credentials only for another computer.
        for v in [user, domain, password] {
            if !matches!(variant(v), Ok(Arg::Empty)) {
                return hr::E_ACCESSDENIED;
            }
        }
        if let Err(e) = scheduler(|t| t.query(0).map(|_| ())) {
            return e;
        }
        if let Kind::Service(c) = &obj(this).kind {
            c.store(true, Ordering::Release);
        }
        hr::S_OK
    }
}
extern "system" fn service_connected(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u16(out, if connected(this) { 0xFFFF } else { 0 }))
}
pub(crate) fn computer_name() -> String {
    let mut units = [0u16; 64];
    let mut n = 0u32;
    unsafe { DoorGetComputerName(units.as_mut_ptr() as u64, 64, &mut n as *mut u32 as u64) };
    String::from_utf16_lossy(&units[..(n as usize).min(64)])
}
fn user_name() -> String {
    let name: Vec<u16> = "USERNAME".encode_utf16().chain([0]).collect();
    let mut units = [0u16; 128];
    let n = unsafe {
        DoorGetEnvironmentVariableW(name.as_ptr() as u64, units.as_mut_ptr() as u64, 128)
    };
    String::from_utf16_lossy(&units[..(n as usize).min(128)])
}
extern "system" fn service_server(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        bstr_out(out, Some(&computer_name()))
    }
}
extern "system" fn service_user(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        bstr_out(out, Some(&user_name()))
    }
}
extern "system" fn service_domain(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        bstr_out(out, Some(&computer_name()))
    }
}
extern "system" fn service_version(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if !connected(this) {
            return hr::ONLY_IF_CONNECTED;
        }
        // Task Scheduler 1.6, Windows 10 and later.
        result(write_u32(out, 0x0001_0006))
    }
}

// ---- folders ---------------------------------------------------------------------------------
static FOLDER: Table<20> = dispatch1!(
    folder_name,
    folder_path,
    folder_get_folder,
    folder_get_folders,
    folder_create,
    folder_delete,
    folder_get_task,
    folder_get_tasks,
    folder_delete_task,
    folder_register,
    folder_register_definition,
    folder_get_sd,
    folder_set_sd,
);
/// Folders created this session that hold no task yet (deviation: an
/// empty folder is not kept by the scheduler).
static EMPTY_FOLDERS: Lock<Vec<String>> = Lock::new(Vec::new());
fn folder_of(this: u64) -> &'static str {
    match &obj(this).kind {
        Kind::Folder(p) => p,
        _ => "\\",
    }
}
/// `path` (absolute, or relative to `base`) as `\A\B` (`\` for the root).
fn resolve(base: &str, path: Option<&str>) -> Result<String, u32> {
    let path = path.unwrap_or("");
    let mut parts: Vec<&str> = if path.starts_with('\\') {
        Vec::new()
    } else {
        base.split('\\').filter(|p| !p.is_empty()).collect()
    };
    for p in path.split('\\').filter(|p| !p.is_empty()) {
        match p {
            "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let mut out = String::new();
    for p in parts {
        out.push('\\');
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('\\');
    }
    Ok(out)
}
/// A scheduler task name for `path`: `A\B\Task` without the leading `\`.
fn task_name(base: &str, path: Option<&str>) -> Result<String, u32> {
    let full = resolve(base, path)?;
    if full == "\\" {
        return Err(hr::E_INVALIDARG);
    }
    Ok(String::from(&full[1..]))
}
fn folder_exists(path: &str) -> Result<bool, u32> {
    if path == "\\" {
        return Ok(true);
    }
    let prefix = alloc::format!("{}\\", &path[1..]);
    if EMPTY_FOLDERS.with(|f| f.iter().any(|x| x.eq_ignore_ascii_case(path))) {
        return Ok(true);
    }
    Ok(all_tasks()?
        .iter()
        .any(|s| starts_with_ci(s.task.name(), &prefix)))
}
fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}
fn open_folder(base: &str, path: Option<&str>, out: u64) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let full = match resolve(base, path) {
        Ok(f) => f,
        Err(e) => return e,
    };
    match folder_exists(&full) {
        Ok(true) => result(write_pointer(out, new_folder(full))),
        Ok(false) => hr::FILE_NOT_FOUND,
        Err(e) => e,
    }
}
pub(crate) fn new_folder(path: String) -> u64 {
    new_obj(&FOLDER, iid::FOLDER, &[iid::FOLDER], Kind::Folder(path))
}
extern "system" fn folder_name(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let p = folder_of(this);
        bstr_out(
            out,
            Some(if p == "\\" {
                "\\"
            } else {
                p.rsplit('\\').next().unwrap_or(p)
            }),
        )
    }
}
extern "system" fn folder_path(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    bstr_out(out, Some(folder_of(this)))
}
extern "system" fn folder_get_folder(this: usize, path: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    open_folder(folder_of(this), bstr_in(path).as_deref(), out)
}
/// The immediate subfolders of `path`.
fn subfolders(path: &str) -> Result<Vec<String>, u32> {
    let prefix = if path == "\\" {
        String::new()
    } else {
        alloc::format!("{}\\", &path[1..])
    };
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !starts_with_ci(name, &prefix) {
            return;
        }
        if let Some(child) = name[prefix.len()..].split('\\').next()
            && name[prefix.len()..].contains('\\')
        {
            let full = alloc::format!("\\{}{}", prefix, child);
            if !out.iter().any(|x| x.eq_ignore_ascii_case(&full)) {
                out.push(full);
            }
        }
    };
    for s in all_tasks()? {
        add(s.task.name());
    }
    for f in EMPTY_FOLDERS.with(|f| f.clone()) {
        let name = alloc::format!("{}\\x", &f[1..]);
        add(&name);
    }
    Ok(out)
}
extern "system" fn folder_get_folders(this: usize, _flags: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match subfolders(folder_of(this)) {
            Ok(list) => {
                let items = list.into_iter().map(new_folder).collect();
                result(write_pointer(
                    out,
                    new_obj(
                        &COLLECTION,
                        iid::FOLDERS,
                        &[iid::FOLDERS],
                        Kind::Collection(items),
                    ),
                ))
            }
            Err(e) => e,
        }
    }
}
extern "system" fn folder_create(this: usize, name: usize, _sddl: VariantArg, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    {
        let full = match resolve(folder_of(this), bstr_in(name).as_deref()) {
            Ok(f) if f != "\\" => f,
            Ok(_) => return hr::E_INVALIDARG,
            Err(e) => return e,
        };
        if full[1..].split('\\').any(|p| !wire::valid_name(p)) {
            return hr::INVALID_NAME;
        }
        match folder_exists(&full) {
            Ok(true) => return hr::ALREADY_EXISTS,
            Ok(false) => {}
            Err(e) => return e,
        }
        EMPTY_FOLDERS.with(|f| f.push(full.clone()));
        if out != 0 {
            return result(write_pointer(out, new_folder(full)));
        }
        hr::S_OK
    }
}
extern "system" fn folder_delete(this: usize, name: usize, flags: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    {
        if flags != 0 {
            return hr::E_INVALIDARG;
        }
        let full = match resolve(folder_of(this), bstr_in(name).as_deref()) {
            Ok(f) if f != "\\" => f,
            Ok(_) => return hr::E_ACCESSDENIED,
            Err(e) => return e,
        };
        let prefix = alloc::format!("{}\\", &full[1..]);
        match all_tasks() {
            Ok(t) if t.iter().any(|s| starts_with_ci(s.task.name(), &prefix)) => {
                return hr::DIR_NOT_EMPTY;
            }
            Ok(_) => {}
            Err(e) => return e,
        }
        let removed = EMPTY_FOLDERS.with(|f| {
            let n = f.len();
            f.retain(|x| !x.eq_ignore_ascii_case(&full));
            n != f.len()
        });
        if removed {
            hr::S_OK
        } else {
            hr::FILE_NOT_FOUND
        }
    }
}
extern "system" fn folder_get_task(this: usize, path: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let name = match task_name(folder_of(this), bstr_in(path).as_deref()) {
            Ok(n) => n,
            Err(_) => return hr::FILE_NOT_FOUND,
        };
        match find_task(&name) {
            Ok(s) => result(write_pointer(
                out,
                new_registered(String::from(s.task.name())),
            )),
            Err(e) => e,
        }
    }
}
/// TASK_ENUM_HIDDEN.
const ENUM_HIDDEN: i32 = 1;
extern "system" fn folder_get_tasks(this: usize, flags: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let folder = folder_of(this);
        let prefix = if folder == "\\" {
            String::new()
        } else {
            alloc::format!("{}\\", &folder[1..])
        };
        let tasks = match all_tasks() {
            Ok(t) => t,
            Err(e) => return e,
        };
        let mut items = Vec::new();
        for s in tasks {
            let name = s.task.name();
            if !starts_with_ci(name, &prefix) || name[prefix.len()..].contains('\\') {
                continue;
            }
            if flags & ENUM_HIDDEN == 0 && registered_definition(&s).settings.hidden {
                continue;
            }
            items.push(new_registered(String::from(name)));
        }
        result(write_pointer(
            out,
            new_obj(
                &COLLECTION,
                iid::REGISTERED_TASKS,
                &[iid::REGISTERED_TASKS],
                Kind::Collection(items),
            ),
        ))
    }
}
extern "system" fn folder_delete_task(this: usize, name: usize, flags: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    {
        if flags != 0 {
            return hr::E_INVALIDARG;
        }
        let name = match task_name(folder_of(this), bstr_in(name).as_deref()) {
            Ok(n) => n,
            Err(_) => return hr::FILE_NOT_FOUND,
        };
        let task = match find_task(&name) {
            Ok(s) => String::from(s.task.name()),
            Err(e) => return e,
        };
        result(scheduler(|t| t.delete(&task)))
    }
}
/// TASK_CREATION flags.
const VALIDATE_ONLY: i32 = 1;
const CREATE: i32 = 2;
const UPDATE: i32 = 4;
const DISABLE: i32 = 8;
const CREATION_FLAGS: i32 = 0x3F;
/// RegisterTask and RegisterTaskDefinition: `d` registered as `path`.
#[allow(clippy::too_many_arguments)]
fn register(
    base: &str,
    path: Option<String>,
    d: Definition,
    flags: i32,
    user: u64,
    password: u64,
    logon: u32,
    out: u64,
) -> u32 {
    if out != 0 {
        let _ = write_pointer(out, 0);
    }
    if flags & !CREATION_FLAGS != 0
        || logon > 6
        || (flags & VALIDATE_ONLY != 0 && flags & (CREATE | UPDATE) != 0)
    {
        return hr::E_INVALIDARG;
    }
    if let Err(e) = d.validate() {
        return e;
    }
    if flags & VALIDATE_ONLY != 0 {
        return hr::S_OK;
    }
    if flags & (CREATE | UPDATE) == 0 {
        return hr::E_INVALIDARG;
    }
    // Another account's credentials: the scheduler runs a task as its
    // creator only.
    let named_user = match variant(user) {
        Ok(Arg::Text(u)) => Some(u),
        Ok(_) => None,
        Err(e) => return e,
    };
    if named_user
        .as_deref()
        .is_some_and(|u| !u.eq_ignore_ascii_case(&user_name()))
        || !matches!(variant(password), Ok(Arg::Empty))
    {
        return hr::SCHED_E_UNSUPPORTED_ACCOUNT_OPTION;
    }
    let name = match task_name(base, path.as_deref()) {
        Ok(n) => n,
        Err(e) => return e,
    };
    if name.len() > wire::NAME_BYTES {
        return hr::NAME_TOO_LONG;
    }
    if !wire::valid_name(&name) {
        return hr::INVALID_NAME;
    }
    let plan = match plan(&d) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let start = match plan.start.as_ref() {
        Some(b) => match boundary_utc(b) {
            Some(t) => t,
            None => return hr::SCHED_E_INVALIDVALUE,
        },
        None => 0,
    };
    let exists = match find_task(&name) {
        Ok(_) => true,
        Err(hr::FILE_NOT_FOUND) => false,
        Err(e) => return e,
    };
    if exists && flags & UPDATE == 0 {
        return hr::ALREADY_EXISTS;
    }
    if !exists && flags & CREATE == 0 {
        return hr::FILE_NOT_FOUND;
    }
    let Some(mut task) = wire::Task::new(&name, &plan.command, plan.schedule, start) else {
        return hr::E_INVALIDARG;
    };
    task.replace = exists;
    let enabled = plan.enabled && flags & DISABLE == 0;
    let mut stored = d.clone();
    stored.settings.enabled = enabled;
    let text = xml::write(&stored);
    let r = scheduler(|t| {
        t.create(&task)?;
        if !enabled {
            t.enable(&name, false)?;
        }
        t.put_xml(&name, text.as_bytes())
    });
    if let Err(e) = r {
        return e;
    }
    EMPTY_FOLDERS.with(|f| f.retain(|x| !starts_with_ci(&name, &alloc::format!("{}\\", &x[1..]))));
    if out == 0 {
        return hr::S_OK;
    }
    result(write_pointer(out, new_registered(name)))
}
#[allow(clippy::too_many_arguments)]
extern "system" fn folder_register(
    this: usize,
    path: usize,
    text: usize,
    flags: i32,
    user: VariantArg,
    password: VariantArg,
    logon: u32,
    _sddl: VariantArg,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let path = door_windows_abi::widen(path);
    let text = door_windows_abi::widen(text);
    let user = user.at();
    let password = password.at();
    let out = door_windows_abi::widen(out);
    {
        let Some(text) = bstr_in(text) else {
            return hr::E_INVALIDARG;
        };
        match xml::read(&text) {
            Ok(d) => register(
                folder_of(this),
                bstr_in(path),
                d,
                flags,
                user,
                password,
                logon,
                out,
            ),
            Err(e) => e,
        }
    }
}
#[allow(clippy::too_many_arguments)]
extern "system" fn folder_register_definition(
    this: usize,
    path: usize,
    definition: usize,
    flags: i32,
    user: VariantArg,
    password: VariantArg,
    logon: u32,
    _sddl: VariantArg,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let path = door_windows_abi::widen(path);
    let definition = door_windows_abi::widen(definition);
    let user = user.at();
    let password = password.at();
    let out = door_windows_abi::widen(out);
    {
        let Some(d) = def::definition_of(definition) else {
            return hr::E_INVALIDARG;
        };
        register(
            folder_of(this),
            bstr_in(path),
            d,
            flags,
            user,
            password,
            logon,
            out,
        )
    }
}
/// Security descriptors of tasks and folders are not kept (deviation:
/// the scheduler checks the creator or an administrator).
extern "system" fn folder_get_sd(_this: usize, _info: i32, _out: usize) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn folder_set_sd(_this: usize, _sddl: usize, _flags: i32) -> u32 {
    hr::E_NOTIMPL
}

// ---- collections and enumerators ---------------------------------------------------------------
static COLLECTION: Table<10> = dispatch1!(coll_count, coll_item, coll_enum);
fn items(this: u64) -> &'static [u64] {
    match &obj(this).kind {
        Kind::Collection(v) | Kind::Enum(v, _) => v,
        _ => &[],
    }
}
extern "system" fn coll_count(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, items(this).len() as u32))
}
extern "system" fn coll_item(this: usize, index: VariantArg, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = index.at();
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        let _ = write_pointer(out, 0);
        let list = items(this);
        let found = match variant(index) {
            Ok(Arg::Number(n)) if n >= 1 => list.get(n as usize - 1).copied(),
            Ok(Arg::Number(_)) => None,
            Ok(Arg::Text(name)) => list.iter().copied().find(|&p| match &obj(p).kind {
                Kind::Folder(f) | Kind::Registered(f) | Kind::Running(f, _) => {
                    f.rsplit('\\')
                        .next()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                        || f.eq_ignore_ascii_case(&name)
                }
                _ => false,
            }),
            Ok(Arg::Empty) => None,
            Err(e) => return e,
        };
        match found {
            Some(p) => {
                add_ref(p);
                result(write_pointer(out, p))
            }
            None => hr::E_INVALIDARG,
        }
    }
}
extern "system" fn coll_enum(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        result(write_pointer(out, new_enum(items(this).to_vec())))
    }
}
/// IEnumVARIANT over `items` (each AddRef'd here).
pub(crate) fn new_enum(items: Vec<u64>) -> u64 {
    for &i in &items {
        com_add_ref(i);
    }
    Box::into_raw(Box::new(Obj {
        vtbl: ENUM.0.as_ptr() as *const (),
        second: core::ptr::null(),
        refs: AtomicU32::new(1),
        iid: iid::ENUM_VARIANT,
        primary: &[iid::ENUM_VARIANT],
        secondary: &[],
        second_iid: [0; 16],
        kind: Kind::Enum(items, AtomicU64::new(0)),
    })) as u64
}
static ENUM: Table<7> = Table([
    q1 as *const (),
    a1 as *const (),
    r1 as *const (),
    enum_next as *const (),
    enum_skip as *const (),
    enum_reset as *const (),
    enum_clone as *const (),
]);
fn position(this: u64) -> &'static AtomicU64 {
    match &obj(this).kind {
        Kind::Enum(_, at) => at,
        _ => unreachable!(),
    }
}
extern "system" fn enum_next(this: usize, n: u32, out: usize, fetched: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let fetched = door_windows_abi::widen(fetched);
    {
        let list = items(this);
        let at = position(this);
        let mut k = 0u32;
        while k < n {
            let i = at.load(Ordering::Acquire) as usize;
            let Some(&p) = list.get(i) else {
                break;
            };
            at.store(i as u64 + 1, Ordering::Release);
            com_add_ref(p);
            let mut v = [0u8; 24];
            // VT_DISPATCH.
            v[..2].copy_from_slice(&9u16.to_le_bytes());
            v[8..8 + POINTER].copy_from_slice(&p.to_le_bytes()[..POINTER]);
            if write(
                out + u64::from(k) * VARIANT_BYTES as u64,
                &v[..VARIANT_BYTES],
            )
            .is_err()
            {
                return hr::E_POINTER;
            }
            k += 1;
        }
        if fetched != 0 {
            let _ = write_u32(fetched, k);
        }
        if k == n { hr::S_OK } else { hr::S_FALSE }
    }
}
extern "system" fn enum_skip(this: usize, n: u32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let at = position(this);
        let len = items(this).len() as u64;
        let next = at.load(Ordering::Acquire) + u64::from(n);
        at.store(next.min(len), Ordering::Release);
        if next <= len { hr::S_OK } else { hr::S_FALSE }
    }
}
extern "system" fn enum_reset(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        position(this).store(0, Ordering::Release);
        hr::S_OK
    }
}
extern "system" fn enum_clone(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let e = new_enum(items(this).to_vec());
        position(e).store(position(this).load(Ordering::Acquire), Ordering::Release);
        result(write_pointer(out, e))
    }
}

// ---- registered tasks ------------------------------------------------------------------------
static REGISTERED: Table<25> = dispatch1!(
    task_name_get,
    task_path,
    task_state,
    task_enabled,
    task_put_enabled,
    task_run,
    task_run_ex,
    task_instances,
    task_last_run,
    task_last_result,
    task_missed,
    task_next_run,
    task_definition,
    task_xml,
    task_get_sd,
    task_set_sd,
    task_stop,
    task_run_times,
);
pub(crate) fn new_registered(name: String) -> u64 {
    new_obj(
        &REGISTERED,
        iid::REGISTERED,
        &[iid::REGISTERED],
        Kind::Registered(name),
    )
}
fn registered(this: u64) -> &'static str {
    match &obj(this).kind {
        Kind::Registered(n) | Kind::Running(n, _) => n,
        _ => "",
    }
}
fn status(this: u64) -> Result<wire::Status, u32> {
    find_task(registered(this))
}
extern "system" fn task_name_get(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let n = registered(this);
        bstr_out(out, Some(n.rsplit('\\').next().unwrap_or(n)))
    }
}
extern "system" fn task_path(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    bstr_out(out, Some(&alloc::format!("\\{}", registered(this))))
}
/// TASK_STATE of a scheduler status.
fn state(s: &wire::Status) -> u32 {
    match s.state {
        wire::Status::RUNNING => 4,
        wire::Status::DISABLED => 1,
        _ => 3,
    }
}
extern "system" fn task_state(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write_u32(out, state(&s))),
            Err(e) => e,
        }
    }
}
extern "system" fn task_enabled(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write_u16(
                out,
                if s.state == wire::Status::DISABLED {
                    0
                } else {
                    0xFFFF
                },
            )),
            Err(e) => e,
        }
    }
}
extern "system" fn task_put_enabled(this: usize, enabled: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let enabled = door_windows_abi::widen(enabled);
    {
        let name = registered(this);
        let on = enabled as u16 != 0;
        let r = scheduler(|t| t.enable(name, on));
        if r.is_ok()
            && let Some(text) = stored_xml(name)
            && let Ok(mut d) = xml::read(&text)
        {
            d.settings.enabled = on;
            let _ = scheduler(|t| t.put_xml(name, xml::write(&d).as_bytes()));
        }
        result(r)
    }
}
fn run(this: u64, out: u64) -> u32 {
    if out != 0 {
        let _ = write_pointer(out, 0);
    }
    let s = match status(this) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if s.state == wire::Status::DISABLED {
        return hr::SCHED_E_TASK_DISABLED;
    }
    if !registered_definition(&s).settings.demand_start {
        return hr::SCHED_E_START_ON_DEMAND;
    }
    let name = String::from(registered(this));
    if let Err(e) = scheduler(|t| t.run(&name)) {
        return e;
    }
    if out == 0 {
        return hr::S_OK;
    }
    result(write_pointer(out, new_running(name)))
}
extern "system" fn task_run(this: usize, _params: VariantArg, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    run(this, out)
}
extern "system" fn task_run_ex(
    this: usize,
    _params: VariantArg,
    _flags: i32,
    _session: i32,
    _user: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    run(this, out)
}
fn running_collection(out: u64, filter: impl Fn(&str) -> bool, _flags: i32) -> u32 {
    if out == 0 {
        return hr::E_POINTER;
    }
    match all_tasks() {
        Ok(tasks) => {
            let items = tasks
                .iter()
                .filter(|s| s.state == wire::Status::RUNNING && filter(s.task.name()))
                .map(|s| new_running(String::from(s.task.name())))
                .collect();
            result(write_pointer(
                out,
                new_obj(
                    &COLLECTION,
                    iid::RUNNING_TASKS,
                    &[iid::RUNNING_TASKS],
                    Kind::Collection(items),
                ),
            ))
        }
        Err(e) => e,
    }
}
extern "system" fn task_instances(this: usize, flags: i32, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let name = String::from(registered(this));
        running_collection(out, |n| n.eq_ignore_ascii_case(&name), flags)
    }
}
extern "system" fn task_last_run(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write(out, &local_date(s.last_run).to_le_bytes())),
            Err(e) => e,
        }
    }
}
extern "system" fn task_last_result(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write_u32(out, s.last_result)),
            Err(e) => e,
        }
    }
}
extern "system" fn task_missed(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(_) => result(write_u32(out, 0)),
            Err(e) => e,
        }
    }
}
extern "system" fn task_next_run(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write(out, &local_date(s.next_run).to_le_bytes())),
            Err(e) => e,
        }
    }
}
extern "system" fn task_definition(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_POINTER;
        }
        match status(this) {
            Ok(s) => result(write_pointer(
                out,
                def::new_definition(registered_definition(&s)),
            )),
            Err(e) => e,
        }
    }
}
extern "system" fn task_xml(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => bstr_out(out, Some(&xml::write(&registered_definition(&s)))),
            Err(e) => e,
        }
    }
}
extern "system" fn task_get_sd(_this: usize, _info: i32, _out: usize) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn task_set_sd(_this: usize, _sddl: usize, _flags: i32) -> u32 {
    hr::E_NOTIMPL
}
extern "system" fn task_stop(this: usize, _flags: i32) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match scheduler(|t| t.end(registered(this))) {
            Ok(()) | Err(hr::FILE_NOT_FOUND) => hr::S_OK,
            Err(e) => e,
        }
    }
}
/// GetRunTimes: the scheduled runs between two local times (SYSTEMTIMEs),
/// at most `*count`; S_FALSE when there are fewer.
extern "system" fn task_run_times(
    this: usize,
    start: usize,
    end: usize,
    count: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let start = door_windows_abi::widen(start);
    let end = door_windows_abi::widen(end);
    let count = door_windows_abi::widen(count);
    let out = door_windows_abi::widen(out);
    {
        use door_service_protocol::tasks::Schedule;
        let s = match status(this) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let system = |p: u64| -> Result<u64, u32> {
            let mut b = [0u8; 16];
            read(p, &mut b)?;
            let w = |i: usize| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
            boundary_utc(&Boundary {
                year: w(0),
                month: w(1) as u8,
                day: w(3) as u8,
                hour: w(4) as u8,
                minute: w(5) as u8,
                second: w(6) as u8,
                offset: None,
            })
            .ok_or(hr::E_INVALIDARG)
        };
        let (Ok(from), Ok(to)) = (
            system(start),
            if end == 0 { Ok(u64::MAX) } else { system(end) },
        ) else {
            return hr::E_INVALIDARG;
        };
        let Ok(limit) = read_u32(count) else {
            return hr::E_POINTER;
        };
        let mut times = Vec::new();
        let mut at = from;
        while (times.len() as u32) < limit {
            let Some(next) = s.task.schedule.next(s.task.start, at) else {
                break;
            };
            if next > to || matches!(s.task.schedule, Schedule::OnLogon) {
                break;
            }
            times.push(next);
            at = next + 1;
            if matches!(s.task.schedule, Schedule::Once) {
                break;
            }
        }
        let _ = write_u32(count, times.len() as u32);
        let block = unsafe { CoTaskMemAlloc(times.len().max(1) * 16) };
        if block == 0 {
            return hr::E_OUTOFMEMORY;
        }
        for (i, &t) in times.iter().enumerate() {
            let (offset, _) = zone().offset_at(t);
            let local = (t as i64 + i64::from(offset) * 600_000_000) as u64;
            let mut st = [0u8; 16];
            if let Some(w) = door_abi::clock::WallClock::from_filetime(local) {
                for (k, v) in [
                    w.year,
                    u16::from(w.month),
                    0,
                    u16::from(w.day),
                    u16::from(w.hour),
                    u16::from(w.minute),
                    u16::from(w.second),
                    0,
                ]
                .into_iter()
                .enumerate()
                {
                    st[2 * k..2 * k + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
            let _ = write(
                door_windows_abi::widen(block + door_windows_abi::narrow(16 * i as u64)),
                &st,
            );
        }
        let _ = write_pointer(out, door_windows_abi::widen(block));
        if (times.len() as u32) < limit {
            hr::S_FALSE
        } else {
            hr::S_OK
        }
    }
}

// ---- running tasks ---------------------------------------------------------------------------
static RUNNING: Table<15> = dispatch1!(
    running_name,
    running_guid,
    running_path,
    running_state,
    running_action,
    running_stop,
    running_refresh,
    running_pid,
);
fn new_running(name: String) -> u64 {
    let mut guid = [0u8; 16];
    unsafe { DoorGenRandom(guid.as_mut_ptr() as u64, 16) };
    guid[7] = (guid[7] & 0x0F) | 0x40;
    guid[8] = (guid[8] & 0x3F) | 0x80;
    new_obj(
        &RUNNING,
        iid::RUNNING,
        &[iid::RUNNING],
        Kind::Running(name, guid),
    )
}
extern "system" fn running_name(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        task_name_get(
            door_windows_abi::narrow(this),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn running_guid(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let Kind::Running(_, guid) = &obj(this).kind else {
            return hr::E_FAIL;
        };
        let t = crate::guid_text(guid);
        bstr_out(out, Some(&alloc::format!("{{{}}}", t.to_uppercase())))
    }
}
extern "system" fn running_path(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        task_path(
            door_windows_abi::narrow(this),
            door_windows_abi::narrow(out),
        )
    }
}
extern "system" fn running_state(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => result(write_u32(out, state(&s))),
            Err(e) => e,
        }
    }
}
extern "system" fn running_action(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        match status(this) {
            Ok(s) => {
                let (path, _) = split_command(s.task.command());
                bstr_out(out, Some(&path))
            }
            Err(e) => e,
        }
    }
}
extern "system" fn running_stop(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    task_stop(door_windows_abi::narrow(this), 0)
}
extern "system" fn running_refresh(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match status(this) {
            Ok(_) => hr::S_OK,
            Err(e) => e,
        }
    }
}
extern "system" fn running_pid(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_u32(out, 0))
}
/// Tables of this file (the definition side has its own).
fn is_table(t: u64) -> bool {
    [
        SERVICE.0.as_ptr() as u64,
        FOLDER.0.as_ptr() as u64,
        COLLECTION.0.as_ptr() as u64,
        ENUM.0.as_ptr() as u64,
        REGISTERED.0.as_ptr() as u64,
        RUNNING.0.as_ptr() as u64,
    ]
    .contains(&t)
}
