//! cryptui exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}
fn fail<T: Default>(code: u32) -> T {
    unsafe { SetLastError(code) };
    T::default()
}
const ERROR_NOT_SUPPORTED: u32 = 50;
#[unsafe(no_mangle)]
pub extern "system" fn CryptUIDlgViewCertificateW(_info: usize, changed: usize) -> i32 {
    if changed != 0 {
        // SAFETY: the caller's BOOL out-parameter.
        unsafe { (changed as *mut i32).write_unaligned(0) };
    }
    fail(ERROR_NOT_SUPPORTED)
}
