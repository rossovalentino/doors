//! oleacc exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const E_NOTIMPL: u32 = 0x8000_4001;
const E_FAIL: u32 = 0x8000_4005;
const E_INVALIDARG: u32 = 0x8007_0057;
#[unsafe(no_mangle)]
pub extern "system" fn AccessibleObjectFromWindow(
    _window: usize,
    _object: u32,
    _iid: usize,
    out: usize,
) -> u32 {
    clear(out);
    E_FAIL
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateStdAccessibleObject(
    _window: usize,
    _object: i32,
    _iid: usize,
    out: usize,
) -> u32 {
    clear(out);
    E_NOTIMPL
}
/// LresultFromObject: no marshalled reference (E_NOTIMPL as an LRESULT).
#[unsafe(no_mangle)]
pub extern "system" fn LresultFromObject(_iid: usize, _wparam: usize, _object: usize) -> isize {
    E_NOTIMPL as i32 as isize
}
#[unsafe(no_mangle)]
pub extern "system" fn ObjectFromLresult(
    _result: isize,
    _iid: usize,
    _wparam: usize,
    out: usize,
) -> u32 {
    clear(out);
    E_INVALIDARG
}
#[unsafe(no_mangle)]
pub extern "system" fn AccessibleChildren(
    _container: usize,
    _start: i32,
    _count: i32,
    _children: usize,
    obtained: usize,
) -> u32 {
    if obtained != 0 {
        // SAFETY: the caller's LONG out-parameter.
        unsafe { (obtained as *mut i32).write_unaligned(0) };
    }
    E_INVALIDARG
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromAccessibleObject(_object: usize, window: usize) -> u32 {
    clear(window);
    E_FAIL
}
