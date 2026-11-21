//! Native thread ownership and Windows call boundaries. Start descriptors remain
//! owned until the child captures them, regardless of parent scheduling order.
use super::guest::RtlSetLastWin32Error;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use door_windows_abi::error;
/// `NtQuerySystemTime`: UTC as a FILETIME (100 ns since 1601-01-01);
/// STATUS_UNSUCCESSFUL while the clock is not set.
#[unsafe(no_mangle)]
pub extern "system" fn NtQuerySystemTime(time: usize) -> u32 {
    let time = door_windows_abi::widen(time);
    {
        let Ok(now) = door_sys::wall_time() else {
            return 0xC000_0001;
        };
        match super::guest::copy(time, &now as *const u64 as u64, 8) {
            Ok(()) => 0,
            Err(_) => 0xC000_0005,
        }
    }
}
/// `length` bytes from the kernel entropy source into `buffer` (what
/// RtlGenRandom and BCryptGenRandom draw on); a Win32 error otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGenRandom(buffer: u64, length: u32) -> u32 {
    let mut done = 0u64;
    let mut chunk = [0u8; door_abi::entropy::READ_MAX];
    while done < u64::from(length) {
        let n = (u64::from(length) - done).min(chunk.len() as u64) as usize;
        if door_sys::entropy_read(&mut chunk[..n]) != Ok(n) {
            return error::ACCESS_DENIED;
        }
        if super::guest::copy(buffer + done, chunk.as_ptr() as u64, n).is_err() {
            return 998; // ERROR_NOACCESS
        }
        done += n as u64;
    }
    chunk.fill(0);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetTickCount64() -> u64 {
    door_sys::time()
        .unwrap_or(0)
        .saturating_mul(door_abi::platform::MONOTONIC_TICK_MILLIS)
}

struct Start {
    state: AtomicU32,
    entry: AtomicU64,
    argument: AtomicU64,
    id: AtomicU32,
    /// CreateThread's dwStackSize and STACK_SIZE_PARAM_IS_A_RESERVATION.
    stack: AtomicU64,
    stack_flags: AtomicU32,
}
impl Start {
    const fn new() -> Self {
        Self {
            state: AtomicU32::new(0),
            entry: AtomicU64::new(0),
            argument: AtomicU64::new(0),
            id: AtomicU32::new(0),
            stack: AtomicU64::new(0),
            stack_flags: AtomicU32::new(0),
        }
    }
}
/// Threads created and not yet running (a CREATE_SUSPENDED thread holds its
/// record until resumed, and runtimes create several suspended at once).
static STARTS: [Start; super::waits::THREADS] = [const { Start::new() }; super::waits::THREADS];
static NEXT_ID: AtomicU32 = AtomicU32::new(2);

fn fail(native: door_abi::Status) {
    let error = match native {
        door_abi::Status::BadHandle => error::INVALID_HANDLE,
        door_abi::Status::AccessDenied => 5,
        door_abi::Status::ResourceExhausted => error::NOT_ENOUGH_MEMORY,
        _ => error::INVALID_PARAMETER,
    };
    RtlSetLastWin32Error(error);
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateThread(entry: u64, argument: u64) -> u64 {
    create_thread(entry, argument, 0, false, 0, 0)
}
/// CreateThread's flags: CREATE_SUSPENDED (4) starts the thread held until
/// ResumeThread.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateThreadFlags(
    entry: u64,
    argument: u64,
    id: u64,
    flags: u32,
) -> u64 {
    if flags & !4 != 0 {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    }
    create_thread(entry, argument, id, flags & 4 != 0, 0, 0)
}
/// CreateThread with its stack request: `bytes` is the commit, or with
/// STACK_SIZE_PARAM_IS_A_RESERVATION (0x10000) the reservation (P18 M0h).
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateThreadStack(
    entry: u64,
    argument: u64,
    id: u64,
    flags: u32,
    bytes: u64,
) -> u64 {
    if flags & !(4 | 0x1_0000) != 0 {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    }
    create_thread(entry, argument, id, flags & 4 != 0, bytes, flags & 0x1_0000)
}
/// `SuspendThread` of another thread of this process: the previous count,
/// or u32::MAX.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSuspendThread(handle: u64) -> u32 {
    match door_sys::thread_suspend(handle) {
        Ok(previous) => previous as u32,
        Err(error) => {
            fail(error);
            u32::MAX
        }
    }
}
/// `ResumeThread`: a thread of this process, or the primary thread of a
/// process this one started with CREATE_SUSPENDED (its hThread).
#[unsafe(no_mangle)]
pub extern "system" fn DoorResumeThread(handle: u64) -> u32 {
    let process = door_sys::inspect(handle)
        .is_ok_and(|i| door_abi::HandleKind::from_raw(i[0]) == Some(door_abi::HandleKind::Process));
    let result = if process {
        door_sys::process_resume(handle)
    } else {
        door_sys::thread_resume(handle)
    };
    match result {
        Ok(previous) => previous as u32,
        Err(error) => {
            fail(error);
            u32::MAX
        }
    }
}
/// As `DoorCreateThread`, also storing the new Windows thread ID at `id`
/// (validated before the thread exists; 0 = not requested).
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64 {
    create_thread(entry, argument, id, false, 0, 0)
}
fn create_thread(
    entry: u64,
    argument: u64,
    output: u64,
    suspended: bool,
    stack: u64,
    stack_flags: u32,
) -> u64 {
    if output != 0 && super::guest::copy(output, output, 4).is_err() {
        RtlSetLastWin32Error(error::NOACCESS);
        return 0;
    }
    if entry == 0 {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    }
    let Some((index, start)) = STARTS.iter().enumerate().find(|(_, start)| {
        start
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }) else {
        RtlSetLastWin32Error(error::NOT_ENOUGH_MEMORY);
        return 0;
    };
    let Ok(id) = NEXT_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
    else {
        start.state.store(0, Ordering::Release);
        RtlSetLastWin32Error(error::NOT_ENOUGH_MEMORY);
        return 0;
    };
    start.entry.store(entry, Ordering::Relaxed);
    start.argument.store(argument, Ordering::Relaxed);
    start.id.store(id, Ordering::Relaxed);
    start.stack.store(stack, Ordering::Relaxed);
    start.stack_flags.store(stack_flags, Ordering::Relaxed);
    if !super::waits::prepare_thread(id) {
        start.state.store(0, Ordering::Release);
        RtlSetLastWin32Error(error::NOT_ENOUGH_MEMORY);
        return 0;
    }
    start.state.store(2, Ordering::Release);
    let created = if suspended {
        unsafe { door_sys::thread_create_suspended(thread_start as *const () as u64, index as u64) }
    } else {
        unsafe { door_sys::thread_create(thread_start as *const () as u64, index as u64) }
    };
    match created {
        Ok(handle) => {
            super::waits::register_thread(handle, id);
            if output != 0 {
                // Range checked above; a concurrent unmap only loses the report.
                let _ = super::guest::copy(output, &id as *const u32 as u64, 4);
            }
            handle
        }
        Err(error) => {
            super::waits::retire_thread(id);
            start.state.store(0, Ordering::Release);
            fail(error);
            0
        }
    }
}

#[cfg(target_arch = "x86_64")]
extern "system" fn thread_start(index: u64) -> ! {
    start_thread(index)
}
/// x86 (P17): a 32-bit thread starts here with its argument in ECX.
#[cfg(target_arch = "x86")]
#[unsafe(naked)]
unsafe extern "C" fn thread_start() -> ! {
    core::arch::naked_asm!("push ecx", "call {f}", "ud2", f = sym thread_start32)
}
#[cfg(target_arch = "x86")]
extern "system" fn thread_start32(index: u32) -> ! {
    start_thread(u64::from(index))
}
fn start_thread(index: u64) -> ! {
    let Some(start) = STARTS.get(index as usize) else {
        door_sys::exit(-310);
    };
    if start.state.load(Ordering::Acquire) != 2 {
        door_sys::exit(-311);
    }
    let entry = start.entry.load(Ordering::Relaxed);
    let argument = start.argument.load(Ordering::Relaxed);
    let id = start.id.load(Ordering::Relaxed);
    #[cfg(target_arch = "x86_64")]
    let stack = (
        start.stack.load(Ordering::Relaxed),
        start.stack_flags.load(Ordering::Relaxed),
    );
    start.state.store(0, Ordering::Release);
    super::arch::set_thread_id(id);
    // TLS blocks of DLLs ntdll's loader mapped, before DLL_THREAD_ATTACH.
    super::ldr::thread_started();
    if super::loader::notify_thread(2).is_err() {
        door_sys::exit(-312);
    }
    // x64: the entry runs on a stack of the size the image or CreateThread
    // asked for (P18 M0h).
    #[cfg(target_arch = "x86_64")]
    {
        let (reserve, commit) = super::stacks::sizes(stack.0, stack.1);
        let call = [entry, argument];
        let code = super::stacks::run(thread_entry, call.as_ptr() as u64, reserve, commit);
        DoorExitThread(code as u32)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // LPTHREAD_START_ROUTINE: one pointer-sized argument.
        let function: extern "system" fn(usize) -> u32 =
            unsafe { core::mem::transmute(entry as usize) };
        DoorExitThread(function(argument as usize))
    }
}
/// LPTHREAD_START_ROUTINE(argument) from `[entry, argument]` at `call`.
#[cfg(target_arch = "x86_64")]
extern "system" fn thread_entry(call: u64) -> u64 {
    // SAFETY: `call` is start_thread's array, alive below this stack switch.
    let [entry, argument] = unsafe { (call as *const [u64; 2]).read() };
    let function: extern "system" fn(usize) -> u32 =
        unsafe { core::mem::transmute(entry as usize) };
    u64::from(function(argument as usize))
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorGetCurrentThreadId() -> u32 {
    let id = super::arch::thread_id();
    if id == 0 { 1 } else { id }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorExitThread(code: u32) -> ! {
    if super::loader::notify_thread(3).is_err() {
        door_sys::exit(-313);
    }
    // After DLL_THREAD_DETACH, as LdrShutdownThread orders them.
    super::tls::run_thread_callbacks();
    super::ldr::thread_exited();
    super::waits::retire_thread(DoorGetCurrentThreadId());
    // x64: off the reserved stack, which is released (P18 M0h).
    #[cfg(target_arch = "x86_64")]
    super::stacks::exit(code);
    #[cfg(not(target_arch = "x86_64"))]
    door_sys::thread_exit(i64::from(code))
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorWaitThread(handle: u64, millis: u32) -> u32 {
    let deadline = if millis == u32::MAX {
        u64::MAX
    } else {
        match door_sys::time() {
            Ok(now) => now.saturating_add(
                door_abi::platform::ticks_from_millis(u64::from(millis)).expect("u32 duration"),
            ),
            Err(error) => {
                fail(error);
                return u32::MAX;
            }
        }
    };
    match door_sys::thread_join(handle, deadline) {
        Ok(_) => 0,
        Err(door_abi::Status::Timeout | door_abi::Status::WouldBlock) => 258,
        Err(error) => {
            fail(error);
            u32::MAX
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorGetExitCodeThread(handle: u64, output: u64) -> i32 {
    let code = match door_sys::thread_join(handle, 0) {
        Ok(code) => code as u32,
        Err(door_abi::Status::Timeout | door_abi::Status::WouldBlock) => 259,
        Err(error) => {
            fail(error);
            return 0;
        }
    };
    if super::guest::copy(output, &code as *const u32 as u64, 4).is_err() {
        RtlSetLastWin32Error(error::NOACCESS);
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCloseHandle(handle: u64) -> i32 {
    super::sections::handle_closing(handle);
    match door_sys::close(handle) {
        Ok(()) => {
            super::waits::close_thread(handle);
            super::completion::close_port(handle);
            super::completion::forget_handle(handle);
            1
        }
        Err(error) => {
            fail(error);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorSleep(millis: u32) {
    if millis == 0 {
        let _ = door_sys::yield_now();
    } else if let Ok(now) = door_sys::time() {
        let _ = door_sys::sleep(now.saturating_add(
            door_abi::platform::ticks_from_millis(u64::from(millis)).expect("u32 duration"),
        ));
    }
}
/// A real handle to the calling thread (DuplicateHandle of the
/// GetCurrentThread pseudo-handle): 0 and the error otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenCurrentThread() -> u64 {
    match door_sys::thread_open_self() {
        Ok(handle) => handle,
        Err(error) => {
            fail(error);
            0
        }
    }
}
/// RtlDllShutdownInProgress: FALSE while the process runs (DLL_PROCESS_DETACH
/// at ExitProcess would answer TRUE; nothing calls it there yet).
#[unsafe(no_mangle)]
pub extern "system" fn RtlDllShutdownInProgress() -> u8 {
    0
}
/// A real handle to the calling process (OpenProcess of its own ID,
/// DuplicateHandle of GetCurrentProcess); 0 with the error set.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenCurrentProcess() -> u64 {
    match door_sys::process_open_self() {
        Ok(handle) => handle,
        Err(error) => {
            fail(error);
            0
        }
    }
}
/// OpenThread of a thread of this process by its ID: a new handle to it
/// (the caller itself through THREAD_OPEN_SELF); 0 with
/// ERROR_INVALID_PARAMETER when no running thread has that ID.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenThreadById(id: u32) -> u64 {
    if id == DoorGetCurrentThreadId() {
        return DoorOpenCurrentThread();
    }
    let Some(handle) = super::waits::thread_handle(id) else {
        RtlSetLastWin32Error(error::INVALID_PARAMETER);
        return 0;
    };
    let rights = match door_sys::inspect(handle) {
        Ok(info) => info[1],
        Err(error) => {
            fail(error);
            return 0;
        }
    };
    match door_sys::duplicate(handle, rights) {
        Ok(copy) => copy,
        Err(error) => {
            fail(error);
            0
        }
    }
}
