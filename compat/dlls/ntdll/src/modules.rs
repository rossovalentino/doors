//! Process module inventory captured from the trusted startup record, and the
//! lookups derived from it: module names and handles, `GetModuleFileNameW`
//! and resources. The table is written once by the bootstrap before any DLL
//! or application code runs, then only read.
use super::guest::{RtlSetLastWin32Error, copy};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_pe_loader::resources::{self, Directory, Id, Reader};
use door_windows_abi::{
    bootstrap::{self, ModuleInit},
    error, modules, status,
};

pub(super) const MAX_IMAGES: usize = bootstrap::MAX_MODULES + 1;

pub(super) struct Inventory {
    /// The executable first, then DLLs in dependency initialization order.
    pub images: [ModuleInit; MAX_IMAGES],
    pub count: usize,
    pub subsystem: u16,
    /// DOORWIN2: extents and resource directories are known.
    pub described: bool,
}
struct Table {
    ready: AtomicBool,
    value: UnsafeCell<Inventory>,
}
// Written once before `ready` is published (Release), then read-only.
unsafe impl Sync for Table {}
static TABLE: Table = Table {
    ready: AtomicBool::new(false),
    value: UnsafeCell::new(Inventory {
        images: [ModuleInit {
            base: 0,
            entry: 0,
            image_bytes: 0,
            resource_rva: 0,
            resource_bytes: 0,
            flags: 0,
        }; MAX_IMAGES],
        count: 0,
        subsystem: 0,
        described: false,
    }),
};

pub(super) fn publish(inventory: Inventory) -> Result<(), u32> {
    if TABLE.ready.load(Ordering::Acquire) || inventory.count == 0 {
        return Err(status::INVALID_PARAMETER);
    }
    // SAFETY: single bootstrap thread, before any reader can run.
    unsafe { *TABLE.value.get() = inventory };
    TABLE.ready.store(true, Ordering::Release);
    Ok(())
}
pub(super) fn inventory() -> Option<&'static Inventory> {
    TABLE
        .ready
        .load(Ordering::Acquire)
        // SAFETY: immutable after publication.
        .then(|| unsafe { &*TABLE.value.get() })
}
pub(super) fn executable_base() -> u64 {
    let program = PROGRAM.load(Ordering::Acquire);
    if program != 0 {
        return program;
    }
    inventory().map_or(modules::own::EXECUTABLE_BASE, |table| table.images[0].base)
}
/// A program Init started through the image host (`DOOR_IMAGE`: too large
/// to be captured whole), which ntdll's loader laid out from its file: the
/// process's executable from then on (GetModuleHandle(NULL), resources,
/// stack sizes, PEB.ImageBaseAddress).
static PROGRAM: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub(super) fn set_program(base: u64) {
    PROGRAM.store(base, Ordering::Release);
}
/// DLL records in initialization order (excluding the executable).
pub(super) fn dlls() -> &'static [ModuleInit] {
    inventory().map_or(&[], |table| &table.images[1..table.count])
}
pub(super) fn present(base: u64) -> Option<&'static ModuleInit> {
    let table = inventory()?;
    table.images[..table.count]
        .iter()
        .chain(loaded())
        .find(|module| module.base == base)
}

/// System DLLs LoadLibrary mapped after startup, in initialization order.
/// Entries are written before `count` publishes them and never change.
struct Loaded {
    count: core::sync::atomic::AtomicUsize,
    images: UnsafeCell<[ModuleInit; LOADED]>,
}
const LOADED: usize = 256;
// Written only under `LOADED_WRITER`; read after `count`. An unloaded entry
// keeps its slot with base 0 (P18 user-mode loader), which readers skip.
static LOADED_WRITER: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
fn with_loaded_writer<R>(f: impl FnOnce() -> R) -> R {
    while LOADED_WRITER
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        let _ = door_sys::yield_now();
    }
    let result = f();
    LOADED_WRITER.store(false, Ordering::Release);
    result
}
unsafe impl Sync for Loaded {}
static LOADED_TABLE: Loaded = Loaded {
    count: core::sync::atomic::AtomicUsize::new(0),
    images: UnsafeCell::new(
        [ModuleInit {
            base: 0,
            entry: 0,
            image_bytes: 0,
            resource_rva: 0,
            resource_bytes: 0,
            flags: 0,
        }; LOADED],
    ),
};
/// DLLs loaded after startup, in initialization order.
pub(super) fn loaded() -> &'static [ModuleInit] {
    let count = LOADED_TABLE.count.load(Ordering::Acquire);
    // SAFETY: entries below `count` are published and immutable.
    unsafe { &(&*LOADED_TABLE.images.get())[..count] }
}
/// Records a DLL mapped after startup (by the kernel or ntdll's loader).
pub(super) fn add_loaded(module: ModuleInit) -> Result<(), u32> {
    with_loaded_writer(|| {
        let count = LOADED_TABLE.count.load(Ordering::Acquire);
        if count == LOADED {
            return Err(status::NO_MEMORY);
        }
        // SAFETY: slot `count` is not yet published; the writer lock
        // excludes other writers.
        unsafe { (*LOADED_TABLE.images.get())[count] = module };
        LOADED_TABLE.count.store(count + 1, Ordering::Release);
        Ok(())
    })
}
/// Forget an unloaded DLL: its entry's base, entry and size become 0.
pub(super) fn remove_loaded(base: u64) {
    with_loaded_writer(|| {
        let count = LOADED_TABLE.count.load(Ordering::Acquire);
        // SAFETY: the writer lock excludes other writers; readers skip a
        // zero base.
        let images = unsafe { &mut *LOADED_TABLE.images.get() };
        if let Some(entry) = images[..count].iter_mut().find(|m| m.base == base) {
            *entry = ModuleInit::default();
        }
    });
}
pub(super) fn subsystem() -> u16 {
    inventory().map_or(0, |table| table.subsystem)
}

/// A system module of this process by (case-insensitive) file name or stem.
pub(super) fn system_module(name: &str) -> Option<u64> {
    let host = door_pe_loader::api_set_host(name).unwrap_or(name);
    let (_, provider) = modules::own::by_name(host)?;
    present(provider.base).map(|module| module.base)
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorGetProcessSubsystem() -> u32 {
    u32::from(subsystem())
}

/// `GetModuleFileNameW`: the image path for the executable, the system
/// directory spelling for Door's DLLs. Truncation follows Windows: the
/// buffer is filled and terminated, ERROR_INSUFFICIENT_BUFFER is set and the
/// capacity is returned.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32 {
    let result = (|| {
        let module = if module == 0 {
            executable_base()
        } else {
            module
        };
        present(module).ok_or(error::MOD_NOT_FOUND)?;
        // Long paths too (the loader's application-folder search uses it).
        let mut name = [0u16; 1024];
        let length = if module == executable_base() {
            // RTL_USER_PROCESS_PARAMETERS.ImagePathName (length, buffer).
            let mut descriptor = [0u8; 16];
            copy(
                descriptor.as_mut_ptr() as u64,
                door_windows_abi::PARAMETERS_ADDRESS + 96,
                16,
            )?;
            let bytes = usize::from(u16::from_le_bytes([descriptor[0], descriptor[1]]));
            let pointer = u64::from_le_bytes(descriptor[8..16].try_into().unwrap());
            let units = (bytes / 2).min(name.len());
            if units != 0 {
                copy(name.as_mut_ptr() as u64, pointer, units * 2)?;
            }
            units
        } else if let Some(units) = super::ldr::file_name_of(module, &mut name) {
            // A DLL ntdll's loader mapped: the path it was loaded from.
            units
        } else {
            let (_, provider) = modules::own::by_base(module).ok_or(error::MOD_NOT_FOUND)?;
            let mut units = 0;
            for unit in modules::own::SYSTEM_DIRECTORY
                .encode_utf16()
                .chain(provider.name.encode_utf16())
            {
                name[units] = unit;
                units += 1;
            }
            units
        };
        if capacity == 0 {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        let capacity = capacity as usize;
        let stored = length.min(capacity - 1);
        let mut out = [0u16; 1025];
        out[..stored].copy_from_slice(&name[..stored]);
        copy(buffer, out.as_ptr() as u64, (stored + 1) * 2)?;
        if length >= capacity {
            RtlSetLastWin32Error(error::INSUFFICIENT_BUFFER);
            return Ok(capacity as u32);
        }
        RtlSetLastWin32Error(0);
        Ok(length as u32)
    })();
    result.unwrap_or_else(|code| {
        RtlSetLastWin32Error(if code < 65536 {
            code
        } else {
            door_windows_abi::dos_error(code)
        });
        0
    })
}

/// Reads the resource directory of a mapped module through the checked-copy
/// boundary; the kernel validated the tree before the process existed.
struct Mapped {
    base: u64,
}
impl Reader for Mapped {
    fn read(&mut self, offset: u32, output: &mut [u8]) -> Result<(), resources::Error> {
        for (index, chunk) in output.chunks_mut(door_windows_abi::COPY_BYTES).enumerate() {
            let at = self.base + u64::from(offset) + (index * door_windows_abi::COPY_BYTES) as u64;
            copy(chunk.as_mut_ptr() as u64, at, chunk.len()).map_err(|_| resources::Error::Read)?;
        }
        Ok(())
    }
}
/// A module's resource directory: a loaded image's, or a data file's
/// (LoadLibraryEx).
fn directory(module: u64) -> Result<(ModuleInit, Directory<Mapped>), u32> {
    let module = if module == 0 {
        executable_base()
    } else {
        module
    };
    let image = match super::datafile::find(module) {
        Some(image) => image,
        None => *present(module).ok_or(error::RESOURCE_DATA_NOT_FOUND)?,
    };
    if image.resource_bytes == 0 {
        return Err(error::RESOURCE_DATA_NOT_FOUND);
    }
    let directory = Directory::new(
        Mapped {
            base: image.base + u64::from(image.resource_rva),
        },
        image.resource_bytes,
    )
    .map_err(|_| error::RESOURCE_DATA_NOT_FOUND)?;
    Ok((image, directory))
}
fn resource_error(value: resources::Error) -> u32 {
    match value {
        resources::Error::TypeNotFound => error::RESOURCE_TYPE_NOT_FOUND,
        resources::Error::NameNotFound => error::RESOURCE_NAME_NOT_FOUND,
        resources::Error::LanguageNotFound => error::RESOURCE_LANG_NOT_FOUND,
        resources::Error::Read => error::NOACCESS,
        _ => error::RESOURCE_DATA_NOT_FOUND,
    }
}
/// `MAKEINTRESOURCE` integer or a NUL-terminated UTF-16 name (at most 255
/// units; `#123` spells an integer, as `FindResourceW` accepts).
fn identifier<'a>(value: u64, storage: &'a mut [u16; 256]) -> Result<Id<'a>, u32> {
    if value < 0x1_0000 {
        return Ok(Id::Integer(value as u16));
    }
    let mut length = 0;
    loop {
        if length == storage.len() {
            return Err(error::INVALID_PARAMETER);
        }
        let mut unit = [0u8; 2];
        copy(unit.as_mut_ptr() as u64, value + length as u64 * 2, 2)?;
        storage[length] = u16::from_le_bytes(unit);
        if storage[length] == 0 {
            break;
        }
        length += 1;
    }
    let name = &storage[..length];
    if name.first() == Some(&35) && length > 1 {
        let mut number = 0u32;
        for &unit in &name[1..] {
            if !(48..=57).contains(&unit) {
                return Ok(Id::Name(name));
            }
            number = number * 10 + u32::from(unit - 48);
            if number > 0xffff {
                return Err(error::INVALID_PARAMETER);
            }
        }
        return Ok(Id::Integer(number as u16));
    }
    Ok(Id::Name(name))
}
fn last_error(code: u32) {
    RtlSetLastWin32Error(if code < 65536 {
        code
    } else {
        door_windows_abi::dos_error(code)
    });
}

/// `FindResourceExW` over a present module (0 = the executable). The handle
/// is the address of the data entry inside the mapped resource directory.
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64 {
    let result = (|| {
        let (image, mut directory) = directory(module)?;
        let mut kind_units = [0u16; 256];
        let mut name_units = [0u16; 256];
        let kind = identifier(kind, &mut kind_units)?;
        let name = identifier(name, &mut name_units)?;
        let entry = directory
            .find(kind, name, language)
            .map_err(resource_error)?;
        Ok(image.base + u64::from(image.resource_rva) + u64::from(entry.offset))
    })();
    result.unwrap_or_else(|code| {
        last_error(code);
        0
    })
}
/// EnumResourceNames' walk: the name at `index` under type `kind` of
/// `module`: an integer ID (below 0x10000), or `out` (a buffer of 256
/// units) holding the NUL-terminated string name; 0 past the last name.
#[unsafe(no_mangle)]
pub extern "system" fn DoorResourceName(module: u64, kind: u64, index: u32, out: u64) -> u64 {
    let result = (|| {
        let (_, mut directory) = directory(module)?;
        let mut kind_units = [0u16; 256];
        let kind = identifier(kind, &mut kind_units)?;
        let mut scratch = [0u16; resources::MAX_NAME_UNITS];
        let mut at = 0u32;
        let mut found = 0u64;
        let mut name = [0u16; 256];
        directory
            .names(kind, &mut scratch, |id| {
                if at == index {
                    found = match id {
                        Id::Integer(value) => u64::from(value),
                        Id::Name(units) => {
                            let n = units.len().min(255);
                            name[..n].copy_from_slice(&units[..n]);
                            name[n] = 0;
                            u64::MAX
                        }
                    };
                    return false;
                }
                at += 1;
                true
            })
            .map_err(resource_error)?;
        if found == u64::MAX {
            copy(out, name.as_ptr() as u64, 512)?;
            return Ok(out);
        }
        Ok(found)
    })();
    result.unwrap_or_else(|code| {
        last_error(code);
        0
    })
}
fn entry(module: u64, handle: u64) -> Result<(ModuleInit, resources::Entry), u32> {
    let (image, mut directory) = directory(module)?;
    let start = image.base + u64::from(image.resource_rva);
    let offset = handle
        .checked_sub(start)
        .and_then(|offset| u32::try_from(offset).ok())
        .ok_or(error::INVALID_HANDLE)?;
    if !directory
        .contains_data_entry(offset)
        .map_err(resource_error)?
    {
        return Err(error::INVALID_HANDLE);
    }
    let entry = directory.data_entry(offset).map_err(resource_error)?;
    // The bytes must lie inside the image.
    if u64::from(entry.data_rva) + u64::from(entry.size) > u64::from(image.image_bytes) {
        return Err(error::RESOURCE_DATA_NOT_FOUND);
    }
    Ok((image, entry))
}
/// `LoadResource`: address of the resource bytes inside the mapped image.
#[unsafe(no_mangle)]
pub extern "system" fn DoorLoadResource(module: u64, handle: u64) -> u64 {
    match entry(module, handle) {
        Ok((image, entry)) => image.base + u64::from(entry.data_rva),
        Err(code) => {
            last_error(code);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorSizeofResource(module: u64, handle: u64) -> u32 {
    match entry(module, handle) {
        Ok((_, entry)) => entry.size,
        Err(code) => {
            last_error(code);
            0
        }
    }
}

/// The process's modules as (base, size) pairs of u64 into `out` (at most
/// `capacity`): EnumProcessModules and Toolhelp's module snapshots. Returns
/// how many there are.
#[unsafe(no_mangle)]
pub extern "system" fn DoorEnumModules(out: u64, capacity: u32) -> u32 {
    // The executable first (the program the image host started, when it
    // did), then every DLL, as EnumProcessModules lists them.
    let program = PROGRAM.load(Ordering::Acquire);
    let fixed = inventory().map(|t| &t.images[..t.count]).unwrap_or(&[]);
    let mut list = [(0u64, 0u64); 256];
    let mut n = 0;
    let mut push = |base: u64, bytes: u64| {
        if base != 0 && n < list.len() && !list[..n].iter().any(|e| e.0 == base) {
            list[n] = (base, bytes);
            n += 1;
        }
    };
    if program != 0
        && let Some(m) = loaded().iter().find(|m| m.base == program)
    {
        push(m.base, u64::from(m.image_bytes));
    }
    for (i, m) in fixed.iter().enumerate() {
        if i == 0 && program != 0 {
            continue;
        }
        if m.base != 0x1_8400_0000 || super::loader::dynamic_present() {
            push(m.base, u64::from(m.image_bytes));
        }
    }
    for m in loaded() {
        push(m.base, u64::from(m.image_bytes));
    }
    for (i, (base, bytes)) in list[..n].iter().enumerate().take(capacity as usize) {
        let mut record = [0u8; 16];
        record[..8].copy_from_slice(&base.to_le_bytes());
        record[8..].copy_from_slice(&bytes.to_le_bytes());
        if copy(out + 16 * i as u64, record.as_ptr() as u64, 16).is_err() {
            return 0;
        }
    }
    n as u32
}
/// The base of the mapped image holding `address`, 0 when none
/// (GetModuleHandleEx with GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS).
#[unsafe(no_mangle)]
pub extern "system" fn DoorModuleFromAddress(address: u64) -> u64 {
    inventory()
        .map(|t| &t.images[..t.count])
        .unwrap_or(&[])
        .iter()
        .chain(loaded())
        .filter(|m| m.base != 0x1_8400_0000 || super::loader::dynamic_present())
        .find(|m| {
            m.image_bytes != 0 && address >= m.base && address < m.base + u64::from(m.image_bytes)
        })
        .map_or(0, |m| m.base)
}
