//! File mappings (P18): CreateFileMapping, OpenFileMapping, MapViewOfFile
//! (Ex, 3, FromApp), UnmapViewOfFile (Ex, 2) and FlushViewOfFile over the
//! kernel's Windows sections, whose pages every view of a section shares.
//!
//! - Pagefile-backed sections (SEC_COMMIT, SEC_RESERVE) are kernel
//!   sections as they are: CoreCLR's double-mapped code heap writes through
//!   one view and executes through another, Chromium's processes share
//!   memory through them.
//! - A file-backed section is filled from the file when it is created, a
//!   storage request per 256 KiB (READ_SECTION). A shared writable view of
//!   it writes back to the file on FlushViewOfFile and UnmapViewOfFile
//!   (the file stays open for that past CloseHandle); FILE_MAP_COPY views
//!   copy on write in the kernel.
//! - SEC_IMAGE lays the file out as the loader would (headers, then each
//!   section at its RVA); a view goes at the image's preferred base when
//!   that range is free, is relocated (copy on write) elsewhere, and gets
//!   each section's protection, as on Windows.
//!
//! Names are process-local: a second CreateFileMapping or OpenFileMapping of
//! a name in the same process gets the same section; across processes a
//! section travels as a handle (DuplicateHandle, inheritance).
use super::guest::{RtlSetLastWin32Error, copy};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_abi::{Status, operation as op};
use door_pe_loader::image;
use door_windows_abi::memory::{self as vm, page, section};

const MAX_RECORDS: usize = 256;
const MAX_VIEWS: usize = 1024;
const NAME_UNITS: usize = 260;
const PAGE_BYTES: u64 = 4096;
const GRANULARITY: u64 = 0x1_0000;

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_INVALID_ADDRESS: u32 = 487;
const ERROR_FILE_INVALID: u32 = 1006;
const ERROR_MAPPED_ALIGNMENT: u32 = 1132;
const ERROR_BAD_EXE_FORMAT: u32 = 193;
const ERROR_PRIVILEGE_NOT_HELD: u32 = 1314;
const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
const SEC_WRITECOMBINE: u32 = 0x4000_0000;

/// One section this process created or opened by name.
#[derive(Clone, Copy)]
struct Record {
    /// The kernel object (HandleInfo identity): every handle to it.
    object: u64,
    /// ntdll's own handle: names resolve through it, write-back uses it.
    held: u64,
    bytes: u64,
    /// SEC_IMAGE: the headers' preferred base (0 otherwise).
    image_base: u64,
    /// The file a shared writable view writes back to (pinned), or 0.
    file: u64,
    views: u32,
    name: [u16; NAME_UNITS],
    name_len: usize,
}
#[derive(Clone, Copy)]
struct View {
    base: u64,
    bytes: u64,
    /// The record's index, or None for a section without one.
    record: Option<usize>,
    /// The section byte the view starts at.
    offset: u64,
    /// A shared writable view of a file-backed section.
    write_back: bool,
}
struct Tables {
    records: [Option<Record>; MAX_RECORDS],
    views: [Option<View>; MAX_VIEWS],
}
struct State {
    busy: AtomicBool,
    tables: UnsafeCell<Tables>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    busy: AtomicBool::new(false),
    tables: UnsafeCell::new(Tables {
        records: [None; MAX_RECORDS],
        views: [None; MAX_VIEWS],
    }),
};
/// Whether any record was ever made (CloseHandle's fast path).
static RECORDED: AtomicBool = AtomicBool::new(false);
/// The tables, one thread at a time (never held across a storage request).
struct Guard;
impl Guard {
    fn acquire() -> Self {
        while STATE
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let _ = door_sys::yield_now();
        }
        Self
    }
    #[allow(clippy::mut_from_ref)]
    /// The tables; only the guard's holder reaches them.
    fn tables(&self) -> &mut Tables {
        // SAFETY: `busy` admits one holder at a time.
        unsafe { &mut *STATE.tables.get() }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        STATE.busy.store(false, Ordering::Release);
    }
}

fn call(operation: u32, args: [u64; 6]) -> Result<u64, Status> {
    unsafe { door_sys::raw(operation, args) }.result()
}
fn status_error(status: Status) -> u32 {
    match status {
        Status::ResourceExhausted => ERROR_NOT_ENOUGH_MEMORY,
        Status::AccessDenied => ERROR_ACCESS_DENIED,
        Status::BadHandle => ERROR_INVALID_HANDLE,
        Status::AlreadyExists | Status::BadAddress => ERROR_INVALID_ADDRESS,
        Status::NotFound => ERROR_FILE_NOT_FOUND,
        _ => ERROR_INVALID_PARAMETER,
    }
}
fn fail(error: u32) -> u64 {
    RtlSetLastWin32Error(error);
    0
}
fn object_of(handle: u64) -> Result<u64, u32> {
    let info = door_sys::inspect(handle).map_err(|_| ERROR_INVALID_HANDLE)?;
    if info[0] != door_abi::HandleKind::Section as u64 {
        return Err(ERROR_INVALID_HANDLE);
    }
    Ok(info[2])
}
/// A name at `pointer` (UTF-16, NUL-terminated), without a `Local\` or
/// `Global\` prefix; length 0 for none.
fn read_name(pointer: u64, out: &mut [u16; NAME_UNITS]) -> Result<usize, u32> {
    if pointer == 0 {
        return Ok(0);
    }
    let mut length = 0;
    loop {
        if length == NAME_UNITS {
            return Err(ERROR_FILENAME_EXCED_RANGE);
        }
        let mut unit = 0u16;
        copy(&mut unit as *mut u16 as u64, pointer + 2 * length as u64, 2).map_err(|_| 998u32)?;
        if unit == 0 {
            break;
        }
        out[length] = unit;
        length += 1;
    }
    for prefix in ["Local\\", "Global\\", "Session\\"] {
        let p: [u16; 8] =
            core::array::from_fn(|i| prefix.as_bytes().get(i).map_or(0, |b| *b as u16));
        let n = prefix.len();
        if length > n && out[..n].iter().zip(&p[..n]).all(|(a, b)| a == b) {
            out.copy_within(n..length, 0);
            length -= n;
            break;
        }
    }
    Ok(length)
}
fn same_name(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}
/// A duplicate of the named section's held handle, if the name exists.
fn open_named(name: &[u16], access: u64) -> Option<Result<u64, u32>> {
    let guard = Guard::acquire();
    let record = guard
        .tables()
        .records
        .iter()
        .flatten()
        .find(|r| r.name_len != 0 && same_name(&r.name[..r.name_len], name))?;
    Some(door_sys::duplicate(record.held, access).map_err(status_error))
}

/// CreateFileMappingW (`file` 0 or INVALID_HANDLE_VALUE: the pagefile;
/// `maximum` 0: the file's size; `name` a UTF-16 string or 0). The
/// handle, or 0 with the last error; an existing name returns its section
/// with ERROR_ALREADY_EXISTS.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateFileMapping(
    file: u64,
    protect: u32,
    maximum: u64,
    name: u64,
) -> u64 {
    match create(file, protect, maximum, name) {
        Ok((handle, existed)) => {
            RtlSetLastWin32Error(if existed { ERROR_ALREADY_EXISTS } else { 0 });
            handle
        }
        Err(error) => fail(error),
    }
}
fn create(file: u64, protect: u32, maximum: u64, name: u64) -> Result<(u64, bool), u32> {
    let base = protect & 0xFF;
    let attributes = protect & !0xFF;
    if !matches!(
        base,
        page::READONLY
            | page::READWRITE
            | page::WRITECOPY
            | page::EXECUTE_READ
            | page::EXECUTE_READWRITE
            | page::EXECUTE_WRITECOPY
    ) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if attributes & section::SEC_LARGE_PAGES != 0 {
        return Err(ERROR_PRIVILEGE_NOT_HELD);
    }
    // SEC_NOCACHE and SEC_WRITECOMBINE change nothing for RAM here.
    let known = section::SEC_FILE
        | section::SEC_IMAGE
        | section::SEC_RESERVE
        | section::SEC_COMMIT
        | section::SEC_NOCACHE
        | SEC_WRITECOMBINE;
    if attributes & !known != 0
        || attributes & section::SEC_RESERVE != 0 && attributes & section::SEC_COMMIT != 0
    {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut units = [0u16; NAME_UNITS];
    let name_len = read_name(name, &mut units)?;
    if name_len != 0
        && let Some(existing) = open_named(&units[..name_len], files_rights())
    {
        return existing.map(|handle| (handle, true));
    }
    let image = attributes & section::SEC_IMAGE != 0;
    let reserve = attributes & section::SEC_RESERVE != 0;
    let pagefile = file == 0 || file == u64::MAX || file == u64::from(u32::MAX);
    let (bytes, image_base, write_back, fill) = if pagefile {
        if image || maximum == 0 {
            return Err(ERROR_INVALID_PARAMETER);
        }
        (maximum, 0, false, None)
    } else {
        if !door_windows_abi::file::is_file(file) {
            return Err(ERROR_INVALID_HANDLE);
        }
        let (size, access) = super::files::mapping_file(file)?;
        if access & door_service_client::READ == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        let shared_write = matches!(base, page::READWRITE | page::EXECUTE_READWRITE);
        if shared_write && access & door_service_client::WRITE == 0 {
            return Err(ERROR_ACCESS_DENIED);
        }
        if image {
            let mut headers = [0u8; 4096];
            let n = super::files::mapping_peek(file, 0, &mut headers)?;
            let parsed = image::parse_headers(&headers[..n]).map_err(|_| ERROR_BAD_EXE_FORMAT)?;
            (
                u64::from(parsed.size_of_image),
                parsed.image_base,
                false,
                Some((size, Some(parsed))),
            )
        } else {
            if size == 0 && maximum == 0 {
                return Err(ERROR_FILE_INVALID);
            }
            let wanted = if maximum == 0 { size } else { maximum };
            if wanted > size {
                if !shared_write {
                    return Err(ERROR_NOT_ENOUGH_MEMORY);
                }
                super::files::mapping_extend(file, wanted)?;
            }
            (wanted, 0, shared_write, Some((size.min(wanted), None)))
        }
    };
    let section_bytes = bytes
        .checked_next_multiple_of(PAGE_BYTES)
        .ok_or(ERROR_NOT_ENOUGH_MEMORY)?;
    // The kernel section: an image's pages are filled as the loader lays
    // them out, and its views copy on write.
    let kernel_protect = if image { page::EXECUTE_WRITECOPY } else { base };
    let flags = if reserve && pagefile {
        section::RESERVE
    } else {
        0
    };
    let handle = call(
        op::VM_SECTION_CREATE,
        [
            section_bytes,
            flags,
            u64::from(kernel_protect),
            super::files::SECTION_RIGHTS,
            0,
            0,
        ],
    )
    .map_err(status_error)?;
    let filled = match fill {
        None => Ok(()),
        Some((size, None)) => super::files::mapping_read(file, 0, handle, 0, size).map(|_| ()),
        Some((size, Some(headers))) => fill_image(file, handle, size, &headers),
    };
    if let Err(error) = filled {
        let _ = door_sys::close(handle);
        return Err(error);
    }
    if write_back && let Err(error) = super::files::mapping_pin(file) {
        let _ = door_sys::close(handle);
        return Err(error);
    }
    // A plain pagefile section needs no record. Otherwise ntdll keeps its
    // own handle: names resolve through it, write-back and image views use
    // it while views remain.
    if name_len == 0 && !write_back && !image {
        return Ok((handle, false));
    }
    let record = (|| {
        let object = object_of(handle)?;
        let held =
            door_sys::duplicate(handle, super::files::SECTION_RIGHTS).map_err(status_error)?;
        let guard = Guard::acquire();
        let tables = guard.tables();
        let slot = tables
            .records
            .iter()
            .position(Option::is_none)
            .ok_or(ERROR_NOT_ENOUGH_MEMORY);
        let slot = match slot {
            Ok(slot) => slot,
            Err(error) => {
                let _ = door_sys::close(held);
                return Err(error);
            }
        };
        RECORDED.store(true, Ordering::Release);
        tables.records[slot] = Some(Record {
            object,
            held,
            bytes,
            image_base: if image { image_base } else { 0 },
            file: if write_back { file } else { 0 },
            views: 0,
            name: units,
            name_len,
        });
        Ok(())
    })();
    if let Err(error) = record {
        if write_back {
            super::files::mapping_unpin(file);
        }
        let _ = door_sys::close(handle);
        return Err(error);
    }
    Ok((handle, false))
}
/// Rights of the handles ntdll gives out for sections.
fn files_rights() -> u64 {
    super::files::SECTION_RIGHTS
}
/// SEC_IMAGE: the headers and every section's raw data at its RVA.
fn fill_image(file: u64, section: u64, size: u64, headers: &image::Headers) -> Result<(), u32> {
    let image_bytes = u64::from(headers.size_of_image);
    let header_bytes = u64::from(headers.size_of_headers)
        .min(size)
        .min(image_bytes);
    super::files::mapping_read(file, 0, section, 0, header_bytes)?;
    for s in headers.sections() {
        let raw = u64::from(s.raw_size)
            .min(u64::from(s.mapped_size()))
            .min(size.saturating_sub(u64::from(s.raw_offset)));
        let end = u64::from(s.rva) + raw;
        if raw == 0 {
            continue;
        }
        if end > image_bytes {
            return Err(ERROR_BAD_EXE_FORMAT);
        }
        super::files::mapping_read(
            file,
            u64::from(s.raw_offset),
            section,
            u64::from(s.rva),
            raw,
        )?;
    }
    Ok(())
}

/// OpenFileMappingW: the section of `name` in this process.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenFileMapping(access: u32, name: u64) -> u64 {
    let _ = access;
    let mut units = [0u16; NAME_UNITS];
    match read_name(name, &mut units) {
        Ok(0) => fail(ERROR_INVALID_PARAMETER),
        Ok(length) => match open_named(&units[..length], files_rights()) {
            Some(Ok(handle)) => handle,
            Some(Err(error)) => fail(error),
            None => fail(ERROR_FILE_NOT_FOUND),
        },
        Err(error) => fail(error),
    }
}

/// MapViewOfFile(Ex, 3): a view of `section` from byte `offset` for
/// `bytes` (0: to the end) at `address` (0: anywhere). `access` is the
/// FILE_MAP_* access, or 0 when `protection` (MapViewOfFile3's PAGE_*)
/// gives the view's protection; `allocation` is MapViewOfFile3's
/// allocation type (MEM_REPLACE_PLACEHOLDER, MEM_RESERVE). The base, or 0
/// with the last error.
#[unsafe(no_mangle)]
pub extern "system" fn DoorMapViewOfFile(
    section: u64,
    access: u32,
    offset: u64,
    bytes: u64,
    address: u64,
    allocation: u32,
    protection: u32,
) -> u64 {
    match map(
        section, access, offset, bytes, address, allocation, protection,
    ) {
        Ok(base) => base,
        Err(error) => fail(error),
    }
}
fn map(
    handle: u64,
    access: u32,
    offset: u64,
    bytes: u64,
    address: u64,
    allocation: u32,
    protection: u32,
) -> Result<u64, u32> {
    let object = object_of(handle)?;
    let replace = allocation & vm::REPLACE_PLACEHOLDER != 0;
    if allocation & !(vm::REPLACE_PLACEHOLDER | vm::RESERVE | vm::TOP_DOWN) != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let protect = if protection != 0 {
        protection
    } else {
        section::view_protection(access & !section::FILE_MAP_RESERVE)
            .ok_or(ERROR_INVALID_PARAMETER)?
    };
    let alignment = if replace { PAGE_BYTES } else { GRANULARITY };
    if !offset.is_multiple_of(alignment) || (address != 0 && !address.is_multiple_of(alignment)) {
        return Err(ERROR_MAPPED_ALIGNMENT);
    }
    let found = {
        let guard = Guard::acquire();
        guard
            .tables()
            .records
            .iter()
            .position(|r| r.is_some_and(|r| r.object == object))
            .map(|i| (i, guard.tables().records[i].unwrap()))
    };
    let flags = if replace { section::REPLACE } else { 0 }
        | if allocation & vm::TOP_DOWN != 0 {
            vm::RESERVE_TOP_DOWN
        } else {
            0
        };
    let image = found.is_some_and(|(_, r)| r.image_base != 0);
    let base = if image {
        map_image(handle, found.unwrap().1, address, flags, protect)?
    } else {
        call(
            op::VM_SECTION_MAP,
            [handle, address, offset, bytes, u64::from(protect), flags],
        )
        .map_err(status_error)?
    };
    let size = match bytes {
        0 => {
            let total = call(op::VM_SECTION_SIZE, [handle, 0, 0, 0, 0, 0]).unwrap_or(0);
            if image {
                total
            } else {
                total.saturating_sub(offset)
            }
        }
        n => n.next_multiple_of(PAGE_BYTES),
    };
    let guard = Guard::acquire();
    let tables = guard.tables();
    let write_back = found.is_some_and(|(_, r)| {
        r.file != 0 && matches!(protect & 0xFF, page::READWRITE | page::EXECUTE_READWRITE)
    });
    if let Some((index, _)) = found
        && let Some(record) = tables.records[index].as_mut()
    {
        record.views += 1;
    }
    match tables.views.iter().position(Option::is_none) {
        Some(slot) => {
            tables.views[slot] = Some(View {
                base,
                bytes: size,
                record: found.map(|(i, _)| i),
                offset: if image { 0 } else { offset },
                write_back,
            });
            Ok(base)
        }
        None => {
            drop(guard);
            let _ = call(op::VM_SECTION_UNMAP, [base, 0, 0, 0, 0, 0]);
            Err(ERROR_NOT_ENOUGH_MEMORY)
        }
    }
}
/// A SEC_IMAGE view: the whole image at its preferred base when free
/// (elsewhere otherwise, then relocated), each section with its own
/// protection (writable sections copy on write).
fn map_image(
    handle: u64,
    record: Record,
    address: u64,
    flags: u64,
    protect: u32,
) -> Result<u64, u32> {
    let _ = protect;
    let wanted = if address != 0 {
        address
    } else {
        record.image_base
    };
    let base = match call(
        op::VM_SECTION_MAP,
        [
            handle,
            wanted,
            0,
            0,
            u64::from(page::EXECUTE_WRITECOPY),
            flags | section::IMAGE,
        ],
    ) {
        Ok(base) => base,
        Err(_) if address == 0 => call(
            op::VM_SECTION_MAP,
            [
                handle,
                0,
                0,
                0,
                u64::from(page::EXECUTE_WRITECOPY),
                flags | section::IMAGE,
            ],
        )
        .map_err(status_error)?,
        Err(error) => return Err(status_error(error)),
    };
    let size = record.bytes.next_multiple_of(PAGE_BYTES);
    let result = (|| {
        // SAFETY: the whole view was just mapped (copy on write) here.
        let memory = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, size as usize) };
        let headers = image::parse_headers(memory).map_err(|_| ERROR_BAD_EXE_FORMAT)?;
        if base != record.image_base {
            image::relocate(memory, &headers, base).map_err(|_| ERROR_BAD_EXE_FORMAT)?;
        }
        let mut old = 0u32;
        let header_bytes = u64::from(headers.size_of_headers).next_multiple_of(PAGE_BYTES);
        protect_range(base, header_bytes.min(size), page::READONLY, &mut old)?;
        for s in headers.sections() {
            let start = u64::from(s.rva);
            let length = u64::from(s.mapped_size()).next_multiple_of(PAGE_BYTES);
            if length == 0 || start >= size {
                continue;
            }
            // Writable image sections copy on write, as Windows maps them.
            let protection = match s.protection() {
                0x40 => page::EXECUTE_WRITECOPY,
                0x04 => page::WRITECOPY,
                other => other,
            };
            protect_range(base + start, length.min(size - start), protection, &mut old)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = call(op::VM_SECTION_UNMAP, [base, 0, 0, 0, 0, 0]);
        return Err(error);
    }
    Ok(base)
}
fn protect_range(address: u64, bytes: u64, protection: u32, old: &mut u32) -> Result<(), u32> {
    if super::memory::DoorVirtualProtect(address, bytes, protection, old as *mut u32 as u64) == 0 {
        return Err(ERROR_INVALID_ADDRESS);
    }
    Ok(())
}

/// The view holding `address` (removed from the table when `take`).
fn view_at(address: u64, take: bool) -> Option<(View, Option<Record>)> {
    let guard = Guard::acquire();
    let tables = guard.tables();
    let index = tables.views.iter().position(|v| {
        v.is_some_and(|v| address >= v.base && address < v.base + v.bytes.max(PAGE_BYTES))
    })?;
    let view = tables.views[index]?;
    if take {
        tables.views[index] = None;
    }
    let record = view.record.and_then(|i| tables.records[i]);
    Some((view, record))
}
/// Write the view's bytes `[from, from + bytes)` back to its file.
fn write_back(view: &View, record: &Record, from: u64, bytes: u64) -> Result<(), u32> {
    let start = from & !(PAGE_BYTES - 1);
    let end = (from + bytes)
        .next_multiple_of(PAGE_BYTES)
        .min(view.base + view.bytes);
    if end <= start {
        return Ok(());
    }
    let section_offset = view.offset + (start - view.base);
    // Not past the mapping's size (the file was sized to it).
    let length = (end - start).min(record.bytes.saturating_sub(section_offset));
    if length == 0 {
        return Ok(());
    }
    super::files::mapping_write(
        record.file,
        section_offset,
        record.held,
        section_offset,
        length,
    )
}

/// UnmapViewOfFile(Ex, 2): the view holding `address` goes (written back
/// first when it is a shared writable view of a file);
/// MEM_PRESERVE_PLACEHOLDER in `flags` leaves a placeholder.
#[unsafe(no_mangle)]
pub extern "system" fn DoorUnmapViewOfFile(address: u64, flags: u32) -> i32 {
    if flags & !(vm::PRESERVE_PLACEHOLDER | vm::UNMAP_WITH_TRANSIENT_BOOST) != 0 {
        RtlSetLastWin32Error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let kernel_flags = if flags & vm::PRESERVE_PLACEHOLDER != 0 {
        section::PRESERVE
    } else {
        0
    };
    let found = view_at(address, false);
    if let Some((view, Some(record))) = found
        && view.write_back
    {
        // Best effort, as Windows' lazy writer: the view goes regardless.
        let _ = write_back(&view, &record, view.base, view.bytes);
    }
    if let Err(error) = call(op::VM_SECTION_UNMAP, [address, kernel_flags, 0, 0, 0, 0]) {
        RtlSetLastWin32Error(if error == Status::InvalidArgument {
            ERROR_INVALID_ADDRESS
        } else {
            status_error(error)
        });
        return 0;
    }
    if let Some((view, _)) = view_at(address, true) {
        release_view(&view);
    }
    1
}
/// A view's record loses it; a record with no views left whose name no
/// one can reach any more goes, with its file pin.
fn release_view(view: &View) {
    let Some(index) = view.record else { return };
    let gone = {
        let guard = Guard::acquire();
        let tables = guard.tables();
        let Some(record) = tables.records[index].as_mut() else {
            return;
        };
        record.views = record.views.saturating_sub(1);
        if record.views == 0 && record.name_len == 0 {
            tables.records[index].take()
        } else {
            None
        }
    };
    if let Some(record) = gone {
        if record.file != 0 {
            super::files::mapping_unpin(record.file);
        }
        let _ = door_sys::close(record.held);
    }
}

/// CloseHandle of a section handle (before the kernel closes it): an
/// unnamed record with no views is done (its other handles, if any, map
/// plain views from now on).
pub(super) fn handle_closing(handle: u64) {
    // Most processes never make a record: no lookup per CloseHandle then.
    if !RECORDED.load(Ordering::Acquire) {
        return;
    }
    let Ok(object) = object_of(handle) else {
        return;
    };
    let gone = {
        let guard = Guard::acquire();
        let tables = guard.tables();
        let Some(index) = tables
            .records
            .iter()
            .position(|r| r.is_some_and(|r| r.object == object))
        else {
            return;
        };
        let record = tables.records[index].unwrap();
        if record.views == 0 && record.name_len == 0 && record.held != handle {
            tables.records[index].take()
        } else {
            None
        }
    };
    if let Some(record) = gone {
        if record.file != 0 {
            super::files::mapping_unpin(record.file);
        }
        let _ = door_sys::close(record.held);
    }
}

/// FlushViewOfFile: `bytes` (0: to the view's end) from `address` written
/// back to the file of a shared writable file view; other views have
/// nothing to write.
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlushViewOfFile(address: u64, bytes: u64) -> i32 {
    let Some((view, record)) = view_at(address, false) else {
        RtlSetLastWin32Error(ERROR_INVALID_ADDRESS);
        return 0;
    };
    if !view.write_back {
        return 1;
    }
    let Some(record) = record else { return 1 };
    let length = if bytes == 0 {
        view.base + view.bytes - address
    } else {
        bytes
    };
    match write_back(&view, &record, address, length) {
        Ok(()) => 1,
        Err(error) => {
            RtlSetLastWin32Error(error);
            0
        }
    }
}
