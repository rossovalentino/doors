//! Win32 uses only original ntdll imports. User pointers are integer addresses
//! until the native checked-copy operation captures or validates their storage.
use door_windows_abi::{self as abi, status};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorLoadLibraryExW(pointer: u64, flags: u32) -> u64;
    fn DoorLoadLibraryDatafile(pointer: u64, flags: u32) -> u64;
    fn DoorFreeLibrary(module: u64) -> i32;
    fn DoorGetModuleHandleW(pointer: u64) -> u64;
    fn DoorGetProcAddress(module: u64, pointer: u64) -> u64;
    #[link_name = "DoorLookupFunctionEntry"]
    fn NtLookupFunctionEntry(pc: u64, base: u64, history: u64) -> u64;
    #[link_name = "DoorVirtualUnwind"]
    fn NtVirtualUnwind(
        kind: u32,
        base: u64,
        pc: u64,
        entry: u64,
        context: u64,
        data: u64,
        frame: u64,
        pointers: u64,
    ) -> u64;
    fn DoorGetStandardHandle(selector: u32, output: u64) -> u32;
    fn DoorReadPipe(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorWritePipe(handle: u64, source: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
    fn DoorClosePipe(handle: u64) -> i32;
    fn NtWriteVirtualMemory(
        process: usize,
        destination: usize,
        source: usize,
        bytes: usize,
        written: usize,
    ) -> u32;
    fn NtWriteFile(
        handle: usize,
        event: usize,
        apc: usize,
        context: usize,
        result: usize,
        source: usize,
        bytes: u32,
        offset: usize,
        key: usize,
    ) -> u32;
    fn RtlExitUserProcess(code: u32) -> !;
    fn DoorTlsAlloc() -> u32;
    fn DoorTlsFree(index: u32) -> i32;
    fn DoorTlsGetValue(index: u32) -> u64;
    fn DoorTlsSetValue(index: u32, value: u64) -> i32;
    fn DoorCreateThreadStack(entry: u64, argument: u64, id: u64, flags: u32, bytes: u64) -> u64;
    fn DoorSuspendThread(handle: u64) -> u32;
    fn DoorResumeThread(handle: u64) -> u32;
    fn DoorExitThread(code: u32) -> !;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorCreateEventW(attributes: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorResetEvent(handle: u64) -> i32;
    fn DoorWaitForSingleObjectEx(handle: u64, millis: u32, alert: i32) -> u32;
    fn DoorWaitForMultipleObjectsEx(
        count: u32,
        handles: u64,
        all: i32,
        millis: u32,
        alert: i32,
    ) -> u32;
    fn DoorSleepEx(millis: u32, alert: i32) -> u32;
    fn DoorQueueUserAPC(function: u64, thread: u64, argument: u64) -> u32;
    fn DoorCancelIoEx(handle: u64, overlapped: u64) -> i32;
    fn DoorGetOverlappedResult(handle: u64, overlapped: u64, count: u64, wait: i32) -> i32;
    fn DoorCreateIoCompletionPort(file: u64, existing: u64, key: u64, threads: u32) -> u64;
    fn DoorPostQueuedCompletionStatus(port: u64, bytes: u32, key: u64, overlapped: u64) -> i32;
    fn DoorGetQueuedCompletionStatus(
        port: u64,
        bytes: u64,
        key: u64,
        overlapped: u64,
        millis: u32,
    ) -> i32;
    fn DoorGetQueuedCompletionStatusEx(
        port: u64,
        entries: u64,
        count: u32,
        removed: u64,
        millis: u32,
        alertable: i32,
    ) -> i32;
    fn DoorGetExitCodeThread(handle: u64, output: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorSleep(millis: u32);
    fn DoorGetTickCount64() -> u64;
    fn DoorVirtualAlloc(address: u64, bytes: u64, flags: u32, protection: u32) -> u64;
    fn DoorVirtualFree(address: u64, bytes: u64, flags: u32) -> i32;
    fn DoorVirtualProtect(address: u64, bytes: u64, protection: u32, previous: u64) -> i32;
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorWriteFile(handle: u64, source: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
    fn DoorFlushFileBuffers(handle: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn RtlAddVectoredExceptionHandler(first: u32, function: usize) -> usize;
    fn RtlRemoveVectoredExceptionHandler(token: usize) -> u32;
    fn RtlGetLastWin32Error() -> u32;
    fn RtlSetLastWin32Error(value: u32);
    fn RtlNtStatusToDosError(value: u32) -> u32;
}

#[unsafe(no_mangle)]
pub extern "system" fn LoadLibraryW(pointer: usize) -> usize {
    LoadLibraryExW(pointer, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadLibraryExW(pointer: usize, file: usize, flags: u32) -> usize {
    let pointer = door_windows_abi::widen(pointer);
    let file = door_windows_abi::widen(file);
    door_windows_abi::narrow((move || -> u64 {
        // AS_DATAFILE, AS_IMAGE_RESOURCE and AS_DATAFILE_EXCLUSIVE: the
        // file for its resources only (a bare name already loaded is that
        // module). LOAD_LIBRARY_SEARCH_* restrict the folders searched.
        // DONT_RESOLVE_DLL_REFERENCES (deprecated; Windows maps without
        // DllMain) and LOAD_WITH_ALTERED_SEARCH_PATH load as LoadLibrary
        // does: a Doors system DLL resolves from the catalog, a private DLL
        // is planned with its imports.
        const DATA: u32 = 0x2 | 0x20 | 0x40;
        if pointer == 0 || file != 0 || flags & !0xffff != 0 {
            unsafe {
                RtlSetLastWin32Error(87);
            }
            return 0;
        }
        let search = flags & door_windows_abi::dll_search::flags::ANY;
        if flags & DATA != 0 {
            return unsafe { DoorLoadLibraryDatafile(pointer, flags & (0x20 | search)) };
        }
        unsafe { DoorLoadLibraryExW(pointer, search) }
    })())
}
/// The ANSI forms: the name widened through the ANSI code page.
#[unsafe(no_mangle)]
pub extern "system" fn LoadLibraryA(pointer: usize) -> usize {
    LoadLibraryExA(pointer, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadLibraryExA(pointer: usize, file: usize, flags: u32) -> usize {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(door_windows_abi::widen(pointer), &mut units) {
        Ok(wide) if wide != 0 => LoadLibraryExW(door_windows_abi::narrow(wide), file, flags),
        _ => crate::environment::fail(abi::error::INVALID_PARAMETER) as usize,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetModuleHandleA(pointer: usize) -> usize {
    if pointer == 0 {
        return GetModuleHandleW(0);
    }
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(door_windows_abi::widen(pointer), &mut units) {
        Ok(wide) if wide != 0 => GetModuleHandleW(door_windows_abi::narrow(wide)),
        _ => crate::environment::fail(abi::error::INVALID_PARAMETER) as usize,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeLibrary(module: usize) -> i32 {
    let module = door_windows_abi::widen(module);
    unsafe { DoorFreeLibrary(module) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetModuleHandleW(pointer: usize) -> usize {
    let pointer = door_windows_abi::widen(pointer);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorGetModuleHandleW(pointer) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcAddress(module: usize, pointer: usize) -> usize {
    let module = door_windows_abi::widen(module);
    let pointer = door_windows_abi::widen(pointer);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorGetProcAddress(module, pointer) }
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlLookupFunctionEntry(pc: usize, base: usize, history: usize) -> usize {
    let pc = door_windows_abi::widen(pc);
    let base = door_windows_abi::widen(base);
    let history = door_windows_abi::widen(history);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { NtLookupFunctionEntry(pc, base, history) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlVirtualUnwind(
    kind: u32,
    base: usize,
    pc: usize,
    entry: usize,
    context: usize,
    data: usize,
    frame: usize,
    pointers: usize,
) -> usize {
    let base = door_windows_abi::widen(base);
    let pc = door_windows_abi::widen(pc);
    let entry = door_windows_abi::widen(entry);
    let context = door_windows_abi::widen(context);
    let data = door_windows_abi::widen(data);
    let frame = door_windows_abi::widen(frame);
    let pointers = door_windows_abi::widen(pointers);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { NtVirtualUnwind(kind, base, pc, entry, context, data, frame, pointers) }
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn AddVectoredExceptionHandler(first: u32, function: usize) -> usize {
    let function = door_windows_abi::widen(function);
    door_windows_abi::narrow((move || -> u64 {
        unsafe {
            door_windows_abi::widen(RtlAddVectoredExceptionHandler(
                first,
                door_windows_abi::narrow(function),
            ))
        }
    })())
}
/// RaiseException (x86, P17): the record on this frame, dispatched from
/// 32-bit ntdll's RtlRaiseException, so continuing returns from here.
/// Windows keeps only the noncontinuable flag, at most 15 arguments, and
/// none without an argument array.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "system" fn RaiseException(code: u32, flags: u32, count: u32, arguments: usize) {
    #[link(name = "ntdll", kind = "dylib")]
    unsafe extern "system" {
        fn RtlRaiseException(record: u32);
    }
    let mut record = [0u32; 20];
    record[0] = code;
    record[1] = flags & 1;
    record[3] = RaiseException as *const () as usize as u32;
    let count = if arguments == 0 { 0 } else { count.min(15) };
    record[4] = count;
    for index in 0..count as usize {
        record[5 + index] =
            unsafe { core::ptr::read_volatile((arguments as *const u32).add(index)) };
    }
    unsafe { RtlRaiseException(record.as_mut_ptr() as u32) };
}
/// RtlUnwind (x86): Windows forwards kernel32's to ntdll's.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "system" fn RtlUnwind(target: u32, target_ip: u32, record: u32, value: u32) -> u32 {
    #[link(name = "ntdll", kind = "dylib")]
    unsafe extern "system" {
        fn DoorUnwind(target: u32, target_ip: u32, record: u32, value: u32) -> u32;
    }
    unsafe { DoorUnwind(target, target_ip, record, value) }
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveVectoredExceptionHandler(token: usize) -> u32 {
    let token = door_windows_abi::widen(token);
    unsafe { RtlRemoveVectoredExceptionHandler(door_windows_abi::narrow(token)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn VirtualAlloc(
    address: usize,
    bytes: usize,
    flags: u32,
    protection: u32,
) -> usize {
    let address = door_windows_abi::widen(address);
    let bytes = door_windows_abi::widen(bytes);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorVirtualAlloc(address, bytes, flags, protection) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualFree(address: usize, bytes: usize, flags: u32) -> i32 {
    let address = door_windows_abi::widen(address);
    let bytes = door_windows_abi::widen(bytes);
    unsafe { DoorVirtualFree(address, bytes, flags) }
}
#[unsafe(no_mangle)]
pub extern "system" fn VirtualProtect(
    address: usize,
    bytes: usize,
    protection: u32,
    previous: usize,
) -> i32 {
    let address = door_windows_abi::widen(address);
    let bytes = door_windows_abi::widen(bytes);
    let previous = door_windows_abi::widen(previous);
    unsafe { DoorVirtualProtect(address, bytes, protection, previous) }
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateThread(
    attributes: usize,
    stack_bytes: usize,
    entry: usize,
    argument: usize,
    flags: u32,
    thread_id: usize,
) -> usize {
    let attributes = door_windows_abi::widen(attributes);
    let entry = door_windows_abi::widen(entry);
    let argument = door_windows_abi::widen(argument);
    let thread_id = door_windows_abi::widen(thread_id);
    door_windows_abi::narrow((move || -> u64 {
        // dwStackSize is the commit, or with STACK_SIZE_PARAM_IS_A_RESERVATION
        // the reservation; 0 takes the executable's sizes (P18 M0h). A
        // handle's inheritance from security attributes has no meaning for
        // thread handles here. CREATE_SUSPENDED holds the thread until
        // ResumeThread.
        const CREATE_SUSPENDED: u32 = 0x4;
        const STACK_SIZE_PARAM_IS_A_RESERVATION: u32 = 0x1_0000;
        let _ = attributes;
        if flags & !(STACK_SIZE_PARAM_IS_A_RESERVATION | CREATE_SUSPENDED) != 0 {
            unsafe {
                RtlSetLastWin32Error(abi::error::INVALID_PARAMETER);
            }
            return 0;
        }
        unsafe {
            DoorCreateThreadStack(
                entry,
                argument,
                thread_id,
                flags,
                door_windows_abi::widen(stack_bytes),
            )
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SuspendThread(thread: usize) -> u32 {
    let thread = door_windows_abi::widen(thread);
    unsafe { DoorSuspendThread(thread) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ResumeThread(thread: usize) -> u32 {
    let thread = door_windows_abi::widen(thread);
    unsafe { DoorResumeThread(thread) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ExitThread(code: u32) -> ! {
    unsafe { DoorExitThread(code) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThreadId() -> u32 {
    unsafe { DoorGetCurrentThreadId() }
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForSingleObject(handle: usize, millis: u32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorWaitForSingleObjectEx(handle, millis, 0) }
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateEventW(
    attributes: usize,
    manual: i32,
    initial: i32,
    name: usize,
) -> usize {
    let attributes = door_windows_abi::widen(attributes);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorCreateEventW(attributes, manual, initial, name) }
    })())
}
/// CreateEventA: the ANSI name (at most MAX_PATH characters) as Unicode.
#[unsafe(no_mangle)]
pub extern "system" fn CreateEventA(
    attributes: usize,
    manual: i32,
    initial: i32,
    name: usize,
) -> usize {
    let attributes = door_windows_abi::widen(attributes);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        if name == 0 {
            return abi::widen(CreateEventW(abi::narrow(attributes), manual, initial, 0));
        }
        let mut bytes = [0u8; 261];
        let mut wide = [0u16; 261];
        let mut length = 0;
        while length < 260 {
            // SAFETY: a checked one-byte copy from the caller's string.
            let copied = unsafe {
                NtWriteVirtualMemory(
                    door_windows_abi::narrow(abi::CURRENT_PROCESS),
                    door_windows_abi::narrow(bytes[length..].as_mut_ptr() as u64),
                    door_windows_abi::narrow(name + length as u64),
                    1,
                    0,
                )
            };
            if copied != status::SUCCESS {
                fail(abi::error::NOACCESS);
                return 0;
            }
            if bytes[length] == 0 {
                break;
            }
            length += 1;
        }
        if length == 260 {
            fail(abi::error::FILENAME_EXCED_RANGE);
            return 0;
        }
        for (unit, byte) in wide.iter_mut().zip(&bytes[..length]) {
            *unit = u16::from(*byte);
        }
        abi::widen(CreateEventW(
            abi::narrow(attributes),
            manual,
            initial,
            wide.as_ptr() as usize,
        ))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEvent(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorSetEvent(handle) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ResetEvent(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorResetEvent(handle) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForSingleObjectEx(handle: usize, millis: u32, alert: i32) -> u32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorWaitForSingleObjectEx(handle, millis, alert) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForMultipleObjectsEx(
    count: u32,
    handles: usize,
    all: i32,
    millis: u32,
    alert: i32,
) -> u32 {
    let handles = door_windows_abi::widen(handles);
    unsafe { DoorWaitForMultipleObjectsEx(count, handles, all, millis, alert) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitForMultipleObjects(
    count: u32,
    handles: usize,
    all: i32,
    millis: u32,
) -> u32 {
    let handles = door_windows_abi::widen(handles);
    unsafe { DoorWaitForMultipleObjectsEx(count, handles, all, millis, 0) }
}
#[unsafe(no_mangle)]
pub extern "system" fn SleepEx(millis: u32, alert: i32) -> u32 {
    unsafe { DoorSleepEx(millis, alert) }
}
#[unsafe(no_mangle)]
pub extern "system" fn QueueUserAPC(function: usize, thread: usize, argument: usize) -> u32 {
    let function = door_windows_abi::widen(function);
    let thread = door_windows_abi::widen(thread);
    let argument = door_windows_abi::widen(argument);
    unsafe { DoorQueueUserAPC(function, thread, argument) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentThread() -> usize {
    door_windows_abi::narrow((move || -> u64 { (-2i64) as u64 })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CancelIoEx(handle: usize, overlapped: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let overlapped = door_windows_abi::widen(overlapped);
    if crate::named_pipes::is(handle) {
        return crate::named_pipes::cancel(handle, overlapped);
    }
    unsafe { DoorCancelIoEx(handle, overlapped) }
}
/// CancelIo: the calling thread's I/O on `handle` (the thread is not
/// tracked, so all of it).
#[unsafe(no_mangle)]
pub extern "system" fn CancelIo(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    if crate::named_pipes::is(handle) {
        return crate::named_pipes::cancel(handle, 0);
    }
    unsafe { DoorCancelIoEx(handle, 0) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateIoCompletionPort(
    file: usize,
    existing: usize,
    key: usize,
    threads: u32,
) -> usize {
    let file = door_windows_abi::widen(file);
    let existing = door_windows_abi::widen(existing);
    let key = door_windows_abi::widen(key);
    door_windows_abi::narrow((move || -> u64 {
        unsafe { DoorCreateIoCompletionPort(file, existing, key, threads) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn PostQueuedCompletionStatus(
    port: usize,
    bytes: u32,
    key: usize,
    overlapped: usize,
) -> i32 {
    let port = door_windows_abi::widen(port);
    let key = door_windows_abi::widen(key);
    let overlapped = door_windows_abi::widen(overlapped);
    unsafe { DoorPostQueuedCompletionStatus(port, bytes, key, overlapped) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetQueuedCompletionStatus(
    port: usize,
    bytes: usize,
    key: usize,
    overlapped: usize,
    millis: u32,
) -> i32 {
    let port = door_windows_abi::widen(port);
    let bytes = door_windows_abi::widen(bytes);
    let key = door_windows_abi::widen(key);
    let overlapped = door_windows_abi::widen(overlapped);
    unsafe { DoorGetQueuedCompletionStatus(port, bytes, key, overlapped, millis) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetQueuedCompletionStatusEx(
    port: usize,
    entries: usize,
    count: u32,
    removed: usize,
    millis: u32,
    alertable: i32,
) -> i32 {
    let port = door_windows_abi::widen(port);
    let entries = door_windows_abi::widen(entries);
    let removed = door_windows_abi::widen(removed);
    unsafe { DoorGetQueuedCompletionStatusEx(port, entries, count, removed, millis, alertable) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetOverlappedResult(
    handle: usize,
    overlapped: usize,
    count: usize,
    wait: i32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let overlapped = door_windows_abi::widen(overlapped);
    let count = door_windows_abi::widen(count);
    if crate::named_pipes::is(handle) {
        return crate::named_pipes::overlapped_result(handle, overlapped, count, wait != 0);
    }
    unsafe { DoorGetOverlappedResult(handle, overlapped, count, wait) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetExitCodeThread(handle: usize, output: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let output = door_windows_abi::widen(output);
    unsafe { DoorGetExitCodeThread(handle, output) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseHandle(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if matches!(
            handle,
            abi::INPUT_HANDLE | abi::OUTPUT_HANDLE | abi::ERROR_HANDLE | abi::NUL_HANDLE
        ) {
            // A console handle (a standard handle, or CONIN$/CONOUT$ opened
            // again): the console stays the process's.
            1
        } else if crate::named_pipes::is(handle) {
            crate::named_pipes::close(handle)
        } else if crate::fileinfo::is_directory(handle) {
            crate::fileinfo::close_directory(handle)
        } else if abi::pipe::is_pipe(handle) {
            unsafe { DoorClosePipe(handle) }
        } else if abi::file::is_file(handle) {
            unsafe { DoorCloseFile(handle) }
        } else {
            if crate::jobs::is(handle) {
                crate::jobs::close(handle);
            }
            // Snapshots and waitable timers are events with a record here.
            crate::process_info::forget(handle);
            crate::system_extras::forget_timer(handle);
            unsafe { DoorCloseHandle(handle) }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Sleep(millis: u32) {
    unsafe {
        DoorSleep(millis);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn TlsAlloc() -> u32 {
    unsafe { DoorTlsAlloc() }
}
#[unsafe(no_mangle)]
pub extern "system" fn TlsFree(index: u32) -> i32 {
    unsafe { DoorTlsFree(index) }
}
#[unsafe(no_mangle)]
pub extern "system" fn TlsGetValue(index: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 { unsafe { DoorTlsGetValue(index) } })())
}
#[unsafe(no_mangle)]
pub extern "system" fn TlsSetValue(index: u32, value: usize) -> i32 {
    let value = door_windows_abi::widen(value);
    unsafe { DoorTlsSetValue(index, value) }
}

fn fail(value: u32) -> i32 {
    // SAFETY: scalar-only original import contracts.
    unsafe {
        RtlSetLastWin32Error(RtlNtStatusToDosError(value));
    }
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn GetLastError() -> u32 {
    unsafe { RtlGetLastWin32Error() }
}

#[unsafe(no_mangle)]
pub extern "system" fn SetLastError(value: u32) {
    unsafe {
        RtlSetLastWin32Error(value);
    }
}

/// RTL_USER_PROCESS_PARAMETERS.CommandLine (offset 112): its length in
/// UTF-16 units and its buffer (the parameter page, or ntdll's long-line
/// buffer when Init staged the arguments).
fn command_line() -> (usize, u64) {
    let mut descriptor = [0u8; 16];
    // SAFETY: a checked copy of the process's own parameter block.
    let copied = unsafe {
        NtWriteVirtualMemory(
            door_windows_abi::narrow(abi::CURRENT_PROCESS),
            door_windows_abi::narrow(descriptor.as_mut_ptr() as u64),
            door_windows_abi::narrow(abi::PARAMETERS_ADDRESS + 112),
            16,
            0,
        )
    };
    let buffer = u64::from_le_bytes(descriptor[8..].try_into().unwrap_or([0; 8]));
    if copied != status::SUCCESS || buffer == 0 {
        return (0, abi::PARAMETERS_ADDRESS + 512);
    }
    let units = usize::from(u16::from_le_bytes([descriptor[0], descriptor[1]])) / 2;
    (units.min(ANSI_LINE_BYTES - 1), buffer)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCommandLineW() -> usize {
    door_windows_abi::narrow((move || -> u64 { command_line().1 })())
}
const ANSI_LINE_BYTES: usize = 2050;
/// The command line in the ANSI code page, converted once on first use.
struct AnsiCommandLine {
    /// 0: not built, 1: being built, 2: ready.
    state: core::sync::atomic::AtomicU32,
    bytes: core::cell::UnsafeCell<[u8; ANSI_LINE_BYTES]>,
}
// SAFETY: written once by the thread that wins `state`, read-only after.
unsafe impl Sync for AnsiCommandLine {}
static ANSI_COMMAND_LINE: AnsiCommandLine = AnsiCommandLine {
    state: core::sync::atomic::AtomicU32::new(0),
    bytes: core::cell::UnsafeCell::new([0; ANSI_LINE_BYTES]),
};
/// Unicode to the ANSI code page (Windows-1252 subset): characters it
/// cannot represent become '?', as WideCharToMultiByte's default does.
fn ansi_byte(unit: u16) -> u8 {
    match unit {
        0..=0x7f | 0xa0..=0xff => unit as u8,
        0x20ac => 0x80,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201c => 0x93,
        0x201d => 0x94,
        0x2013 => 0x96,
        0x2014 => 0x97,
        _ => b'?',
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCommandLineA() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        use core::sync::atomic::Ordering;
        let line = &ANSI_COMMAND_LINE;
        if line
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let (length, buffer) = command_line();
            let mut units = [0u16; ANSI_LINE_BYTES];
            // SAFETY: checked copies of the process's own command line, in
            // pieces the copy limit allows.
            let copied = units[..length]
            .chunks_mut(abi::COPY_BYTES / 2)
            .enumerate()
            .all(|(i, piece)| unsafe {
                NtWriteVirtualMemory(
                    door_windows_abi::narrow(abi::CURRENT_PROCESS),
                    door_windows_abi::narrow(piece.as_mut_ptr() as u64),
                    door_windows_abi::narrow(buffer + (i * abi::COPY_BYTES) as u64),
                    2 * piece.len(),
                    0,
                )
            } == status::SUCCESS);
            if copied {
                // SAFETY: only this thread writes, before publishing state 2.
                let bytes = unsafe { &mut *line.bytes.get() };
                for (byte, unit) in bytes
                    .iter_mut()
                    .zip(units[..length].iter().take_while(|u| **u != 0))
                {
                    *byte = ansi_byte(*unit);
                }
            }
            line.state.store(2, Ordering::Release);
        }
        while line.state.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
        line.bytes.get() as u64
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTickCount64() -> u64 {
    unsafe { DoorGetTickCount64() }
}
/// Milliseconds since start, wrapping every 49.7 days.
#[unsafe(no_mangle)]
pub extern "system" fn GetTickCount() -> u32 {
    unsafe { DoorGetTickCount64() as u32 }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetStdHandle(selector: u32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        let mut result = 0u64;
        let value = unsafe { DoorGetStandardHandle(selector, &mut result as *mut u64 as u64) };
        if value != status::SUCCESS {
            fail(value);
            return u64::MAX;
        }
        result
    })())
}

fn count_output(destination: u64, value: u32) -> u32 {
    unsafe {
        NtWriteVirtualMemory(
            door_windows_abi::narrow(abi::CURRENT_PROCESS),
            door_windows_abi::narrow(destination),
            door_windows_abi::narrow(&value as *const u32 as u64),
            core::mem::size_of::<u32>(),
            0,
        )
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn WriteFile(
    handle: usize,
    buffer: usize,
    bytes: u32,
    written: usize,
    overlapped: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let written = door_windows_abi::widen(written);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        if abi::file::is_file(handle) {
            return unsafe { DoorWriteFile(handle, buffer, bytes, written, overlapped) };
        }
        if crate::named_pipes::is(handle) {
            return crate::named_pipes::io(handle);
        }
        if abi::pipe::is_pipe(handle) {
            return unsafe { DoorWritePipe(handle, buffer, bytes, written, overlapped) };
        }
        let checked = count_output(written, 0);
        if checked != status::SUCCESS {
            return fail(checked);
        }
        if overlapped != 0 {
            return fail(status::NOT_SUPPORTED);
        }
        let mut result = [0u64; 2];
        let value = unsafe {
            NtWriteFile(
                door_windows_abi::narrow(handle),
                0,
                0,
                0,
                door_windows_abi::narrow(result.as_mut_ptr() as u64),
                door_windows_abi::narrow(buffer),
                bytes,
                0,
                0,
            )
        };
        if value != status::SUCCESS {
            return fail(value);
        }
        if result[1] > u64::from(bytes) {
            return fail(status::IO_DEVICE_ERROR);
        }
        let checked = count_output(written, result[1] as u32);
        if checked != status::SUCCESS {
            return fail(checked);
        }
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateFileW(
    path: usize,
    access: u32,
    share: u32,
    security: usize,
    disposition: u32,
    flags: u32,
    template: usize,
) -> usize {
    let path = door_windows_abi::widen(path);
    let security = door_windows_abi::widen(security);
    let template = door_windows_abi::widen(template);
    door_windows_abi::narrow((move || -> u64 {
        if let Some(result) = crate::named_pipes::client_open(path) {
            return result;
        }
        if let Some(result) = crate::fileinfo::open_directory(path, disposition, flags) {
            return result;
        }
        unsafe { DoorCreateFileW(path, access, share, security, disposition, flags, template) }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ReadFile(
    handle: usize,
    buffer: usize,
    bytes: u32,
    read: usize,
    overlapped: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    let read = door_windows_abi::widen(read);
    let overlapped = door_windows_abi::widen(overlapped);
    {
        if abi::pipe::is_pipe(handle) {
            return unsafe { DoorReadPipe(handle, buffer, bytes, read, overlapped) };
        }
        if crate::named_pipes::is(handle) {
            return crate::named_pipes::io(handle);
        }
        unsafe { DoorReadFile(handle, buffer, bytes, read, overlapped) }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FlushFileBuffers(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    unsafe { DoorFlushFileBuffers(handle) }
}

#[unsafe(no_mangle)]
pub extern "system" fn ExitProcess(code: u32) -> ! {
    unsafe { RtlExitUserProcess(code) }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    ExitProcess(0xffff_fed3)
}

#[unsafe(no_mangle)]
extern "system" fn __CxxFrameHandler3() -> ! {
    ExitProcess(0xffff_fed2)
}

#[unsafe(no_mangle)]
static _fltused: i32 = 0;

/// CreateFileA: the path through the ANSI code page, then CreateFileW.
#[unsafe(no_mangle)]
pub extern "system" fn CreateFileA(
    path: usize,
    access: u32,
    share: u32,
    security: usize,
    disposition: u32,
    flags: u32,
    template: usize,
) -> usize {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(door_windows_abi::widen(path), &mut units) {
        Ok(wide) if wide != 0 => CreateFileW(
            door_windows_abi::narrow(wide),
            access,
            share,
            security,
            disposition,
            flags,
            template,
        ),
        _ => {
            unsafe { RtlSetLastWin32Error(87) };
            door_windows_abi::narrow(u64::MAX)
        }
    }
}
/// WriteConsoleW: UTF-16 to the console's UTF-8, through WriteFile; the
/// characters consumed go to `written`.
#[unsafe(no_mangle)]
pub extern "system" fn WriteConsoleW(
    handle: usize,
    buffer: usize,
    characters: u32,
    written: usize,
    _reserved: usize,
) -> i32 {
    let source = buffer as *const u16;
    let mut chunk = [0u8; 1024];
    let mut used = 0usize;
    let mut consumed = 0u32;
    let mut i = 0u32;
    while i < characters {
        let unit = unsafe { source.add(i as usize).read_unaligned() };
        let (c, width) = if (0xd800..0xdc00).contains(&unit) && i + 1 < characters {
            let low = unsafe { source.add(i as usize + 1).read_unaligned() };
            match char::decode_utf16([unit, low]).next() {
                Some(Ok(c)) => (c, 2),
                _ => ('\u{fffd}', 1),
            }
        } else {
            (char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}'), 1)
        };
        if used + 4 > chunk.len() {
            let mut done = 0u32;
            if WriteFile(
                handle,
                chunk.as_ptr() as usize,
                used as u32,
                &mut done as *mut u32 as usize,
                0,
            ) == 0
            {
                return 0;
            }
            used = 0;
        }
        used += c.encode_utf8(&mut chunk[used..]).len();
        i += width;
        consumed = i;
    }
    if used > 0 {
        let mut done = 0u32;
        if WriteFile(
            handle,
            chunk.as_ptr() as usize,
            used as u32,
            &mut done as *mut u32 as usize,
            0,
        ) == 0
        {
            return 0;
        }
    }
    if written != 0 {
        unsafe { (written as *mut u32).write_unaligned(consumed) };
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn WriteConsoleA(
    handle: usize,
    buffer: usize,
    characters: u32,
    written: usize,
    _reserved: usize,
) -> i32 {
    WriteFile(handle, buffer, characters, written, 0)
}
/// Console control handlers (SetConsoleCtrlHandler): ntdll runs them
/// (`control_routine`) when Ctrl+C reaches a program reading processed
/// input, most recently added first, until one handles the event; none
/// handling it ends the process with STATUS_CONTROL_C_EXIT, as Windows'
/// default handler does. A NULL handler with `add` ignores Ctrl+C.
const MAX_CONTROL_HANDLERS: usize = 32;
struct ControlHandlers {
    handlers: [usize; MAX_CONTROL_HANDLERS],
    count: usize,
    ignore_ctrl_c: bool,
}
struct ControlShared {
    held: core::sync::atomic::AtomicBool,
    registered: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<ControlHandlers>,
}
// `held` gives one thread the cell at a time.
unsafe impl Sync for ControlShared {}
static CONTROL: ControlShared = ControlShared {
    held: core::sync::atomic::AtomicBool::new(false),
    registered: core::sync::atomic::AtomicBool::new(false),
    value: core::cell::UnsafeCell::new(ControlHandlers {
        handlers: [0; MAX_CONTROL_HANDLERS],
        count: 0,
        ignore_ctrl_c: false,
    }),
};
fn with_control<R>(f: impl FnOnce(&mut ControlHandlers) -> R) -> R {
    use core::sync::atomic::Ordering;
    while CONTROL.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *CONTROL.value.get() });
    CONTROL.held.store(false, Ordering::Release);
    r
}
/// The handlers, newest first, until one returns TRUE.
extern "system" fn control_routine(event: u32) -> i32 {
    let (handlers, count, ignore) = with_control(|c| (c.handlers, c.count, c.ignore_ctrl_c));
    if event == 0 && ignore {
        return 1;
    }
    for &handler in handlers[..count].iter().rev() {
        // SAFETY: the program registered a HandlerRoutine.
        let routine: extern "system" fn(u32) -> i32 = unsafe { core::mem::transmute(handler) };
        if routine(event) != 0 {
            return 1;
        }
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn SetConsoleCtrlHandler(handler: usize, add: i32) -> i32 {
    #[link(name = "ntdll", kind = "dylib")]
    unsafe extern "system" {
        fn DoorSetControlDispatcher(routine: u64) -> i32;
    }
    if !CONTROL
        .registered
        .swap(true, core::sync::atomic::Ordering::AcqRel)
    {
        unsafe { DoorSetControlDispatcher(control_routine as *const () as usize as u64) };
    }
    if handler == 0 {
        with_control(|c| c.ignore_ctrl_c = add != 0);
        return 1;
    }
    let done = with_control(|c| {
        if add != 0 {
            if c.count == MAX_CONTROL_HANDLERS {
                return Err(8);
            }
            c.handlers[c.count] = handler;
            c.count += 1;
            return Ok(());
        }
        let Some(at) = c.handlers[..c.count].iter().rposition(|&h| h == handler) else {
            return Err(87);
        };
        c.handlers.copy_within(at + 1..c.count, at);
        c.count -= 1;
        Ok(())
    });
    match done {
        Ok(()) => 1,
        Err(code) => {
            unsafe { RtlSetLastWin32Error(code) };
            0
        }
    }
}

/// GetModuleHandleExW. PIN and the default (counted) form do not change a
/// module's lifetime here: a module stays mapped while it has any
/// LoadLibrary reference, and system DLLs are never unmapped.
#[unsafe(no_mangle)]
pub extern "system" fn GetModuleHandleExW(flags: u32, name: usize, module: usize) -> i32 {
    #[link(name = "ntdll", kind = "dylib")]
    unsafe extern "system" {
        fn DoorModuleFromAddress(address: u64) -> u64;
    }
    const PIN: u32 = 1;
    const UNCHANGED_REFCOUNT: u32 = 2;
    const FROM_ADDRESS: u32 = 4;
    if module == 0
        || flags & !7 != 0
        || flags & (PIN | UNCHANGED_REFCOUNT) == PIN | UNCHANGED_REFCOUNT
    {
        unsafe { RtlSetLastWin32Error(87) };
        return 0;
    }
    let base = if flags & FROM_ADDRESS != 0 {
        door_windows_abi::narrow(unsafe { DoorModuleFromAddress(door_windows_abi::widen(name)) })
    } else {
        GetModuleHandleW(name)
    };
    unsafe { (module as *mut usize).write_unaligned(base) };
    if base == 0 {
        unsafe { RtlSetLastWin32Error(126) };
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetModuleHandleExA(flags: u32, name: usize, module: usize) -> i32 {
    if flags & 4 != 0 || name == 0 {
        return GetModuleHandleExW(flags, name, module);
    }
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(door_windows_abi::widen(name), &mut units) {
        Ok(wide) => GetModuleHandleExW(flags, door_windows_abi::narrow(wide), module),
        Err(_) => {
            unsafe { RtlSetLastWin32Error(87) };
            0
        }
    }
}
