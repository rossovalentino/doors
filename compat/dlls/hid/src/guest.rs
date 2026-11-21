//! hid exports.

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
const ERROR_INVALID_FUNCTION: u32 = 1;
const HIDP_STATUS_INVALID_PREPARSED_DATA: u32 = 0xC011_0001;
/// The HidD_ queries: no handle is a HID device.
macro_rules! not_hid {
    ($($name:ident($($arg:ident),*) $(-> $out:ident)?;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "system" fn $name($($arg: usize),*) -> u8 {
            $(clear($out);)?
            fail(ERROR_INVALID_FUNCTION)
        }
    )*};
}
not_hid! {
    HidD_GetAttributes(_device, _attributes);
    HidD_GetPreparsedData(_device, data) -> data;
    HidD_GetProductString(_device, _buffer, _length);
    HidD_GetSerialNumberString(_device, _buffer, _length);
    HidD_GetManufacturerString(_device, _buffer, _length);
    HidD_GetFeature(_device, _buffer, _length);
    HidD_SetFeature(_device, _buffer, _length);
    HidD_GetInputReport(_device, _buffer, _length);
    HidD_SetOutputReport(_device, _buffer, _length);
}
#[unsafe(no_mangle)]
pub extern "system" fn HidD_FreePreparsedData(_data: usize) -> u8 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn HidD_GetHidGuid(guid: usize) {
    // GUID_DEVINTERFACE_HID {4D1E55B2-F16F-11CF-88CB-001111000030}.
    const HID: [u8; 16] = [
        0xB2, 0x55, 0x1E, 0x4D, 0x6F, 0xF1, 0xCF, 0x11, 0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00,
        0x30,
    ];
    if guid != 0 {
        // SAFETY: the caller's GUID out-parameter.
        unsafe { (guid as *mut [u8; 16]).write_unaligned(HID) };
    }
}
/// The HidP_ parsers: no preparsed data is valid.
macro_rules! invalid_data {
    ($($name:ident($($arg:ident),*);)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            HIDP_STATUS_INVALID_PREPARSED_DATA
        }
    )*};
}
invalid_data! {
    HidP_GetCaps(_data, _caps);
    HidP_GetButtonCaps(_kind, _caps, _length, _data);
    HidP_GetValueCaps(_kind, _caps, _length, _data);
    HidP_GetUsageValue(_kind, _page, _collection, _usage, _value, _data, _report, _length);
    HidP_GetScaledUsageValue(_kind, _page, _collection, _usage, _value, _data, _report, _length);
    HidP_GetUsagesEx(_kind, _collection, _list, _length, _data, _report, _report_length);
}
