//! P13 application exports: heaps and fixed global/local memory, file
//! position and size, current directory, directory enumeration and
//! attributes, module file names, resources, startup information, MulDiv,
//! `lstr*W` and UTF-8 code-page conversion. Every application pointer is
//! reached only through the checked-copy boundary; unsupported flags and
//! modes fail explicitly with their documented error.
use door_windows_abi::{self as abi, error, heap};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorSetFilePointerEx(handle: u64, distance: i64, destination: u64, method: u32) -> i32;
    fn DoorGetFileSizeEx(handle: u64, destination: u64) -> i32;
    fn DoorGetCurrentDirectoryW(capacity: u32, buffer: u64) -> u32;
    fn DoorSetCurrentDirectoryW(path: u64) -> i32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorGetFileAttributesExW(path: u64, level: u32, destination: u64) -> i32;
    fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64;
    fn DoorFindNextFileW(handle: u64, destination: u64) -> i32;
    fn DoorFindClose(handle: u64) -> i32;
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
    fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64;
    fn DoorLoadResource(module: u64, handle: u64) -> u64;
    fn DoorSizeofResource(module: u64, handle: u64) -> u32;
    fn DoorGetProcessHeap() -> u64;
    fn DoorGetCurrentProcessId() -> u32;
    #[allow(clippy::too_many_arguments)]
    fn DoorCreateProcessExW(
        application: u64,
        command_line: u64,
        directory: u64,
        flags: u32,
        info: u64,
        environment: u64,
        inherit: i32,
        startup: u64,
    ) -> i32;
    fn DoorGetExitCodeProcess(handle: u64, out: u64) -> i32;
    fn DoorGetProcessId(handle: u64) -> u32;
    fn DoorTerminateProcess(handle: u64, code: u32) -> i32;
    fn RtlCreateHeap(
        flags: u32,
        base: usize,
        reserve: usize,
        commit: usize,
        lock: usize,
        parameters: usize,
    ) -> usize;
    fn RtlDestroyHeap(heap: usize) -> usize;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlReAllocateHeap(heap: usize, flags: u32, pointer: usize, bytes: usize) -> usize;
    fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    fn RtlValidateHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlSetLastWin32Error(value: u32);
    fn NtQuerySystemTime(time: usize) -> u32;
    fn NtWriteVirtualMemory(
        process: usize,
        destination: usize,
        source: usize,
        bytes: usize,
        written: usize,
    ) -> u32;
}

fn set_error(value: u32) {
    // SAFETY: scalar-only import.
    unsafe { RtlSetLastWin32Error(value) }
}
/// Checked copy of at most `COPY_BYTES` through ntdll's native boundary.
fn copy(destination: u64, source: u64, bytes: usize) -> Result<(), u32> {
    if bytes == 0 {
        return Ok(());
    }
    // SAFETY: integer addresses are data to the checked kernel boundary.
    match unsafe {
        NtWriteVirtualMemory(
            door_windows_abi::narrow(abi::CURRENT_PROCESS),
            door_windows_abi::narrow(destination),
            door_windows_abi::narrow(source),
            bytes,
            0,
        )
    } {
        abi::status::SUCCESS => Ok(()),
        _ => Err(error::NOACCESS),
    }
}
fn store<T: Copy>(destination: u64, value: &T) -> Result<(), u32> {
    copy(
        destination,
        value as *const T as u64,
        core::mem::size_of::<T>(),
    )
}
fn load<T: Copy + Default>(source: u64) -> Result<T, u32> {
    let mut value = T::default();
    copy(
        &mut value as *mut T as u64,
        source,
        core::mem::size_of::<T>(),
    )?;
    Ok(value)
}

// ---- file position, size and current directory ----

const INVALID_SET_FILE_POINTER: u32 = u32::MAX;

/// Stores a zero position or size for the NUL device (none when the
/// destination is null).
fn nul_zero<T: Copy + Default>(destination: u64) -> i32 {
    if destination == 0 {
        return 1;
    }
    match store(destination, &T::default()) {
        Ok(()) => 1,
        Err(code) => {
            set_error(code);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetFilePointerEx(
    handle: usize,
    distance: i64,
    destination: usize,
    method: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let destination = door_windows_abi::widen(destination);
    if handle == abi::NUL_HANDLE {
        // The NUL device has no position: it stays at 0.
        return nul_zero::<i64>(destination);
    }
    unsafe { DoorSetFilePointerEx(handle, distance, destination, method) }
}
/// Without `high`, the 32-bit distance is sign-extended; with it, `*high`
/// supplies the upper half and receives the upper half of the new position.
#[unsafe(no_mangle)]
pub extern "system" fn SetFilePointer(handle: usize, low: i32, high: usize, method: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let high = door_windows_abi::widen(high);
    {
        let distance = if high == 0 {
            i64::from(low)
        } else {
            match load::<i32>(high) {
                Ok(upper) => (i64::from(upper) << 32) | i64::from(low as u32),
                Err(code) => {
                    set_error(code);
                    return INVALID_SET_FILE_POINTER;
                }
            }
        };
        let mut position = 0i64;
        if unsafe {
            DoorSetFilePointerEx(handle, distance, &mut position as *mut i64 as u64, method)
        } == 0
        {
            return INVALID_SET_FILE_POINTER;
        }
        if high != 0 {
            if let Err(code) = store(high, &((position >> 32) as i32)) {
                set_error(code);
                return INVALID_SET_FILE_POINTER;
            }
        } else if position > i64::from(u32::MAX - 1) {
            set_error(error::INVALID_PARAMETER);
            return INVALID_SET_FILE_POINTER;
        }
        // A legitimate low half equal to INVALID_SET_FILE_POINTER reports NO_ERROR.
        set_error(0);
        position as u32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileSizeEx(handle: usize, destination: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let destination = door_windows_abi::widen(destination);
    if handle == abi::NUL_HANDLE {
        return nul_zero::<u64>(destination);
    }
    unsafe { DoorGetFileSizeEx(handle, destination) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileSize(handle: usize, high: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let high = door_windows_abi::widen(high);
    {
        let mut bytes = 0u64;
        if unsafe { DoorGetFileSizeEx(handle, &mut bytes as *mut u64 as u64) } == 0 {
            return u32::MAX;
        }
        if high != 0 {
            if let Err(code) = store(high, &((bytes >> 32) as u32)) {
                set_error(code);
                return u32::MAX;
            }
        } else if bytes > u64::from(u32::MAX - 1) {
            set_error(error::INVALID_PARAMETER);
            return u32::MAX;
        }
        set_error(0);
        bytes as u32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentDirectoryW(capacity: u32, buffer: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    unsafe { DoorGetCurrentDirectoryW(capacity, buffer) }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCurrentDirectoryW(path: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorSetCurrentDirectoryW(path) }
}
/// The pseudo-handle of the calling process.
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcess() -> usize {
    door_windows_abi::narrow((move || -> u64 { abi::CURRENT_PROCESS })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcessId() -> u32 {
    unsafe { DoorGetCurrentProcessId() }
}

// ---- directory enumeration and attributes ----

#[unsafe(no_mangle)]
pub extern "system" fn GetFileAttributesW(path: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorGetFileAttributesW(path) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileAttributesExW(path: usize, level: u32, destination: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    let destination = door_windows_abi::widen(destination);
    unsafe { DoorGetFileAttributesExW(path, level, destination) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstFileW(pattern: usize, destination: usize) -> usize {
    let pattern = door_windows_abi::widen(pattern);
    let destination = door_windows_abi::widen(destination);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorFindFirstFileW(pattern, destination) }
    })())
}
/// Standard or basic information (no short names are reported either way),
/// name matching; the directory-only search is advisory, as documented.
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstFileExW(
    pattern: usize,
    level: u32,
    destination: usize,
    operation: u32,
    filter: usize,
    flags: u32,
) -> usize {
    let pattern = door_windows_abi::widen(pattern);
    let destination = door_windows_abi::widen(destination);
    let filter = door_windows_abi::widen(filter);
    door_windows_abi::narrow((move || -> u64 {
        if level > 1 || operation > 1 || filter != 0 || flags & !2 != 0 {
            set_error(error::INVALID_PARAMETER);
            return u64::MAX;
        }
        unsafe { DoorFindFirstFileW(pattern, destination) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextFileW(handle: usize, destination: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let destination = door_windows_abi::widen(destination);
    unsafe { DoorFindNextFileW(handle, destination) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FindClose(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorFindClose(handle) }
}

// ---- modules, resources and startup information ----

#[unsafe(no_mangle)]
pub extern "system" fn GetModuleFileNameW(module: usize, buffer: usize, capacity: u32) -> u32 {
    let module = door_windows_abi::widen(module);
    let buffer = door_windows_abi::widen(buffer);
    unsafe { DoorGetModuleFileNameW(module, buffer, capacity) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FindResourceW(module: usize, name: usize, kind: usize) -> usize {
    let module = door_windows_abi::widen(module);
    let name = door_windows_abi::widen(name);
    let kind = door_windows_abi::widen(kind);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorFindResource(module, kind, name, 0) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FindResourceExW(
    module: usize,
    kind: usize,
    name: usize,
    language: u16,
) -> usize {
    let module = door_windows_abi::widen(module);
    let kind = door_windows_abi::widen(kind);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorFindResource(module, kind, name, language) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadResource(module: usize, handle: usize) -> usize {
    let module = door_windows_abi::widen(module);
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorLoadResource(module, handle) }
    })())
}
/// Resource data is mapped with its image; the loaded handle is its address.
#[unsafe(no_mangle)]
pub extern "system" fn LockResource(data: usize) -> usize {
    let data = door_windows_abi::widen(data);
    door_windows_abi::narrow((move || -> u64 { data })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SizeofResource(module: usize, handle: usize) -> u32 {
    let module = door_windows_abi::widen(module);
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorSizeofResource(module, handle) }
}
/// Obsolete; image resources stay mapped. Returns FALSE as documented.
#[unsafe(no_mangle)]
pub extern "system" fn FreeResource(_data: usize) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetStartupInfoW(destination: usize) {
    let destination = door_windows_abi::widen(destination);
    {
        let mut page = [0u8; abi::startup::PARAMETER_FIELDS_BYTES];
        if copy(
            page.as_mut_ptr() as u64,
            abi::PARAMETERS_ADDRESS,
            page.len(),
        )
        .is_err()
        {
            set_error(error::NOACCESS);
            return;
        }
        match abi::startup::startup_info(&page) {
            Ok(info) => {
                if copy(destination, info.as_ptr() as u64, info.len()).is_err() {
                    set_error(error::NOACCESS);
                }
            }
            Err(_) => set_error(error::INVALID_PARAMETER),
        }
    }
}

// ---- heaps and fixed global/local memory ----

#[unsafe(no_mangle)]
pub extern "system" fn GetProcessHeap() -> usize {
    door_windows_abi::narrow((move || -> u64 { unsafe { DoorGetProcessHeap() } })())
}
/// Growable private heaps; a fixed maximum size is not supported.
#[unsafe(no_mangle)]
pub extern "system" fn HeapCreate(options: u32, _initial: usize, maximum: usize) -> usize {
    // A maximum makes a Windows heap fixed-size; Doors' heap stays
    // growable (allocations past the maximum succeed rather than fail).
    // HEAP_CREATE_ENABLE_EXECUTE heaps (CoreCLR's stub heap) are honoured.
    let _ = maximum;
    door_windows_abi::narrow((move || -> u64 {
        unsafe { door_windows_abi::widen(RtlCreateHeap(options | 0x2, 0, 0, 0, 0, 0)) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapDestroy(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    i32::from(unsafe { RtlDestroyHeap(door_windows_abi::narrow(handle)) } == 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapAlloc(handle: usize, flags: u32, bytes: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    door_windows_abi::narrow((move || -> u64 {
        unsafe {
            door_windows_abi::widen(RtlAllocateHeap(
                door_windows_abi::narrow(handle),
                flags,
                bytes,
            ))
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapFree(handle: usize, flags: u32, pointer: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    i32::from(unsafe {
        RtlFreeHeap(
            door_windows_abi::narrow(handle),
            flags,
            door_windows_abi::narrow(pointer),
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapReAlloc(
    handle: usize,
    flags: u32,
    pointer: usize,
    bytes: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    door_windows_abi::narrow((move || -> u64 {
        unsafe {
            door_windows_abi::widen(RtlReAllocateHeap(
                door_windows_abi::narrow(handle),
                flags,
                door_windows_abi::narrow(pointer),
                bytes,
            ))
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapSize(handle: usize, flags: u32, pointer: usize) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    unsafe {
        RtlSizeHeap(
            door_windows_abi::narrow(handle),
            flags,
            door_windows_abi::narrow(pointer),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn HeapValidate(handle: usize, flags: u32, pointer: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let pointer = door_windows_abi::widen(pointer);
    i32::from(unsafe {
        RtlValidateHeap(
            door_windows_abi::narrow(handle),
            flags,
            door_windows_abi::narrow(pointer),
        )
    })
}

const MEM_MOVEABLE: u32 = 0x2;
const MEM_ZEROINIT: u32 = 0x40;
const MEM_MODIFY: u32 = 0x80;
/// GMEM_/LMEM_ FIXED blocks on the process heap. Moveable (handle-based)
/// memory is outside this profile and fails with ERROR_NOT_SUPPORTED.
fn fixed_alloc(flags: u32, bytes: usize) -> u64 {
    if flags & MEM_MOVEABLE != 0 {
        set_error(error::NOT_SUPPORTED);
        return 0;
    }
    if flags & !(MEM_ZEROINIT | 0x2000 | 0x0100) != 0 {
        set_error(error::INVALID_PARAMETER);
        return 0;
    }
    let zero = if flags & MEM_ZEROINIT != 0 {
        heap::HEAP_ZERO_MEMORY
    } else {
        0
    };
    unsafe {
        door_windows_abi::widen(RtlAllocateHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            zero,
            bytes,
        ))
    }
}
fn fixed_free(block: u64) -> u64 {
    if block == 0
        || unsafe {
            RtlFreeHeap(
                door_windows_abi::narrow(DoorGetProcessHeap()),
                0,
                door_windows_abi::narrow(block),
            )
        } != 0
    {
        0
    } else {
        set_error(error::INVALID_HANDLE);
        block
    }
}
fn fixed_lock(block: u64) -> u64 {
    if block != 0
        && unsafe {
            RtlValidateHeap(
                door_windows_abi::narrow(DoorGetProcessHeap()),
                0,
                door_windows_abi::narrow(block),
            )
        } != 0
    {
        block
    } else {
        set_error(error::INVALID_HANDLE);
        0
    }
}
/// Fixed memory has no lock count: the block stays valid and unlocked.
fn fixed_unlock(block: u64) -> i32 {
    if fixed_lock(block) != 0 {
        set_error(0);
    }
    0
}
fn fixed_size(block: u64) -> usize {
    match unsafe {
        RtlSizeHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            0,
            door_windows_abi::narrow(block),
        )
    } {
        usize::MAX => {
            set_error(error::INVALID_HANDLE);
            0
        }
        bytes => bytes,
    }
}
/// A fixed block may move only with MOVEABLE; otherwise it resizes in place.
fn fixed_realloc(block: u64, bytes: usize, flags: u32) -> u64 {
    if flags & MEM_MODIFY != 0 || flags & !(MEM_MOVEABLE | MEM_ZEROINIT | MEM_MODIFY) != 0 {
        set_error(error::INVALID_PARAMETER);
        return 0;
    }
    let mut heap_flags = if flags & MEM_ZEROINIT != 0 {
        heap::HEAP_ZERO_MEMORY
    } else {
        0
    };
    if flags & MEM_MOVEABLE == 0 {
        heap_flags |= heap::HEAP_REALLOC_IN_PLACE_ONLY;
    }
    unsafe {
        door_windows_abi::widen(RtlReAllocateHeap(
            door_windows_abi::narrow(DoorGetProcessHeap()),
            heap_flags,
            door_windows_abi::narrow(block),
            bytes,
        ))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalAlloc(flags: u32, bytes: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 { fixed_alloc(flags, bytes) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalFree(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_free(block) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalLock(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_lock(block) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalUnlock(block: usize) -> i32 {
    let block = door_windows_abi::widen(block);
    fixed_unlock(block)
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalSize(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    fixed_size(block)
}
#[unsafe(no_mangle)]
pub extern "system" fn GlobalReAlloc(block: usize, bytes: usize, flags: u32) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_realloc(block, bytes, flags) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalAlloc(flags: u32, bytes: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 { fixed_alloc(flags, bytes) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalFree(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_free(block) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalLock(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_lock(block) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalUnlock(block: usize) -> i32 {
    let block = door_windows_abi::widen(block);
    fixed_unlock(block)
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalSize(block: usize) -> usize {
    let block = door_windows_abi::widen(block);
    fixed_size(block)
}
#[unsafe(no_mangle)]
pub extern "system" fn LocalReAlloc(block: usize, bytes: usize, flags: u32) -> usize {
    let block = door_windows_abi::widen(block);
    door_windows_abi::narrow((move || -> u64 { fixed_realloc(block, bytes, flags) })())
}

// ---- arithmetic and strings ----

/// `(a * b) / c` with a 64-bit intermediate, rounded half away from zero;
/// -1 when `c` is zero or the result does not fit.
#[unsafe(no_mangle)]
pub extern "system" fn MulDiv(a: i32, b: i32, c: i32) -> i32 {
    abi::text::mul_div(a, b, c)
}

/// Sequential reader of application memory in page-bounded chunks, so a
/// terminated string never forces a read past its last mapped page.
struct Source {
    address: u64,
    limit: Option<usize>,
    buffer: [u8; abi::COPY_BYTES],
    filled: usize,
    offset: usize,
    consumed: usize,
}
impl Source {
    fn new(address: u64, limit: Option<usize>) -> Self {
        Self {
            address,
            limit,
            buffer: [0; abi::COPY_BYTES],
            filled: 0,
            offset: 0,
            consumed: 0,
        }
    }
    fn byte(&mut self) -> Result<Option<u8>, u32> {
        if self.limit.is_some_and(|limit| self.consumed == limit) {
            return Ok(None);
        }
        if self.offset == self.filled {
            let page = 4096 - (self.address % 4096) as usize;
            let mut length = page.min(self.buffer.len());
            if let Some(limit) = self.limit {
                length = length.min(limit - self.consumed);
            }
            copy(self.buffer.as_mut_ptr() as u64, self.address, length)?;
            self.address += length as u64;
            self.filled = length;
            self.offset = 0;
        }
        let value = self.buffer[self.offset];
        self.offset += 1;
        self.consumed += 1;
        Ok(Some(value))
    }
    fn unit(&mut self) -> Result<Option<u16>, u32> {
        let Some(low) = self.byte()? else {
            return Ok(None);
        };
        let high = self.byte()?.ok_or(error::INVALID_PARAMETER)?;
        Ok(Some(u16::from_le_bytes([low, high])))
    }
}
/// Buffered writer that only counts when `capacity` is zero (size query).
struct Sink {
    address: u64,
    capacity: usize,
    count: usize,
    buffer: [u8; abi::COPY_BYTES],
    filled: usize,
}
impl Sink {
    fn new(address: u64, capacity: usize) -> Self {
        Self {
            address,
            capacity,
            count: 0,
            buffer: [0; abi::COPY_BYTES],
            filled: 0,
        }
    }
    /// Append one element of `bytes.len()` bytes (counted as one unit).
    fn put(&mut self, bytes: &[u8]) -> Result<(), u32> {
        if self.capacity != 0 {
            if self.count == self.capacity {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            if self.filled + bytes.len() > self.buffer.len() {
                self.flush()?;
            }
            self.buffer[self.filled..self.filled + bytes.len()].copy_from_slice(bytes);
            self.filled += bytes.len();
        }
        self.count += 1;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), u32> {
        copy(self.address, self.buffer.as_ptr() as u64, self.filled)?;
        self.address += self.filled as u64;
        self.filled = 0;
        Ok(())
    }
}
fn units_until_nul(pointer: u64) -> Result<usize, u32> {
    let mut source = Source::new(pointer, None);
    let mut length = 0;
    while source.unit()?.ok_or(error::INVALID_PARAMETER)? != 0 {
        length += 1;
    }
    Ok(length)
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrlenW(pointer: usize) -> i32 {
    let pointer = door_windows_abi::widen(pointer);
    {
        if pointer == 0 {
            return 0;
        }
        units_until_nul(pointer).map_or(0, |length| length as i32)
    }
}
/// Copies through the terminator; NULL on an invalid pointer.
fn copy_string(destination: u64, source: u64, capacity: Option<usize>) -> u64 {
    let result = (|| {
        let mut reader = Source::new(source, None);
        let mut writer = Sink::new(destination, usize::MAX);
        let mut written = 0;
        loop {
            let unit = reader.unit()?.ok_or(error::INVALID_PARAMETER)?;
            let last = capacity.is_some_and(|capacity| written + 1 == capacity);
            let unit = if last { 0 } else { unit };
            writer.put(&unit.to_le_bytes())?;
            written += 1;
            if unit == 0 {
                break;
            }
        }
        writer.flush()
    })();
    match result {
        Ok(()) => destination,
        Err(code) => {
            set_error(code);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcpyW(destination: usize, source: usize) -> usize {
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    door_windows_abi::narrow((move || -> u64 { copy_string(destination, source, None) })())
}
/// Copies at most `count - 1` units and always terminates (count > 0).
#[unsafe(no_mangle)]
pub extern "system" fn lstrcpynW(destination: usize, source: usize, count: i32) -> usize {
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    door_windows_abi::narrow((move || -> u64 {
        if count <= 0 {
            return if count == 0 { destination } else { 0 };
        }
        copy_string(destination, source, Some(count as usize))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcatW(destination: usize, source: usize) -> usize {
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    door_windows_abi::narrow((move || -> u64 {
        match units_until_nul(destination) {
            Ok(length) if copy_string(destination + length as u64 * 2, source, None) != 0 => {
                destination
            }
            Ok(_) => 0,
            Err(code) => {
                set_error(code);
                0
            }
        }
    })())
}

// ---- code pages: Door's ANSI and OEM code page is UTF-8 (65001) ----

const CP_UTF8: u32 = 65001;
const MB_ERR_INVALID_CHARS: u32 = 0x8;
const WC_ERR_INVALID_CHARS: u32 = 0x80;
fn utf8_code_page(code_page: u32) -> bool {
    matches!(code_page, 0 | 1 | 3 | CP_UTF8)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetACP() -> u32 {
    CP_UTF8
}
#[unsafe(no_mangle)]
pub extern "system" fn GetOEMCP() -> u32 {
    CP_UTF8
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidCodePage(code_page: u32) -> i32 {
    i32::from(code_page == CP_UTF8)
}
fn source_limit(pointer: u64, length: i32) -> Result<Option<usize>, u32> {
    if pointer == 0 || length == 0 || length < -1 {
        return Err(error::INVALID_PARAMETER);
    }
    Ok((length != -1).then_some(length as usize))
}
#[unsafe(no_mangle)]
pub extern "system" fn MultiByteToWideChar(
    code_page: u32,
    flags: u32,
    source: usize,
    length: i32,
    destination: usize,
    capacity: i32,
) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    {
        let result = (|| {
            if !utf8_code_page(code_page) {
                return Err(error::INVALID_PARAMETER);
            }
            if flags & !MB_ERR_INVALID_CHARS != 0 {
                return Err(error::INVALID_FLAGS);
            }
            if capacity < 0 || (capacity > 0 && destination == 0) {
                return Err(error::INVALID_PARAMETER);
            }
            let terminated = length == -1;
            let limit = source_limit(source, length)?;
            let mut reader = Source::new(source, limit);
            let mut writer = Sink::new(destination, capacity as usize);
            let mut decoder = abi::text::Utf8Decoder::new();
            let strict = flags & MB_ERR_INVALID_CHARS != 0;
            loop {
                let byte = reader.byte()?;
                let mut emit = |scalar: Option<char>| -> Result<(), u32> {
                    let Some(character) = scalar else {
                        return if strict {
                            Err(error::NO_UNICODE_TRANSLATION)
                        } else {
                            writer.put(&0xfffdu16.to_le_bytes())
                        };
                    };
                    let mut units = [0u16; 2];
                    for unit in character.encode_utf16(&mut units) {
                        writer.put(&unit.to_le_bytes())?;
                    }
                    Ok(())
                };
                match byte {
                    None => {
                        if decoder.pending() {
                            emit(None)?;
                        }
                        if terminated {
                            return Err(error::INVALID_PARAMETER);
                        }
                        break;
                    }
                    Some(byte) => {
                        let (first, second) = decoder.push(byte);
                        if let Some(value) = first {
                            emit(value)?;
                        }
                        if let Some(value) = second {
                            emit(value)?;
                        }
                        if terminated && byte == 0 && !decoder.pending() {
                            break;
                        }
                    }
                }
            }
            writer.flush()?;
            Ok(writer.count as i32)
        })();
        result.unwrap_or_else(|code| {
            set_error(code);
            0
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn WideCharToMultiByte(
    code_page: u32,
    flags: u32,
    source: usize,
    length: i32,
    destination: usize,
    capacity: i32,
    default_char: usize,
    used_default: usize,
) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    let default_char = door_windows_abi::widen(default_char);
    let used_default = door_windows_abi::widen(used_default);
    {
        let result = (|| {
            if !utf8_code_page(code_page) || default_char != 0 || used_default != 0 {
                return Err(error::INVALID_PARAMETER);
            }
            if flags & !WC_ERR_INVALID_CHARS != 0 {
                return Err(error::INVALID_FLAGS);
            }
            if capacity < 0 || (capacity > 0 && destination == 0) {
                return Err(error::INVALID_PARAMETER);
            }
            let terminated = length == -1;
            let limit = source_limit(source, length)?
                .map(|units| units.checked_mul(2).ok_or(error::INVALID_PARAMETER))
                .transpose()?;
            let mut reader = Source::new(source, limit);
            // Bytes, not units, are counted: emit each UTF-8 byte separately.
            let mut writer = Sink::new(destination, capacity as usize);
            let strict = flags & WC_ERR_INVALID_CHARS != 0;
            let mut pending: Option<u16> = None;
            let emit = |writer: &mut Sink, scalar: Option<char>| -> Result<(), u32> {
                let character = match scalar {
                    Some(character) => character,
                    None if strict => return Err(error::NO_UNICODE_TRANSLATION),
                    None => '\u{fffd}',
                };
                let mut bytes = [0u8; 4];
                for &byte in character.encode_utf8(&mut bytes).as_bytes() {
                    writer.put(&[byte])?;
                }
                Ok(())
            };
            loop {
                let Some(unit) = reader.unit()? else {
                    if pending.take().is_some() {
                        emit(&mut writer, None)?;
                    }
                    if terminated {
                        return Err(error::INVALID_PARAMETER);
                    }
                    break;
                };
                if let Some(high) = pending.take() {
                    if (0xdc00..0xe000).contains(&unit) {
                        let scalar = 0x10000
                            + ((u32::from(high) - 0xd800) << 10)
                            + (u32::from(unit) - 0xdc00);
                        emit(&mut writer, char::from_u32(scalar))?;
                        continue;
                    }
                    emit(&mut writer, None)?;
                }
                if (0xd800..0xdc00).contains(&unit) {
                    pending = Some(unit);
                    continue;
                }
                emit(&mut writer, char::from_u32(u32::from(unit)))?;
                if terminated && unit == 0 {
                    break;
                }
            }
            writer.flush()?;
            Ok(writer.count as i32)
        })();
        result.unwrap_or_else(|code| {
            set_error(code);
            0
        })
    }
}

// ---- system time ----

/// FILETIME ticks per day and the days from 1601-01-01 to 1970-01-01.
const TICKS_PER_DAY: u64 = 864_000_000_000;
const DAYS_1601_TO_1970: i64 = 134_774;
fn now_filetime() -> u64 {
    let mut time = 0u64;
    // SAFETY: the import writes eight bytes at a local address.
    match unsafe { NtQuerySystemTime(door_windows_abi::narrow(&mut time as *mut u64 as u64)) } {
        0 => time,
        _ => 0,
    }
}
/// Days since 1970-01-01 to (year, month, day).
fn civil(days: i64) -> (i64, u16, u16) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u16;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u16;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}
fn days_from_civil(year: i64, month: u16, day: u16) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}
/// SYSTEMTIME { wYear, wMonth, wDayOfWeek, wDay, wHour, wMinute, wSecond,
/// wMilliseconds } of a FILETIME.
fn system_time(filetime: u64) -> [u16; 8] {
    let days = (filetime / TICKS_PER_DAY) as i64 - DAYS_1601_TO_1970;
    let within = filetime % TICKS_PER_DAY / 10_000;
    let (year, month, day) = civil(days);
    // 1970-01-01 was a Thursday (4).
    let weekday = (days + 4).rem_euclid(7) as u16;
    [
        year as u16,
        month,
        weekday,
        day,
        (within / 3_600_000) as u16,
        (within / 60_000 % 60) as u16,
        (within / 1000 % 60) as u16,
        (within % 1000) as u16,
    ]
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemTimeAsFileTime(destination: usize) {
    let destination = door_windows_abi::widen(destination);
    {
        let _ = store(destination, &now_filetime());
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemTimePreciseAsFileTime(destination: usize) {
    let destination = door_windows_abi::widen(destination);
    {
        let _ = store(destination, &now_filetime());
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemTime(destination: usize) {
    let destination = door_windows_abi::widen(destination);
    {
        let _ = store(destination, &system_time(now_filetime()));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FileTimeToSystemTime(source: usize, destination: usize) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    {
        let result = load::<u64>(source).and_then(|filetime| {
            // FILETIMEs from 0x8000000000000000 on are not times.
            if filetime >= 1 << 63 {
                return Err(87);
            }
            store(destination, &system_time(filetime))
        });
        match result {
            Ok(()) => 1,
            Err(e) => {
                set_error(e);
                0
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SystemTimeToFileTime(source: usize, destination: usize) -> i32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    {
        let result = load::<[u16; 8]>(source).and_then(|t| {
            let [year, month, _, day, hour, minute, second, millis] = t;
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            let month_days = [
                31,
                if leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            if !(1601..=30827).contains(&year)
                || !(1..=12).contains(&month)
                || day == 0
                || day > month_days[usize::from(month - 1)]
                || hour > 23
                || minute > 59
                || second > 59
                || millis > 999
            {
                return Err(87);
            }
            let days = (days_from_civil(i64::from(year), month, day) + DAYS_1601_TO_1970) as u64;
            let ticks = days * TICKS_PER_DAY
                + (u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(second))
                    * 10_000_000
                + u64::from(millis) * 10_000;
            store(destination, &ticks)
        });
        match result {
            Ok(()) => 1,
            Err(e) => {
                set_error(e);
                0
            }
        }
    }
}

// ---- processes --------------------------------------------------------------------
/// `CreateProcessW`: ntdll finds the program and Init starts it, with
/// lpEnvironment (which replaces the session's variables),
/// CREATE_SUSPENDED (ResumeThread on hThread starts it) and, with
/// bInheritHandles and STARTF_USESTDHANDLES, pipe ends as the child's
/// standard handles. Other inheritable handles and security attributes are
/// not carried; STARTUPINFO's window fields are the child's to read from
/// its own record.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateProcessW(
    application: usize,
    command_line: usize,
    _process_attributes: usize,
    _thread_attributes: usize,
    inherit: i32,
    flags: u32,
    environment: usize,
    directory: usize,
    startup: usize,
    information: usize,
) -> i32 {
    let application = door_windows_abi::widen(application);
    let command_line = door_windows_abi::widen(command_line);
    let environment = door_windows_abi::widen(environment);
    let directory = door_windows_abi::widen(directory);
    let startup = door_windows_abi::widen(startup);
    let information = door_windows_abi::widen(information);
    {
        unsafe {
            DoorCreateProcessExW(
                application,
                command_line,
                directory,
                flags,
                information,
                environment,
                inherit,
                startup,
            )
        }
    }
}
/// An ANSI string as NUL-terminated UTF-16 (None for a NULL pointer).
fn widened(pointer: u64, out: &mut [u16]) -> Result<Option<usize>, u32> {
    if pointer == 0 {
        return Ok(None);
    }
    let n = MultiByteToWideChar(
        0,
        0,
        abi::narrow(pointer),
        -1,
        out.as_mut_ptr() as usize,
        out.len() as i32,
    );
    if n <= 0 {
        return Err(error::INVALID_PARAMETER);
    }
    Ok(Some(n as usize))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateProcessA(
    application: usize,
    command_line: usize,
    process_attributes: usize,
    thread_attributes: usize,
    inherit: i32,
    flags: u32,
    environment: usize,
    directory: usize,
    startup: usize,
    information: usize,
) -> i32 {
    let application = door_windows_abi::widen(application);
    let command_line = door_windows_abi::widen(command_line);
    let process_attributes = door_windows_abi::widen(process_attributes);
    let thread_attributes = door_windows_abi::widen(thread_attributes);
    let environment = door_windows_abi::widen(environment);
    let directory = door_windows_abi::widen(directory);
    let startup = door_windows_abi::widen(startup);
    let information = door_windows_abi::widen(information);
    {
        let mut app = [0u16; 520];
        let mut line = [0u16; 2048];
        let mut dir = [0u16; 520];
        let converted = (|| {
            Ok::<_, u32>((
                widened(application, &mut app)?.is_some(),
                widened(command_line, &mut line)?.is_some(),
                widened(directory, &mut dir)?.is_some(),
            ))
        })();
        let Ok((a, l, d)) = converted else {
            set_error(error::INVALID_PARAMETER);
            return 0;
        };
        let pointer =
            |present: bool, buffer: &[u16]| if present { buffer.as_ptr() as usize } else { 0 };
        CreateProcessW(
            pointer(a, &app),
            pointer(l, &line),
            abi::narrow(process_attributes),
            abi::narrow(thread_attributes),
            inherit,
            flags,
            abi::narrow(environment),
            pointer(d, &dir),
            abi::narrow(startup),
            abi::narrow(information),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetExitCodeProcess(handle: usize, code: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let code = door_windows_abi::widen(code);
    unsafe { DoorGetExitCodeProcess(handle, code) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessId(handle: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorGetProcessId(handle) }
}
#[unsafe(no_mangle)]
pub extern "system" fn TerminateProcess(handle: usize, code: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if handle == u64::MAX {
            super::guest::ExitProcess(code);
        }
        unsafe { DoorTerminateProcess(handle, code) }
    }
}
