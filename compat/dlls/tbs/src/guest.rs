//! tbs exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const TBS_E_INVALID_CONTEXT: u32 = 0x8028_4004;
const TBS_E_TPM_NOT_FOUND: u32 = 0x8028_400F;
#[unsafe(no_mangle)]
pub extern "system" fn Tbsi_GetDeviceInfo(_size: u32, _info: usize) -> u32 {
    TBS_E_TPM_NOT_FOUND
}
#[unsafe(no_mangle)]
pub extern "system" fn Tbsi_Context_Create(_parameters: usize, context: usize) -> u32 {
    clear(context);
    TBS_E_TPM_NOT_FOUND
}
#[unsafe(no_mangle)]
pub extern "system" fn Tbsip_Context_Close(_context: usize) -> u32 {
    TBS_E_INVALID_CONTEXT
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Tbsip_Submit_Command(
    _context: usize,
    _locality: u32,
    _priority: u32,
    _command: usize,
    _command_size: u32,
    _result: usize,
    _result_size: usize,
) -> u32 {
    TBS_E_INVALID_CONTEXT
}
