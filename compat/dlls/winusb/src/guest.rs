//! winusb exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}
fn fail<T: Default>(code: u32) -> T {
    unsafe { SetLastError(code) };
    T::default()
}
const ERROR_INVALID_HANDLE: u32 = 6;
/// Every WinUSB call: the handle is not a WinUSB device.
macro_rules! invalid {
    ($($name:ident($($arg:ident),*) $(-> $out:ident)?;)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub extern "system" fn $name($($arg: usize),*) -> i32 {
            $(clear($out);)?
            fail(ERROR_INVALID_HANDLE)
        }
    )*};
}
invalid! {
    WinUsb_Initialize(_device, handle) -> handle;
    WinUsb_GetAssociatedInterface(_handle, _index, handle) -> handle;
    WinUsb_ReadPipe(_handle, _pipe, _buffer, _length, _transferred, _overlapped);
    WinUsb_WritePipe(_handle, _pipe, _buffer, _length, _transferred, _overlapped);
    WinUsb_ResetPipe(_handle, _pipe);
    WinUsb_SetCurrentAlternateSetting(_handle, _setting);
    WinUsb_GetOverlappedResult(_handle, _overlapped, _transferred, _wait);
    WinUsb_Free(_handle);
}
/// WINUSB_SETUP_PACKET (8 bytes) by value.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Setup([u8; 8]);
#[unsafe(no_mangle)]
pub extern "system" fn WinUsb_ControlTransfer(
    _handle: usize,
    _setup: Setup,
    _buffer: usize,
    _length: u32,
    _transferred: usize,
    _overlapped: usize,
) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
