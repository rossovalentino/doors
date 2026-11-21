//! LoadLibraryEx's data-file loads (LOAD_LIBRARY_AS_DATAFILE,
//! AS_DATAFILE_EXCLUSIVE, AS_IMAGE_RESOURCE, DONT_RESOLVE_DLL_REFERENCES):
//! the file's headers and sections laid out as its image in a heap block,
//! never executed, for the resource functions (FindResource, LoadResource,
//! LoadString, LoadIcon, dialogs). The module handle is the block's
//! address with bit 0 (a data file) or bit 1 (an image resource) set, as
//! Windows marks such handles; FreeLibrary releases the block.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_windows_abi::bootstrap::ModuleInit;

const SLOTS: usize = 16;
struct Table {
    lock: AtomicBool,
    slots: UnsafeCell<[Option<ModuleInit>; SLOTS]>,
}
// The slots are read and written only with `lock` held.
unsafe impl Sync for Table {}
static TABLE: Table = Table {
    lock: AtomicBool::new(false),
    slots: UnsafeCell::new([None; SLOTS]),
};
fn with<R>(f: impl FnOnce(&mut [Option<ModuleInit>; SLOTS]) -> R) -> R {
    while TABLE
        .lock
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held.
    let result = f(unsafe { &mut *TABLE.slots.get() });
    TABLE.lock.store(false, Ordering::Release);
    result
}

/// The image of a data-file handle (`module` with its low bits).
pub(super) fn find(module: u64) -> Option<ModuleInit> {
    if module & 3 == 0 {
        return None;
    }
    let base = module & !3;
    with(|t| t.iter().flatten().find(|m| m.base == base).copied())
}
/// FreeLibrary of a data-file handle: whether it was one.
pub(super) fn free(module: u64) -> bool {
    if module & 3 == 0 {
        return false;
    }
    let base = module & !3;
    let found = with(|t| {
        t.iter_mut()
            .find(|m| m.is_some_and(|m| m.base == base))
            .map(|m| m.take())
            .is_some()
    });
    if found {
        let heap = super::heap::DoorGetProcessHeap();
        super::heap::RtlFreeHeap(
            door_windows_abi::narrow(heap),
            0,
            door_windows_abi::narrow(base),
        );
    }
    found
}

fn u16_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(
        b.get(at..at + 2)?.try_into().ok()?,
    )))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
/// A PE file's image layout, checked against the file and the image.
struct Layout {
    size_of_image: u32,
    headers: u32,
    resource_rva: u32,
    resource_bytes: u32,
    /// (virtual address, raw offset, bytes) of each section.
    sections: [(u32, u32, u32); 96],
    count: usize,
}
fn layout(file: &[u8]) -> Option<Layout> {
    if file.get(..2)? != b"MZ" {
        return None;
    }
    // Checked: a crafted e_lfanew must not overflow a 32-bit usize.
    let pe = u32_at(file, 0x3c)? as usize;
    if file.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let sections = u16_at(file, pe + 6)? as usize;
    let optional_bytes = u16_at(file, pe + 20)? as usize;
    let optional = pe + 24;
    // NumberOfRvaAndSizes, then the data directories.
    let (count_at, directories) = match u16_at(file, optional)? {
        0x10b => (optional + 92, optional + 96),
        0x20b => (optional + 108, optional + 112),
        _ => return None,
    };
    let size_of_image = u32_at(file, optional + 56)?;
    let headers = u32_at(file, optional + 60)?;
    // The resource directory: data directory 2.
    let (resource_rva, resource_bytes) = if u32_at(file, count_at)? > 2 {
        (
            u32_at(file, directories + 16)?,
            u32_at(file, directories + 20)?,
        )
    } else {
        (0, 0)
    };
    if !(1..=96).contains(&sections)
        || size_of_image == 0
        || size_of_image > 64 << 20
        || headers > size_of_image
        || headers as usize > file.len()
        || u64::from(resource_rva) + u64::from(resource_bytes) > u64::from(size_of_image)
    {
        return None;
    }
    let mut table = [(0u32, 0u32, 0u32); 96];
    let first = optional + optional_bytes;
    for (i, slot) in table.iter_mut().take(sections).enumerate() {
        let s = first + i * 40;
        let virtual_size = u32_at(file, s + 8)?;
        let address = u32_at(file, s + 12)?;
        let raw_size = u32_at(file, s + 16)?;
        let raw = u32_at(file, s + 20)?;
        let copy = if virtual_size == 0 {
            raw_size
        } else {
            raw_size.min(virtual_size)
        };
        if u64::from(address) + u64::from(copy) > u64::from(size_of_image)
            || u64::from(raw) + u64::from(copy) > file.len() as u64
        {
            return None;
        }
        *slot = (address, raw, copy);
    }
    Some(Layout {
        size_of_image,
        headers,
        resource_rva,
        resource_bytes,
        sections: table,
        count: sections,
    })
}

/// The largest file a data-file load reads (resource-only DLLs, setup
/// programs).
const DATAFILE_MAX_BYTES: usize = 64 << 20;
/// LoadLibraryEx with a data-file flag (`flags`: LOAD_LIBRARY_AS_IMAGE_
/// RESOURCE 0x20 marks the handle bit 1, else bit 0; the search flags as
/// LoadLibraryEx's): the module at `path`. A bare name of a module already
/// loaded is that module, a Doors system DLL is the module itself (Windows
/// maps System32's copy); otherwise it is searched for as LoadLibrary
/// searches.
#[unsafe(no_mangle)]
pub extern "system" fn DoorLoadLibraryDatafile(path: u64, flags: u32) -> u64 {
    let result = (|| {
        let mut named = [0u16; 1030];
        super::loader::module_file_name(path, &mut named)?;
        let pointer = named.as_ptr() as u64;
        let length = named.iter().position(|&u| u == 0).unwrap_or(0);
        let bare = !named[..length]
            .iter()
            .any(|&u| u == u16::from(b'\\') || u == u16::from(b'/') || u == u16::from(b':'));
        if bare {
            let loaded = super::loader::DoorGetModuleHandleW(pointer);
            if loaded != 0 {
                return Ok(loaded);
            }
            let mut ascii = [0u8; 64];
            let system = length <= ascii.len()
                && named[..length]
                    .iter()
                    .zip(ascii.iter_mut())
                    .all(|(&u, a)| u8::try_from(u).map(|b| *a = b).is_ok())
                && core::str::from_utf8(&ascii[..length])
                    .ok()
                    .and_then(door_windows_abi::modules::own::by_name)
                    .is_some_and(|(_, provider)| provider.system);
            if system {
                return match super::loader::DoorLoadLibraryW(pointer) {
                    0 => Err(126),
                    module => Ok(module),
                };
            }
        }
        let image = flags & 0x20 != 0;
        super::loader::search(
            pointer,
            flags & door_windows_abi::dll_search::flags::ANY,
            |p| load(p, image),
        )
    })();
    result.unwrap_or_else(|code| {
        super::guest::RtlSetLastWin32Error(code);
        0
    })
}
fn load(path: u64, image: bool) -> Result<u64, u32> {
    let (block, length) =
        super::files::whole_file(path, DATAFILE_MAX_BYTES).map_err(|e| match e {
            2 | 3 => 126,
            e => e,
        })?;
    let heap = super::heap::DoorGetProcessHeap();
    let free = |p: u64| {
        super::heap::RtlFreeHeap(
            door_windows_abi::narrow(heap),
            0,
            door_windows_abi::narrow(p),
        );
    };
    // SAFETY: the heap block whole_file filled.
    let file = unsafe { core::slice::from_raw_parts(block as *const u8, length) };
    let Some(Layout {
        size_of_image: size,
        headers,
        resource_rva,
        resource_bytes,
        sections,
        count,
    }) = layout(file)
    else {
        free(block);
        return Err(193);
    };
    let base = super::heap::RtlAllocateHeap(
        door_windows_abi::narrow(heap),
        8, // HEAP_ZERO_MEMORY
        size as usize,
    );
    if base == 0 {
        free(block);
        return Err(8);
    }
    let base = door_windows_abi::widen(base);
    // SAFETY: a fresh zeroed block of `size` bytes.
    let out = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, size as usize) };
    out[..headers as usize].copy_from_slice(&file[..headers as usize]);
    for &(address, raw, bytes) in &sections[..count] {
        let (a, r, n) = (address as usize, raw as usize, bytes as usize);
        out[a..a + n].copy_from_slice(&file[r..r + n]);
    }
    free(block);
    let module = ModuleInit {
        base,
        entry: 0,
        image_bytes: size,
        resource_rva,
        resource_bytes,
        flags: 0,
    };
    if !with(|t| match t.iter_mut().find(|m| m.is_none()) {
        Some(slot) => {
            *slot = Some(module);
            true
        }
        None => false,
    }) {
        free(base);
        return Err(8);
    }
    Ok(base | if image { 2 } else { 1 })
}
