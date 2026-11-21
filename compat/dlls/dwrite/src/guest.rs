//! dwrite.dll exports: DWriteCreateFactory and the COM objects behind it.
//! Every object is a `Com<T>`: its interface table, a reference count and
//! its data; interface tables are static arrays of `extern "system"`
//! functions in the order of dwrite.h.
use crate::font::{self, Bytes, Collection, File};
use crate::*;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use door_windows_abi::widen;

mod files;
mod fonts;
mod render;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn DoorSystemFontFiles(out: usize, capacity: u32) -> u32;
}

// ---- COM plumbing -------------------------------------------------------------------
pub(crate) type Guid = [u8; 16];
pub(crate) const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}
pub(crate) const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// An interface table: `N` function pointers.
#[repr(transparent)]
pub(crate) struct Vtbl<const N: usize>(pub [*const (); N]);
unsafe impl<const N: usize> Sync for Vtbl<N> {}

/// A COM object: the interface table first, as COM requires.
#[repr(C)]
pub(crate) struct Com<T> {
    vtbl: *const (),
    refs: AtomicU32,
    pub data: T,
}
/// What an object type answers in QueryInterface.
pub(crate) trait Object: Sized + 'static {
    const IIDS: &'static [Guid];
    fn vtbl() -> *const ();
}
pub(crate) fn new<T: Object>(data: T) -> usize {
    Box::into_raw(Box::new(Com {
        vtbl: T::vtbl(),
        refs: AtomicU32::new(1),
        data,
    })) as usize
}
/// The data of `this` (one of our objects of type `T`).
pub(crate) fn data<'a, T: Object>(this: usize) -> &'a T {
    unsafe { &(*(this as *const Com<T>)).data }
}
/// `pointer` as our object of type `T`, if it is one.
pub(crate) fn ours<'a, T: Object>(pointer: usize) -> Option<&'a T> {
    (pointer != 0 && unsafe { *(pointer as *const *const ()) } == T::vtbl())
        .then(|| data::<T>(pointer))
}
pub(crate) extern "system" fn add_ref<T: Object>(this: usize) -> u32 {
    unsafe {
        (*(this as *const Com<T>))
            .refs
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }
}
pub(crate) extern "system" fn release<T: Object>(this: usize) -> u32 {
    let left = unsafe {
        (*(this as *const Com<T>))
            .refs
            .fetch_sub(1, Ordering::AcqRel)
            - 1
    };
    if left == 0 {
        drop(unsafe { Box::from_raw(this as *mut Com<T>) });
    }
    left
}
pub(crate) extern "system" fn query<T: Object>(this: usize, iid: usize, out: usize) -> u32 {
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let wanted: Guid = unsafe { (iid as *const Guid).read_unaligned() };
    if wanted == IID_IUNKNOWN || T::IIDS.contains(&wanted) {
        add_ref::<T>(this);
        put_ptr(out, this);
        S_OK
    } else {
        put_ptr(out, 0);
        E_NOINTERFACE
    }
}
/// QueryInterface of a static object (no count to raise: its AddRef and
/// Release are `static_count`).
pub(crate) extern "system" fn static_query<T: Object>(this: usize, iid: usize, out: usize) -> u32 {
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let wanted: Guid = unsafe { (iid as *const Guid).read_unaligned() };
    if wanted == IID_IUNKNOWN || T::IIDS.contains(&wanted) {
        put_ptr(out, this);
        S_OK
    } else {
        put_ptr(out, 0);
        E_NOINTERFACE
    }
}
/// A static object's AddRef and Release (never freed).
pub(crate) extern "system" fn static_count(_this: usize) -> u32 {
    1
}
/// Slot `index` of a (possibly foreign) object's interface table.
pub(crate) fn method(object: usize, index: usize) -> *const () {
    unsafe { *(*(object as *const *const *const ())).add(index) }
}
pub(crate) fn com_add_ref(object: usize) {
    if object != 0 {
        let f: extern "system" fn(usize) -> u32 =
            unsafe { core::mem::transmute(method(object, 1)) };
        f(object);
    }
}
pub(crate) fn com_release(object: usize) {
    if object != 0 {
        let f: extern "system" fn(usize) -> u32 =
            unsafe { core::mem::transmute(method(object, 2)) };
        f(object);
    }
}

// ---- guest memory -------------------------------------------------------------------
pub(crate) fn put_ptr(at: usize, value: usize) {
    if at != 0 {
        unsafe { (at as *mut usize).write_unaligned(value) };
    }
}
pub(crate) fn put_u32(at: usize, value: u32) {
    if at != 0 {
        unsafe { (at as *mut u32).write_unaligned(value) };
    }
}
pub(crate) fn put_f32(at: usize, value: f32) {
    if at != 0 {
        unsafe { (at as *mut f32).write_unaligned(value) };
    }
}
pub(crate) fn put_bytes(at: usize, bytes: &[u8]) {
    if at != 0 {
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, bytes.len()) };
    }
}
pub(crate) fn get_u32(at: usize) -> u32 {
    unsafe { (at as *const u32).read_unaligned() }
}
pub(crate) fn get_u16(at: usize) -> u16 {
    unsafe { (at as *const u16).read_unaligned() }
}
pub(crate) fn get_f32(at: usize) -> f32 {
    unsafe { (at as *const f32).read_unaligned() }
}
pub(crate) fn get_ptr(at: usize) -> usize {
    unsafe { (at as *const usize).read_unaligned() }
}
pub(crate) fn bytes<'a>(at: usize, n: usize) -> &'a [u8] {
    if at == 0 || n == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(at as *const u8, n) }
    }
}
/// A NUL-terminated UTF-16 string.
pub(crate) fn wide(at: usize, max: usize) -> Vec<u16> {
    let mut out = Vec::new();
    if at == 0 {
        return out;
    }
    while out.len() < max {
        let u = get_u16(at + 2 * out.len());
        if u == 0 {
            break;
        }
        out.push(u);
    }
    out
}
pub(crate) fn text(units: &[u16]) -> alloc::string::String {
    char::decode_utf16(units.iter().copied())
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect()
}

// ---- the system font collection -----------------------------------------------------
pub(crate) struct Once<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<Option<T>>,
}
unsafe impl<T> Sync for Once<T> {}
impl<T: Clone> Once<T> {
    pub(crate) const fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(None),
        }
    }
    pub(crate) fn get(&self, make: impl FnOnce() -> T) -> T {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        let slot = unsafe { &mut *self.value.get() };
        let value = slot.get_or_insert_with(make).clone();
        self.held.store(false, Ordering::Release);
        value
    }
}
/// The system font files: gdi32's shared views, as C:\Windows\Fonts paths.
static SYSTEM_FILES: Once<Arc<Vec<Arc<File>>>> = Once::new();
static SYSTEM: Once<Arc<Collection>> = Once::new();
pub(crate) fn system_files() -> Arc<Vec<Arc<File>>> {
    SYSTEM_FILES.get(|| {
        let width = core::mem::size_of::<usize>();
        let count = unsafe { DoorSystemFontFiles(0, 0) } as usize;
        let mut records = alloc::vec![0usize; 3 * count];
        let written =
            unsafe { DoorSystemFontFiles(records.as_mut_ptr() as usize, count as u32) } as usize;
        let mut out = Vec::new();
        for r in records.chunks_exact(3).take(written.min(count)) {
            let (address, length, name) = (r[0], r[1], r[2]);
            let _ = width;
            let name = {
                let mut n = 0;
                while n < 64 && unsafe { *(name as *const u8).add(n) } != 0 {
                    n += 1;
                }
                bytes(name, n)
            };
            let mut path: Vec<u16> = "C:\\Windows\\Fonts\\".encode_utf16().collect();
            path.extend(name.iter().map(|b| u16::from(*b)));
            let key: Vec<u8> = path.iter().flat_map(|u| u.to_le_bytes()).collect();
            out.push(Arc::new(File {
                bytes: Bytes::Static(unsafe {
                    core::slice::from_raw_parts(address as *const u8, length)
                }),
                path,
                key,
                loader: 0,
            }));
        }
        Arc::new(out)
    })
}
pub(crate) fn system() -> Arc<Collection> {
    SYSTEM.get(|| {
        let mut all = Vec::new();
        for f in system_files().iter() {
            all.extend(font::faces(f));
        }
        let mut c = Collection::from_faces(all);
        for (alias, target) in SUBSTITUTES {
            c.alias(alias, target);
        }
        Arc::new(c)
    })
}

// ---- the factory -------------------------------------------------------------------
const IID_FACTORY: Guid = guid(
    0xb859ee5a,
    0xd838,
    0x4b5b,
    [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48],
);
const IID_FACTORY1: Guid = guid(
    0x30572f99,
    0xdac6,
    0x41db,
    [0xa1, 0x6e, 0x04, 0x86, 0x30, 0x7e, 0x60, 0x6a],
);
const IID_FACTORY2: Guid = guid(
    0x0439fc60,
    0xca44,
    0x4994,
    [0x8d, 0xee, 0x3a, 0x9a, 0xf7, 0xb7, 0x32, 0xec],
);

pub(crate) struct Factory;
static FACTORY_VTBL: Vtbl<31> = Vtbl([
    static_query::<Factory> as *const (),
    static_count as *const (),
    static_count as *const (),
    // IDWriteFactory.
    get_system_font_collection as *const (),
    files::create_custom_font_collection as *const (),
    register_loader as *const (),
    register_loader as *const (),
    files::create_font_file_reference as *const (),
    files::create_custom_font_file_reference as *const (),
    fonts::create_font_face as *const (),
    render::create_rendering_params as *const (),
    render::create_monitor_rendering_params as *const (),
    render::create_custom_rendering_params as *const (),
    register_loader as *const (),
    register_loader as *const (),
    not_implemented8 as *const (),
    not_implemented1 as *const (),
    not_implemented1 as *const (),
    not_implemented6 as *const (),
    not_implemented9 as *const (),
    not_implemented2 as *const (),
    not_implemented1 as *const (),
    not_implemented4 as *const (),
    render::create_glyph_run_analysis as *const (),
    // IDWriteFactory1.
    get_eudc_font_collection as *const (),
    render::create_custom_rendering_params1 as *const (),
    // IDWriteFactory2.
    render::get_system_font_fallback as *const (),
    not_implemented1 as *const (),
    render::translate_color_glyph_run as *const (),
    render::create_custom_rendering_params2 as *const (),
    render::create_glyph_run_analysis2 as *const (),
]);
impl Object for Factory {
    const IIDS: &'static [Guid] = &[IID_FACTORY, IID_FACTORY1, IID_FACTORY2];
    fn vtbl() -> *const () {
        &FACTORY_VTBL as *const _ as *const ()
    }
}
#[repr(C)]
struct StaticObject {
    vtbl: *const (),
}
unsafe impl Sync for StaticObject {}
static FACTORY: StaticObject = StaticObject {
    vtbl: &FACTORY_VTBL as *const _ as *const (),
};
pub(crate) fn factory() -> usize {
    &FACTORY as *const StaticObject as usize
}
/// DWriteCreateFactory: the shared factory (isolated factories share it
/// too: the system collection is immutable).
#[unsafe(no_mangle)]
pub extern "system" fn DWriteCreateFactory(_kind: u32, iid: usize, out: usize) -> u32 {
    let (iid, out) = (widen(iid) as usize, widen(out) as usize);
    if out == 0 {
        return E_POINTER;
    }
    static_query::<Factory>(factory(), iid, out)
}
extern "system" fn get_system_font_collection(_this: usize, out: usize, _check: i32) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    put_ptr(out, fonts::new_collection(system()));
    S_OK
}
extern "system" fn get_eudc_font_collection(_this: usize, out: usize, _check: i32) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    put_ptr(out, fonts::new_collection(Arc::new(Collection::default())));
    S_OK
}
/// Loader registration: loaders are called through the pointers files and
/// collections carry, so registering records nothing.
extern "system" fn register_loader(_this: usize, loader: usize) -> u32 {
    if loader == 0 { E_INVALIDARG } else { S_OK }
}
macro_rules! not_implemented {
    ($($name:ident($($arg:ident),*)),* $(,)?) => {$(
        /// Text formats, layouts, analyzers and GDI interop are not
        /// provided (Chromium shapes and lays out with HarfBuzz).
        extern "system" fn $name(_this: usize, $($arg: usize),*) -> u32 {
            let args = [$($arg),*];
            // The out pointer is the last argument.
            if let Some(out) = args.last() {
                put_ptr(*out, 0);
            }
            E_NOTIMPL
        }
    )*};
}
// Float arguments are pointer-sized in these slots on x64 (registers) and
// four bytes on x86, as usize is; their values are unused.
not_implemented!(
    not_implemented1(a),
    not_implemented2(a, b),
    not_implemented4(a, b, c, d),
    not_implemented6(a, b, c, d, e, f),
    not_implemented8(a, b, c, d, e, f, g, h),
    not_implemented9(a, b, c, d, e, f, g, h, i),
);
