//! mfplat exports.

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
const E_INVALIDARG: u32 = 0x8007_0057;
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn QueryUnbiasedInterruptTime(time: usize) -> i32;
}
#[unsafe(no_mangle)]
pub extern "system" fn MFStartup(_version: u32, _flags: u32) -> u32 {
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn MFShutdown() -> u32 {
    S_OK
}
/// MFGetSystemTime: 100 ns since boot.
#[unsafe(no_mangle)]
pub extern "system" fn MFGetSystemTime() -> i64 {
    let mut t = 0u64;
    unsafe { QueryUnbiasedInterruptTime(&mut t as *mut u64 as usize) };
    t as i64
}
#[unsafe(no_mangle)]
pub extern "system" fn MFFrameRateToAverageTimePerFrame(
    numerator: u32,
    denominator: u32,
    out: usize,
) -> u32 {
    match (crate::average_time_per_frame(numerator, denominator), out) {
        (Some(t), o) if o != 0 => {
            // SAFETY: the caller's UINT64 out-parameter.
            unsafe { (o as *mut u64).write_unaligned(t) };
            S_OK
        }
        _ => E_INVALIDARG,
    }
}
/// The object factories: E_NOTIMPL with the output cleared. `$out` is
/// the parameter index of the interface out-pointer.
macro_rules! unavailable {
    ($($name:ident($($arg:ident),*) $(-> $out:ident)?;)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            $(clear($out);)?
            E_NOTIMPL
        }
    )*};
}
unavailable! {
    MFCreateAttributes(out, _size) -> out;
    MFCreateMediaType(out) -> out;
    MFCreateSample(out) -> out;
    MFCreateMemoryBuffer(_length, out) -> out;
    MFCreateAlignedMemoryBuffer(_length, _alignment, out) -> out;
    MFCreateMediaBufferWrapper(_buffer, _offset, _length, out) -> out;
    MFCreateDXGISurfaceBuffer(_iid, _surface, _index, _bottom_up, out) -> out;
    MFCreateDXGIDeviceManager(_token, out) -> out;
    MFCreateEventQueue(out) -> out;
    MFCreateMediaEvent(_kind, _extended, _status, _value, out) -> out;
    MFCreateAsyncResult(_object, _callback, _state, out) -> out;
    MFCreatePresentationDescriptor(_count, _streams, out) -> out;
    MFCreateStreamDescriptor(_id, _count, _types, out) -> out;
    MFCreateWaveFormatExFromMFMediaType(_media, out, _size, _flags) -> out;
    MFInitMediaTypeFromWaveFormatEx(_media, _format, _size);
    MFLockDXGIDeviceManager(_token, out) -> out;
    MFSerializeAttributesToStream(_attributes, _options, _stream);
}
#[unsafe(no_mangle)]
pub extern "system" fn MFUnlockDXGIDeviceManager() -> u32 {
    S_OK
}
/// A GUID passed by value (16 bytes on the stack on x86).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Guid([u32; 4]);
/// MFTEnumEx: no transforms are registered.
#[unsafe(no_mangle)]
pub extern "system" fn MFTEnumEx(
    _category: Guid,
    _flags: u32,
    _input: usize,
    _output: usize,
    activates: usize,
    count: usize,
) -> u32 {
    clear(activates);
    if count != 0 {
        // SAFETY: the caller's UINT32 out-parameter.
        unsafe { (count as *mut u32).write_unaligned(0) };
    }
    S_OK
}
