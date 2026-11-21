//! mfreadwrite exports.

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
#[unsafe(no_mangle)]
pub extern "system" fn MFCreateSourceReaderFromMediaSource(
    _source: usize,
    _attributes: usize,
    out: usize,
) -> u32 {
    clear(out);
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn MFCreateSourceReaderFromURL(
    _url: usize,
    _attributes: usize,
    out: usize,
) -> u32 {
    clear(out);
    E_NOTIMPL
}
