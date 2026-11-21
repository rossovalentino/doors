//! uiautomationcore exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const S_OK: u32 = 0;
const E_NOTIMPL: u32 = 0x8000_4001;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_INVALIDARG: u32 = 0x8007_0057;

/// The reserved "not supported" and "mixed attribute" values: static
/// objects answering IUnknown only, never freed.
#[repr(transparent)]
struct Vtbl([*const (); 3]);
// SAFETY: immutable function addresses.
unsafe impl Sync for Vtbl {}
#[repr(C)]
struct Reserved {
    vtbl: &'static Vtbl,
}
static VTBL: Vtbl = Vtbl([query as *const (), count as *const (), count as *const ()]);
static NOT_SUPPORTED: Reserved = Reserved { vtbl: &VTBL };
static MIXED: Reserved = Reserved { vtbl: &VTBL };
extern "system" fn query(this: usize, iid: usize, out: usize) -> u32 {
    const IUNKNOWN: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    if out == 0 || iid == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's IID and out-pointer.
    let wanted = unsafe { (iid as *const [u8; 16]).read_unaligned() };
    if wanted == IUNKNOWN {
        unsafe { (out as *mut usize).write_unaligned(this) };
        S_OK
    } else {
        clear(out);
        E_NOINTERFACE
    }
}
extern "system" fn count(_this: usize) -> u32 {
    1
}
fn reserved(object: &'static Reserved, out: usize) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's IUnknown** out-parameter.
    unsafe { (out as *mut usize).write_unaligned(object as *const Reserved as usize) };
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaGetReservedNotSupportedValue(out: usize) -> u32 {
    reserved(&NOT_SUPPORTED, out)
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaGetReservedMixedAttributeValue(out: usize) -> u32 {
    reserved(&MIXED, out)
}
/// WM_GETOBJECT for UiaRootObjectId: no client asked, nothing returned.
#[unsafe(no_mangle)]
pub extern "system" fn UiaReturnRawElementProvider(
    _window: usize,
    _wparam: usize,
    _lparam: isize,
    _provider: usize,
) -> isize {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaHostProviderFromHwnd(_window: usize, out: usize) -> u32 {
    clear(out);
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaClientsAreListening() -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseAutomationEvent(_provider: usize, _event: i32) -> u32 {
    S_OK
}
/// A VARIANT by value: 24 bytes on x64 (passed by reference), 16 on the
/// x86 stack.
#[cfg(target_pointer_width = "64")]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Variant([u64; 3]);
#[cfg(not(target_pointer_width = "64"))]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Variant([u32; 4]);
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseAutomationPropertyChangedEvent(
    _provider: usize,
    _property: i32,
    _old: Variant,
    _new: Variant,
) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaRaiseStructureChangedEvent(
    _provider: usize,
    _kind: i32,
    _id: usize,
    _length: i32,
) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaDisconnectProvider(_provider: usize) -> u32 {
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn UiaDisconnectAllProviders() -> u32 {
    S_OK
}
