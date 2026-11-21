//! ncrypt exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const NTE_INVALID_HANDLE: u32 = 0x8009_0026;
const NTE_NOT_SUPPORTED: u32 = 0x8009_0029;
const NTE_PROV_DLL_NOT_FOUND: u32 = 0x8009_001E;
#[unsafe(no_mangle)]
pub extern "system" fn NCryptOpenStorageProvider(
    provider: usize,
    _name: usize,
    _flags: u32,
) -> u32 {
    clear(provider);
    NTE_PROV_DLL_NOT_FOUND
}
#[unsafe(no_mangle)]
pub extern "system" fn NCryptIsAlgSupported(
    _provider: usize,
    _algorithm: usize,
    _flags: u32,
) -> u32 {
    NTE_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn NCryptFreeObject(_object: usize) -> u32 {
    NTE_INVALID_HANDLE
}
/// Every operation on a provider or key: no handle is valid. `$out` is
/// the parameter cleared (an out-handle), if any.
macro_rules! invalid {
    ($($name:ident($($arg:ident),*) $(-> $out:ident)?;)*) => {$(
        #[unsafe(no_mangle)]
        #[allow(clippy::too_many_arguments)]
        pub extern "system" fn $name($($arg: usize),*) -> u32 {
            $(clear($out);)?
            NTE_INVALID_HANDLE
        }
    )*};
}
invalid! {
    NCryptOpenKey(_provider, key, _name, _spec, _flags) -> key;
    NCryptCreatePersistedKey(_provider, key, _algorithm, _name, _spec, _flags) -> key;
    NCryptImportKey(_provider, _import, _type, _parameters, key, _data, _size, _flags) -> key;
    NCryptExportKey(_key, _export, _type, _parameters, _output, _size, _result, _flags);
    NCryptFinalizeKey(_key, _flags);
    NCryptGetProperty(_object, _property, _output, _size, _result, _flags);
    NCryptSetProperty(_object, _property, _input, _size, _flags);
    NCryptSignHash(_key, _padding, _hash, _hash_size, _signature, _size, _result, _flags);
    NCryptDeleteKey(_key, _flags);
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn NCryptCreateClaim(
    _subject: usize,
    _authority: usize,
    _kind: u32,
    _parameters: usize,
    _data: usize,
    _size: u32,
    _result: usize,
    _flags: u32,
) -> u32 {
    NTE_NOT_SUPPORTED
}
