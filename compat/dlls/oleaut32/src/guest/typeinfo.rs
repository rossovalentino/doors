//! ITypeLib2 and ITypeInfo2 over `crate::typelib`: the loaded libraries
//! (files, PE TYPELIB resources, the built-in stdole2, CreateDispTypeInfo's
//! in-memory ones), their imports resolved on demand through the registry,
//! and the TYPEATTR/FUNCDESC/VARDESC/TLIBATTR blocks Windows' x64 layouts
//! describe. Libraries and typeinfos live for the process (as Windows
//! caches them); one object per library and per typeinfo, so pointers
//! compare equal.
use super::*;
use crate::typelib::{
    self, Entry, Func, Library, Linker, Ref, Side, TypeDesc, View, funckind, paramflag, stdole,
    tkind,
};
use alloc::boxed::Box;
use alloc::string::String;
use core::sync::atomic::AtomicU32;

pub(super) const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
pub(super) const IID_IDISPATCH: [u8; 16] =
    [0, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_ITYPEINFO: [u8; 16] = [1, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_ITYPEINFO2: [u8; 16] = [
    0x12, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46,
];
const IID_ITYPELIB: [u8; 16] = [2, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
const IID_ITYPELIB2: [u8; 16] = [
    0x11, 0x04, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46,
];
/// The HREFTYPE GetRefTypeOfImplType(-1) gives for a dual's vtable side.
const DUAL_HREF: u32 = 0xFFFF_FFFE;
const TYPE_E_WRONGTYPEKIND: u32 = 0x8002_802A;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorGetFileSizeEx(handle: u64, destination: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64;
    fn DoorLoadResource(module: u64, handle: u64) -> u64;
    fn DoorSizeofResource(module: u64, handle: u64) -> u32;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetCurrentDirectoryW(capacity: u32, buffer: usize) -> u32;
}
#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> usize;
    fn CoTaskMemFree(block: usize);
    fn CoCreateInstance(clsid: usize, outer: usize, context: u32, iid: usize, out: usize) -> u32;
}

// ---- the loaded libraries ---------------------------------------------------------------
struct Loaded {
    lib: &'static Library,
    /// The full path, lower-cased (empty: built in or in memory).
    path: Vec<u16>,
}
static LIBS: Lock<Vec<Loaded>> = Lock::new(Vec::new());
pub(super) fn library(id: usize) -> Option<&'static Library> {
    LIBS.with(|l| l.get(id).map(|e| e.lib))
}
fn library_path(id: usize) -> Vec<u16> {
    LIBS.with(|l| l.get(id).map(|e| e.path.clone()).unwrap_or_default())
}
pub(super) fn insert(lib: Library, path: Vec<u16>) -> usize {
    let lib: &'static Library = Box::leak(Box::new(lib));
    LIBS.with(|l| {
        if !path.is_empty()
            && let Some(i) = l.iter().position(|e| e.path == path)
        {
            return i;
        }
        l.push(Loaded { lib, path });
        l.len() - 1
    })
}
fn stdole_id() -> usize {
    if let Some(i) = LIBS.with(|l| l.iter().position(|e| e.lib.guid == stdole::GUID)) {
        return i;
    }
    insert(
        stdole::library(),
        wide("c:\\windows\\system32\\stdole2.tlb"),
    )
}
pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn lower(units: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|&u| {
            if (b'A' as u16..=b'Z' as u16).contains(&u) {
                u + 32
            } else {
                u
            }
        })
        .collect()
}
fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// The type references of every library, through imports as needed.
pub(super) struct Guest;
impl Linker for Guest {
    fn library(&self, id: usize) -> Option<&Library> {
        library(id)
    }
    fn resolve(&self, id: usize, r: Ref) -> Option<(usize, u32)> {
        match r {
            Ref::Local(i) => Some((id, i)),
            Ref::Import(j) => {
                let lib = library(id)?;
                let import = lib.imports.get(j as usize)?;
                let file = lib.files.get(import.file)?;
                let target = import_library(id, file)?;
                let index = match import.guid {
                    Some(g) => library(target)?.by_guid(&g)?,
                    None => import.index,
                };
                Some((target, index))
            }
        }
    }
}
/// The library an import names: stdole2, one already loaded, the
/// registered one, or the file beside the importing library or in System32.
fn import_library(from: usize, file: &typelib::ImportFile) -> Option<usize> {
    if file.guid == stdole::GUID {
        return Some(stdole_id());
    }
    if let Some(i) = LIBS.with(|l| {
        l.iter()
            .position(|e| e.lib.guid == file.guid && e.lib.major == file.major)
    }) {
        return Some(i);
    }
    if let Ok(path) =
        super::register::registered_path(&file.guid, file.major, file.minor, file.lcid)
        && let Ok(id) = load(&path)
    {
        return Some(id);
    }
    let name = wide(&file.name);
    let mut beside = library_path(from);
    if let Some(slash) = beside.iter().rposition(|&u| u == b'\\' as u16) {
        beside.truncate(slash + 1);
        beside.extend_from_slice(&name);
        if let Ok(id) = load(&beside) {
            return Some(id);
        }
    }
    let mut system = wide(door_windows_abi::modules::own::SYSTEM_DIRECTORY);
    system.extend_from_slice(&name);
    load(&system).ok()
}

// ---- files ----------------------------------------------------------------------
/// TYPELIB resource `index` of a system DLL (`C:\Windows\System32\x.dll`):
/// those are mapped from the boot image, not files, so the resource comes
/// from the loaded module.
fn module_resource(path: &[u16], index: u32) -> Option<Vec<u8>> {
    let slash = path.iter().rposition(|&u| u == b'\\' as u16);
    let (folder, name) = match slash {
        Some(i) => (lower(&path[..i]), &path[i + 1..]),
        None => (wide("c:\\windows\\system32"), path),
    };
    if folder != wide("c:\\windows\\system32") {
        return None;
    }
    let text = String::from_utf16(name).ok()?;
    door_windows_abi::modules::own::by_name(&text)?;
    let mut z = name.to_vec();
    z.push(0);
    let module = unsafe { DoorLoadLibraryW(z.as_ptr() as u64) };
    if module == 0 {
        return None;
    }
    let kind = wide("TYPELIB\0");
    let handle = unsafe { DoorFindResource(module, kind.as_ptr() as u64, u64::from(index), 0) };
    if handle == 0 {
        return None;
    }
    let data = unsafe { DoorLoadResource(module, handle) };
    let size = unsafe { DoorSizeofResource(module, handle) } as usize;
    if data == 0 || size == 0 {
        return None;
    }
    // SAFETY: the resource lies in the module's mapped image, which stays
    // loaded (system DLLs are never unloaded).
    Some(unsafe { core::slice::from_raw_parts(data as *const u8, size) }.to_vec())
}
fn read_file(path: &[u16]) -> Option<Vec<u8>> {
    let mut z = path.to_vec();
    z.push(0);
    // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL.
    let h = unsafe { DoorCreateFileW(z.as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0x80, 0) };
    if h == u64::MAX || h == 0 {
        return None;
    }
    let mut size = 0u64;
    let mut data = Vec::new();
    if unsafe { DoorGetFileSizeEx(h, &mut size as *mut u64 as u64) } != 0 && size < 1 << 30 {
        data = vec![0u8; size as usize];
        let mut at = 0usize;
        while at < data.len() {
            let mut got = 0u32;
            let chunk = (data.len() - at).min(1 << 20) as u32;
            let ok = unsafe {
                DoorReadFile(
                    h,
                    data.as_mut_ptr() as u64 + at as u64,
                    chunk,
                    &mut got as *mut u32 as u64,
                    0,
                )
            };
            if ok == 0 || got == 0 {
                break;
            }
            at += got as usize;
        }
        data.truncate(at);
    }
    unsafe { DoorCloseFile(h) };
    Some(data)
}
/// An absolute path for `path` (relative ones against the current
/// directory).
pub(super) fn full_path(path: &[u16]) -> Vec<u16> {
    // `C:\…`, `\\…`, or a native `/…` path (Doors' own folders).
    let absolute = path.len() >= 3 && path[1] == b':' as u16 && path[2] == b'\\' as u16
        || path.starts_with(&[b'\\' as u16, b'\\' as u16])
        || path.first() == Some(&(b'/' as u16));
    if absolute {
        return path.to_vec();
    }
    let mut buffer = vec![0u16; 512];
    let n = unsafe {
        GetCurrentDirectoryW(
            buffer.len() as u32,
            door_windows_abi::narrow(buffer.as_mut_ptr() as u64),
        )
    };
    buffer.truncate((n as usize).min(512));
    if buffer.last() != Some(&(b'\\' as u16)) {
        buffer.push(b'\\' as u16);
    }
    if path.first() == Some(&(b'\\' as u16)) {
        buffer.truncate(2);
    }
    buffer.extend_from_slice(path);
    buffer
}
/// Loads (or finds) the library at `path`: an MSFT file, a PE image's
/// TYPELIB resource (`path\N` picks resource N), or stdole2.
pub(super) fn load(path: &[u16]) -> Result<usize, u32> {
    let bare = !path
        .iter()
        .any(|&u| u == b'\\' as u16 || u == b'/' as u16 || u == b':' as u16);
    let name_only = |p: &[u16]| -> Vec<u16> {
        let start = p
            .iter()
            .rposition(|&u| u == b'\\' as u16)
            .map_or(0, |i| i + 1);
        lower(&p[start..])
    };
    if name_only(path) == wide("stdole2.tlb") {
        let full = lower(&full_path(path));
        if bare || full == wide("c:\\windows\\system32\\stdole2.tlb") || read_file(&full).is_none()
        {
            return Ok(stdole_id());
        }
    }
    let mut candidates = Vec::from([full_path(path)]);
    if bare {
        let mut system = wide(door_windows_abi::modules::own::SYSTEM_DIRECTORY);
        system.extend_from_slice(path);
        candidates.push(system);
    }
    for full in candidates {
        let key = lower(&full);
        if let Some(i) = LIBS.with(|l| l.iter().position(|e| e.path == key)) {
            return Ok(i);
        }
        let (file, index) = match read_file(&full).or_else(|| module_resource(&full, 1)) {
            Some(data) => (data, 1),
            None => {
                // `image.dll\2`: resource 2 of image.dll.
                let Some(slash) = full.iter().rposition(|&u| u == b'\\' as u16) else {
                    continue;
                };
                let digits = &full[slash + 1..];
                if digits.is_empty()
                    || !digits
                        .iter()
                        .all(|&u| (b'0' as u16..=b'9' as u16).contains(&u))
                {
                    continue;
                }
                let n = digits
                    .iter()
                    .fold(0u32, |n, &u| n * 10 + (u - b'0' as u16) as u32);
                match read_file(&full[..slash]).or_else(|| module_resource(&full[..slash], n)) {
                    Some(d) => (d, n),
                    None => continue,
                }
            }
        };
        let bytes = if file.starts_with(b"MZ") {
            typelib::resource(&file, index).ok_or(hr::TYPE_E_CANTLOADLIBRARY)?
        } else {
            &file[..]
        };
        let lib = typelib::parse(bytes)?;
        return Ok(insert(lib, key));
    }
    Err(hr::TYPE_E_CANTLOADLIBRARY)
}

// ---- out-blocks ---------------------------------------------------------------------
/// A structure built with what it points at in one heap block: offsets
/// until `commit` relocates them; BSTRs and VARIANTs it owns are freed
/// with it.
struct Block {
    bytes: Vec<u8>,
    relocs: Vec<usize>,
    owned: Vec<usize>,
}
static BLOCKS: Lock<Vec<(u64, Vec<u64>)>> = Lock::new(Vec::new());
impl Block {
    fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
            relocs: Vec::new(),
            owned: Vec::new(),
        }
    }
    fn alloc(&mut self, size: usize, align: usize) -> usize {
        let at = self.bytes.len().div_ceil(align) * align;
        self.bytes.resize(at + size, 0);
        at
    }
    fn put(&mut self, at: usize, b: &[u8]) {
        self.bytes[at..at + b.len()].copy_from_slice(b);
    }
    fn u16(&mut self, at: usize, v: u16) {
        self.put(at, &v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.put(at, &v.to_le_bytes());
    }
    /// A pointer (at the program's width) to offset `target` of the block.
    fn ptr(&mut self, at: usize, target: usize) {
        self.put(at, &(target as u64).to_le_bytes()[..POINTER]);
        self.relocs.push(at);
    }
    /// A VARIANT holding `v` at `at` (its BSTR freed with the block).
    fn variant(&mut self, at: usize, v: &Value) -> Result<(), u32> {
        let mut tmp = [0u8; 24];
        write_value(tmp.as_mut_ptr() as u64, v)?;
        self.put(at, &tmp[..VARIANT_BYTES as usize]);
        if matches!(v, Value::Bstr(_)) {
            self.owned.push(at + 8);
        }
        Ok(())
    }
    fn commit(mut self) -> Result<u64, u32> {
        let base = unsafe {
            RtlAllocateHeap(
                door_windows_abi::narrow(heap()),
                0x8,
                self.bytes.len().max(8),
            )
        };
        if base == 0 {
            return Err(hr::E_OUTOFMEMORY);
        }
        let word = |b: &[u8], at: usize| {
            let mut w = [0u8; 8];
            w[..POINTER].copy_from_slice(&b[at..at + POINTER]);
            u64::from_le_bytes(w)
        };
        for &at in &self.relocs {
            let off = word(&self.bytes, at);
            let address = door_windows_abi::widen(base) + off;
            self.bytes[at..at + POINTER].copy_from_slice(&address.to_le_bytes()[..POINTER]);
        }
        let owned: Vec<u64> = self.owned.iter().map(|&at| word(&self.bytes, at)).collect();
        write(door_windows_abi::widen(base), &self.bytes)?;
        BLOCKS.with(|b| b.push((door_windows_abi::widen(base), owned)));
        Ok(door_windows_abi::widen(base))
    }
}
fn release_block(p: u64) {
    if p == 0 {
        return;
    }
    if let Some(owned) = BLOCKS.with(|b| {
        b.iter()
            .position(|(base, _)| *base == p)
            .map(|i| b.remove(i).1)
    }) {
        owned.into_iter().for_each(bstr_free);
        unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(heap()),
                0,
                door_windows_abi::narrow(p),
            )
        };
    }
}

// ---- ITypeInfo2 -----------------------------------------------------------------------
type M1 = extern "system" fn(usize) -> u32;
type M2 = extern "system" fn(usize, usize) -> u32;
type M3 = extern "system" fn(usize, usize, usize) -> u32;
type M4 = extern "system" fn(usize, usize, usize, usize) -> u32;
type M5 = extern "system" fn(usize, usize, usize, usize, usize) -> u32;
type M6 = extern "system" fn(usize, usize, usize, usize, usize, usize) -> u32;
type M8 = extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> u32;

#[repr(C)]
struct InfoVtbl {
    query: M3,
    add_ref: M1,
    release: M1,
    get_type_attr: M2,
    get_type_comp: M2,
    get_func_desc: M3,
    get_var_desc: M3,
    get_names: M5,
    get_ref_type_of_impl_type: M3,
    get_impl_type_flags: M3,
    get_ids_of_names: M4,
    invoke: M8,
    get_documentation: M6,
    get_dll_entry: M6,
    get_ref_type_info: M3,
    address_of_member: M4,
    create_instance: M4,
    get_mops: M3,
    get_containing_type_lib: M3,
    release_type_attr: M2,
    release_func_desc: M2,
    release_var_desc: M2,
    get_type_kind: M2,
    get_type_flags: M2,
    get_func_index_of_mem_id: M4,
    get_var_index_of_mem_id: M3,
    get_cust_data: M3,
    get_func_cust_data: M4,
    get_param_cust_data: M5,
    get_var_cust_data: M4,
    get_impl_type_cust_data: M4,
    get_documentation2: M6,
    get_all_cust_data: M2,
    get_all_func_cust_data: M3,
    get_all_param_cust_data: M4,
    get_all_var_cust_data: M3,
    get_all_impl_type_cust_data: M3,
}
static INFO_VTBL: InfoVtbl = InfoVtbl {
    query: ti_query,
    add_ref: ti_add_ref,
    release: ti_release,
    get_type_attr,
    get_type_comp: not_impl2,
    get_func_desc,
    get_var_desc,
    get_names,
    get_ref_type_of_impl_type,
    get_impl_type_flags,
    get_ids_of_names,
    invoke: super::dispatch::type_info_invoke,
    get_documentation,
    get_dll_entry,
    get_ref_type_info,
    address_of_member: not_impl4,
    create_instance,
    get_mops,
    get_containing_type_lib,
    release_type_attr: release_desc,
    release_func_desc: release_desc,
    release_var_desc: release_desc,
    get_type_kind,
    get_type_flags,
    get_func_index_of_mem_id,
    get_var_index_of_mem_id,
    get_cust_data: type_cust_data,
    get_func_cust_data,
    get_param_cust_data,
    get_var_cust_data,
    get_impl_type_cust_data: empty_cust_data4,
    get_documentation2: type_documentation2,
    get_all_cust_data: type_all_cust_data,
    get_all_func_cust_data,
    get_all_param_cust_data,
    get_all_var_cust_data,
    get_all_impl_type_cust_data: empty_all_cust_data3,
};
#[repr(C)]
pub(super) struct TypeInfo {
    vtbl: &'static InfoVtbl,
    references: AtomicU32,
    pub(super) lib: usize,
    pub(super) index: u32,
    pub(super) side: Side,
    pub(super) view: View,
    foreign: Vec<(usize, Ref)>,
}
/// (library, type, vtable side) and the object.
type InfoKey = (usize, u32, bool);
static INFOS: Lock<Vec<(InfoKey, u64)>> = Lock::new(Vec::new());
/// The ITypeInfo for type `index` of library `lib` (a new reference).
pub(super) fn type_info(lib: usize, index: u32, side: Side) -> Result<u64, u32> {
    let key = (lib, index, side == Side::Vtable);
    if let Some(p) = INFOS.with(|i| i.iter().find(|(k, _)| *k == key).map(|e| e.1)) {
        ti_add_ref(door_windows_abi::narrow(p));
        return Ok(p);
    }
    let view = typelib::view(&Guest, lib, index, side)?;
    let mut foreign = Vec::new();
    let mut note = |home: usize, r: Ref| {
        if home != lib && !foreign.contains(&(home, r)) {
            foreign.push((home, r));
        }
    };
    for (home, f) in &view.funcs {
        each_ref(&f.ret, &mut |r| note(*home, r));
        for p in &f.params {
            each_ref(&p.ty, &mut |r| note(*home, r));
        }
    }
    for (home, i) in &view.impls {
        note(*home, i.target);
    }
    let object = Box::into_raw(Box::new(TypeInfo {
        vtbl: &INFO_VTBL,
        references: AtomicU32::new(1),
        lib,
        index,
        side,
        view,
        foreign,
    })) as u64;
    Ok(INFOS.with(|i| match i.iter().find(|(k, _)| *k == key) {
        Some(e) => {
            // Another thread made it first; this one is never shared.
            ti_add_ref(door_windows_abi::narrow(e.1));
            e.1
        }
        None => {
            i.push((key, object));
            object
        }
    }))
}
fn each_ref(t: &TypeDesc, f: &mut impl FnMut(Ref)) {
    match t {
        TypeDesc::Ptr(i) | TypeDesc::SafeArray(i) | TypeDesc::CArray(i, _) => each_ref(i, f),
        TypeDesc::User(r) => f(*r),
        TypeDesc::Base(_) => {}
    }
}
/// The TypeInfo behind `this` when it is one of ours.
pub(super) fn ours(this: u64) -> Option<&'static TypeInfo> {
    if this == 0 {
        return None;
    }
    // SAFETY: an interface pointer's first word is its vtable.
    let vtbl = unsafe { *(this as *const u64) };
    (vtbl == &INFO_VTBL as *const InfoVtbl as u64).then(|| unsafe { &*(this as *const TypeInfo) })
}
fn info(this: u64) -> &'static TypeInfo {
    // SAFETY: the vtable's methods are only called on TypeInfo objects.
    unsafe { &*(this as *const TypeInfo) }
}
impl TypeInfo {
    pub(super) fn href(&self, home: usize, r: Ref) -> u32 {
        if home == self.lib {
            return r.href();
        }
        match self.foreign.iter().position(|e| *e == (home, r)) {
            Some(k) => 0x4000_0000 | ((k as u32) << 2) | 2,
            None => r.href(),
        }
    }
    /// The library and Ref an HREFTYPE from this typeinfo names.
    fn unhref(&self, h: u32) -> Option<(usize, Ref)> {
        if h & 3 == 2 && h & 0x4000_0000 != 0 {
            return self.foreign.get(((h & 0x3FFF_FFFF) >> 2) as usize).copied();
        }
        Ref::from_href(h).map(|r| (self.lib, r))
    }
}
extern "system" fn ti_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_POINTER;
        };
        if g == IID_IUNKNOWN || g == IID_ITYPEINFO || g == IID_ITYPEINFO2 {
            ti_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn ti_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    info(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
/// Typeinfos stay for the process (their library keeps them); the count
/// only answers callers.
extern "system" fn ti_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        let r = &info(this).references;
        match r.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1)) {
            Ok(n) => n - 1,
            Err(_) => 0,
        }
    }
}
extern "system" fn not_impl2(_this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        hr::E_NOTIMPL
    }
}
extern "system" fn not_impl4(_this: usize, _a: usize, _b: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let _ = write_pointer(out, 0);
        hr::E_NOTIMPL
    }
}
// The type descriptions at the program's width (P17): TYPEDESC { a
// pointer or HREFTYPE, vt } is 16 bytes on x64 and 8 on x86; ELEMDESC
// (TYPEDESC, PARAMDESC { pparamdescex, wParamFlags }) 32 and 16.
const TYPEDESC_BYTES: usize = 2 * POINTER;
const TD_VT: usize = POINTER;
const ELEMDESC_BYTES: usize = 4 * POINTER;
const ED_PARAMDESCEX: usize = TYPEDESC_BYTES;
const ED_FLAGS: usize = TYPEDESC_BYTES + POINTER;
/// PARAMDESCEX { cBytes, VARIANT varDefaultValue (8-aligned) }.
const PARAMDESCEX_BYTES: usize = 8 + VARIANT_BYTES as usize;
/// TYPEDESC at `at`.
fn put_typedesc(b: &mut Block, at: usize, t: &TypeDesc, ti: &TypeInfo, home: usize) {
    match t {
        TypeDesc::Base(v) => b.u16(at + TD_VT, *v),
        TypeDesc::Ptr(i) | TypeDesc::SafeArray(i) => {
            let o = b.alloc(TYPEDESC_BYTES, 8);
            put_typedesc(b, o, i, ti, home);
            b.ptr(at, o);
            b.u16(at + TD_VT, t.vt());
        }
        TypeDesc::CArray(e, bounds) => {
            // ARRAYDESC: tdescElem, cDims, SAFEARRAYBOUND rgbounds[].
            let o = b.alloc(TYPEDESC_BYTES + 4 + 8 * bounds.len().max(1), 8);
            put_typedesc(b, o, e, ti, home);
            b.u16(o + TYPEDESC_BYTES, bounds.len() as u16);
            for (k, (count, low)) in bounds.iter().enumerate() {
                b.u32(o + TYPEDESC_BYTES + 4 + 8 * k, *count);
                b.u32(o + TYPEDESC_BYTES + 8 + 8 * k, *low as u32);
            }
            b.ptr(at, o);
            b.u16(at + TD_VT, vt::CARRAY);
        }
        TypeDesc::User(r) => {
            b.u32(at, ti.href(home, *r));
            b.u16(at + TD_VT, vt::USERDEFINED);
        }
    }
}
/// ELEMDESC at `at`: the type and PARAMDESC.
fn put_elemdesc(
    b: &mut Block,
    at: usize,
    t: &TypeDesc,
    flags: u16,
    default: Option<&Value>,
    ti: &TypeInfo,
    home: usize,
) -> Result<(), u32> {
    put_typedesc(b, at, t, ti, home);
    b.u16(at + ED_FLAGS, flags);
    if flags & paramflag::HASDEFAULT != 0
        && let Some(v) = default
    {
        // PARAMDESCEX: cBytes, VARIANT.
        let o = b.alloc(PARAMDESCEX_BYTES, 8);
        b.u32(o, PARAMDESCEX_BYTES as u32);
        b.variant(o + 8, v)?;
        b.ptr(at + ED_PARAMDESCEX, o);
    }
    Ok(())
}
extern "system" fn get_type_attr(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let v = &ti.view;
        let lcid = library(ti.lib).map_or(0, |l| l.lcid);
        // TYPEATTR: guid, lcid, dwReserved, memidConstructor,
        // memidDestructor, lpstrSchema, cbSizeInstance, typekind, cFuncs,
        // cVars, cImplTypes, cbSizeVft, cbAlignment, wTypeFlags,
        // wMajorVerNum, wMinorVerNum, tdescAlias, idldescType (96 bytes on
        // x64, 76 on x86).
        use door_windows_abi::layout::{self, Field::*};
        const TYPEATTR: [layout::Field; 20] = [
            Bytes(16),
            U32,
            U32,
            U32,
            U32,
            Pointer,
            U32,
            U32,
            U16,
            U16,
            U16,
            U16,
            U16,
            U16,
            U16,
            U16,
            Pointer,
            U16,
            Pointer,
            U16,
        ];
        let at = |index| layout::offset(&TYPEATTR, index);
        let mut b = Block::new(layout::size(&TYPEATTR));
        b.put(0, &v.guid);
        b.u32(at(1), lcid);
        b.u32(at(3), u32::MAX);
        b.u32(at(4), u32::MAX);
        b.u32(at(6), v.size);
        b.u32(at(7), v.kind);
        b.u16(at(8), v.funcs.len() as u16);
        b.u16(at(9), v.vars.len() as u16);
        b.u16(at(10), v.impls.len() as u16);
        // Bytes of this process's pointer width (`typelib::vft_bytes`).
        b.u16(at(11), v.vft);
        b.u16(at(12), v.align);
        b.u16(at(13), v.flags);
        b.u16(at(14), v.major);
        b.u16(at(15), v.minor);
        if let Some(a) = &v.alias {
            put_typedesc(&mut b, at(16), a, ti, ti.lib);
        }
        result(b.commit().and_then(|p| write_pointer(out, p)))
    }
}
extern "system" fn get_func_desc(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let Some((home, f)) = ti.view.funcs.get(index as u32 as usize) else {
            let _ = write_pointer(out, 0);
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        result(func_desc(ti, *home, f).and_then(|p| write_pointer(out, p)))
    }
}
fn func_desc(ti: &TypeInfo, home: usize, f: &Func) -> Result<u64, u32> {
    // FUNCDESC: memid, lprgscode, lprgelemdescParam, funckind, invkind,
    // callconv, cParams, cParamsOpt, oVft, cScodes, elemdescFunc,
    // wFuncFlags (88 bytes on x64, 52 on x86).
    use door_windows_abi::layout::{self, Field::*};
    const FUNCDESC: [layout::Field; 15] = [
        U32, Pointer, Pointer, U32, U32, U32, U16, U16, U16, U16, Pointer, Pointer, Pointer,
        Pointer, U16,
    ];
    let at = |index| layout::offset(&FUNCDESC, index);
    let mut b = Block::new(layout::size(&FUNCDESC));
    b.u32(0, f.memid as u32);
    if !f.params.is_empty() {
        let list = b.alloc(ELEMDESC_BYTES * f.params.len(), 8);
        for (k, p) in f.params.iter().enumerate() {
            put_elemdesc(
                &mut b,
                list + ELEMDESC_BYTES * k,
                &p.ty,
                p.flags,
                p.default.as_ref(),
                ti,
                home,
            )?;
        }
        b.ptr(at(2), list);
    }
    b.u32(at(3), f.kind);
    b.u32(at(4), f.invoke);
    b.u32(at(5), f.callconv);
    b.u16(at(6), f.params.len() as u16);
    b.u16(at(7), f.optional as u16);
    // oVft in bytes of this process's pointer width; a dispinterface's
    // members keep the library's own value, as Windows reports them.
    b.u16(at(8), f.vft as u16);
    put_elemdesc(&mut b, at(10), &f.ret, 0, None, ti, home)?;
    b.u16(at(14), f.flags);
    b.commit()
}
extern "system" fn get_var_desc(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let Some(var) = ti.view.vars.get(index as u32 as usize) else {
            let _ = write_pointer(out, 0);
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        // VARDESC: memid, lpstrSchema, oInst / lpvarValue, elemdescVar,
        // wVarFlags, varkind (64 bytes on x64, 36 on x86).
        use door_windows_abi::layout::{self, Field::*};
        const VARDESC: [layout::Field; 9] = [
            U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer, U16, U32,
        ];
        let at = |index| layout::offset(&VARDESC, index);
        let mut b = Block::new(layout::size(&VARDESC));
        b.u32(0, var.memid as u32);
        let r = (|| {
            match &var.value {
                Some(value) => {
                    let o = b.alloc(VARIANT_BYTES as usize, 8);
                    b.variant(o, value)?;
                    b.ptr(at(2), o);
                }
                None => b.u32(at(2), var.offset),
            }
            put_elemdesc(&mut b, at(3), &var.ty, 0, None, ti, ti.lib)?;
            b.u16(at(7), var.flags);
            b.u32(at(8), var.kind);
            b.commit()
        })();
        result(r.and_then(|p| write_pointer(out, p)))
    }
}
extern "system" fn release_desc(_this: usize, p: usize) -> u32 {
    let p = door_windows_abi::widen(p);
    {
        release_block(p);
        hr::S_OK
    }
}
extern "system" fn get_names(
    this: usize,
    memid: usize,
    names: usize,
    max: usize,
    count: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let names = door_windows_abi::widen(names);
    let max = door_windows_abi::widen(max);
    let count = door_windows_abi::widen(count);
    {
        let ti = info(this);
        let Some(list) = ti.view.names(memid as u32 as i32) else {
            let _ = write_u32(count, 0);
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        let n = list.len().min(max as u32 as usize);
        for (k, name) in list.iter().take(n).enumerate() {
            if let Err(e) = write_pointer(names + 8 * k as u64, bstr_from(&wide(name))) {
                return e;
            }
        }
        result(write_u32(count, n as u32))
    }
}
/// The ITypeInfo of the interface `this` (an interface typeinfo) derives
/// from, AddRef'd.
pub(super) fn base_info(this: u64) -> Option<u64> {
    let mut href = 0u32;
    if get_ref_type_of_impl_type(
        door_windows_abi::narrow(this),
        0,
        door_windows_abi::narrow(&mut href as *mut u32 as u64),
    ) != hr::S_OK
    {
        return None;
    }
    let mut base = 0u64;
    (get_ref_type_info(
        door_windows_abi::narrow(this),
        door_windows_abi::narrow(u64::from(href)),
        door_windows_abi::narrow(&mut base as *mut u64 as u64),
    ) == hr::S_OK
        && base != 0)
        .then_some(base)
}
extern "system" fn get_ref_type_of_impl_type(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let index = index as u32;
        if index == u32::MAX {
            if ti.view.dual {
                return result(write_u32(out, DUAL_HREF));
            }
            return hr::TYPE_E_ELEMENTNOTFOUND;
        }
        match ti.view.impls.get(index as usize) {
            Some((home, i)) => result(write_u32(out, ti.href(*home, i.target))),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_impl_type_flags(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        match info(this).view.impls.get(index as u32 as usize) {
            Some((_, i)) => result(write_u32(out, i.flags as u32)),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_ids_of_names(this: usize, names: usize, count: usize, ids: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let names = door_windows_abi::widen(names);
    let count = door_windows_abi::widen(count);
    let ids = door_windows_abi::widen(ids);
    {
        let count = count as u32 as usize;
        let mut list = Vec::new();
        for k in 0..count {
            match read_pointer(names + (POINTER * k) as u64).and_then(read_wide) {
                Ok(n) => list.push(text(&n)),
                Err(e) => return e,
            }
        }
        let (out, ok) = info(this).view.ids_of_names(&list);
        // A member the type does not declare: its base interface's (as
        // Windows looks one up for an interface that derives from another).
        if out.first().is_none_or(|id| *id == -1)
            && let Some(base) = base_info(this)
        {
            let r = get_ids_of_names(
                door_windows_abi::narrow(base),
                door_windows_abi::narrow(names),
                door_windows_abi::narrow(count as u64),
                door_windows_abi::narrow(ids),
            );
            release(base);
            return r;
        }
        for (k, id) in out.iter().enumerate() {
            if let Err(e) = write_u32(ids + 4 * k as u64, *id as u32) {
                return e;
            }
        }
        if ok { hr::S_OK } else { hr::DISP_E_UNKNOWNNAME }
    }
}
fn give(out: u64, s: &Option<String>) -> Result<(), u32> {
    if out == 0 {
        return Ok(());
    }
    write_pointer(
        out,
        match s {
            Some(t) => bstr_from(&wide(t)),
            None => 0,
        },
    )
}
extern "system" fn get_documentation(
    this: usize,
    memid: usize,
    name: usize,
    doc: usize,
    context: usize,
    file: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let name = door_windows_abi::widen(name);
    let doc = door_windows_abi::widen(doc);
    let context = door_windows_abi::widen(context);
    let file = door_windows_abi::widen(file);
    {
        let ti = info(this);
        let Some((n, d, ctx)) = ti.view.documentation(memid as u32 as i32) else {
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        let help = library(ti.lib).and_then(|l| l.help_file.clone());
        let r = give(name, &Some(n))
            .and_then(|_| give(doc, &d))
            .and_then(|_| {
                if context != 0 {
                    write_u32(context, ctx)
                } else {
                    Ok(())
                }
            })
            .and_then(|_| give(file, &help));
        result(r)
    }
}
extern "system" fn get_dll_entry(
    this: usize,
    memid: usize,
    invoke: usize,
    dll: usize,
    name: usize,
    ordinal: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let invoke = door_windows_abi::widen(invoke);
    let dll = door_windows_abi::widen(dll);
    let name = door_windows_abi::widen(name);
    let ordinal = door_windows_abi::widen(ordinal);
    {
        let ti = info(this);
        if ti.view.kind != tkind::MODULE {
            return hr::TYPE_E_BADMODULEKIND;
        }
        let Some(f) = ti.view.func(memid as u32 as i32, invoke as u32) else {
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        let (n, o) = match &f.entry {
            Some(Entry::Name(n)) => (Some(n.clone()), 0),
            Some(Entry::Ordinal(o)) => (None, *o),
            None => (None, 0),
        };
        let r = give(dll, &ti.view.dll)
            .and_then(|_| give(name, &n))
            .and_then(|_| {
                if ordinal != 0 {
                    write_u16(ordinal, o)
                } else {
                    Ok(())
                }
            });
        result(r)
    }
}
extern "system" fn get_ref_type_info(this: usize, href: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let href = door_windows_abi::widen(href);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let h = href as u32;
        let r = if h == DUAL_HREF {
            if !ti.view.dual {
                return hr::TYPE_E_ELEMENTNOTFOUND;
            }
            type_info(ti.lib, ti.index, Side::Vtable)
        } else {
            match ti.unhref(h).and_then(|(home, r)| Guest.resolve(home, r)) {
                Some((lib, index)) => type_info(lib, index, Side::Natural),
                None => Err(hr::TYPE_E_ELEMENTNOTFOUND),
            }
        };
        match r {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => {
                let _ = write_pointer(out, 0);
                e
            }
        }
    }
}
/// ITypeInfo::CreateInstance: a coclass through CoCreateInstance.
extern "system" fn create_instance(this: usize, outer: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let outer = door_windows_abi::widen(outer);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        if ti.view.kind != tkind::COCLASS {
            let _ = write_pointer(out, 0);
            return TYPE_E_WRONGTYPEKIND;
        }
        let clsid = ti.view.guid;
        // CLSCTX_SERVER.
        unsafe {
            CoCreateInstance(
                door_windows_abi::narrow(clsid.as_ptr() as u64),
                door_windows_abi::narrow(outer),
                0x15,
                door_windows_abi::narrow(iid),
                door_windows_abi::narrow(out),
            )
        }
    }
}
extern "system" fn get_mops(_this: usize, _memid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_pointer(out, 0))
}
extern "system" fn get_containing_type_lib(this: usize, out: usize, index: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    let index = door_windows_abi::widen(index);
    {
        let ti = info(this);
        if index != 0
            && let Err(e) = write_u32(index, ti.index)
        {
            return e;
        }
        if out == 0 {
            return hr::S_OK;
        }
        result(write_pointer(out, type_lib(ti.lib)))
    }
}
extern "system" fn get_type_kind(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, info(this).view.kind))
}
extern "system" fn get_type_flags(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    result(write_u32(out, info(this).view.flags as u32))
}
extern "system" fn get_func_index_of_mem_id(
    this: usize,
    memid: usize,
    invoke: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let invoke = door_windows_abi::widen(invoke);
    let out = door_windows_abi::widen(out);
    {
        let ti = info(this);
        let (memid, invoke) = (memid as u32 as i32, invoke as u32);
        match ti
            .view
            .funcs
            .iter()
            .position(|(_, f)| f.memid == memid && (invoke == 0 || f.invoke & invoke != 0))
        {
            Some(i) => result(write_u32(out, i as u32)),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_var_index_of_mem_id(this: usize, memid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let out = door_windows_abi::widen(out);
    {
        match info(this)
            .view
            .vars
            .iter()
            .position(|v| v.memid == memid as u32 as i32)
        {
            Some(i) => result(write_u32(out, i as u32)),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
fn custom_value(list: &[typelib::Custom], guid: u64, out: u64) -> u32 {
    let Ok(g) = read_n::<16>(guid) else {
        return hr::E_POINTER;
    };
    let value = list
        .iter()
        .find(|c| c.guid == g)
        .map_or(Value::Empty, |c| c.value.clone());
    result(write_value(out, &value))
}
fn type_custom(ti: &TypeInfo) -> &'static [typelib::Custom] {
    library(ti.lib)
        .and_then(|l| l.types.get(ti.index as usize))
        .map_or(&[], |t| &t.custom[..])
}
extern "system" fn type_cust_data(this: usize, guid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    custom_value(type_custom(info(this)), guid, out)
}
extern "system" fn get_func_cust_data(this: usize, index: usize, guid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        match info(this).view.funcs.get(index as u32 as usize) {
            Some((_, f)) => custom_value(&f.custom, guid, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_param_cust_data(
    this: usize,
    index: usize,
    param: usize,
    guid: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let param = door_windows_abi::widen(param);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        match info(this)
            .view
            .funcs
            .get(index as u32 as usize)
            .and_then(|(_, f)| f.params.get(param as u32 as usize))
        {
            Some(p) => custom_value(&p.custom, guid, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_var_cust_data(this: usize, index: usize, guid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        match info(this).view.vars.get(index as u32 as usize) {
            Some(v) => custom_value(&v.custom, guid, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn empty_cust_data4(_this: usize, _index: usize, _guid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    result(write_value(out, &Value::Empty))
}
extern "system" fn type_documentation2(
    this: usize,
    memid: usize,
    _lcid: usize,
    help: usize,
    context: usize,
    dll: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let memid = door_windows_abi::widen(memid);
    let help = door_windows_abi::widen(help);
    let context = door_windows_abi::widen(context);
    let dll = door_windows_abi::widen(dll);
    {
        let ti = info(this);
        let memid = memid as u32 as i32;
        let (doc, ctx) = if memid == typelib::NIL {
            (ti.view.doc.clone(), ti.view.help_string_context)
        } else if let Some(f) = ti.view.func(memid, 0) {
            (f.doc.clone(), f.help_string_context)
        } else if let Some(v) = ti.view.vars.iter().find(|v| v.memid == memid) {
            (v.doc.clone(), v.help_string_context)
        } else {
            return hr::TYPE_E_ELEMENTNOTFOUND;
        };
        let r = give(help, &doc)
            .and_then(|_| {
                if context != 0 {
                    write_u32(context, ctx)
                } else {
                    Ok(())
                }
            })
            .and_then(|_| give(dll, &None));
        result(r)
    }
}
/// CUSTDATA {cCustData, prgCustData} with CoTaskMemAlloc'd items
/// (CUSTDATAITEM: GUID, VARIANT); ClearCustData frees them.
fn all_custom(list: &[typelib::Custom], out: u64) -> u32 {
    let items = if list.is_empty() {
        0
    } else {
        let p = unsafe { CoTaskMemAlloc(40 * list.len()) };
        if p == 0 {
            return hr::E_OUTOFMEMORY;
        }
        for (k, c) in list.iter().enumerate() {
            let at = p + door_windows_abi::narrow(40 * k as u64);
            if let Err(e) = write(door_windows_abi::widen(at), &c.guid)
                .and_then(|_| write_value(door_windows_abi::widen(at + 16), &c.value))
            {
                return e;
            }
        }
        p
    };
    result(
        write_u32(out, list.len() as u32)
            .and_then(|_| write_pointer(out + 8, door_windows_abi::widen(items))),
    )
}
extern "system" fn type_all_cust_data(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    all_custom(type_custom(info(this)), out)
}
extern "system" fn get_all_func_cust_data(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        match info(this).view.funcs.get(index as u32 as usize) {
            Some((_, f)) => all_custom(&f.custom, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_all_param_cust_data(
    this: usize,
    index: usize,
    param: usize,
    out: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let param = door_windows_abi::widen(param);
    let out = door_windows_abi::widen(out);
    {
        match info(this)
            .view
            .funcs
            .get(index as u32 as usize)
            .and_then(|(_, f)| f.params.get(param as u32 as usize))
        {
            Some(p) => all_custom(&p.custom, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_all_var_cust_data(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        match info(this).view.vars.get(index as u32 as usize) {
            Some(v) => all_custom(&v.custom, out),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn empty_all_cust_data3(_this: usize, _index: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    all_custom(&[], out)
}
#[unsafe(no_mangle)]
pub extern "system" fn ClearCustData(data: usize) {
    let data = door_windows_abi::widen(data);
    {
        // CUSTDATA { cCustData, prgCustData }; CUSTDATAITEM { guid, varValue }
        // (40 bytes on x64, 32 on x86).
        let (Ok(n), Ok(items)) = (read_u32(data), read_pointer(data + POINTER as u64)) else {
            return;
        };
        for k in 0..n as u64 {
            let _ = clear(items + (16 + VARIANT_BYTES) * k + 16);
        }
        if items != 0 {
            unsafe { CoTaskMemFree(door_windows_abi::narrow(items)) };
        }
        let _ = write_u32(data, 0).and_then(|_| write_pointer(data + 8, 0));
    }
}

// ---- ITypeLib2 ---------------------------------------------------------------------------
#[repr(C)]
struct LibVtbl {
    query: M3,
    add_ref: M1,
    release: M1,
    get_type_info_count: M1,
    get_type_info: M3,
    get_type_info_type: M3,
    get_type_info_of_guid: M3,
    get_lib_attr: M2,
    get_type_comp: M2,
    get_documentation: M6,
    is_name: M4,
    find_name: M6,
    release_tlib_attr: M2,
    get_cust_data: M3,
    get_lib_statistics: M3,
    get_documentation2: M6,
    get_all_cust_data: M2,
}
static LIB_VTBL: LibVtbl = LibVtbl {
    query: tl_query,
    add_ref: tl_add_ref,
    release: tl_release,
    get_type_info_count,
    get_type_info,
    get_type_info_type,
    get_type_info_of_guid,
    get_lib_attr,
    get_type_comp: not_impl2,
    get_documentation: lib_documentation,
    is_name,
    find_name,
    release_tlib_attr: release_desc,
    get_cust_data: lib_cust_data,
    get_lib_statistics,
    get_documentation2: lib_documentation2,
    get_all_cust_data: lib_all_cust_data,
};
#[repr(C)]
struct TypeLib {
    vtbl: &'static LibVtbl,
    references: AtomicU32,
    lib: usize,
}
static TYPELIBS: Lock<Vec<(usize, u64)>> = Lock::new(Vec::new());
/// The ITypeLib of library `lib` (a new reference).
pub(super) fn type_lib(lib: usize) -> u64 {
    let p = TYPELIBS.with(|t| match t.iter().find(|e| e.0 == lib) {
        Some(e) => e.1,
        None => {
            let p = Box::into_raw(Box::new(TypeLib {
                vtbl: &LIB_VTBL,
                references: AtomicU32::new(0),
                lib,
            })) as u64;
            t.push((lib, p));
            p
        }
    });
    tl_add_ref(door_windows_abi::narrow(p));
    p
}
/// The library behind an ITypeLib of ours.
pub(super) fn lib_of(p: u64) -> Option<usize> {
    if p == 0 {
        return None;
    }
    // SAFETY: an interface pointer's first word is its vtable.
    let vtbl = unsafe { *(p as *const u64) };
    (vtbl == &LIB_VTBL as *const LibVtbl as u64).then(|| tl(p).lib)
}
fn tl(this: u64) -> &'static TypeLib {
    // SAFETY: the vtable's methods are only called on TypeLib objects.
    unsafe { &*(this as *const TypeLib) }
}
fn lib(this: u64) -> &'static Library {
    library(tl(this).lib).unwrap_or_else(|| unreachable!())
}
extern "system" fn tl_query(this: usize, iid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let iid = door_windows_abi::widen(iid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_n::<16>(iid) else {
            return hr::E_POINTER;
        };
        if g == IID_IUNKNOWN || g == IID_ITYPELIB || g == IID_ITYPELIB2 {
            tl_add_ref(door_windows_abi::narrow(this));
            return result(write_pointer(out, this));
        }
        let _ = write_pointer(out, 0);
        hr::E_NOINTERFACE
    }
}
extern "system" fn tl_add_ref(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    tl(this).references.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn tl_release(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    {
        match tl(this)
            .references
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
        {
            Ok(n) => n - 1,
            Err(_) => 0,
        }
    }
}
extern "system" fn get_type_info_count(this: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    lib(this).types.len() as u32
}
extern "system" fn get_type_info(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        let index = index as u32;
        if index as usize >= lib(this).types.len() {
            let _ = write_pointer(out, 0);
            return hr::TYPE_E_ELEMENTNOTFOUND;
        }
        match type_info(tl(this).lib, index, Side::Natural) {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
extern "system" fn get_type_info_type(this: usize, index: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let out = door_windows_abi::widen(out);
    {
        match lib(this).types.get(index as u32 as usize) {
            Some(t) => result(write_u32(out, t.kind)),
            None => hr::TYPE_E_ELEMENTNOTFOUND,
        }
    }
}
extern "system" fn get_type_info_of_guid(this: usize, guid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        let Ok(g) = read_n::<16>(guid) else {
            return hr::E_POINTER;
        };
        match lib(this).by_guid(&g) {
            Some(i) => get_type_info(
                door_windows_abi::narrow(this),
                door_windows_abi::narrow(i as u64),
                door_windows_abi::narrow(out),
            ),
            None => {
                let _ = write_pointer(out, 0);
                hr::TYPE_E_ELEMENTNOTFOUND
            }
        }
    }
}
extern "system" fn get_lib_attr(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    {
        let l = lib(this);
        let mut b = Block::new(32);
        b.put(0, &l.guid);
        b.u32(16, l.lcid);
        b.u32(20, l.syskind);
        b.u16(24, l.major);
        b.u16(26, l.minor);
        b.u16(28, l.flags | typelib::HAS_DISK_IMAGE);
        result(b.commit().and_then(|p| write_pointer(out, p)))
    }
}
extern "system" fn lib_documentation(
    this: usize,
    index: usize,
    name: usize,
    doc: usize,
    context: usize,
    file: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let name = door_windows_abi::widen(name);
    let doc = door_windows_abi::widen(doc);
    let context = door_windows_abi::widen(context);
    let file = door_windows_abi::widen(file);
    {
        let l = lib(this);
        let index = index as u32 as i32;
        let (n, d, ctx) = if index == -1 {
            (l.name.clone(), l.doc.clone(), l.help_context)
        } else {
            match l.types.get(index as usize) {
                Some(t) => (t.name.clone(), t.doc.clone(), t.help_context),
                None => return hr::TYPE_E_ELEMENTNOTFOUND,
            }
        };
        let r = give(name, &Some(n))
            .and_then(|_| give(doc, &d))
            .and_then(|_| {
                if context != 0 {
                    write_u32(context, ctx)
                } else {
                    Ok(())
                }
            })
            .and_then(|_| give(file, &l.help_file));
        result(r)
    }
}
extern "system" fn lib_documentation2(
    this: usize,
    index: usize,
    _lcid: usize,
    help: usize,
    context: usize,
    dll: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let index = door_windows_abi::widen(index);
    let help = door_windows_abi::widen(help);
    let context = door_windows_abi::widen(context);
    let dll = door_windows_abi::widen(dll);
    {
        let l = lib(this);
        let index = index as u32 as i32;
        let (d, ctx) = if index == -1 {
            (l.doc.clone(), l.help_string_context)
        } else {
            match l.types.get(index as usize) {
                Some(t) => (t.doc.clone(), t.help_string_context),
                None => return hr::TYPE_E_ELEMENTNOTFOUND,
            }
        };
        let r = give(help, &d)
            .and_then(|_| {
                if context != 0 {
                    write_u32(context, ctx)
                } else {
                    Ok(())
                }
            })
            .and_then(|_| give(dll, &None));
        result(r)
    }
}
/// ITypeLib::IsName: TRUE, with the library's spelling written back.
extern "system" fn is_name(this: usize, name: usize, _hash: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    {
        let n = match read_wide(name) {
            Ok(n) => n,
            Err(e) => return e,
        };
        match lib(this).is_name(&text(&n)) {
            Some(stored) => {
                let units = wide(stored);
                if units.len() == n.len()
                    && let Err(e) = write(
                        name,
                        &units
                            .iter()
                            .flat_map(|u| u.to_le_bytes())
                            .collect::<Vec<_>>(),
                    )
                {
                    return e;
                }
                result(write_u32(out, 1))
            }
            None => result(write_u32(out, 0)),
        }
    }
}
/// ITypeLib::FindName: the typeinfos naming `name` (as a type: MEMBERID_NIL)
/// or holding a member of that name, up to *found.
extern "system" fn find_name(
    this: usize,
    name: usize,
    _hash: usize,
    infos: usize,
    memids: usize,
    found: usize,
) -> u32 {
    let this = door_windows_abi::widen(this);
    let name = door_windows_abi::widen(name);
    let infos = door_windows_abi::widen(infos);
    let memids = door_windows_abi::widen(memids);
    let found = door_windows_abi::widen(found);
    {
        let n = match read_wide(name) {
            Ok(n) => text(&n),
            Err(e) => return e,
        };
        let Ok(capacity) = read_u16(found) else {
            return hr::E_POINTER;
        };
        let l = lib(this);
        let mut hits = Vec::new();
        for (i, t) in l.types.iter().enumerate() {
            if hits.len() >= capacity as usize {
                break;
            }
            if typelib::same_name(&t.name, &n) {
                hits.push((i as u32, typelib::NIL));
            } else if let Some(f) = t.funcs.iter().find(|f| typelib::same_name(&f.name, &n)) {
                hits.push((i as u32, f.memid));
            } else if let Some(v) = t.vars.iter().find(|v| typelib::same_name(&v.name, &n)) {
                hits.push((i as u32, v.memid));
            }
        }
        for (k, (index, memid)) in hits.iter().enumerate() {
            let p = match type_info(tl(this).lib, *index, Side::Natural) {
                Ok(p) => p,
                Err(e) => return e,
            };
            if let Err(e) = write_pointer(infos + 8 * k as u64, p)
                .and_then(|_| write_u32(memids + 4 * k as u64, *memid as u32))
            {
                return e;
            }
        }
        result(write_u16(found, hits.len() as u16))
    }
}
extern "system" fn lib_cust_data(this: usize, guid: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    custom_value(&lib(this).custom, guid, out)
}
extern "system" fn lib_all_cust_data(this: usize, out: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let out = door_windows_abi::widen(out);
    all_custom(&lib(this).custom, out)
}
extern "system" fn get_lib_statistics(this: usize, names: usize, chars: usize) -> u32 {
    let this = door_windows_abi::widen(this);
    let names = door_windows_abi::widen(names);
    let chars = door_windows_abi::widen(chars);
    {
        let l = lib(this);
        let mut list: Vec<&str> = Vec::new();
        let mut add = |s: &'static str| {
            if !list.iter().any(|e| typelib::same_name(e, s)) {
                list.push(s);
            }
        };
        add(&l.name);
        for t in &l.types {
            add(&t.name);
            for f in &t.funcs {
                add(&f.name);
                for p in f.params.iter().filter_map(|p| p.name.as_deref()) {
                    add(p);
                }
            }
            for v in &t.vars {
                add(&v.name);
            }
        }
        let total: usize = list.iter().map(|s| s.len()).sum();
        let r = if names != 0 {
            write_u32(names, list.len() as u32)
        } else {
            Ok(())
        }
        .and_then(|_| {
            if chars != 0 {
                write_u32(chars, total as u32)
            } else {
                Ok(())
            }
        });
        result(r)
    }
}

// ---- exports ----------------------------------------------------------------------------
/// REGKIND_DEFAULT registers a library LoadTypeLib found by bare name.
const REGKIND_DEFAULT: u32 = 0;
const REGKIND_REGISTER: u32 = 1;
#[unsafe(no_mangle)]
pub extern "system" fn LoadTypeLibEx(path: usize, kind: u32, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let p = match read_wide(path) {
            Ok(p) if !p.is_empty() => p,
            Ok(_) => return hr::E_INVALIDARG,
            Err(e) => return e,
        };
        let id = match load(&p) {
            Ok(id) => id,
            Err(e) => return e,
        };
        let bare = !p
            .iter()
            .any(|&u| u == b'\\' as u16 || u == b'/' as u16 || u == b':' as u16);
        let object = type_lib(id);
        if (kind == REGKIND_REGISTER || (kind == REGKIND_DEFAULT && bare)) && id != stdole_id() {
            let mut stored = library_path(id);
            if stored.is_empty() {
                stored = full_path(&p);
            }
            let mut z = stored;
            z.push(0);
            let r = super::register::RegisterTypeLib(
                door_windows_abi::narrow(object),
                door_windows_abi::narrow(z.as_ptr() as u64),
                0,
            );
            if r != hr::S_OK {
                tl_release(door_windows_abi::narrow(object));
                return r;
            }
        }
        result(write_pointer(out, object))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadTypeLib(path: usize, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        LoadTypeLibEx(
            door_windows_abi::narrow(path),
            REGKIND_DEFAULT,
            door_windows_abi::narrow(out),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadRegTypeLib(
    guid: usize,
    major: u16,
    minor: u16,
    lcid: u32,
    out: usize,
) -> u32 {
    let guid = door_windows_abi::widen(guid);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let Ok(g) = read_n::<16>(guid) else {
            return hr::E_INVALIDARG;
        };
        let id = if g == stdole::GUID && major == 2 {
            stdole_id()
        } else {
            let path = match super::register::registered_path(&g, major, minor, lcid) {
                Ok(p) => p,
                Err(e) => return e,
            };
            match load(&path) {
                Ok(id) => id,
                Err(e) => return e,
            }
        };
        result(write_pointer(out, type_lib(id)))
    }
}
/// CreateDispTypeInfo: an interface from INTERFACEDATA (METHODDATA:
/// szName, ppdata, dispid, iMeth, cc, cArgs, wFlags, vtReturn; PARAMDATA:
/// szName, vt).
#[unsafe(no_mangle)]
pub extern "system" fn CreateDispTypeInfo(data: usize, lcid: u32, out: usize) -> u32 {
    let data = door_windows_abi::widen(data);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| -> Result<u64, u32> {
            // INTERFACEDATA { pmethdata, cMembers }; METHODDATA { szName,
            // ppdata, dispid, iMeth, cc, cArgs, wFlags, vtReturn } (40 bytes on
            // x64, 28 on x86); PARAMDATA { szName, vt } (16, 8).
            use door_windows_abi::layout::{self, Field::*};
            const METHODDATA: [layout::Field; 8] = [Pointer, Pointer, U32, U32, U32, U32, U16, U16];
            let at = |m: u64, index: usize| m + layout::offset(&METHODDATA, index) as u64;
            let methods = read_pointer(data)?;
            let count = read_u32(data + POINTER as u64)?;
            let mut funcs = Vec::new();
            for k in 0..count as u64 {
                let m = methods + layout::size(&METHODDATA) as u64 * k;
                let mut params = Vec::new();
                let pdata = read_pointer(at(m, 1))?;
                for j in 0..read_u32(at(m, 5))? as u64 {
                    let p = pdata + 2 * POINTER as u64 * j;
                    params.push(typelib::Param {
                        ty: TypeDesc::Base(read_u16(p + POINTER as u64)?),
                        flags: paramflag::IN,
                        default: None,
                        name: Some(text(&read_wide(read_pointer(p)?)?)),
                        custom: Vec::new(),
                    });
                }
                funcs.push(Func {
                    memid: read_u32(at(m, 2))? as i32,
                    kind: funckind::PUREVIRTUAL,
                    invoke: read_u16(at(m, 6))? as u32,
                    callconv: read_u32(at(m, 4))?,
                    ret: TypeDesc::Base(read_u16(at(m, 7))?),
                    params,
                    optional: 0,
                    // iMeth, in the type information's 8-byte slots.
                    vft: (read_u32(at(m, 3))? * 8) as i16,
                    flags: 0,
                    name: text(&read_wide(read_pointer(m)?)?),
                    doc: None,
                    help_context: 0,
                    help_string_context: 0,
                    entry: None,
                    custom: Vec::new(),
                });
            }
            let vft = funcs.iter().map(|f| f.vft as u16 + 8).max().unwrap_or(0);
            let lib = Library {
                guid: [0; 16],
                lcid,
                syskind: typelib::SYS_WIN64,
                major: 0,
                minor: 0,
                flags: 0,
                name: String::new(),
                doc: None,
                help_context: 0,
                help_string_context: 0,
                help_file: None,
                types: Vec::from([typelib::Type {
                    kind: tkind::INTERFACE,
                    guid: [0; 16],
                    flags: 0,
                    name: String::new(),
                    doc: None,
                    help_context: 0,
                    help_string_context: 0,
                    major: 0,
                    minor: 0,
                    size: 8,
                    align: 8,
                    vft,
                    impls: Vec::new(),
                    alias: None,
                    dll: None,
                    funcs,
                    vars: Vec::new(),
                    custom: Vec::new(),
                }]),
                imports: Vec::new(),
                files: Vec::new(),
                custom: Vec::new(),
            };
            let id = insert(lib, Vec::new());
            type_info(id, 0, Side::Natural)
        })();
        match r {
            Ok(p) => result(write_pointer(out, p)),
            Err(e) => e,
        }
    }
}
