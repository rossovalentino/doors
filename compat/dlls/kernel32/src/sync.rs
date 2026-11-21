//! Win32 names of ntdll's synchronization (P18 M0c): SRW locks, critical
//! sections, condition variables, WaitOnAddress and interlocked SLists.
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorWaitOnAddress(address: usize, compare: usize, size: usize, millis: u32) -> i32;
    fn DoorWakeByAddressSingle(address: usize);
    fn DoorWakeByAddressAll(address: usize);
    fn RtlInitializeSRWLock(lock: usize);
    fn RtlAcquireSRWLockExclusive(lock: usize);
    fn RtlAcquireSRWLockShared(lock: usize);
    fn RtlTryAcquireSRWLockExclusive(lock: usize) -> u8;
    fn RtlTryAcquireSRWLockShared(lock: usize) -> u8;
    fn RtlReleaseSRWLockExclusive(lock: usize);
    fn RtlReleaseSRWLockShared(lock: usize);
    fn RtlInitializeCriticalSectionEx(section: usize, spin: u32, flags: u32) -> u32;
    fn RtlSetCriticalSectionSpinCount(section: usize, spin: u32) -> u32;
    fn RtlDeleteCriticalSection(section: usize) -> u32;
    fn RtlEnterCriticalSection(section: usize) -> u32;
    fn RtlTryEnterCriticalSection(section: usize) -> u8;
    fn RtlLeaveCriticalSection(section: usize) -> u32;
    fn RtlInitializeConditionVariable(variable: usize);
    fn RtlWakeConditionVariable(variable: usize);
    fn RtlWakeAllConditionVariable(variable: usize);
    fn RtlSleepConditionVariableSRW(variable: usize, lock: usize, millis: u32, flags: u32) -> u32;
    fn RtlSleepConditionVariableCS(variable: usize, section: usize, millis: u32) -> u32;
    fn RtlInitializeSListHead(header: usize);
    fn RtlQueryDepthSList(header: usize) -> u16;
    fn RtlInterlockedPushEntrySList(header: usize, entry: usize) -> usize;
    fn RtlInterlockedPopEntrySList(header: usize) -> usize;
    fn RtlInterlockedFlushSList(header: usize) -> usize;
    fn RtlSetLastWin32Error(value: u32);
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeSRWLock(lock: usize) {
    unsafe { RtlInitializeSRWLock(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn AcquireSRWLockExclusive(lock: usize) {
    unsafe { RtlAcquireSRWLockExclusive(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn AcquireSRWLockShared(lock: usize) {
    unsafe { RtlAcquireSRWLockShared(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn TryAcquireSRWLockExclusive(lock: usize) -> u8 {
    unsafe { RtlTryAcquireSRWLockExclusive(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn TryAcquireSRWLockShared(lock: usize) -> u8 {
    unsafe { RtlTryAcquireSRWLockShared(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseSRWLockExclusive(lock: usize) {
    unsafe { RtlReleaseSRWLockExclusive(lock) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseSRWLockShared(lock: usize) {
    unsafe { RtlReleaseSRWLockShared(lock) }
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeCriticalSection(section: usize) {
    unsafe { RtlInitializeCriticalSectionEx(section, 0, 0) };
}
#[unsafe(no_mangle)]
pub extern "system" fn InitializeCriticalSectionAndSpinCount(section: usize, spin: u32) -> i32 {
    unsafe { RtlInitializeCriticalSectionEx(section, spin, 0) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn InitializeCriticalSectionEx(section: usize, spin: u32, flags: u32) -> i32 {
    unsafe { RtlInitializeCriticalSectionEx(section, spin, flags) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCriticalSectionSpinCount(section: usize, spin: u32) -> u32 {
    unsafe { RtlSetCriticalSectionSpinCount(section, spin) }
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteCriticalSection(section: usize) {
    unsafe { RtlDeleteCriticalSection(section) };
}
#[unsafe(no_mangle)]
pub extern "system" fn EnterCriticalSection(section: usize) {
    unsafe { RtlEnterCriticalSection(section) };
}
#[unsafe(no_mangle)]
pub extern "system" fn TryEnterCriticalSection(section: usize) -> i32 {
    i32::from(unsafe { RtlTryEnterCriticalSection(section) })
}
#[unsafe(no_mangle)]
pub extern "system" fn LeaveCriticalSection(section: usize) {
    unsafe { RtlLeaveCriticalSection(section) };
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeConditionVariable(variable: usize) {
    unsafe { RtlInitializeConditionVariable(variable) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WakeConditionVariable(variable: usize) {
    unsafe { RtlWakeConditionVariable(variable) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WakeAllConditionVariable(variable: usize) {
    unsafe { RtlWakeAllConditionVariable(variable) }
}
/// FALSE with ERROR_TIMEOUT when the wait timed out.
fn sleep_result(status: u32) -> i32 {
    if status == 0 {
        1
    } else {
        unsafe { RtlSetLastWin32Error(1460) };
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SleepConditionVariableSRW(
    variable: usize,
    lock: usize,
    millis: u32,
    flags: u32,
) -> i32 {
    sleep_result(unsafe { RtlSleepConditionVariableSRW(variable, lock, millis, flags) })
}
#[unsafe(no_mangle)]
pub extern "system" fn SleepConditionVariableCS(
    variable: usize,
    section: usize,
    millis: u32,
) -> i32 {
    sleep_result(unsafe { RtlSleepConditionVariableCS(variable, section, millis) })
}

#[unsafe(no_mangle)]
pub extern "system" fn WaitOnAddress(
    address: usize,
    compare: usize,
    size: usize,
    millis: u32,
) -> i32 {
    unsafe { DoorWaitOnAddress(address, compare, size, millis) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WakeByAddressSingle(address: usize) {
    unsafe { DoorWakeByAddressSingle(address) }
}
#[unsafe(no_mangle)]
pub extern "system" fn WakeByAddressAll(address: usize) {
    unsafe { DoorWakeByAddressAll(address) }
}

#[unsafe(no_mangle)]
pub extern "system" fn InitializeSListHead(header: usize) {
    unsafe { RtlInitializeSListHead(header) }
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryDepthSList(header: usize) -> u16 {
    unsafe { RtlQueryDepthSList(header) }
}
#[unsafe(no_mangle)]
pub extern "system" fn InterlockedPushEntrySList(header: usize, entry: usize) -> usize {
    unsafe { RtlInterlockedPushEntrySList(header, entry) }
}
#[unsafe(no_mangle)]
pub extern "system" fn InterlockedPopEntrySList(header: usize) -> usize {
    unsafe { RtlInterlockedPopEntrySList(header) }
}
#[unsafe(no_mangle)]
pub extern "system" fn InterlockedFlushSList(header: usize) -> usize {
    unsafe { RtlInterlockedFlushSList(header) }
}

// ---- fiber-local storage (a thread's FLS is its TLS; no fibers) ----------

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorFlsAlloc(callback: u64) -> u32;
    fn DoorFlsFree(index: u32) -> i32;
    fn DoorFlsGetValue(index: u32) -> u64;
    fn DoorFlsSetValue(index: u32, value: u64) -> i32;
}
#[unsafe(no_mangle)]
pub extern "system" fn FlsAlloc(callback: usize) -> u32 {
    unsafe { DoorFlsAlloc(door_windows_abi::widen(callback)) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FlsFree(index: u32) -> i32 {
    unsafe { DoorFlsFree(index) }
}
#[unsafe(no_mangle)]
pub extern "system" fn FlsGetValue(index: u32) -> usize {
    door_windows_abi::narrow(unsafe { DoorFlsGetValue(index) })
}
/// FlsGetValue2: the same value without touching the last error.
#[unsafe(no_mangle)]
pub extern "system" fn FlsGetValue2(index: u32) -> usize {
    let saved = crate::guest::GetLastError();
    let value = FlsGetValue(index);
    crate::guest::SetLastError(saved);
    value
}
#[unsafe(no_mangle)]
pub extern "system" fn FlsSetValue(index: u32, value: usize) -> i32 {
    unsafe { DoorFlsSetValue(index, door_windows_abi::widen(value)) }
}

// ---- semaphores, named-object lookups, SignalObjectAndWait -------------

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateSemaphoreW(attributes: u64, initial: i32, maximum: i32, name: u64) -> u64;
    fn DoorReleaseSemaphore(handle: u64, amount: i32, previous: u64) -> i32;
    fn DoorSetEvent(handle: u64) -> i32;
    fn DoorWaitForSingleObjectEx(handle: u64, millis: u32, alertable: i32) -> u32;
    fn DoorOpenNamedObject(kind: u32, name: u64) -> u64;
    fn DoorCreateMutexW(attributes: u64, owned: i32, name: u64) -> u64;
    fn DoorReleaseMutex(handle: u64) -> i32;
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateSemaphoreW(
    attributes: usize,
    initial: i32,
    maximum: i32,
    name: usize,
) -> usize {
    door_windows_abi::narrow(unsafe {
        DoorCreateSemaphoreW(
            door_windows_abi::widen(attributes),
            initial,
            maximum,
            door_windows_abi::widen(name),
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateSemaphoreA(
    attributes: usize,
    initial: i32,
    maximum: i32,
    name: usize,
) -> usize {
    CreateSemaphoreW(attributes, initial, maximum, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateSemaphoreExW(
    attributes: usize,
    initial: i32,
    maximum: i32,
    name: usize,
    _flags: u32,
    _access: u32,
) -> usize {
    CreateSemaphoreW(attributes, initial, maximum, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseSemaphore(handle: usize, amount: i32, previous: usize) -> i32 {
    unsafe {
        DoorReleaseSemaphore(
            door_windows_abi::widen(handle),
            amount,
            door_windows_abi::widen(previous),
        )
    }
}
/// Kernel-object names live in ntdll's process namespace (P18); one shared
/// between processes waits for M0k's object broker.
fn open_named(kind: u32, name: usize) -> usize {
    door_windows_abi::narrow(unsafe { DoorOpenNamedObject(kind, door_windows_abi::widen(name)) })
}
/// An ANSI object name as UTF-16 (`f` gets the wide pointer, or 0).
fn with_wide_name(name: usize, f: impl FnOnce(usize) -> usize) -> usize {
    let name = door_windows_abi::widen(name);
    if name == 0 {
        return f(0);
    }
    let mut wide = [0u16; crate::environment::UNITS];
    match crate::environment::widen(name, &mut wide) {
        Ok(pointer) => f(door_windows_abi::narrow(pointer)),
        Err(code) => {
            unsafe { RtlSetLastWin32Error(code) };
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenEventW(_access: u32, _inherit: i32, name: usize) -> usize {
    open_named(1, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenEventA(access: u32, inherit: i32, name: usize) -> usize {
    with_wide_name(name, |wide| OpenEventW(access, inherit, wide))
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenSemaphoreW(_access: u32, _inherit: i32, name: usize) -> usize {
    open_named(2, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenSemaphoreA(access: u32, inherit: i32, name: usize) -> usize {
    with_wide_name(name, |wide| OpenSemaphoreW(access, inherit, wide))
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenMutexW(_access: u32, _inherit: i32, name: usize) -> usize {
    open_named(3, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenMutexA(access: u32, inherit: i32, name: usize) -> usize {
    with_wide_name(name, |wide| OpenMutexW(access, inherit, wide))
}
/// CreateEventEx: CREATE_EVENT_MANUAL_RESET (1) and CREATE_EVENT_INITIAL_SET
/// (2); the access mask asks nothing more of an event here.
#[unsafe(no_mangle)]
pub extern "system" fn CreateEventExW(
    attributes: usize,
    name: usize,
    flags: u32,
    _access: u32,
) -> usize {
    if flags & !3 != 0 {
        unsafe { RtlSetLastWin32Error(87) };
        return 0;
    }
    crate::guest::CreateEventW(
        attributes,
        (flags & 1) as i32,
        ((flags >> 1) & 1) as i32,
        name,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateEventExA(
    attributes: usize,
    name: usize,
    flags: u32,
    access: u32,
) -> usize {
    with_wide_name(name, |wide| CreateEventExW(attributes, wide, flags, access))
}
/// CreateMutex: owned by the caller when `owned` (a name that exists gives
/// the existing mutex, ERROR_ALREADY_EXISTS, its ownership unchanged).
#[unsafe(no_mangle)]
pub extern "system" fn CreateMutexW(attributes: usize, owned: i32, name: usize) -> usize {
    door_windows_abi::narrow(unsafe {
        DoorCreateMutexW(
            door_windows_abi::widen(attributes),
            owned,
            door_windows_abi::widen(name),
        )
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateMutexA(attributes: usize, owned: i32, name: usize) -> usize {
    with_wide_name(name, |wide| CreateMutexW(attributes, owned, wide))
}
/// CreateMutexEx: CREATE_MUTEX_INITIAL_OWNER (1).
#[unsafe(no_mangle)]
pub extern "system" fn CreateMutexExW(
    attributes: usize,
    name: usize,
    flags: u32,
    _access: u32,
) -> usize {
    if flags & !1 != 0 {
        unsafe { RtlSetLastWin32Error(87) };
        return 0;
    }
    CreateMutexW(attributes, (flags & 1) as i32, name)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateMutexExA(
    attributes: usize,
    name: usize,
    flags: u32,
    access: u32,
) -> usize {
    with_wide_name(name, |wide| CreateMutexExW(attributes, wide, flags, access))
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseMutex(handle: usize) -> i32 {
    unsafe { DoorReleaseMutex(door_windows_abi::widen(handle)) }
}
/// SignalObjectAndWait: signal an event or semaphore, then wait. The two
/// steps are not one kernel transaction; a waiter woken by the signal may
/// run first, which Windows also allows.
#[unsafe(no_mangle)]
pub extern "system" fn SignalObjectAndWait(
    signal: usize,
    wait: usize,
    millis: u32,
    alertable: i32,
) -> u32 {
    let signal = door_windows_abi::widen(signal);
    let signaled =
        unsafe { DoorSetEvent(signal) } != 0 || unsafe { DoorReleaseSemaphore(signal, 1, 0) } != 0;
    if !signaled {
        return u32::MAX;
    }
    unsafe { DoorWaitForSingleObjectEx(door_windows_abi::widen(wait), millis, alertable) }
}
