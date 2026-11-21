//! winspool exports.

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
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INVALID_PRINTER_NAME: u32 = 1801;
/// EnumPrintersW: no printers (TRUE, nothing returned, nothing needed).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn EnumPrintersW(
    _flags: u32,
    _name: usize,
    _level: u32,
    _buffer: usize,
    _size: u32,
    needed: usize,
    returned: usize,
) -> i32 {
    for out in [needed, returned] {
        if out != 0 {
            // SAFETY: the caller's DWORD out-parameters.
            unsafe { (out as *mut u32).write_unaligned(0) };
        }
    }
    1
}
/// GetDefaultPrinterW (ordinal 203): there is none.
#[unsafe(no_mangle)]
pub extern "system" fn GetDefaultPrinterW(_buffer: usize, length: usize) -> i32 {
    if length != 0 {
        // SAFETY: the caller's DWORD in/out parameter.
        unsafe { (length as *mut u32).write_unaligned(0) };
    }
    fail(ERROR_FILE_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenPrinterW(_name: usize, printer: usize, _defaults: usize) -> i32 {
    clear(printer);
    fail(ERROR_INVALID_PRINTER_NAME)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenPrinter2W(
    _name: usize,
    printer: usize,
    _defaults: usize,
    _options: usize,
) -> i32 {
    clear(printer);
    fail(ERROR_INVALID_PRINTER_NAME)
}
#[unsafe(no_mangle)]
pub extern "system" fn ClosePrinter(_printer: usize) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPrinterW(
    _printer: usize,
    _level: u32,
    _buffer: usize,
    _size: u32,
    needed: usize,
) -> i32 {
    if needed != 0 {
        // SAFETY: the caller's DWORD out-parameter.
        unsafe { (needed as *mut u32).write_unaligned(0) };
    }
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPrinterDriverW(
    _printer: usize,
    _environment: usize,
    _level: u32,
    _buffer: usize,
    _size: u32,
    needed: usize,
) -> i32 {
    if needed != 0 {
        // SAFETY: the caller's DWORD out-parameter.
        unsafe { (needed as *mut u32).write_unaligned(0) };
    }
    fail(ERROR_INVALID_HANDLE)
}
/// DeviceCapabilitiesW: -1, no such printer.
#[unsafe(no_mangle)]
pub extern "system" fn DeviceCapabilitiesW(
    _device: usize,
    _port: usize,
    _capability: u16,
    _output: usize,
    _mode: usize,
) -> i32 {
    let _: i32 = fail(ERROR_INVALID_PRINTER_NAME);
    -1
}
/// DocumentPropertiesW: a negative value, no such printer.
#[unsafe(no_mangle)]
pub extern "system" fn DocumentPropertiesW(
    _window: usize,
    _printer: usize,
    _name: usize,
    _output: usize,
    _input: usize,
    _mode: u32,
) -> i32 {
    let _: i32 = fail(ERROR_INVALID_HANDLE);
    -1
}
