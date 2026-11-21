//! CPL3-only bridge. Every application pointer is captured by USER_COPY before
//! Rust forms a reference; service IPC only sees owned, bounded buffers.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use door_abi::platform::{STARTUP_V2_BYTES, StartupV2, inherited, manifest, mode};
use door_service_client::Console;
use door_windows_abi::{self as abi, status};

struct ConsoleState {
    locked: AtomicBool,
    value: UnsafeCell<Option<Console>>,
}
// The atomic lease owns all accesses, including initialization. A holder
// keeps it for one console request only (`console::read` waits for input
// without it), so another thread waits briefly; reentrancy or a holder
// that never returns is a contained failure, never a second mutable
// reference or a wait under a VM lock.
unsafe impl Sync for ConsoleState {}
static CONSOLE: ConsoleState = ConsoleState {
    locked: AtomicBool::new(false),
    value: UnsafeCell::new(None),
};

struct ConsoleLease;
impl ConsoleLease {
    fn acquire() -> Result<Self, u32> {
        for attempt in 0..100_000u32 {
            if CONSOLE
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(Self);
            }
            if attempt % 64 == 63 {
                door_sys::yield_now().map_err(|_| status::IO_DEVICE_ERROR)?;
            } else {
                core::hint::spin_loop();
            }
        }
        Err(status::IO_DEVICE_ERROR)
    }
    fn value(&mut self) -> &mut Option<Console> {
        // SAFETY: this lease exclusively owns the cell until Drop.
        unsafe { &mut *CONSOLE.value.get() }
    }
}
impl Drop for ConsoleLease {
    fn drop(&mut self) {
        CONSOLE.locked.store(false, Ordering::Release);
    }
}

pub(super) fn copy(destination: u64, source: u64, bytes: usize) -> Result<(), u32> {
    if bytes > abi::COPY_BYTES {
        return Err(status::INVALID_PARAMETER);
    }
    // SAFETY: guest callers own their process memory. Invalid pointers are data
    // to the checked kernel boundary, which validates both complete ranges.
    unsafe { door_sys::user_copy(destination, source, bytes) }.map_err(|_| status::ACCESS_VIOLATION)
}

fn output(pointer: u64, bytes: &[u8]) -> Result<(), u32> {
    copy(pointer, bytes.as_ptr() as u64, bytes.len())
}
/// Write `bytes` to the program's memory at `pointer`.
pub(super) fn put(pointer: u64, bytes: &[u8]) -> Result<(), u32> {
    output(pointer, bytes)
}
/// One request on the process's console connection.
pub(super) fn with_console<R>(f: impl FnOnce(&mut Console) -> Result<R, u32>) -> Result<R, u32> {
    let mut lease = ConsoleLease::acquire().map_err(|_| 1117u32)?;
    let console = lease.value().as_mut().ok_or(6u32)?;
    f(console)
}
/// ReadFile on the console input (`console::read`): the destination is
/// validated before any input is consumed.
pub(super) fn console_input(destination: u64, bytes: u32, read: u64) -> Result<(), u32> {
    output(read, &0u32.to_le_bytes())?;
    if bytes > 1024 {
        return Err(87);
    }
    let mut data = [0u8; 1024];
    if bytes != 0 {
        copy(data.as_mut_ptr() as u64, destination, bytes as usize).map_err(|_| 998u32)?;
        copy(destination, data.as_ptr() as u64, bytes as usize).map_err(|_| 998u32)?;
    }
    let length = super::console::read(&mut data[..bytes as usize])?;
    output(destination, &data[..length]).map_err(|_| 998u32)?;
    output(read, &(length as u32).to_le_bytes()).map_err(|_| 998u32)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlGetLastWin32Error() -> u32 {
    super::arch::last_error()
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlSetLastWin32Error(value: u32) {
    #[cfg(target_arch = "x86_64")]
    super::seh64::log_error_site(value);
    super::arch::set_last_error(value)
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlNtStatusToDosError(value: u32) -> u32 {
    abi::dos_error(value)
}

/// Set when a GUI process started without a console connection.
static GUI_WITHOUT_CONSOLE: AtomicBool = AtomicBool::new(false);

/// A command line longer than the parameter page holds (Init staged the
/// arguments): RTL_USER_PROCESS_PARAMETERS.CommandLine points here.
struct LongCommandLine(UnsafeCell<[u16; LONG_LINE_UNITS]>);
// SAFETY: written once during single-threaded startup, read-only after.
unsafe impl Sync for LongCommandLine {}
const LONG_LINE_UNITS: usize = door_service_protocol::application::LONG_ARGUMENT_BYTES + 1;
static LONG_COMMAND_LINE: LongCommandLine = LongCommandLine(UnsafeCell::new([0; LONG_LINE_UNITS]));

/// Fetch the staged launch arguments from Init and point the process
/// parameters' CommandLine at their Windows command line.
fn long_command_line(registry: u64, split: Option<usize>) -> Result<(), u32> {
    let mut arguments = [0u8; door_service_protocol::application::LONG_ARGUMENT_BYTES];
    let length = door_service_client::Applications::connect(registry)
        .and_then(|mut apps| apps.staged_arguments(&mut arguments))
        .map_err(|_| status::IO_DEVICE_ERROR)?;
    // `split` argument bytes, then the launcher's environment block.
    let length = match split {
        Some(split) if split <= length => {
            super::environment::replace(&arguments[split..length]);
            split
        }
        Some(_) => return Err(status::INVALID_PARAMETER),
        None => length,
    };
    // argv[0] on C: takes its Win32 spelling, as the kernel gives a short
    // command line (`windows_image_path`).
    let mut rewritten = [0u8; door_service_protocol::application::LONG_ARGUMENT_BYTES + 8];
    let first = arguments[..length]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(length);
    let tail = length - first;
    let arguments: &[u8] = match abi::windows_image_path(&arguments[..first], &mut rewritten) {
        Some(n) if n + tail <= rewritten.len() => {
            rewritten[n..n + tail].copy_from_slice(&arguments[first..length]);
            &rewritten[..n + tail]
        }
        _ => &arguments[..length],
    };
    let length = arguments.len();
    // SAFETY: startup runs before any other thread of the process exists.
    let line = unsafe { &mut *LONG_COMMAND_LINE.0.get() };
    let units = abi::command_line(&arguments[..length], &mut line[..LONG_LINE_UNITS - 1])
        .map_err(|_| status::INVALID_PARAMETER)?;
    line[units] = 0;
    let mut descriptor = [0u8; 16];
    descriptor[..2].copy_from_slice(&((units * 2) as u16).to_le_bytes());
    descriptor[2..4].copy_from_slice(&(((units + 1) * 2) as u16).to_le_bytes());
    descriptor[8..].copy_from_slice(&(line.as_ptr() as u64).to_le_bytes());
    output(abi::PARAMETERS_ADDRESS + 112, &descriptor)
}

/// Private translation helper. These adapter IDs never grant kernel authority.
/// A GUI process without a console has NULL standard handles (success).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetStandardHandle(selector: u32, destination: u64) -> u32 {
    let result = (|| {
        let handle = abi::standard_handle(selector)?;
        let slot = match selector {
            abi::STD_INPUT_HANDLE => 0,
            abi::STD_OUTPUT_HANDLE => 1,
            _ => 2,
        };
        if let Some(pipe) = super::pipes::standard(slot) {
            return output(destination, &pipe.to_le_bytes());
        }
        if GUI_WITHOUT_CONSOLE.load(Ordering::Acquire) {
            return output(destination, &0u64.to_le_bytes());
        }
        let mut lease = ConsoleLease::acquire()?;
        if lease.value().is_none() {
            return Err(status::INVALID_HANDLE);
        }
        output(destination, &handle.to_le_bytes())
    })();
    result.map_or_else(|status| status, |()| status::SUCCESS)
}

/// Initial self-process subset. No remote-process access is inferred from an
/// integer, and both user ranges are checked atomically by the kernel.
#[unsafe(no_mangle)]
pub extern "system" fn NtWriteVirtualMemory(
    process: usize,
    destination: usize,
    source: usize,
    bytes: usize,
    written: usize,
) -> u32 {
    let process = door_windows_abi::widen(process);
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    let written = door_windows_abi::widen(written);
    {
        let result = (|| {
            if process != abi::CURRENT_PROCESS {
                return Err(status::INVALID_HANDLE);
            }
            // *lpNumberOfBytesWritten is a SIZE_T (P17: the program's width).
            let size = core::mem::size_of::<usize>();
            if written != 0 {
                output(written, &0u64.to_le_bytes()[..size])?;
            }
            copy(destination, source, bytes)?;
            if written != 0 {
                output(written, &(bytes as u64).to_le_bytes()[..size])?;
            }
            Ok(())
        })();
        result.map_or_else(|status| status, |()| status::SUCCESS)
    }
}

/// IO_STATUS_BLOCK (Status, Information), at the program's width (P17).
fn io_status(pointer: u64, result: u32, bytes: usize) -> Result<(), u32> {
    use super::shapes::{IO_STATUS_BLOCK, set, size};
    let mut value = [0u8; size(&IO_STATUS_BLOCK)];
    set(&mut value, &IO_STATUS_BLOCK, 0, u64::from(result));
    set(&mut value, &IO_STATUS_BLOCK, 1, bytes as u64);
    output(pointer, &value)
}

/// P09 synchronous console subset. Capture the whole request before the first
/// output effect. Asynchronous I/O, offsets and non-ASCII code-page conversion
/// explicitly fail until their contracts are implemented and qualified.
#[unsafe(no_mangle)]
pub extern "system" fn NtWriteFile(
    handle: usize,
    event: usize,
    apc: usize,
    context: usize,
    result: usize,
    source: usize,
    bytes: u32,
    offset: usize,
    key: usize,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let event = door_windows_abi::widen(event);
    let apc = door_windows_abi::widen(apc);
    let context = door_windows_abi::widen(context);
    let result = door_windows_abi::widen(result);
    let source = door_windows_abi::widen(source);
    let offset = door_windows_abi::widen(offset);
    let key = door_windows_abi::widen(key);
    {
        let mut completed = 0;
        let outcome = (|| {
            io_status(result, status::SUCCESS, 0)?;
            if handle == abi::NUL_HANDLE {
                // NUL discards everything written to it.
                completed = bytes as usize;
                return Ok(());
            }
            if !abi::writable_console(handle) {
                return Err(status::INVALID_HANDLE);
            }
            if event != 0 || apc != 0 || context != 0 || offset != 0 || key != 0 {
                return Err(status::NOT_SUPPORTED);
            }
            let count = bytes as usize;
            if count > abi::COPY_BYTES {
                return Err(status::INVALID_PARAMETER);
            }
            let mut capture = [0u8; abi::COPY_BYTES];
            if count != 0 {
                copy(capture.as_mut_ptr() as u64, source, count)
                    .map_err(|_| status::INVALID_USER_BUFFER)?;
            }
            // The Door console is UTF-8 (code page 65001): any valid UTF-8 text
            // is written, in chunks that never split a character.
            let text = core::str::from_utf8(&capture[..count])
                .map_err(|_| status::UNMAPPABLE_CHARACTER)?;
            let mut lease = ConsoleLease::acquire()?;
            let console = lease.value().as_mut().ok_or(status::INVALID_HANDLE)?;
            for chunk in crate::utf8_chunks(text, door_service_protocol::CHUNK_BYTES) {
                let accepted = console.write(chunk).map_err(|_| status::IO_DEVICE_ERROR)?;
                completed += accepted;
                if accepted != chunk.len() {
                    return Err(status::IO_DEVICE_ERROR);
                }
            }
            Ok(())
        })();
        let value = outcome.map_or_else(|status| status, |()| status::SUCCESS);
        // The enforced single-thread personality keeps this validated range stable
        // throughout service IPC. P10 must revisit this before adding Windows threads.
        match io_status(result, value, completed) {
            Ok(()) => value,
            Err(status) => status,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn NtTerminateProcess(process: usize, code: u32) -> u32 {
    let process = door_windows_abi::widen(process);
    {
        if process != abi::CURRENT_PROCESS {
            return status::INVALID_HANDLE;
        }
        door_sys::exit(i64::from(code))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlExitUserProcess(code: u32) -> ! {
    // Its pipe ends close with it (a reader sees the end of the data).
    super::pipes::close_all();
    if super::loader::orderly_exit(code).is_err() {
        door_sys::exit(-304);
    }
    door_sys::exit(i64::from(code))
}

#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub extern "system" fn DoorProcessBootstrap(pointer: u64) -> ! {
    process_bootstrap(pointer)
}
/// x86 (P17): the kernel starts a 32-bit process here with the startup
/// record's address in ECX (as it passes RCX to an x64 one).
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn DoorProcessBootstrap() -> ! {
    core::arch::naked_asm!("push ecx", "call {f}", "ud2", f = sym process_bootstrap32)
}
#[cfg(target_arch = "x86")]
extern "system" fn process_bootstrap32(pointer: u32) -> ! {
    process_bootstrap(u64::from(pointer))
}
fn process_bootstrap(pointer: u64) -> ! {
    let result = bootstrap(pointer);
    match result {
        Ok(code) => RtlExitUserProcess(code),
        Err(_) => door_sys::exit(-300),
    }
}

/// Common view of DOORWIN1 (P09/P10) and DOORWIN2 (application path).
struct Record {
    native_startup: u64,
    executable_entry: u64,
    teb: u64,
    peb: u64,
    parameters: u64,
    version: u32,
    subsystem: u16,
    /// Some DLL record has an entry point (refused for the P09 profile).
    dll_entries: bool,
}
/// Capture and decode the startup record, then publish its module inventory.
/// Kept out of line so the bootstrap frame stays below one page.
#[inline(never)]
fn record(pointer: u64) -> Result<Record, u32> {
    let mut bytes = [0u8; abi::bootstrap::BOOTSTRAP_BYTES];
    copy(bytes.as_mut_ptr() as u64, pointer, bytes.len())?;
    let bytes = &bytes;
    let mut inventory = super::modules::Inventory {
        images: [abi::bootstrap::ModuleInit::default(); super::modules::MAX_IMAGES],
        count: 1,
        subsystem: 3,
        described: false,
    };
    if &bytes[..8] == b"DOORWIN2" {
        let b = abi::bootstrap::Bootstrap::decode(bytes).map_err(|_| status::INVALID_PARAMETER)?;
        inventory.images[0] = b.executable;
        for (out, module) in inventory.images[1..].iter_mut().zip(b.images().skip(1)) {
            *out = *module;
            inventory.count += 1;
        }
        inventory.subsystem = b.subsystem;
        inventory.described = true;
        let dll_entries = b.images().skip(1).any(|m| m.entry != 0);
        super::modules::publish(inventory)?;
        return Ok(Record {
            native_startup: b.native_startup,
            executable_entry: b.executable_entry,
            teb: b.teb,
            peb: b.peb,
            parameters: b.parameters,
            version: 2,
            subsystem: b.subsystem,
            dll_entries,
        });
    }
    let b = abi::Bootstrap::decode(&bytes[..abi::BOOTSTRAP_BYTES])
        .map_err(|_| status::INVALID_PARAMETER)?;
    inventory.images[0].base = b.executable_base;
    inventory.images[0].entry = b.executable_entry;
    for (out, module) in inventory.images[1..]
        .iter_mut()
        .zip(&b.modules[..b.module_count as usize])
    {
        out.base = module.base;
        out.entry = module.entry;
        inventory.count += 1;
    }
    let dll_entries = b.modules[..b.module_count as usize]
        .iter()
        .any(|m| m.entry != 0);
    super::modules::publish(inventory)?;
    Ok(Record {
        native_startup: b.native_startup,
        executable_entry: b.executable_entry,
        teb: b.teb,
        peb: b.peb,
        parameters: b.parameters,
        version: 1,
        subsystem: 3,
        dll_entries,
    })
}

/// Native handles inherited at startup that compat DLLs may use (see the
/// `door_windows_abi` crate documentation). Written once by the bootstrap.
static REGISTRY: AtomicU64 = AtomicU64::new(0);
static APP_ASSETS: AtomicU64 = AtomicU64::new(0);
/// The working directory the launcher granted (what ShellExecute hands a
/// child as its current directory).
static DIRECTORY: AtomicU64 = AtomicU64::new(0);
static PROCESS_ID: AtomicU64 = AtomicU64::new(0);
/// The process's authenticated identity (`Identity::encode`), recorded at
/// bootstrap: advapi32 builds its token from it.
static IDENTITY: [AtomicU64; door_abi::platform::IDENTITY_BYTES / 8] =
    [const { AtomicU64::new(0) }; door_abi::platform::IDENTITY_BYTES / 8];
/// This process's groups (`door_abi::platform::group` bits).
pub(super) fn current_groups() -> u64 {
    let mut bytes = [0u8; door_abi::platform::IDENTITY_BYTES];
    for (chunk, word) in bytes.chunks_exact_mut(8).zip(&IDENTITY) {
        chunk.copy_from_slice(&word.load(Ordering::Acquire).to_le_bytes());
    }
    door_abi::platform::Identity::decode(&bytes).map_or(0, |identity| identity.groups)
}
/// This process's user (0 before bootstrap).
pub(super) fn current_uid() -> u32 {
    let mut bytes = [0u8; door_abi::platform::IDENTITY_BYTES];
    for (chunk, word) in bytes.chunks_exact_mut(8).zip(&IDENTITY) {
        chunk.copy_from_slice(&word.load(Ordering::Acquire).to_le_bytes());
    }
    door_abi::platform::Identity::decode(&bytes).map_or(0, |identity| identity.uid)
}
/// The identity record (`IDENTITY_BYTES`) into `out`; STATUS_SUCCESS.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetIdentity(out: u64) -> u32 {
    let mut bytes = [0u8; door_abi::platform::IDENTITY_BYTES];
    for (chunk, word) in bytes.chunks_exact_mut(8).zip(&IDENTITY) {
        chunk.copy_from_slice(&word.load(Ordering::Acquire).to_le_bytes());
    }
    match copy(out, bytes.as_ptr() as u64, bytes.len()) {
        Ok(()) => status::SUCCESS,
        Err(e) => e,
    }
}

/// `GetCurrentProcessId`: the kernel process ID (low 32 bits, never 0).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetCurrentProcessId() -> u32 {
    (PROCESS_ID.load(Ordering::Acquire) as u32).max(1)
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorGetInheritedHandle(index: u32) -> u64 {
    match index as usize {
        inherited::REGISTRY => REGISTRY.load(Ordering::Acquire),
        inherited::APP_ASSETS => APP_ASSETS.load(Ordering::Acquire),
        inherited::DIRECTORY => DIRECTORY.load(Ordering::Acquire),
        _ => 0,
    }
}

/// Bulk checked copy for the compat DLLs (BitBlt, DIB and string transfers).
/// The kernel validates both complete ranges before moving any byte and
/// enforces the per-manifest limit (64 KiB for WINDOWS_GUI).
#[unsafe(no_mangle)]
pub extern "system" fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32 {
    if bytes > abi::GUI_COPY_BYTES {
        return status::INVALID_PARAMETER;
    }
    if bytes == 0 {
        return status::SUCCESS;
    }
    // SAFETY: integer user addresses are data to the checked kernel boundary.
    match unsafe { door_sys::user_copy(destination, source, bytes) } {
        Ok(()) => status::SUCCESS,
        Err(door_abi::Status::InvalidArgument) => status::INVALID_PARAMETER,
        Err(_) => status::ACCESS_VIOLATION,
    }
}

fn bootstrap(pointer: u64) -> Result<u32, u32> {
    let bootstrap = record(pointer)?;
    if pointer != abi::BOOTSTRAP_ADDRESS
        || bootstrap.teb != abi::TEB_ADDRESS
        || bootstrap.peb != abi::PEB_ADDRESS
        || bootstrap.parameters != abi::PARAMETERS_ADDRESS
    {
        return Err(status::NOT_SUPPORTED);
    }
    let mut native = [0u8; STARTUP_V2_BYTES];
    copy(
        native.as_mut_ptr() as u64,
        bootstrap.native_startup,
        native.len(),
    )?;
    let startup = StartupV2::decode(&native).map_err(|_| status::INVALID_PARAMETER)?;
    let mut identity = [0u8; door_abi::platform::IDENTITY_BYTES];
    if startup.identity.encode(&mut identity).is_ok() {
        for (chunk, word) in identity.chunks_exact(8).zip(&IDENTITY) {
            word.store(
                u64::from_le_bytes(chunk.try_into().unwrap()),
                Ordering::Release,
            );
        }
    }
    let initial = startup.identity.manifest == manifest::WINDOWS && startup.mode == mode::WINDOWS;
    let base = startup.identity.manifest == manifest::WINDOWS_BASE
        && matches!(startup.mode, mode::WINDOWS_BASE | mode::WINDOWS_REOPEN);
    let gui = startup.identity.manifest == manifest::WINDOWS_GUI;
    let application = (startup.identity.manifest == manifest::WINDOWS_APPLICATION || gui)
        && startup.mode == mode::SESSION;
    // Each record version belongs to its own manifests only.
    if !(initial || base || application)
        || (bootstrap.version == 2) != application
        || (gui != (bootstrap.subsystem == 2))
        || startup.identity.capabilities != 0
        || startup.identity.session == 0
    {
        return Err(status::INVALID_PARAMETER);
    }
    if initial && bootstrap.dll_entries {
        return Err(status::NOT_SUPPORTED);
    }
    let executable_entry = bootstrap.executable_entry;
    let teb = super::arch::teb();
    if teb != bootstrap.teb {
        return Err(status::INVALID_PARAMETER);
    }
    if !application {
        probe_memory_and_authority(initial)?;
    }
    RtlSetLastWin32Error(0x1357_9bdf);
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    REGISTRY.store(registry, Ordering::Release);
    PROCESS_ID.store(startup.base.pid, Ordering::Release);
    DIRECTORY.store(
        startup.base.inherited[inherited::DIRECTORY].handle,
        Ordering::Release,
    );
    APP_ASSETS.store(
        startup.base.inherited[inherited::APP_ASSETS].handle,
        Ordering::Release,
    );
    // STARTF_USESTDHANDLES: the launcher's pipes are the standard handles.
    super::pipes::adopt(startup.standard.map(|g| g.handle));
    // A GUI image launched without a console (Explorer: Init marks the
    // environment) runs with NULL standard handles; console images, and GUI
    // images started from a console (Terminal), use the session console.
    let detached = gui
        && startup.environment[..usize::from(startup.environment_bytes)]
            .split(|b| *b == 0)
            .any(|entry| {
                entry == door_service_protocol::application::DETACHED_ENVIRONMENT.as_bytes()
            });
    if detached {
        GUI_WITHOUT_CONSOLE.store(true, Ordering::Release);
    } else {
        match Console::connect(registry) {
            Ok(console) => {
                let mut lease = ConsoleLease::acquire()?;
                if lease.value().is_some() {
                    return Err(status::INVALID_PARAMETER);
                }
                *lease.value() = Some(console);
                drop(lease);
                // A program a Terminal tab started (or a child of one) uses
                // that tab's console.
                super::console::attach(
                    &startup.environment[..usize::from(startup.environment_bytes)],
                );
            }
            Err(_) if gui => GUI_WITHOUT_CONSOLE.store(true, Ordering::Release),
            Err(_) => return Err(status::IO_DEVICE_ERROR),
        }
    }
    if application {
        // PEB.ProcessHeap (0x30 in the PEB, 0x18 in the PEB32) names the
        // ntdll process heap.
        let heap = super::heap::initialize()?;
        let width = core::mem::size_of::<usize>();
        output(
            abi::PEB_ADDRESS + super::arch::PEB_PROCESS_HEAP,
            &heap.to_le_bytes()[..width],
        )?;
    }
    for _ in 0..3 {
        door_sys::yield_now().map_err(|_| status::IO_DEVICE_ERROR)?;
    }
    let (after_teb, after_peb) = (super::arch::teb(), super::arch::peb());
    if after_teb != bootstrap.teb
        || after_peb != bootstrap.peb
        || RtlGetLastWin32Error() != 0x1357_9bdf
    {
        return Err(status::INVALID_PARAMETER);
    }
    let receipt: &[u8] = if initial {
        b"WINDOWS_ADAPTER_PASS copy=1 atomic=1 limits=1 thread_denied=1 heap_denied=1 raw_denied=1 gs=1 fs=1\n"
    } else {
        b"WINDOWS_BASE_ADAPTER_PASS copy=1 atomic=1 limits=1 gs=1 fs=1\n"
    };
    if !application
        && door_sys::write(receipt).map_err(|_| status::IO_DEVICE_ERROR)? != receipt.len()
    {
        return Err(status::IO_DEVICE_ERROR);
    }
    RtlSetLastWin32Error(0);
    if base || application {
        let cwd = if application {
            startup.environment[..startup.environment_bytes as usize]
                .split(|b| *b == 0)
                .find_map(|entry| entry.strip_prefix(b"PWD="))
                .map(core::str::from_utf8)
                .transpose()
                .map_err(|_| status::INVALID_PARAMETER)?
        } else {
            None
        };
        super::net::initialize(startup.base.inherited[inherited::REGISTRY].handle);
        super::files::initialize(startup.base.inherited[inherited::REGISTRY].handle, cwd)
            .map_err(|_| status::IO_DEVICE_ERROR)?;
        if application {
            let environment = &startup.environment[..usize::from(startup.environment_bytes)];
            // The session's block first: a staged lpEnvironment replaces it.
            super::environment::initialize(environment);
            if environment.split(|b| *b == 0).any(|entry| {
                entry == door_service_protocol::application::LONG_ARGUMENTS_ENVIRONMENT.as_bytes()
            }) {
                let split = environment.split(|b| *b == 0).find_map(|entry| {
                    entry
                        .strip_prefix(
                            door_service_protocol::application::STAGED_ENVIRONMENT_PREFIX
                                .as_bytes(),
                        )
                        .and_then(|n| core::str::from_utf8(n).ok())
                        .and_then(|n| n.parse::<usize>().ok())
                });
                long_command_line(startup.base.inherited[inherited::REGISTRY].handle, split)?;
            }
        }
    }
    let mut executable_entry = executable_entry;
    if base || application {
        super::loader::attach()?;
        // A program too large for Init to capture whole runs through the
        // image host: Init names it (`DOOR_IMAGE`), the loader lays it out
        // from its file and it becomes the process's executable.
        let environment = &startup.environment[..usize::from(startup.environment_bytes)];
        if environment
            .split(|b| *b == 0)
            .any(|entry| entry == door_service_protocol::application::IMAGE_ENVIRONMENT.as_bytes())
        {
            // The program's own path: the process's image path, which
            // Init gave the program's.
            let mut units = [0u16; 260];
            let n =
                super::modules::DoorGetModuleFileNameW(0, units.as_mut_ptr() as u64, 260) as usize;
            if n == 0 || n >= units.len() {
                return Err(status::INVALID_PARAMETER);
            }
            executable_entry = super::ldr::load_program(&units[..n]).inspect_err(|_code| {
                #[cfg(target_arch = "x86_64")]
                super::seh64::log_error_site(*_code);
                let _ = door_sys::write(b"DOOR_LDR program failed to load\n");
            })?;
        }
        // In a Terminal tab: Ctrl+C reaches the program as CTRL_C_EVENT.
        super::console::start_watching();
    }
    // SAFETY: the loader verified an executable entry inside the authenticated
    // immutable PE mapping. The compiler supplies Win64 shadow space/alignment.
    // x64: on the stack the executable's headers ask for (P18 M0h).
    #[cfg(target_arch = "x86_64")]
    {
        let (reserve, commit) = super::stacks::sizes(0, 0);
        Ok(super::stacks::run(main_entry, executable_entry, reserve, commit) as u32)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let entry: extern "system" fn() -> u32 =
            unsafe { core::mem::transmute(executable_entry as usize) };
        Ok(entry())
    }
}
/// The executable's entry point (no arguments, its exit code).
#[cfg(target_arch = "x86_64")]
extern "system" fn main_entry(entry: u64) -> u64 {
    // SAFETY: the loader verified `entry` inside the executable's code.
    let entry: extern "system" fn() -> u32 = unsafe { core::mem::transmute(entry as usize) };
    u64::from(entry())
}

fn probe_memory_and_authority(initial: bool) -> Result<(), u32> {
    use door_abi::{Status, operation, rights};
    let mut bytes: [u8; 16] = core::array::from_fn(|index| index as u8);
    let pointer = bytes.as_mut_ptr() as u64;
    // SAFETY: the array belongs exclusively to this probe; overlap is supported.
    unsafe { door_sys::user_copy(pointer + 4, pointer, 12) }
        .map_err(|_| status::IO_DEVICE_ERROR)?;
    if bytes != [0, 1, 2, 3, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
        return Err(status::IO_DEVICE_ERROR);
    }
    let saved = bytes;
    if unsafe { door_sys::user_copy(pointer, 1, bytes.len()) } != Err(Status::BadAddress)
        || bytes != saved
    {
        return Err(status::IO_DEVICE_ERROR);
    }

    // A partly writable destination must not receive even its valid prefix.
    let tail = abi::STATE_ADDRESS + abi::STATE_BYTES as u64 - 8;
    let mut before = [0u8; 8];
    copy(before.as_mut_ptr() as u64, tail, before.len())?;
    if unsafe { door_sys::user_copy(tail, pointer, 16) } != Err(Status::BadAddress) {
        return Err(status::IO_DEVICE_ERROR);
    }
    let mut after = [0u8; 8];
    copy(after.as_mut_ptr() as u64, tail, after.len())?;
    if before != after {
        return Err(status::IO_DEVICE_ERROR);
    }

    let copy_limit = if initial { abi::COPY_BYTES } else { 4096 };
    if unsafe { door_sys::user_copy(pointer, pointer, copy_limit + 1) }
        != Err(Status::InvalidArgument)
        || unsafe { door_sys::raw(operation::USER_COPY, [pointer, pointer, 1, 1, 0, 0]) }.result()
            != Err(Status::InvalidArgument)
    {
        return Err(status::IO_DEVICE_ERROR);
    }
    if initial
        && (unsafe { door_sys::thread_create(denied_thread as *const () as u64, 0) }
            != Err(Status::AccessDenied)
            || unsafe { door_sys::vm_allocate(0x5000_0000, 4096, rights::READ | rights::WRITE) }
                != Err(Status::ResourceExhausted))
    {
        return Err(status::IO_DEVICE_ERROR);
    }
    if door_sys::console_write(0, b"forbidden").is_ok() {
        return Err(status::IO_DEVICE_ERROR);
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
extern "sysv64" fn denied_thread(_argument: u64) -> ! {
    door_sys::exit(-303)
}
#[cfg(target_arch = "x86")]
extern "C" fn denied_thread(_argument: u32) -> ! {
    door_sys::exit(-303)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    door_sys::exit(-301)
}

/// The pinned Windows core library references the compiler's exception symbol.
/// P09 never dispatches foreign unwinds: reaching this boundary terminates the
/// process. P10 must replace it only with a qualified language-runtime contract.
#[unsafe(no_mangle)]
extern "system" fn __CxxFrameHandler3() -> ! {
    door_sys::exit(-302)
}

/// x64 stack probe (as Windows' ntdll exports it): touches every page of a
/// frame larger than one page, from the caller's stack pointer down, so an
/// overflow faults on the guard page instead of skipping it. RAX holds the
/// frame size; every register except R10/R11 and flags is preserved.
/// Compat DLLs that link ntdll's import library resolve their own probes here.
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn __chkstk() {
    core::arch::naked_asm!(
        "push rcx",
        "push rax",
        "cmp rax, 0x1000",
        "lea rcx, [rsp + 24]",
        "jb 3f",
        "2:",
        "sub rcx, 0x1000",
        "test [rcx], rcx",
        "sub rax, 0x1000",
        "cmp rax, 0x1000",
        "ja 2b",
        "3:",
        "sub rcx, rax",
        "test [rcx], rcx",
        "pop rax",
        "pop rcx",
        "ret",
    )
}

/// x86 stack probe (`_chkstk`, as Windows' 32-bit ntdll exports it): EAX
/// holds the frame size; every page from the caller's stack pointer down is
/// touched, then ESP is lowered by the size (x86's probe allocates the frame
/// itself) and the probe returns to its caller. Other registers keep their
/// values.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn _chkstk() {
    core::arch::naked_asm!(
        // ECX: the caller's stack pointer (above the return address and the
        // saved ECX).
        "push ecx",
        "lea ecx, [esp + 8]",
        "cmp eax, 0x1000",
        "jb 3f",
        "2:",
        "sub ecx, 0x1000",
        "test [ecx], eax",
        "sub eax, 0x1000",
        "cmp eax, 0x1000",
        "jae 2b",
        "3:",
        "sub ecx, eax",
        "test [ecx], eax",
        // The new stack pointer is ECX; restore ECX and return through EAX
        // without touching the stack below it.
        "mov eax, esp",
        "mov esp, ecx",
        "mov ecx, [eax]",
        "mov eax, [eax + 4]",
        "jmp eax",
    )
}

/// Linkage marker required by Windows compiler-builtins, not an FP operation.
#[unsafe(no_mangle)]
static _fltused: i32 = 0;

/// x86 (P17): the 64-bit division helpers MSVC-convention code calls (as
/// Windows' 32-bit ntdll exports them): both operands on the stack, the
/// result in EDX:EAX, the callee popping its 16 bytes. Shift-and-subtract,
/// so no helper calls itself.
#[cfg(target_arch = "x86")]
mod long_division {
    fn divide(mut dividend: u64, divisor: u64) -> (u64, u64) {
        if divisor == 0 {
            // A zero divisor faults, as the hardware divide does.
            unsafe { core::arch::asm!("ud2", options(noreturn)) }
        }
        let mut quotient = 0u64;
        let mut remainder = 0u64;
        for _ in 0..64 {
            remainder = (remainder << 1) | (dividend >> 63);
            dividend <<= 1;
            quotient <<= 1;
            if remainder >= divisor {
                remainder -= divisor;
                quotient |= 1;
            }
        }
        (quotient, remainder)
    }
    fn magnitude(value: i64) -> u64 {
        if value < 0 {
            (!(value as u64)).wrapping_add(1)
        } else {
            value as u64
        }
    }
    fn signed(value: u64, negative: bool) -> u64 {
        if negative {
            (!value).wrapping_add(1)
        } else {
            value
        }
    }
    extern "C" fn udiv(dividend: u64, divisor: u64) -> u64 {
        divide(dividend, divisor).0
    }
    extern "C" fn urem(dividend: u64, divisor: u64) -> u64 {
        divide(dividend, divisor).1
    }
    extern "C" fn sdiv(dividend: i64, divisor: i64) -> u64 {
        let (q, _) = divide(magnitude(dividend), magnitude(divisor));
        signed(q, (dividend < 0) != (divisor < 0))
    }
    extern "C" fn srem(dividend: i64, divisor: i64) -> u64 {
        let (_, r) = divide(magnitude(dividend), magnitude(divisor));
        signed(r, dividend < 0)
    }
    // Each re-pushes its four argument words for the cdecl body.
    macro_rules! helper {
        ($name:ident, $body:ident) => {
            #[unsafe(no_mangle)]
            #[unsafe(naked)]
            pub unsafe extern "C" fn $name() {
                core::arch::naked_asm!(
                    "push dword ptr [esp + 16]",
                    "push dword ptr [esp + 16]",
                    "push dword ptr [esp + 16]",
                    "push dword ptr [esp + 16]",
                    "call {f}",
                    "add esp, 16",
                    "ret 16",
                    f = sym $body,
                )
            }
        };
    }
    helper!(_aulldiv, udiv);
    helper!(_aullrem, urem);
    helper!(_alldiv, sdiv);
    helper!(_allrem, srem);
}

// Original freestanding compiler memory primitives. Volatile operations prevent
// LLVM from recursively replacing these implementations with the same symbol.
#[unsafe(no_mangle)]
unsafe extern "C" fn memcpy(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    for index in 0..bytes {
        unsafe {
            destination
                .add(index)
                .write_volatile(source.add(index).read_volatile());
        }
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memset(destination: *mut u8, value: i32, bytes: usize) -> *mut u8 {
    for index in 0..bytes {
        unsafe {
            destination.add(index).write_volatile(value as u8);
        }
    }
    destination
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memmove(destination: *mut u8, source: *const u8, bytes: usize) -> *mut u8 {
    if (destination as usize) <= source as usize {
        unsafe { memcpy(destination, source, bytes) }
    } else {
        for index in (0..bytes).rev() {
            unsafe {
                destination
                    .add(index)
                    .write_volatile(source.add(index).read_volatile());
            }
        }
        destination
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, bytes: usize) -> i32 {
    for index in 0..bytes {
        let (a, b) = unsafe {
            (
                left.add(index).read_volatile(),
                right.add(index).read_volatile(),
            )
        };
        if a != b {
            return i32::from(a) - i32::from(b);
        }
    }
    0
}

/// `copy` for large ranges: `abi::BULK_COPY_BYTES` per checked kernel copy.
pub(super) fn copy_bulk(destination: u64, source: u64, bytes: usize) -> Result<(), u32> {
    let mut done = 0usize;
    while done < bytes {
        let n = (bytes - done).min(abi::BULK_COPY_BYTES);
        // SAFETY: integer user addresses are data to the checked kernel
        // boundary, which validates both complete ranges.
        unsafe { door_sys::user_copy(destination + done as u64, source + done as u64, n) }
            .map_err(|_| status::ACCESS_VIOLATION)?;
        done += n;
    }
    Ok(())
}
