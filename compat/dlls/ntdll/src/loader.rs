//! User-mode callback dispatch. The lease protects metadata only and is always
//! released before entering application code. No kernel/VM lock spans callbacks.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use door_pe_loader::{
    UserAddress,
    lifecycle::{Lifecycle, Phase, Reason},
};
use door_windows_abi::status;

struct State {
    locked: AtomicBool,
    value: UnsafeCell<Option<Lifecycle>>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    locked: AtomicBool::new(false),
    value: UnsafeCell::new(None),
};
static CALLBACK_OWNER: AtomicU32 = AtomicU32::new(0);
static DYNAMIC_REFERENCES: AtomicU32 = AtomicU32::new(0);
static DYNAMIC_ENTRY: AtomicU64 = AtomicU64::new(0);
const APPLICATION_DLL: u64 = 0x184000000;
pub(super) fn dynamic_present() -> bool {
    DYNAMIC_REFERENCES.load(Ordering::Acquire) != 0
}
struct CallbackGate;
impl CallbackGate {
    fn acquire() -> Result<Self, u32> {
        let id = super::threads::DoorGetCurrentThreadId();
        let deadline = door_sys::time()
            .map_err(|_| status::IO_DEVICE_ERROR)?
            .saturating_add(
                door_abi::platform::ticks_from_millis(1000).expect("constant duration"),
            );
        loop {
            match CALLBACK_OWNER.compare_exchange(0, id, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return Ok(Self),
                Err(owner) if owner == id => return Err(status::NOT_SUPPORTED),
                Err(_) => {
                    if door_sys::time().map_err(|_| status::IO_DEVICE_ERROR)? >= deadline {
                        return Err(status::IO_DEVICE_ERROR);
                    }
                    door_sys::yield_now().map_err(|_| status::IO_DEVICE_ERROR)?;
                }
            }
        }
    }
}
impl Drop for CallbackGate {
    fn drop(&mut self) {
        CALLBACK_OWNER.store(0, Ordering::Release);
    }
}
struct Lease;
impl Lease {
    fn acquire() -> Result<Self, u32> {
        STATE
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self)
            .map_err(|_| status::INVALID_PARAMETER)
    }
    fn value(&mut self) -> &mut Option<Lifecycle> {
        unsafe { &mut *STATE.value.get() }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        STATE.locked.store(false, Ordering::Release);
    }
}

/// Run DLL_PROCESS_ATTACH for the published module inventory (DOORWIN1 or
/// DOORWIN2), then the executable's TLS callbacks.
pub fn attach() -> Result<(), u32> {
    // The main thread, for TLS blocks of DLLs loaded later.
    super::ldr::thread_started();
    let _gate = CallbackGate::acquire()?;
    let dlls = super::modules::dlls();
    let mut inventory = [(UserAddress(0), None); super::modules::MAX_IMAGES];
    if dlls.len() >= inventory.len() {
        return Err(status::INVALID_PARAMETER);
    }
    for (out, module) in inventory.iter_mut().zip(dlls) {
        *out = (
            UserAddress(module.base),
            (module.entry != 0).then_some(UserAddress(module.entry)),
        );
    }
    let count = dlls.len();
    let lifecycle =
        Lifecycle::from_inventory(&inventory[..count]).map_err(|_| status::INVALID_PARAMETER)?;
    {
        let mut lease = Lease::acquire()?;
        if lease.value().is_some() {
            return Err(status::INVALID_PARAMETER);
        }
        *lease.value() = Some(lifecycle);
    }
    dispatch()?;
    tls_callbacks(
        0,
        super::modules::executable_base(),
        1,
        door_windows_abi::BOOTSTRAP_ADDRESS,
    )?;
    let mut lease = Lease::acquire()?;
    if lease
        .value()
        .as_ref()
        .is_none_or(|state| state.phase() != Phase::Live)
    {
        return Err(0xc000_0142); // STATUS_DLL_INIT_FAILED
    }
    Ok(())
}

pub fn orderly_exit(code: u32) -> Result<(), u32> {
    let _gate = CallbackGate::acquire()?;
    loop {
        match door_sys::process_quiesce(code) {
            Ok(()) => break,
            Err(door_abi::Status::WouldBlock) => {
                door_sys::yield_now().map_err(|_| status::IO_DEVICE_ERROR)?;
            }
            Err(_) => return Err(status::IO_DEVICE_ERROR),
        }
    }
    if DYNAMIC_REFERENCES.load(Ordering::Acquire) != 0 {
        dynamic_callback(0, door_windows_abi::BOOTSTRAP_ADDRESS);
        DYNAMIC_REFERENCES.store(0, Ordering::Release);
    }
    // DLLs loaded after startup detach first (they depend on the others).
    notify_loaded(0);
    if Lease::acquire()?.value().is_some() {
        tls_callbacks(
            0,
            super::modules::executable_base(),
            0,
            door_windows_abi::BOOTSTRAP_ADDRESS,
        )?;
    }
    {
        let mut lease = Lease::acquire()?;
        let Some(state) = lease.value().as_mut() else {
            return Ok(());
        };
        state
            .begin_detach()
            .map_err(|_| status::INVALID_PARAMETER)?;
    }
    dispatch()
}

pub fn notify_thread(reason: u32) -> Result<(), u32> {
    let _gate = CallbackGate::acquire()?;
    {
        let mut lease = Lease::acquire()?;
        if lease
            .value()
            .as_ref()
            .is_none_or(|state| state.phase() != Phase::Live)
        {
            return Err(status::INVALID_PARAMETER);
        }
    }
    if reason == 3 && DYNAMIC_REFERENCES.load(Ordering::Acquire) != 0 {
        dynamic_callback(reason, 0);
    }
    let executable = super::modules::executable_base();
    if reason == 3 {
        tls_callbacks(0, executable, reason, 0)?;
        notify_loaded(reason);
    }
    let dlls = super::modules::dlls();
    for position in 0..dlls.len() {
        let index = if reason == 3 {
            dlls.len() - 1 - position
        } else {
            position
        };
        let module = dlls[index];
        if module.entry == 0 {
            continue;
        }
        super::arch::dll_main(module.entry, module.base, reason, 0);
    }
    if reason == 2 {
        notify_loaded(reason);
    }
    if reason == 2 && DYNAMIC_REFERENCES.load(Ordering::Acquire) != 0 {
        dynamic_callback(reason, 0);
    }
    if reason == 2 {
        tls_callbacks(0, executable, reason, 0)?;
    }
    Ok(())
}

fn dynamic_callback(reason: u32, reserved: u64) -> bool {
    if tls_callbacks(3, APPLICATION_DLL, reason, reserved).is_err() {
        return false;
    }
    let entry = DYNAMIC_ENTRY.load(Ordering::Acquire);
    if entry == 0 {
        return true;
    }
    super::arch::dll_main(entry, APPLICATION_DLL, reason, reserved) != 0
}
fn tls_callbacks(index: u64, base: u64, reason: u32, reserved: u64) -> Result<(), u32> {
    for position in 0..door_pe_loader::tls::MAX_CALLBACKS {
        let address = unsafe {
            door_sys::raw(
                door_abi::operation::WINDOWS_TLS_CALLBACK,
                [index, position as u64, 0, 0, 0, 0],
            )
        }
        .result()
        .map_err(|_| status::INVALID_PARAMETER)?;
        if address == 0 {
            break;
        }
        super::arch::dll_main(address, base, reason, reserved);
    }
    Ok(())
}
fn module_name(pointer: u64) -> Result<u64, u32> {
    let mut name = [0u8; 96];
    for index in 0..name.len() {
        let mut unit = [0; 2];
        super::guest::copy(
            unit.as_mut_ptr() as u64,
            pointer
                .checked_add(index as u64 * 2)
                .ok_or(status::ACCESS_VIOLATION)?,
            2,
        )?;
        let unit = u16::from_le_bytes(unit);
        if unit == 0 {
            let name =
                core::str::from_utf8(&name[..index]).map_err(|_| status::INVALID_PARAMETER)?;
            // DOORWIN2 processes: a module of the process (its startup
            // closure or loaded since), by file name; a system DLL named
            // with its directory ("C:\Windows\System32\x.dll") is the same.
            if super::modules::inventory().is_some_and(|table| table.described) {
                let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
                return super::modules::system_module(file).ok_or(126);
            }
            if name.eq_ignore_ascii_case("lifecycle.dll") {
                return Ok(APPLICATION_DLL);
            }
            if name.eq_ignore_ascii_case("ntdll.dll") {
                return Ok(0x180000000);
            }
            if name.eq_ignore_ascii_case("msvcrt.dll") {
                return Ok(0x186000000);
            }
            if name.eq_ignore_ascii_case("kernel32.dll")
                || door_pe_loader::api_set_host(name).is_some()
            {
                return Ok(0x182000000);
            }
            return Err(126);
        }
        name[index] = u8::try_from(unit).map_err(|_| 126u32)?;
    }
    Err(126)
}
fn last_error(code: u32) {
    super::guest::RtlSetLastWin32Error(if code < 65536 {
        code
    } else {
        door_windows_abi::dos_error(code)
    });
}
/// LoadLibrary of a system DLL outside the startup closure (DOORWIN2): the
/// kernel maps it and the DLLs it imports; each new DLL then gets
/// DLL_PROCESS_ATTACH in initialization order. They stay mapped for the
/// process (FreeLibrary of a system DLL keeps it).
fn load_system(pointer: u64, flags: u32) -> Result<u64, u32> {
    if !super::modules::inventory().is_some_and(|table| table.described) {
        return Err(126);
    }
    let mut text = [0u8; 96];
    let mut length = 0;
    for (index, slot) in text.iter_mut().enumerate() {
        let mut unit = [0; 2];
        super::guest::copy(unit.as_mut_ptr() as u64, pointer + index as u64 * 2, 2)?;
        let unit = u16::from_le_bytes(unit);
        if unit == 0 {
            break;
        }
        // A long or non-ASCII path names no system DLL.
        let Ok(byte) = u8::try_from(unit) else {
            return load_private(pointer, flags);
        };
        if index + 1 == 96 {
            return load_private(pointer, flags);
        }
        *slot = byte;
        length += 1;
    }
    let name = core::str::from_utf8(&text[..length]).map_err(|_| 126u32)?;
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let host = door_pe_loader::api_set_host(file).unwrap_or(file);
    let Some((index, provider)) = door_windows_abi::modules::own::by_name(host) else {
        // Not a system DLL: the application's (or an installer's) own.
        return load_private(pointer, flags);
    };
    if !provider.system {
        return Err(126);
    }
    let _gate = CallbackGate::acquire()?;
    // A racing load on another thread may have brought it in.
    if let Some(module) = super::modules::present(provider.base) {
        return Ok(module.base);
    }
    const RECORD: usize = door_windows_abi::bootstrap::MODULE_INIT_BYTES;
    let mut records = [0u8; RECORD * door_pe_loader::MAX_MODULES];
    let count = unsafe {
        door_sys::raw(
            door_abi::operation::WINDOWS_PROVIDER_LOAD,
            [
                index as u64,
                records.as_mut_ptr() as u64,
                records.len() as u64,
                0,
                0,
                0,
            ],
        )
    }
    .result()
    .map_err(|e| match e {
        door_abi::Status::NotFound => 126u32,
        door_abi::Status::ResourceExhausted => 8,
        // The DLL's preferred range is already in use: ERROR_INVALID_ADDRESS.
        door_abi::Status::AlreadyExists => 487,
        // The closure no longer plans (an import, a limit): ERROR_BAD_EXE_FORMAT.
        door_abi::Status::InvalidArgument => 193,
        door_abi::Status::BadAddress => 998,
        door_abi::Status::AccessDenied => 5,
        _ => 1114,
    })? as usize;
    let mut added =
        [door_windows_abi::bootstrap::ModuleInit::default(); door_pe_loader::MAX_MODULES];
    for (slot, record) in added
        .iter_mut()
        .zip(records.chunks_exact(RECORD).take(count))
    {
        *slot = door_windows_abi::bootstrap::ModuleInit::decode_record(record)
            .map_err(|_| status::INVALID_PARAMETER)?;
        super::modules::add_loaded(*slot)?;
    }
    for module in &added[..count] {
        if module.entry == 0 {
            continue;
        }
        if super::arch::dll_main(module.entry, module.base, 1, 0) == 0 {
            // ERROR_DLL_INIT_FAILED, as Windows answers a failing DllMain.
            return Err(1114);
        }
    }
    Ok(provider.base)
}
/// LoadLibrary of a DLL that is not a system DLL: ntdll's user-mode loader
/// (`ldr`) maps it, binds its imports (loading the system DLLs it needs)
/// and attaches it; FreeLibrary of its last reference unmaps it.
/// An application's own DLL (the system DLLs were found before this),
/// searched as `search` does.
fn load_private(pointer: u64, flags: u32) -> Result<u64, u32> {
    search(pointer, flags, load_private_at)
}
/// The folders of the application's executable path (`\` or `/`), into
/// `out`: the folder's length including its separator, or None when the
/// path does not fit.
fn application_folder(out: &mut [u16; 1024]) -> Option<usize> {
    let n = super::modules::DoorGetModuleFileNameW(0, out.as_mut_ptr() as u64, 1024) as usize;
    if n == 0 || n >= out.len() {
        return None;
    }
    out[..n]
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map(|i| i + 1)
}
/// `load` of the module `pointer` names. A file name without a folder is
/// looked for where Windows' safe search order looks (`door_windows_abi::
/// dll_search`): KnownDLLs in System32 only; else the application's
/// folder, System32, the Windows folder, then the current directory
/// (never for a name Windows has in System32), or only the folders
/// LOAD_LIBRARY_SEARCH_* `flags` name. ERROR_MOD_NOT_FOUND in one place
/// goes on to the next.
pub(super) fn search(
    pointer: u64,
    flags: u32,
    load: impl Fn(u64) -> Result<u64, u32>,
) -> Result<u64, u32> {
    use door_windows_abi::dll_search::{Place, places};
    let mut name = [0u16; 260];
    let mut length = 0;
    while length < name.len() {
        let mut unit = [0; 2];
        super::guest::copy(
            unit.as_mut_ptr() as u64,
            pointer
                .checked_add(length as u64 * 2)
                .ok_or(status::ACCESS_VIOLATION)?,
            2,
        )?;
        let unit = u16::from_le_bytes(unit);
        if unit == 0 {
            break;
        }
        name[length] = unit;
        length += 1;
    }
    if length == 0 {
        return Err(87);
    }
    let bare = length < name.len()
        && !name[..length]
            .iter()
            .any(|&u| u == u16::from(b'\\') || u == u16::from(b'/') || u == u16::from(b':'));
    if !bare {
        return load(pointer);
    }
    let mut order = [Place::Current; 4];
    let count = places(&name[..length], flags, &mut order);
    for place in &order[..count] {
        let mut path = [0u16; 1024];
        let folder = match place {
            Place::Current => match load(pointer) {
                Err(126) => continue,
                result => return result,
            },
            Place::Application => match application_folder(&mut path) {
                Some(n) => n,
                None => continue,
            },
            Place::System32 | Place::Windows => {
                let dir: &[u8] = if *place == Place::System32 {
                    b"C:\\Windows\\System32\\"
                } else {
                    b"C:\\Windows\\"
                };
                for (o, &c) in path.iter_mut().zip(dir) {
                    *o = u16::from(c);
                }
                dir.len()
            }
        };
        if folder + length + 1 > path.len() {
            continue;
        }
        path[folder..folder + length].copy_from_slice(&name[..length]);
        path[folder + length] = 0;
        match load(path.as_ptr() as u64) {
            Err(126) => (),
            result => return result,
        }
    }
    Err(126)
}
/// An application's own DLL at a full path: ntdll's user-mode loader maps,
/// binds and attaches it (P18; no kernel capture, size or count limit).
fn load_private_at(pointer: u64) -> Result<u64, u32> {
    super::ldr::load(pointer).map_err(|e| match e {
        2 | 3 => 126,
        e => e,
    })
}
/// DLL_THREAD_ATTACH/DETACH (or, at exit, DLL_PROCESS_DETACH) for the DLLs
/// loaded after startup: attach in initialization order, detach in reverse.
fn notify_loaded(reason: u32) {
    let loaded = super::modules::loaded();
    for position in 0..loaded.len() {
        let index = if reason == 1 || reason == 2 {
            position
        } else {
            loaded.len() - 1 - position
        };
        let module = loaded[index];
        if module.entry == 0 {
            continue;
        }
        super::arch::dll_main(module.entry, module.base, reason, 0);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorLoadLibraryW(pointer: u64) -> u64 {
    DoorLoadLibraryExW(pointer, 0)
}
/// The name of `pointer` with ".dll" when its file name has none (as
/// LoadLibrary appends it), NUL-terminated in `out`.
pub(super) fn module_file_name(pointer: u64, out: &mut [u16; 1030]) -> Result<(), u32> {
    let mut name = [0u16; 1024];
    let mut length = 0;
    while length < name.len() {
        let mut unit = [0; 2];
        super::guest::copy(
            unit.as_mut_ptr() as u64,
            pointer
                .checked_add(length as u64 * 2)
                .ok_or(status::ACCESS_VIOLATION)?,
            2,
        )?;
        let unit = u16::from_le_bytes(unit);
        if unit == 0 {
            break;
        }
        name[length] = unit;
        length += 1;
    }
    if length == 0 {
        return Err(87);
    }
    if length == name.len() {
        return Err(206);
    }
    let n = door_windows_abi::dll_search::with_extension(&name[..length], &mut out[..1029])
        .ok_or(206u32)?;
    out[n] = 0;
    Ok(())
}
/// LoadLibraryEx with LOAD_LIBRARY_SEARCH_* `flags` (0: LoadLibrary's
/// standard search).
#[unsafe(no_mangle)]
pub extern "system" fn DoorLoadLibraryExW(pointer: u64, flags: u32) -> u64 {
    let mut named = [0u16; 1030];
    if let Err(code) = module_file_name(pointer, &mut named) {
        last_error(code);
        return 0;
    }
    let pointer = named.as_ptr() as u64;
    let result = (|| {
        let module = match module_name(pointer) {
            Ok(module) => module,
            Err(126) => return load_system(pointer, flags),
            Err(error) => return Err(error),
        };
        if module != APPLICATION_DLL {
            return Ok(module);
        }
        let _gate = CallbackGate::acquire()?;
        let references = DYNAMIC_REFERENCES.load(Ordering::Acquire);
        if references != 0 {
            DYNAMIC_REFERENCES.store(
                references.checked_add(1).ok_or(status::NO_MEMORY)?,
                Ordering::Release,
            );
            return Ok(module);
        }
        let entry = unsafe { door_sys::raw(door_abi::operation::WINDOWS_MODULE_LOAD, [0; 6]) }
            .result()
            .map_err(|_| status::NO_MEMORY)?;
        DYNAMIC_ENTRY.store(entry, Ordering::Release);
        if !dynamic_callback(1, 0) {
            dynamic_callback(0, 0);
            unsafe { door_sys::raw(door_abi::operation::WINDOWS_MODULE_UNLOAD, [0; 6]) }
                .result()
                .map_err(|_| status::IO_DEVICE_ERROR)?;
            DYNAMIC_ENTRY.store(0, Ordering::Release);
            return Err(1114);
        }
        DYNAMIC_REFERENCES.store(1, Ordering::Release);
        Ok(module)
    })();
    result.unwrap_or_else(|code| {
        last_error(code);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFreeLibrary(module: u64) -> i32 {
    let result = (|| {
        // A DLL ntdll's loader mapped: its reference count.
        if let Some(result) = super::ldr::free(module) {
            return result;
        }
        // A data file or image resource (LoadLibraryEx).
        if module & 3 != 0 {
            return if super::datafile::free(module) {
                Ok(())
            } else {
                Err(status::INVALID_HANDLE)
            };
        }
        let described = super::modules::inventory().is_some_and(|table| table.described);
        if described {
            // Statically loaded system modules stay mapped for the process.
            return if module != super::modules::executable_base()
                && super::modules::present(module).is_some()
            {
                Ok(())
            } else {
                Err(status::INVALID_HANDLE)
            };
        }
        if matches!(module, 0x180000000 | 0x182000000 | 0x186000000) {
            return Ok(());
        }
        if module != APPLICATION_DLL {
            return Err(status::INVALID_HANDLE);
        }
        let _gate = CallbackGate::acquire()?;
        let references = DYNAMIC_REFERENCES.load(Ordering::Acquire);
        if references == 0 {
            return Err(status::INVALID_HANDLE);
        }
        if references > 1 {
            DYNAMIC_REFERENCES.store(references - 1, Ordering::Release);
            return Ok(());
        }
        dynamic_callback(0, 0);
        unsafe { door_sys::raw(door_abi::operation::WINDOWS_MODULE_UNLOAD, [0; 6]) }
            .result()
            .map_err(|_| status::IO_DEVICE_ERROR)?;
        DYNAMIC_REFERENCES.store(0, Ordering::Release);
        DYNAMIC_ENTRY.store(0, Ordering::Release);
        Ok(())
    })();
    result.map_or_else(
        |code| {
            last_error(code);
            0
        },
        |()| 1,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetModuleHandleW(pointer: u64) -> u64 {
    if pointer == 0 {
        return super::modules::executable_base();
    }
    // A DLL ntdll's loader mapped, by its file name.
    let mut name = [0u16; 261];
    let mut n = 0;
    while n < 260 {
        let unit = unsafe { ((pointer as *const u16).add(n)).read_unaligned() };
        if unit == 0 {
            break;
        }
        name[n] = unit;
        n += 1;
    }
    if let Some(base) = super::ldr::find(&name[..n]) {
        return base;
    }
    match module_name(pointer) {
        Ok(module)
            if module != APPLICATION_DLL || DYNAMIC_REFERENCES.load(Ordering::Acquire) != 0 =>
        {
            module
        }
        _ => {
            last_error(126);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetProcAddress(module: u64, pointer: u64) -> u64 {
    // A null module is the process's executable (LdrGetProcedureAddress).
    let module = if module == 0 {
        super::modules::executable_base()
    } else {
        module
    };
    if super::ldr::owns(module) {
        let symbol = if pointer < 65536 {
            door_pe_loader::image::Import::Ordinal(pointer as u16)
        } else {
            let mut n = 0usize;
            while n < 4096 && unsafe { ((pointer as *const u8).add(n)).read() } != 0 {
                n += 1;
            }
            let name = unsafe { core::slice::from_raw_parts(pointer as *const u8, n) };
            door_pe_loader::image::Import::Name { hint: 0, name }
        };
        let address = super::ldr::proc_address(module, symbol).unwrap_or(0);
        if address == 0 {
            report_missing(module, pointer);
            last_error(127);
        }
        return address;
    }
    let result = (|| {
        let mut name = [0u8; 128];
        let (source, bytes, ordinal) = if pointer < 65536 {
            (0, 0, pointer)
        } else {
            let mut count = 0;
            loop {
                if count == name.len() {
                    return Err(status::INVALID_PARAMETER);
                }
                super::guest::copy(
                    name[count..].as_mut_ptr() as u64,
                    pointer
                        .checked_add(count as u64)
                        .ok_or(status::ACCESS_VIOLATION)?,
                    1,
                )?;
                if name[count] == 0 {
                    break;
                }
                count += 1;
            }
            (name.as_ptr() as u64, count as u64, 0)
        };
        unsafe {
            door_sys::raw(
                door_abi::operation::WINDOWS_MODULE_SYMBOL,
                [module, source, bytes, ordinal, 0, 0],
            )
        }
        .result()
        .map_err(|_| 127u32)
    })();
    result.unwrap_or_else(|code| {
        report_missing(module, pointer);
        last_error(code);
        0
    })
}
/// A lookup that found nothing, on the serial log as `DOOR_COMPAT GETPROC
/// <module> <name or #ordinal>` (within the process's diagnostic budget):
/// what a program asked for that Doors does not export.
fn report_missing(module: u64, pointer: u64) {
    let mut text = [0u8; 200];
    let mut n = 0;
    let mut push = |bytes: &[u8], n: &mut usize| {
        for &b in bytes {
            if *n < text.len() && (0x20..0x7f).contains(&b) {
                text[*n] = b;
                *n += 1;
            }
        }
    };
    push(b"DOOR_COMPAT GETPROC ", &mut n);
    let mut path = [0u16; 260];
    let length =
        super::modules::DoorGetModuleFileNameW(module, path.as_mut_ptr() as u64, 260) as usize;
    let file = &path[..length.min(path.len())];
    let start = file
        .iter()
        .rposition(|&u| u == 92 || u == 47)
        .map_or(0, |i| i + 1);
    for &u in &file[start..] {
        push(&[if u < 0x80 { u as u8 } else { b'?' }], &mut n);
    }
    push(b" ", &mut n);
    if pointer < 65536 {
        let mut digits = [0u8; 6];
        let mut v = pointer;
        let mut d = digits.len();
        loop {
            d -= 1;
            digits[d] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        push(b"#", &mut n);
        push(&digits[d..], &mut n);
    } else {
        let mut name = [0u8; 120];
        for (i, out) in name.iter_mut().enumerate() {
            let mut b = [0u8; 1];
            if super::guest::copy(b.as_mut_ptr() as u64, pointer + i as u64, 1).is_err()
                || b[0] == 0
            {
                break;
            }
            *out = b[0];
        }
        let len = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        push(&name[..len], &mut n);
    }
    push(
        b"
", &mut n,
    );
    let _ = door_sys::write(&text[..n]);
}

fn dispatch() -> Result<(), u32> {
    loop {
        let callback = {
            let mut lease = Lease::acquire()?;
            lease
                .value()
                .as_mut()
                .ok_or(status::INVALID_PARAMETER)?
                .next_callback()
                .map_err(|_| status::INVALID_PARAMETER)?
        };
        let Some(callback) = callback else {
            return Ok(());
        };
        // Kernel-authenticated RO startup describes final RX entry points. No
        // Rust reference points into mutable application memory across the call.
        let entry = |base: u64, reason: u32, reserved: u64| {
            super::arch::dll_main(callback.entry.0, base, reason, reserved)
        };
        let reason = match callback.reason {
            Reason::ProcessAttach => 1,
            Reason::ProcessDetach => 0,
        };
        // The startup record is a stable non-NULL token, not writable authority.
        let reserved = if callback.reserved_nonnull {
            door_windows_abi::BOOTSTRAP_ADDRESS
        } else {
            0
        };
        let success = entry(callback.base.0, reason, reserved) != 0;
        let mut lease = Lease::acquire()?;
        lease
            .value()
            .as_mut()
            .ok_or(status::INVALID_PARAMETER)?
            .complete(callback, success)
            .map_err(|_| status::INVALID_PARAMETER)?;
    }
}

/// OutputDebugStringA/W's text on the serial log as `DOOR_DEBUG <pid
/// line> <text>` (printable ASCII, other units as '?', within the process's
/// diagnostic budget): what a kernel debugger attached to Windows sees.
/// `units` bounds the read; a NUL ends the text earlier.
#[unsafe(no_mangle)]
pub extern "system" fn DoorDebugOutput(text: u64, units: u64, wide: u32) {
    let mut line = [0u8; 232];
    line[..11].copy_from_slice(b"DOOR_DEBUG ");
    let mut n = 11;
    let width = if wide != 0 { 2 } else { 1 };
    let mut at = 0u64;
    while at < units.min(220) && n < line.len() - 1 {
        let mut unit = [0u8; 2];
        if super::guest::copy(unit.as_mut_ptr() as u64, text + at * width, width as usize).is_err()
        {
            break;
        }
        let u = u16::from_le_bytes(unit);
        if u == 0 {
            break;
        }
        line[n] = match u {
            0x20..=0x7e => u as u8,
            // A line break ends the record; text after it stays on it.
            10 | 13 => b' ',
            _ => b'?',
        };
        n += 1;
        at += 1;
    }
    line[n] = b'\n';
    let _ = door_sys::write(&line[..n + 1]);
}
