//! propsys exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[link(name = "ole32", kind = "dylib")]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> usize;
    fn PropVariantCopy(destination: usize, source: usize) -> u32;
    fn PropVariantClear(value: usize) -> u32;
}
#[link(name = "oleaut32", kind = "dylib")]
unsafe extern "system" {
    fn VarCmp(left: usize, right: usize, lcid: u32, flags: u32) -> u32;
}

const S_OK: u32 = 0;
const E_POINTER: u32 = 0x8000_4003;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_OUTOFMEMORY: u32 = 0x8007_000E;
const POINTER: usize = core::mem::size_of::<usize>();
/// PROPVARIANT: 24 bytes on x64, 16 on x86.
const PROPVARIANT: usize = 8 + 2 * POINTER;
const VT_CLSID: u16 = 72;

/// InitPropVariantFromCLSID: VT_CLSID with a task-memory copy.
#[unsafe(no_mangle)]
pub extern "system" fn InitPropVariantFromCLSID(clsid: usize, out: usize) -> u32 {
    if clsid == 0 || out == 0 {
        return E_INVALIDARG;
    }
    let copy = unsafe { CoTaskMemAlloc(16) };
    if copy == 0 {
        return E_OUTOFMEMORY;
    }
    // SAFETY: the caller's CLSID and PROPVARIANT, a fresh 16-byte block.
    unsafe {
        core::ptr::copy_nonoverlapping(clsid as *const u8, copy as *mut u8, 16);
        core::ptr::write_bytes(out as *mut u8, 0, PROPVARIANT);
        (out as *mut u16).write_unaligned(VT_CLSID);
        ((out + 8) as *mut usize).write_unaligned(copy);
    }
    S_OK
}
/// VariantCompare: VarCmp's order (-1, 0, 1); NULL sorts first.
#[unsafe(no_mangle)]
pub extern "system" fn VariantCompare(left: usize, right: usize) -> i32 {
    match unsafe { VarCmp(left, right, 0x0400, 0) } {
        0 => -1,
        1 => 0,
        2 => 1,
        // VARCMP_NULL or an error: compare the types, NULL/EMPTY first.
        _ => {
            // SAFETY: the caller's VARIANTs.
            let (a, b) = unsafe {
                (
                    (left as *const u16).read_unaligned(),
                    (right as *const u16).read_unaligned(),
                )
            };
            (a.min(2) as i32 - b.min(2) as i32).signum()
        }
    }
}

// ---- the in-memory property store -------------------------------------------------------
const IID_IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
/// IPropertyStore {886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99}.
const IID_IPROPERTYSTORE: [u8; 16] = [
    0xEB, 0x8E, 0x6D, 0x88, 0xF2, 0x8C, 0x46, 0x44, 0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99,
];
type Key = [u8; 20];
#[repr(transparent)]
struct Vtbl([*const (); 8]);
// SAFETY: immutable function addresses.
unsafe impl Sync for Vtbl {}
static VTBL: Vtbl = Vtbl([
    query as *const (),
    add_ref as *const (),
    release as *const (),
    count as *const (),
    get_at as *const (),
    get_value as *const (),
    set_value as *const (),
    commit as *const (),
]);
#[repr(C)]
struct Store {
    vtbl: &'static Vtbl,
    refs: AtomicU32,
    held: AtomicBool,
    /// Keys and PROPVARIANT copies.
    values: core::cell::UnsafeCell<Vec<(Key, [u8; 24])>>,
}
fn store<'a>(this: usize) -> &'a Store {
    // SAFETY: only our objects reach this table.
    unsafe { &*(this as *const Store) }
}
fn with<R>(this: usize, f: impl FnOnce(&mut Vec<(Key, [u8; 24])>) -> R) -> R {
    let s = store(this);
    while s.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *s.values.get() });
    s.held.store(false, Ordering::Release);
    r
}
extern "system" fn query(this: usize, iid: usize, out: usize) -> u32 {
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    // SAFETY: the caller's IID.
    let wanted = unsafe { (iid as *const [u8; 16]).read_unaligned() };
    if wanted == IID_IUNKNOWN || wanted == IID_IPROPERTYSTORE {
        add_ref(this);
        unsafe { (out as *mut usize).write_unaligned(this) };
        S_OK
    } else {
        clear(out);
        E_NOINTERFACE
    }
}
extern "system" fn add_ref(this: usize) -> u32 {
    store(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}
extern "system" fn release(this: usize) -> u32 {
    let left = store(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: the last reference to a Box::into_raw object.
        let s = unsafe { Box::from_raw(this as *mut Store) };
        for (_, mut value) in s.values.into_inner() {
            unsafe { PropVariantClear(value.as_mut_ptr() as usize) };
        }
    }
    left
}
extern "system" fn count(this: usize, out: usize) -> u32 {
    if out == 0 {
        return E_POINTER;
    }
    let n = with(this, |v| v.len() as u32);
    unsafe { (out as *mut u32).write_unaligned(n) };
    S_OK
}
extern "system" fn get_at(this: usize, index: u32, out: usize) -> u32 {
    if out == 0 {
        return E_POINTER;
    }
    match with(this, |v| v.get(index as usize).map(|(k, _)| *k)) {
        Some(k) => {
            unsafe { (out as *mut Key).write_unaligned(k) };
            S_OK
        }
        None => E_INVALIDARG,
    }
}
fn key(at: usize) -> Option<Key> {
    // SAFETY: the caller's PROPERTYKEY.
    (at != 0).then(|| unsafe { (at as *const Key).read_unaligned() })
}
extern "system" fn get_value(this: usize, at: usize, out: usize) -> u32 {
    let (Some(k), true) = (key(at), out != 0) else {
        return E_POINTER;
    };
    let found = with(this, |v| {
        v.iter().find(|(x, _)| *x == k).map(|(_, value)| *value)
    });
    match found {
        Some(value) => unsafe { PropVariantCopy(out, value.as_ptr() as usize) },
        None => {
            // VT_EMPTY.
            unsafe { core::ptr::write_bytes(out as *mut u8, 0, PROPVARIANT) };
            S_OK
        }
    }
}
extern "system" fn set_value(this: usize, at: usize, value: usize) -> u32 {
    let (Some(k), true) = (key(at), value != 0) else {
        return E_POINTER;
    };
    let mut copy = [0u8; 24];
    let r = unsafe { PropVariantCopy(copy.as_mut_ptr() as usize, value) };
    if r != S_OK {
        return r;
    }
    let old = with(this, |v| match v.iter_mut().find(|(x, _)| *x == k) {
        Some(entry) => Some(core::mem::replace(&mut entry.1, copy)),
        None => {
            v.push((k, copy));
            None
        }
    });
    if let Some(mut old) = old {
        unsafe { PropVariantClear(old.as_mut_ptr() as usize) };
    }
    S_OK
}
extern "system" fn commit(_this: usize) -> u32 {
    S_OK
}
/// PSCreateMemoryPropertyStore: a new in-memory IPropertyStore.
#[unsafe(no_mangle)]
pub extern "system" fn PSCreateMemoryPropertyStore(iid: usize, out: usize) -> u32 {
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    clear(out);
    let object = Box::into_raw(Box::new(Store {
        vtbl: &VTBL,
        refs: AtomicU32::new(1),
        held: AtomicBool::new(false),
        values: core::cell::UnsafeCell::new(Vec::new()),
    })) as usize;
    let r = query(object, iid, out);
    release(object);
    r
}
