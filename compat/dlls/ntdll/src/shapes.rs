//! The Windows structures ntdll reads from and writes into a program's
//! memory, at the build's pointer width (P17): the x64 DLL lays them out as
//! 64-bit programs do, the x86 one as 32-bit programs do
//! (`door_windows_abi::layout`), and pointer-sized out-parameters (handles,
//! SIZE_T, ULONG_PTR) are written at that width.
use super::guest::copy;
use door_windows_abi::layout::{self, Field, Field::*};

pub(super) const POINTER: usize = layout::POINTER;
/// OVERLAPPED: Internal, InternalHigh, Offset, OffsetHigh, hEvent.
pub(super) const OVERLAPPED: [Field; 5] = [Pointer, Pointer, U32, U32, Pointer];
/// OVERLAPPED_ENTRY: lpCompletionKey, lpOverlapped, Internal,
/// dwNumberOfBytesTransferred.
pub(super) const OVERLAPPED_ENTRY: [Field; 4] = [Pointer, Pointer, Pointer, U32];
/// PROCESS_INFORMATION: hProcess, hThread, dwProcessId, dwThreadId.
pub(super) const PROCESS_INFORMATION: [Field; 4] = [Pointer, Pointer, U32, U32];
/// STARTUPINFOW: cb, lpReserved, lpDesktop, lpTitle, dwX, dwY, dwXSize,
/// dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags,
/// wShowWindow, cbReserved2, lpReserved2, hStdInput, hStdOutput, hStdError
/// (104 bytes on x64, 68 on x86).
pub(super) const STARTUPINFOW: [Field; 18] = [
    U32, Pointer, Pointer, Pointer, U32, U32, U32, U32, U32, U32, U32, U32, U16, U16, Pointer,
    Pointer, Pointer, Pointer,
];
pub(super) mod startup {
    pub const FLAGS: usize = 11;
    pub const STD_INPUT: usize = 15;
}
/// IO_STATUS_BLOCK: Status (a union with a pointer), Information.
pub(super) const IO_STATUS_BLOCK: [Field; 2] = [Pointer, Pointer];

pub(super) const fn size(fields: &[Field]) -> usize {
    layout::size(fields)
}
pub(super) const fn offset(fields: &[Field], index: usize) -> u64 {
    layout::offset(fields, index) as u64
}
pub(super) fn get(bytes: &[u8], fields: &[Field], index: usize) -> u64 {
    layout::get(bytes, fields, index)
}
pub(super) fn set(bytes: &mut [u8], fields: &[Field], index: usize, value: u64) {
    layout::set(bytes, fields, index, value);
}
/// A pointer-sized value the program stored at `address`, widened.
pub(super) fn read_pointer(address: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    copy(b.as_mut_ptr() as u64, address, POINTER)?;
    Ok(layout::get(&b, &[Pointer], 0))
}
/// A handle, pointer or pointer-sized count into an out-parameter.
pub(super) fn write_pointer(address: u64, value: u64) -> Result<(), u32> {
    copy(address, value.to_le_bytes().as_ptr() as u64, POINTER)
}
