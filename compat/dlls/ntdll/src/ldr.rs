//! The user-mode DLL loader (P18): what LdrLoadDll does on Windows for a
//! DLL that is not a system DLL. The file is read and laid out in a
//! MEM_IMAGE reservation of this process (at its preferred base when that
//! is free, else wherever it fits, relocated), its imports are bound
//! against loaded modules (loading what is missing; API sets and export
//! forwarders followed; a module being loaded is already visible, so
//! import cycles bind), its sections take their protections, its implicit
//! TLS gets a slot and a block in every thread, and its TLS callbacks and
//! DllMain run. LoadLibrary of the same file again counts a reference;
//! FreeLibrary of the last one detaches and unmaps it.
//!
//! The registry's lock is re-entrant for the thread that holds it, so a
//! DllMain may load another DLL, as on Windows.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, Ordering},
};
use door_abi::operation as op;
use door_pe_loader::image::{self, Export, Headers, Import};
use door_windows_abi::{bootstrap::ModuleInit, memory as vm};

const MAX_MODULES: usize = 128;
const NAME_UNITS: usize = 260;
/// Largest DLL file read.
const MAX_FILE_BYTES: usize = 512 << 20;
/// First TLS vector index the user loader gives out (the kernel publishes
/// its static TLS slots below this one).
const FIRST_TLS_INDEX: usize = door_pe_loader::MAX_TLS_MODULES;
/// TLS vector entries a thread page holds: x64 `windows_tls::TLS_VECTOR` up
/// to `TLS_DATA` (1,536 bytes of 8-byte entries); x86 the TEB32 page's array
/// (`PAGE_TLS_ARRAY` up to `PAGE_TLS_DATA`, 4-byte entries).
const TLS_VECTOR_ENTRIES: usize = if cfg!(target_arch = "x86_64") {
    192
} else {
    (door_windows_abi::x86::PAGE_TLS_DATA - door_windows_abi::x86::PAGE_TLS_ARRAY) / 4
};
const ENTRY_BYTES: u64 = core::mem::size_of::<usize>() as u64;

#[derive(Clone, Copy)]
struct Module {
    base: u64,
    size: u64,
    entry: u64,
    references: u32,
    /// DLL_PROCESS_ATTACH has completed.
    attached: bool,
    name: [u16; NAME_UNITS],
    name_len: usize,
    tls_index: Option<usize>,
    tls: Option<image::Tls>,
}
struct Registry {
    owner: AtomicU32,
    depth: UnsafeCell<u32>,
    modules: UnsafeCell<[Option<Module>; MAX_MODULES]>,
}
unsafe impl Sync for Registry {}
static REGISTRY: Registry = Registry {
    owner: AtomicU32::new(0),
    depth: UnsafeCell::new(0),
    modules: UnsafeCell::new([None; MAX_MODULES]),
};
/// The loader lock: re-entrant for its owner.
struct Lock;
impl Lock {
    fn acquire() -> Self {
        let me = super::threads::DoorGetCurrentThreadId();
        if REGISTRY.owner.load(Ordering::Acquire) != me {
            while REGISTRY
                .owner
                .compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                let _ = door_sys::yield_now();
            }
        }
        unsafe { *REGISTRY.depth.get() += 1 };
        Self
    }
    /// The registry. Only the lock's owner thread reaches it, and no
    /// reference is held across a call that may re-enter the loader.
    #[allow(clippy::mut_from_ref)]
    fn modules(&self) -> &mut [Option<Module>; MAX_MODULES] {
        unsafe { &mut *REGISTRY.modules.get() }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            *REGISTRY.depth.get() -= 1;
            if *REGISTRY.depth.get() == 0 {
                REGISTRY.owner.store(0, Ordering::Release);
            }
        }
    }
}

/// LdrLockLoaderLock: the loader lock, held until `unlock_loader`.
pub(super) fn lock_loader() {
    core::mem::forget(Lock::acquire());
}
/// LdrUnlockLoaderLock: false when the caller does not hold it.
pub(super) fn unlock_loader() -> bool {
    let me = super::threads::DoorGetCurrentThreadId();
    if REGISTRY.owner.load(Ordering::Acquire) != me || unsafe { *REGISTRY.depth.get() } == 0 {
        return false;
    }
    drop(Lock);
    true
}
fn call(operation: u32, args: [u64; 6]) -> Result<u64, door_abi::Status> {
    unsafe { door_sys::raw(operation, args) }.result()
}
fn lower(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}
/// The file-name part of a UTF-16 path.
fn file_name(path: &[u16]) -> &[u16] {
    let start = path
        .iter()
        .rposition(|&u| u == u16::from(b'\\') || u == u16::from(b'/'))
        .map_or(0, |i| i + 1);
    &path[start..]
}
fn same_name(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}

/// A module this loader mapped that holds `base`.
pub(super) fn owns(base: u64) -> bool {
    let lock = Lock::acquire();
    lock.modules().iter().flatten().any(|m| m.base == base)
}
/// The base of a loaded module named `name` (file name, any case).
pub(super) fn find(name: &[u16]) -> Option<u64> {
    let wanted = file_name(name);
    let lock = Lock::acquire();
    lock.modules()
        .iter()
        .flatten()
        .find(|m| same_name(file_name(&m.name[..m.name_len]), wanted))
        .map(|m| m.base)
}

/// LoadLibrary of the DLL file at `path` (a full path, NUL-terminated
/// UTF-16): its base.
pub(super) fn load(path: u64) -> Result<u64, u32> {
    let mut name = [0u16; NAME_UNITS];
    let mut len = 0;
    while len < NAME_UNITS {
        let unit = unsafe { ((path as *const u16).add(len)).read_unaligned() };
        if unit == 0 {
            break;
        }
        name[len] = unit;
        len += 1;
    }
    if len == 0 || len == NAME_UNITS {
        return Err(206);
    }
    let lock = Lock::acquire();
    if let Some(module) = lock
        .modules()
        .iter_mut()
        .flatten()
        .find(|m| same_name(&m.name[..m.name_len], &name[..len]))
    {
        module.references += 1;
        return Ok(module.base);
    }
    let (base, headers, length) = map_file(path)?;
    let base = register_and_bind(&lock, base, &headers, &name[..len])?;
    log_mapped(&name[..len], base, length);
    if let Err(error) = attach(&lock, base) {
        let _ = unload(&lock, base);
        return Err(error);
    }
    Ok(base)
}

/// The program the image host starts (`DOOR_IMAGE`, an executable too large
/// for Init to capture whole): laid out from its file as a DLL is, made the
/// process's executable before its imports bind (a DllMain may ask for
/// GetModuleHandle(NULL)), its implicit TLS installed and its TLS callbacks
/// run. Returns its entry point, which the caller runs as the program.
pub(super) fn load_program(path: &[u16]) -> Result<u64, u32> {
    if path.is_empty() || path.len() >= NAME_UNITS {
        return Err(206);
    }
    let mut units = [0u16; NAME_UNITS];
    units[..path.len()].copy_from_slice(path);
    let lock = Lock::acquire();
    let (base, headers, length) = map_file(units.as_ptr() as u64)?;
    if headers.is_dll() || headers.entry == 0 {
        let _ = call(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]);
        return Err(193);
    }
    super::modules::set_program(base);
    let width = core::mem::size_of::<usize>();
    super::guest::put(
        door_windows_abi::PEB_ADDRESS + super::arch::PEB_IMAGE_BASE,
        &base.to_le_bytes()[..width],
    )?;
    let base = register_and_bind(&lock, base, &headers, path)?;
    log_mapped(path, base, length);
    install_tls(&lock, base)?;
    let (resource_rva, resource_bytes) = headers.directory(2);
    // Listed for module lookups and the unwinder; no entry, so thread
    // notifications never call the program's entry point.
    super::modules::add_loaded(ModuleInit {
        base,
        entry: 0,
        image_bytes: u64::from(headers.size_of_image).next_multiple_of(4096) as u32,
        resource_rva,
        resource_bytes,
        flags: 0,
    })?;
    let module = lock
        .modules()
        .iter_mut()
        .flatten()
        .find(|m| m.base == base)
        .ok_or(6u32)?;
    // Its entry is the program's, not a DllMain.
    module.entry = 0;
    module.attached = true;
    let module = *module;
    tls_callbacks(&module, 1);
    Ok(base + u64::from(headers.entry))
}
/// Lay the image at `path` out from its file: its base, its headers as laid
/// out and the file's size. Its imports are not bound yet (binding may load
/// other DLLs, which reads their files).
fn map_file(path: u64) -> Result<(u64, Headers, usize), u32> {
    super::files::read_file_with(path, MAX_FILE_BYTES as u64, |size, read| {
        let mut first = [0u8; 4096];
        let n = size.min(first.len() as u64) as usize;
        read(0, &mut first[..n])?;
        let headers = image::parse_headers(&first[..n]).map_err(|_| 193u32)?;
        let (base, headers) = map_image(size, headers, &mut |offset, out| {
            read(offset, out).map_err(|_| image::Error::Bounds)
        })?;
        Ok((base, headers, size as usize))
    })
}
/// Reads `out.len()` bytes of the image's file from an offset into `out`.
type Reader<'a> = &'a mut dyn FnMut(u64, &mut [u8]) -> Result<(), image::Error>;
/// Reserve, lay out from the file (`read`) and relocate.
fn map_image(size: u64, headers: Headers, read: Reader<'_>) -> Result<(u64, Headers), u32> {
    let machine = if cfg!(target_arch = "x86_64") {
        0x8664
    } else {
        0x14c
    };
    // Another machine maps only as an AnyCPU assembly in a 64-bit process
    // (checked once laid out), as Windows maps IL-only x86 images.
    let foreign = headers.machine != machine;
    if foreign && (machine != 0x8664 || headers.machine != 0x14c) {
        return Err(193);
    }
    let file_bytes = size;
    let size = u64::from(headers.size_of_image).next_multiple_of(4096);
    let image_flags = vm::RESERVE_IMAGE;
    let base = match call(
        op::VM_RESERVE,
        [headers.image_base, size, image_flags, 0x80, 0, 0],
    ) {
        Ok(base) => base,
        Err(_) => {
            // Its preferred range is taken: anywhere (high, as Windows
            // places relocated DLLs), then relocated.
            let top = if cfg!(target_arch = "x86_64") {
                vm::RESERVE_TOP_DOWN
            } else {
                0
            };
            call(op::VM_RESERVE, [0, size, image_flags | top, 0x80, 0, 0]).map_err(|_| 8u32)?
        }
    };
    let release = || {
        let _ = call(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]);
    };
    if call(op::VM_COMMIT, [base, size, 0x04, 0, 0, 0]).is_err() {
        release();
        return Err(8);
    }
    // SAFETY: the whole reservation was just committed read-write.
    let memory = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, size as usize) };
    let laid_out = image::map_from(file_bytes, &headers, memory, read)
        .and_then(|()| {
            if !foreign {
                Ok(headers)
            } else if image::is_il_only(memory, &headers) {
                // PE32+ headers, no imports or entry: nothing native runs.
                image::convert_il_only(memory, &headers)
            } else {
                Err(image::Error::Unsupported)
            }
        })
        .and_then(|headers| image::relocate(memory, &headers, base).map(|()| headers))
        .map_err(|_| 193u32);
    match laid_out {
        Ok(headers) => Ok((base, headers)),
        Err(error) => {
            release();
            Err(error)
        }
    }
}
/// Register a laid-out image under `name`, bind its imports and set its
/// section protections; on failure its reservation is released.
fn register_and_bind(lock: &Lock, base: u64, headers: &Headers, name: &[u16]) -> Result<u64, u32> {
    let headers = *headers;
    let size = u64::from(headers.size_of_image).next_multiple_of(4096);
    let release = || {
        let _ = call(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]);
    };
    // SAFETY: `map_image` laid the image out in this committed reservation.
    let memory = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, size as usize) };
    let tls = image::tls(memory, &headers).ok().flatten();
    let slot = lock.modules().iter().position(Option::is_none);
    let Some(slot) = slot else {
        release();
        return Err(8);
    };
    let mut record = Module {
        base,
        size,
        entry: if headers.entry != 0 {
            base + u64::from(headers.entry)
        } else {
            0
        },
        references: 1,
        attached: false,
        name: [0; NAME_UNITS],
        name_len: name.len(),
        tls_index: None,
        tls,
    };
    record.name[..name.len()].copy_from_slice(name);
    // Visible before its imports bind, so an import cycle finds it.
    lock.modules()[slot] = Some(record);
    let bound = image::bind_imports(memory, &headers, |dll, symbol| resolve(lock, dll, symbol));
    if bound.is_err() {
        lock.modules()[slot] = None;
        release();
        return Err(match bound {
            Err(image::BindError::Resolve(code)) => code,
            _ => 193,
        });
    }
    protect(base, &headers);
    Ok(base)
}
/// Section protections; the headers are read-only.
fn protect(base: u64, headers: &Headers) {
    let mut old = 0u32;
    let header_bytes = u64::from(headers.size_of_headers.max(1)).next_multiple_of(4096);
    let _ = call(op::VM_WINDOWS_PROTECT, [base, header_bytes, 0x02, 0, 0, 0]);
    for section in headers.sections() {
        let bytes = u64::from(section.mapped_size()).next_multiple_of(4096);
        if bytes == 0 {
            continue;
        }
        let _ = call(
            op::VM_WINDOWS_PROTECT,
            [
                base + u64::from(section.rva),
                bytes,
                u64::from(section.protection()),
                0,
                0,
                0,
            ],
        );
    }
    let _ = &mut old;
}

/// The address of `symbol` in the DLL named `dll` (ASCII), loading the DLL
/// when no module has that name.
fn resolve(lock: &Lock, dll: &[u8], symbol: Import<'_>) -> Result<u64, u32> {
    let text = core::str::from_utf8(dll).map_err(|_| 126u32)?;
    let host = door_pe_loader::api_set_host(text).unwrap_or(text);
    let mut wide = [0u16; NAME_UNITS];
    let mut n = 0;
    for byte in host.bytes() {
        if n + 5 >= NAME_UNITS {
            return Err(126);
        }
        wide[n] = u16::from(byte);
        n += 1;
    }
    let has_extension = host.contains('.');
    if !has_extension {
        for byte in b".dll" {
            wide[n] = u16::from(*byte);
            n += 1;
        }
    }
    let base = module_base(lock, &wide[..n])?;
    symbol_address(lock, base, symbol, 0)
}
/// The base of the module named `name`: loaded (by this loader or the
/// kernel), or loaded now through LoadLibrary's search.
fn module_base(lock: &Lock, name: &[u16]) -> Result<u64, u32> {
    if let Some(module) = lock
        .modules()
        .iter()
        .flatten()
        .find(|m| same_name(file_name(&m.name[..m.name_len]), name))
    {
        return Ok(module.base);
    }
    let mut terminated = [0u16; NAME_UNITS + 1];
    terminated[..name.len()].copy_from_slice(name);
    let pointer = terminated.as_ptr() as u64;
    let known = super::loader::DoorGetModuleHandleW(pointer);
    if known != 0 {
        return Ok(known);
    }
    let loaded = super::loader::DoorLoadLibraryExW(pointer, 0);
    if loaded == 0 {
        return Err(126);
    }
    Ok(loaded)
}
/// `symbol` in the module at `base`: this loader's modules through their
/// export directory (forwarders followed, at most 8 deep), the kernel's
/// through GetProcAddress.
fn symbol_address(lock: &Lock, base: u64, symbol: Import<'_>, depth: u32) -> Result<u64, u32> {
    if depth > 8 {
        return Err(127);
    }
    let Some(module) = lock
        .modules()
        .iter()
        .flatten()
        .find(|m| m.base == base)
        .copied()
    else {
        let mut name = [0u8; 513];
        let pointer = match symbol {
            Import::Ordinal(ordinal) => u64::from(ordinal),
            Import::Name { name: text, .. } => {
                let n = text.len().min(512);
                name[..n].copy_from_slice(&text[..n]);
                name.as_ptr() as u64
            }
        };
        let address = super::loader::DoorGetProcAddress(base, pointer);
        return if address == 0 { Err(127) } else { Ok(address) };
    };
    // SAFETY: the module's image stays mapped while it is registered.
    let memory =
        unsafe { core::slice::from_raw_parts(module.base as *const u8, module.size as usize) };
    let headers = image::parse_headers(memory).map_err(|_| 127u32)?;
    match image::export(memory, &headers, symbol).map_err(|_| 127u32)? {
        Some(Export::Rva(rva)) => Ok(module.base + u64::from(rva)),
        Some(Export::Forwarder(target)) => {
            // "DLL.Symbol" or "DLL.#ordinal".
            let dot = target.iter().rposition(|b| *b == b'.').ok_or(127u32)?;
            let (dll, rest) = (&target[..dot], &target[dot + 1..]);
            let mut wide = [0u16; NAME_UNITS];
            let mut n = 0;
            for byte in dll.iter().chain(b".dll") {
                if n == NAME_UNITS {
                    return Err(127);
                }
                wide[n] = u16::from(*byte);
                n += 1;
            }
            let target_base = module_base(lock, &wide[..n])?;
            let forwarded = if let Some(number) = rest.strip_prefix(b"#") {
                let text = core::str::from_utf8(number).map_err(|_| 127u32)?;
                Import::Ordinal(text.parse().map_err(|_| 127u32)?)
            } else {
                Import::Name {
                    hint: 0,
                    name: rest,
                }
            };
            symbol_address(lock, target_base, forwarded, depth + 1)
        }
        None => Err(127),
    }
}

// ---- TLS, attach, detach -----------------------------------------------------------

/// Every thread's TEB, for TLS blocks of DLLs loaded after it started.
pub(super) const MAX_THREADS: usize = 256;
struct Tebs(UnsafeCell<[u64; MAX_THREADS]>);
unsafe impl Sync for Tebs {}
static TEBS: Tebs = Tebs(UnsafeCell::new([0; MAX_THREADS]));
fn tls_vector(teb: u64) -> u64 {
    // TEB.ThreadLocalStoragePointer (TEB32+0x2C on x86).
    if cfg!(target_arch = "x86_64") {
        unsafe { ((teb + 0x58) as *const u64).read_volatile() }
    } else {
        u64::from(unsafe {
            ((teb + door_windows_abi::x86::TEB_TLS_POINTER as u64) as *const u32).read_volatile()
        })
    }
}
/// The thread's TLS vector, allocated (zeroed, from the process heap) when
/// the process started without static TLS (an image-hosted program, or a
/// first DLL with TLS loaded later) and published in the TEB.
fn vector_for(teb: u64) -> u64 {
    let vector = tls_vector(teb);
    if vector != 0 || teb == 0 {
        return vector;
    }
    let heap = super::heap::DoorGetProcessHeap();
    let bytes = TLS_VECTOR_ENTRIES * ENTRY_BYTES as usize;
    let vector = super::heap::RtlAllocateHeap(door_windows_abi::narrow(heap), 8, bytes) as u64;
    if vector != 0 {
        if cfg!(target_arch = "x86_64") {
            unsafe { ((teb + 0x58) as *mut u64).write_volatile(vector) };
        } else {
            unsafe {
                ((teb + door_windows_abi::x86::TEB_TLS_POINTER as u64) as *mut u32)
                    .write_volatile(vector as u32)
            };
        }
    }
    vector
}
fn set_entry(vector: u64, index: usize, block: u64) {
    let at = vector + index as u64 * ENTRY_BYTES;
    if cfg!(target_arch = "x86_64") {
        unsafe { (at as *mut u64).write_volatile(block) };
    } else {
        unsafe { (at as *mut u32).write_volatile(block as u32) };
    }
}
/// A block for `tls` (the template, then zero fill) from the process heap.
fn tls_block(tls: &image::Tls) -> u64 {
    let template = tls.end.saturating_sub(tls.start) as usize;
    let bytes = template + tls.zero_fill as usize;
    let heap = super::heap::DoorGetProcessHeap();
    let block = super::heap::RtlAllocateHeap(door_windows_abi::narrow(heap), 8, bytes.max(8));
    if block != 0 && template != 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(tls.start as *const u8, block as *mut u8, template)
        };
    }
    block as u64
}
/// A new thread starts: blocks for every DLL this loader gave a TLS slot.
pub(super) fn thread_started() {
    let teb = super::arch::teb();
    let lock = Lock::acquire();
    let tebs = unsafe { &mut *TEBS.0.get() };
    if let Some(slot) = tebs.iter_mut().find(|t| **t == 0) {
        *slot = teb;
    }
    if !lock
        .modules()
        .iter()
        .flatten()
        .any(|m| m.tls_index.is_some())
    {
        return;
    }
    let vector = vector_for(teb);
    if vector == 0 {
        return;
    }
    for module in lock.modules().iter().flatten() {
        if let (Some(index), Some(tls)) = (module.tls_index, module.tls) {
            let block = tls_block(&tls);
            set_entry(vector, index, block);
        }
    }
}
/// The thread exits: its record goes (blocks stay with the heap).
pub(super) fn thread_exited() {
    let teb = super::arch::teb();
    let _lock = Lock::acquire();
    let tebs = unsafe { &mut *TEBS.0.get() };
    if let Some(slot) = tebs.iter_mut().find(|t| **t == teb) {
        *slot = 0;
    }
}
/// Give the module at `base` a TLS slot and every known thread a block.
fn install_tls(lock: &Lock, base: u64) -> Result<(), u32> {
    let modules = lock.modules();
    let Some(position) = modules
        .iter()
        .position(|m| m.is_some_and(|m| m.base == base))
    else {
        return Err(6);
    };
    let Some(tls) = modules[position].and_then(|m| m.tls) else {
        return Ok(());
    };
    let used = |i: usize| modules.iter().flatten().any(|m| m.tls_index == Some(i));
    let index = (FIRST_TLS_INDEX..TLS_VECTOR_ENTRIES)
        .find(|i| !used(*i))
        .ok_or(8u32)?;
    if tls.index != 0 {
        unsafe { (tls.index as *mut u32).write_volatile(index as u32) };
    }
    if let Some(module) = modules[position].as_mut() {
        module.tls_index = Some(index);
    }
    let current = super::arch::teb();
    let tebs = unsafe { &mut *TEBS.0.get() };
    if !tebs.contains(&current)
        && let Some(slot) = tebs.iter_mut().find(|t| **t == 0)
    {
        *slot = current;
    }
    for &teb in tebs.iter().filter(|t| **t != 0) {
        let vector = vector_for(teb);
        if vector != 0 {
            let block = tls_block(&tls);
            set_entry(vector, index, block);
        }
    }
    Ok(())
}
/// TLS callbacks of the module (the null-terminated pointer array).
fn tls_callbacks(module: &Module, reason: u32) {
    let Some(tls) = module.tls else {
        return;
    };
    if tls.callbacks == 0 {
        return;
    }
    let width = core::mem::size_of::<usize>() as u64;
    for i in 0..64 {
        let callback = unsafe { ((tls.callbacks + i * width) as *const usize).read_volatile() };
        if callback == 0 {
            break;
        }
        super::arch::dll_main(callback as u64, module.base, reason, 0);
    }
}
/// TLS, TLS callbacks and DllMain(DLL_PROCESS_ATTACH); the module joins the
/// list that gets thread notifications and that the unwinder searches.
fn attach(lock: &Lock, base: u64) -> Result<(), u32> {
    install_tls(lock, base)?;
    let module = lock
        .modules()
        .iter()
        .flatten()
        .find(|m| m.base == base)
        .copied()
        .ok_or(6u32)?;
    super::modules::add_loaded(ModuleInit {
        base,
        entry: module.entry,
        image_bytes: module.size as u32,
        resource_rva: 0,
        resource_bytes: 0,
        flags: 0,
    })?;
    tls_callbacks(&module, 1);
    if module.entry != 0 && super::arch::dll_main(module.entry, base, 1, 0) == 0 {
        return Err(1114);
    }
    if let Some(m) = lock.modules().iter_mut().flatten().find(|m| m.base == base) {
        m.attached = true;
    }
    Ok(())
}
/// Detach (when attached) and unmap the module at `base`.
fn unload(lock: &Lock, base: u64) -> Result<(), u32> {
    let modules = lock.modules();
    let position = modules
        .iter()
        .position(|m| m.is_some_and(|m| m.base == base))
        .ok_or(6u32)?;
    let module = modules[position].expect("located module");
    if module.attached {
        if module.entry != 0 {
            super::arch::dll_main(module.entry, base, 0, 0);
        }
        tls_callbacks(&module, 0);
    }
    super::modules::remove_loaded(base);
    #[cfg(target_arch = "x86_64")]
    super::seh64::forget(base);
    lock.modules()[position] = None;
    call(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]).map_err(|_| 6u32)?;
    Ok(())
}
/// FreeLibrary of a module this loader mapped: Some(result), or None when
/// `base` is not one of them.
pub(super) fn free(base: u64) -> Option<Result<(), u32>> {
    let lock = Lock::acquire();
    let module = lock
        .modules()
        .iter_mut()
        .flatten()
        .find(|m| m.base == base)?;
    if module.references > 1 {
        module.references -= 1;
        return Some(Ok(()));
    }
    Some(unload(&lock, base))
}
/// GetProcAddress in a module this loader mapped: Some(address or 0), or
/// None when `base` is not one of them.
pub(super) fn proc_address(base: u64, symbol: Import<'_>) -> Option<u64> {
    let lock = Lock::acquire();
    lock.modules().iter().flatten().find(|m| m.base == base)?;
    Some(symbol_address(&lock, base, symbol, 0).unwrap_or(0))
}
/// GetModuleFileName of a module this loader mapped.
pub(super) fn file_name_of(base: u64, out: &mut [u16]) -> Option<usize> {
    let lock = Lock::acquire();
    let module = lock.modules().iter().flatten().find(|m| m.base == base)?;
    let n = module.name_len.min(out.len());
    out[..n].copy_from_slice(&module.name[..n]);
    Some(n)
}

/// `DOOR_LDR <file> base=0x... bytes=...` on the serial log (within the
/// process's diagnostic budget): where each DLL this loader mapped lives,
/// so a fault or a sampled instruction pointer names its module.
fn log_mapped(path: &[u16], base: u64, bytes: usize) {
    let start = path
        .iter()
        .rposition(|&u| u == 92 || u == 47)
        .map_or(0, |i| i + 1);
    let mut line = [0u8; 160];
    let mut n = 0;
    let mut push = |b: u8| {
        if n < line.len() {
            line[n] = b;
            n += 1;
        }
    };
    for &b in b"DOOR_LDR " {
        push(b);
    }
    for &u in &path[start..] {
        push(if (0x20..0x7f).contains(&u) {
            u as u8
        } else {
            b'?'
        });
    }
    for &b in b" base=0x" {
        push(b);
    }
    for shift in (0..16).rev() {
        push(b"0123456789abcdef"[((base >> (shift * 4)) & 15) as usize]);
    }
    for &b in b" bytes=" {
        push(b);
    }
    let mut digits = [0u8; 20];
    let mut d = digits.len();
    let mut v = bytes;
    loop {
        d -= 1;
        digits[d] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for &b in &digits[d..] {
        push(b);
    }
    push(b'\n');
    let _ = door_sys::write(&line[..n]);
}
