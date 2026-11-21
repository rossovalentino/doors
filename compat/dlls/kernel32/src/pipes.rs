//! Anonymous pipes (CreatePipe, PeekNamedPipe), handle duplication and
//! the console/file-type queries programs make before using a standard
//! handle. ntdll keeps the pipes; these are the Win32 entry points.
use door_windows_abi::{self as abi, error};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreatePipe(read: u64, write: u64) -> i32;
    fn DoorOpenCurrentThread() -> u64;
    fn DoorOpenCurrentProcess() -> u64;
    fn DoorPeekNamedPipe(
        handle: u64,
        buffer: u64,
        bytes: u32,
        read: u64,
        available: u64,
        left: u64,
    ) -> i32;
    fn DoorDuplicatePipe(handle: u64, target: u64) -> i32;
    fn DoorClosePipe(handle: u64) -> i32;
    fn DoorSetStandardHandle(selector: u32, handle: u64) -> i32;
    fn DoorConsoleMode(which: u32, set: u32, mode: u32) -> u32;
    fn DoorDuplicateKernelHandle(handle: u64, target: u64) -> i32;
    fn NtWriteVirtualMemory(
        process: usize,
        destination: usize,
        source: usize,
        bytes: usize,
        written: usize,
    ) -> u32;
    fn RtlSetLastWin32Error(error: u32);
    fn DoorFindFirstStreamW(path: u64, level: u32, data: u64, flags: u32) -> u64;
    fn DoorFindNextStreamW(handle: u64, data: u64) -> i32;
}
/// `FindFirstStreamW`: a file's data (`::$DATA`), then its named streams
/// (`:Zone.Identifier:$DATA`, ...); FindClose ends the search.
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstStreamW(path: usize, level: u32, data: usize, flags: u32) -> usize {
    let path = door_windows_abi::widen(path);
    let data = door_windows_abi::widen(data);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorFindFirstStreamW(path, level, data, flags) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextStreamW(handle: usize, data: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    unsafe { DoorFindNextStreamW(handle, data) }
}
/// GetCurrentThread()'s pseudo-handle, (HANDLE)-2.
const CURRENT_THREAD: u64 = u64::MAX - 1;
fn put<T: Copy>(pointer: u64, value: T) -> bool {
    pointer != 0
        && unsafe {
            NtWriteVirtualMemory(
                door_windows_abi::narrow(abi::CURRENT_PROCESS),
                door_windows_abi::narrow(pointer),
                door_windows_abi::narrow(&value as *const T as u64),
                core::mem::size_of::<T>(),
                0,
            )
        } == abi::status::SUCCESS
}
fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
/// A console handle or the NUL device: character devices that are not
/// objects of their own (closing or duplicating one keeps the same value).
fn console(handle: u64) -> bool {
    matches!(
        handle,
        abi::INPUT_HANDLE | abi::OUTPUT_HANDLE | abi::ERROR_HANDLE | abi::NUL_HANDLE
    )
}

/// `CreatePipe`: security attributes (inheritance) are not needed, as
/// STARTF_USESTDHANDLES passes the ends a launcher names; the buffer is
/// always 64 KiB.
#[unsafe(no_mangle)]
pub extern "system" fn CreatePipe(
    read: usize,
    write: usize,
    _attributes: usize,
    _size: u32,
) -> i32 {
    let read = door_windows_abi::widen(read);
    let write = door_windows_abi::widen(write);
    unsafe { DoorCreatePipe(read, write) }
}
#[unsafe(no_mangle)]
pub extern "system" fn PeekNamedPipe(
    handle: usize,
    buffer: usize,
    bytes: u32,
    read: usize,
    available: usize,
    left: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let read = door_windows_abi::widen(read);
    let available = door_windows_abi::widen(available);
    let left = door_windows_abi::widen(left);
    {
        if !abi::pipe::is_pipe(handle) {
            return fail(error::INVALID_HANDLE);
        }
        unsafe { DoorPeekNamedPipe(handle, buffer, bytes, read, available, left) }
    }
}
/// `GetFileType`: FILE_TYPE_DISK for files, FILE_TYPE_CHAR for the
/// console, FILE_TYPE_PIPE for pipes.
#[unsafe(no_mangle)]
pub extern "system" fn GetFileType(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    {
        if abi::file::is_file(handle) || crate::fileinfo::is_directory(handle) {
            1
        } else if console(handle) {
            2
        } else if abi::pipe::is_pipe(handle) || crate::named_pipes::is(handle) {
            3
        } else {
            unsafe { RtlSetLastWin32Error(error::INVALID_HANDLE) };
            0
        }
    }
}
/// The console mode a handle names: 0 the input buffer's, 1 the screen's.
fn mode_of(handle: u64) -> Option<u32> {
    match handle {
        abi::INPUT_HANDLE => Some(0),
        abi::OUTPUT_HANDLE | abi::ERROR_HANDLE => Some(1),
        _ => None,
    }
}
/// `GetConsoleMode`: the console input buffer's mode (shared by the
/// programs on the console: line, echo and processed input, VT input) or
/// this process's output mode (processed output, wrapping, VT processing,
/// DISABLE_NEWLINE_AUTO_RETURN); anything else is not a console.
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleMode(handle: usize, mode: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let mode = door_windows_abi::widen(mode);
    let Some(which) = mode_of(handle) else {
        return fail(error::INVALID_HANDLE);
    };
    let value = unsafe { DoorConsoleMode(which, 0, 0) };
    if value == u32::MAX {
        return 0;
    }
    if put(mode, value) {
        1
    } else {
        fail(error::NOACCESS)
    }
}
/// `SetConsoleMode`: the bits Windows defines for each kind of handle
/// (input 0x3ff, output 0x1f); others are ERROR_INVALID_PARAMETER.
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleMode(handle: usize, mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let Some(which) = mode_of(handle) else {
        return fail(error::INVALID_HANDLE);
    };
    let valid = if which == 0 { 0x3ff } else { 0x1f };
    if mode & !valid != 0 {
        return fail(error::INVALID_PARAMETER);
    }
    i32::from(unsafe { DoorConsoleMode(which, 1, mode) } != u32::MAX)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetStdHandle(selector: u32, handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorSetStandardHandle(selector, handle) }
}
/// `SetHandleInformation`: HANDLE_FLAG_INHERIT and
/// HANDLE_FLAG_PROTECT_FROM_CLOSE are accepted (a child gets only the
/// handles STARTF_USESTDHANDLES names).
#[unsafe(no_mangle)]
pub extern "system" fn SetHandleInformation(handle: usize, mask: u32, _flags: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if handle == 0 || handle == u64::MAX || mask & !3 != 0 {
            return fail(error::INVALID_PARAMETER);
        }
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetHandleInformation(handle: usize, flags: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let flags = door_windows_abi::widen(flags);
    {
        if handle == 0 || handle == u64::MAX {
            return fail(error::INVALID_HANDLE);
        }
        if put(flags, 0u32) {
            1
        } else {
            fail(error::NOACCESS)
        }
    }
}
/// `DuplicateHandle` within this process: another pipe end, the console
/// handle itself, or a kernel handle with the same rights.
/// DUPLICATE_CLOSE_SOURCE (1) closes the source.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DuplicateHandle(
    source_process: usize,
    source: usize,
    target_process: usize,
    target: usize,
    _access: u32,
    _inherit: i32,
    options: u32,
) -> i32 {
    let source_process = door_windows_abi::widen(source_process);
    let source = door_windows_abi::widen(source);
    let target_process = door_windows_abi::widen(target_process);
    let target = door_windows_abi::widen(target);
    {
        if !crate::system::current(source_process) || !crate::system::current(target_process) {
            return fail(error::NOT_SUPPORTED);
        }
        if target == 0 {
            return fail(error::INVALID_PARAMETER);
        }
        // GetCurrentThread's pseudo-handle: a real handle to this thread,
        // as Windows gives (CoreCLR keeps it to suspend and wait on the
        // thread from others).
        if source == CURRENT_THREAD {
            let handle = unsafe { DoorOpenCurrentThread() };
            return i32::from(handle != 0 && put(target, door_windows_abi::narrow(handle)));
        }
        // GetCurrentProcess's pseudo-handle: a real handle to this process.
        if source == abi::CURRENT_PROCESS {
            let handle = unsafe { DoorOpenCurrentProcess() };
            return i32::from(handle != 0 && put(target, door_windows_abi::narrow(handle)));
        }
        let done = if abi::pipe::is_pipe(source) {
            unsafe { DoorDuplicatePipe(source, target) }
        } else if crate::named_pipes::is(source) {
            match crate::named_pipes::duplicate(source) {
                Ok(handle) => i32::from(put(target, door_windows_abi::narrow(handle))),
                Err(code) => return fail(code),
            }
        } else if console(source) {
            i32::from(put(target, source))
        } else if abi::file::is_file(source) {
            return fail(error::NOT_SUPPORTED);
        } else {
            unsafe { DoorDuplicateKernelHandle(source, target) }
        };
        if done != 0 && options & 1 != 0 {
            if abi::pipe::is_pipe(source) {
                unsafe { DoorClosePipe(source) };
            } else if !console(source) {
                super::guest::CloseHandle(abi::narrow(source));
            }
        }
        done
    }
}
