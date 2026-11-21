//! msi.dll's exports. Handles (MSIHANDLE) name databases, views, records,
//! summary information and sessions in a process-wide table. Installation
//! runs door-msi's engine here, over `target::Machine` (the files through
//! ntdll, the registry and services through advapi32, programs and DLLs
//! through ntdll) with `ui::Interface` as the internal UI.
use crate::sql::{self, ColumnInfo, Field};
use alloc::{boxed::Box, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use door_msi::engine::{self, Request, Session, State};
use door_msi::execute::{code, records};
use door_msi::summary::Property;
mod msiexec;
mod target;
mod ui;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    pub(crate) fn RtlGetLastWin32Error() -> u32;
    pub(crate) fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    pub(crate) fn DoorGetCurrentDirectoryW(capacity: u32, buffer: u64) -> u32;
}

pub(crate) mod error {
    pub const SUCCESS: u32 = 0;
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_HANDLE: u32 = 6;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const OPEN_FAILED: u32 = 110;
    pub const MORE_DATA: u32 = 234;
    pub const NO_MORE_ITEMS: u32 = 259;
    pub const UNKNOWN_PROPERTY: u32 = 1608;
    pub const UNKNOWN_COMPONENT: u32 = 1607;
    pub const FUNCTION_FAILED: u32 = 1627;
    pub const INVALID_DATATYPE: u32 = 1804;
    pub const CREATE_FAILED: u32 = 1631;
    pub const INVALID_TABLE: u32 = 1628;
    pub const UNKNOWN_PRODUCT: u32 = 1605;
    pub const BAD_CONFIGURATION: u32 = 1610;
}

// ---- application memory ------------------------------------------------------

const COPY: usize = door_windows_abi::APPLICATION_COPY_BYTES;
pub(crate) fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 && !out.is_empty() {
        return Err(error::INVALID_PARAMETER);
    }
    for (i, chunk) in out.chunks_mut(COPY).enumerate() {
        if unsafe {
            DoorUserCopy(
                chunk.as_mut_ptr() as u64,
                pointer + (i * COPY) as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(error::INVALID_PARAMETER);
        }
    }
    Ok(())
}
pub(crate) fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 && !data.is_empty() {
        return Err(error::INVALID_PARAMETER);
    }
    for (i, chunk) in data.chunks(COPY).enumerate() {
        if unsafe {
            DoorUserCopy(
                pointer + (i * COPY) as u64,
                chunk.as_ptr() as u64,
                chunk.len(),
            )
        } != 0
        {
            return Err(error::INVALID_PARAMETER);
        }
    }
    Ok(())
}
pub(crate) fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
pub(crate) fn write_u32(pointer: u64, v: u32) -> Result<(), u32> {
    write(pointer, &v.to_le_bytes())
}
/// A NUL-terminated UTF-16 string (None for a null pointer).
pub(crate) fn wide(pointer: u64) -> Result<Option<String>, u32> {
    if pointer == 0 {
        return Ok(None);
    }
    let mut units: Vec<u16> = Vec::new();
    let mut at = pointer;
    loop {
        let mut chunk = [0u8; 64];
        read(at, &mut chunk)?;
        for pair in chunk.chunks_exact(2) {
            let u = u16::from_le_bytes([pair[0], pair[1]]);
            if u == 0 {
                return Ok(Some(String::from_utf16_lossy(&units)));
            }
            units.push(u);
            if units.len() > 32_768 {
                return Err(error::INVALID_PARAMETER);
            }
        }
        at += 64;
    }
}
/// A NUL-terminated ANSI (Windows-1252) string.
pub(crate) fn ansi(pointer: u64) -> Result<Option<String>, u32> {
    if pointer == 0 {
        return Ok(None);
    }
    let mut out = String::new();
    let mut at = pointer;
    loop {
        let mut b = [0u8; 1];
        read(at, &mut b)?;
        if b[0] == 0 {
            return Ok(Some(out));
        }
        out.push(char::from(b[0]));
        at += 1;
        if out.len() > 32_768 {
            return Err(error::INVALID_PARAMETER);
        }
    }
}
pub(crate) fn utf16z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}
/// Windows' output-string convention: `*length` holds the buffer's size
/// in characters; the text (or as much as fits with its NUL) is copied and
/// `*length` becomes the text's length. ERROR_MORE_DATA when it did not fit.
pub(crate) fn out_wide(text: &str, buffer: u64, length: u64) -> u32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    if length == 0 {
        return if buffer == 0 {
            error::SUCCESS
        } else {
            error::INVALID_PARAMETER
        };
    }
    let Ok(capacity) = read_u32(length) else {
        return error::INVALID_PARAMETER;
    };
    let result = if buffer == 0 {
        error::SUCCESS
    } else if (capacity as usize) <= units.len() {
        if capacity > 0 {
            let mut part: Vec<u8> = units[..capacity as usize - 1]
                .iter()
                .flat_map(|u| u.to_le_bytes())
                .collect();
            part.extend_from_slice(&[0, 0]);
            if write(buffer, &part).is_err() {
                return error::INVALID_PARAMETER;
            }
        }
        error::MORE_DATA
    } else {
        let mut bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend_from_slice(&[0, 0]);
        if write(buffer, &bytes).is_err() {
            return error::INVALID_PARAMETER;
        }
        error::SUCCESS
    };
    let _ = write_u32(length, units.len() as u32);
    result
}
pub(crate) fn out_ansi(text: &str, buffer: u64, length: u64) -> u32 {
    let bytes: Vec<u8> = text
        .chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .collect();
    if length == 0 {
        return if buffer == 0 {
            error::SUCCESS
        } else {
            error::INVALID_PARAMETER
        };
    }
    let Ok(capacity) = read_u32(length) else {
        return error::INVALID_PARAMETER;
    };
    let result = if buffer == 0 {
        error::SUCCESS
    } else if (capacity as usize) <= bytes.len() {
        if capacity > 0 {
            let mut part = bytes[..capacity as usize - 1].to_vec();
            part.push(0);
            let _ = write(buffer, &part);
        }
        error::MORE_DATA
    } else {
        let mut b = bytes.clone();
        b.push(0);
        if write(buffer, &b).is_err() {
            return error::INVALID_PARAMETER;
        }
        error::SUCCESS
    };
    let _ = write_u32(length, bytes.len() as u32);
    result
}

// ---- the handle table ----------------------------------------------------------

pub(crate) struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure the only access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}

/// A database's bytes and where they came from.
pub(crate) struct Package {
    pub bytes: Arc<Vec<u8>>,
    pub path: String,
}
#[allow(clippy::large_enum_variant)]
pub(crate) enum Object {
    Database(Package),
    View {
        package: Arc<Vec<u8>>,
        query: sql::Query,
        columns: Vec<ColumnInfo>,
        rows: Vec<Vec<Field>>,
        next: usize,
        executed: bool,
    },
    /// Fields 0..=n (field 0 is the format template).
    Record(Vec<Field>, Option<Arc<Vec<u8>>>),
    Summary(Vec<(u32, Property)>),
    /// A session a custom action runs in (the engine's state, live for the
    /// call).
    Action(*mut State),
    /// MsiOpenPackage's session: the package and the state actions change.
    Package(Box<(Package, State)>),
}
unsafe impl Send for Object {}
pub(crate) struct Handles {
    next: u32,
    objects: Vec<(u32, Object)>,
}
pub(crate) static HANDLES: Lock<Handles> = Lock::new(Handles {
    next: 1,
    objects: Vec::new(),
});
pub(crate) fn insert(object: Object) -> u32 {
    HANDLES.with(|h| {
        let id = h.next;
        h.next = h.next.wrapping_add(1).max(1);
        h.objects.push((id, object));
        id
    })
}
pub(crate) fn close(handle: u32) -> bool {
    HANDLES.with(|h| {
        let before = h.objects.len();
        h.objects.retain(|o| o.0 != handle);
        before != h.objects.len()
    })
}
pub(crate) fn with_object<R>(handle: u32, f: impl FnOnce(&mut Object) -> R) -> Option<R> {
    HANDLES.with(|h| {
        h.objects
            .iter_mut()
            .find(|o| o.0 == handle)
            .map(|o| f(&mut o.1))
    })
}
/// The engine state behind a session handle (a custom action's or
/// MsiOpenPackage's), for a call that does not re-enter the table.
pub(crate) fn with_state<R>(handle: u32, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let pointer = with_object(handle, |o| match o {
        Object::Action(p) => Some(*p),
        Object::Package(b) => Some(&mut b.1 as *mut State),
        _ => None,
    })??;
    // SAFETY: an Action pointer is live while its custom action runs on
    // this thread; a Package box lives until MsiCloseHandle, which a
    // session's own calls never race.
    Some(f(unsafe { &mut *pointer }))
}

// ---- databases, views, records ---------------------------------------------------

pub(crate) fn load(path: &str) -> Result<Vec<u8>, u32> {
    let mut t = target::Machine::new();
    use door_msi::execute::Target;
    t.read_file(path).map_err(|_| error::OPEN_FAILED)
}
/// A path as Windows resolves it: relative to the current directory.
pub(crate) fn absolute(path: &str) -> String {
    if path.len() > 2 && (path.as_bytes()[1] == b':' || path.starts_with("\\\\")) {
        return String::from(path);
    }
    let mut buffer = [0u16; 520];
    let n = unsafe { DoorGetCurrentDirectoryW(buffer.len() as u32, buffer.as_mut_ptr() as u64) }
        as usize;
    let mut dir = String::from_utf16_lossy(&buffer[..n.min(buffer.len())]);
    if !dir.ends_with('\\') {
        dir.push('\\');
    }
    alloc::format!("{dir}{path}")
}

fn open_database(path: Option<String>, out: u64) -> u32 {
    let Some(path) = path else {
        return error::INVALID_PARAMETER;
    };
    let path = absolute(&path);
    let bytes = match load(&path) {
        Ok(b) => b,
        Err(e) => return e,
    };
    if door_msi::Database::open(&bytes).is_err() {
        return error::OPEN_FAILED;
    }
    let h = insert(Object::Database(Package {
        bytes: Arc::new(bytes),
        path,
    }));
    if write_u32(out, h).is_err() {
        close(h);
        return error::INVALID_PARAMETER;
    }
    error::SUCCESS
}
/// MsiOpenDatabaseW: the package opened read-only (the persist mode's
/// create and transact modes are not offered: ERROR_CREATE_FAILED).
#[unsafe(no_mangle)]
pub extern "system" fn MsiOpenDatabaseW(path: usize, persist: usize, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let persist = door_windows_abi::widen(persist);
    let out = door_windows_abi::widen(out);
    {
        if persist > 0xff && persist != 0 {
            // A path: an output database.
            return error::CREATE_FAILED;
        }
        if persist == 3 || persist == 4 {
            return error::CREATE_FAILED;
        }
        match wide(path) {
            Ok(p) => open_database(p, out),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiOpenDatabaseA(path: usize, persist: usize, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let persist = door_windows_abi::widen(persist);
    let out = door_windows_abi::widen(out);
    {
        if persist == 3 || persist == 4 || persist > 0xff {
            return error::CREATE_FAILED;
        }
        match ansi(path) {
            Ok(p) => open_database(p, out),
            Err(e) => e,
        }
    }
}
fn database_bytes(handle: u32) -> Option<Arc<Vec<u8>>> {
    with_object(handle, |o| match o {
        Object::Database(p) => Some(p.bytes.clone()),
        Object::Package(b) => Some(b.0.bytes.clone()),
        _ => None,
    })
    .flatten()
}
fn open_view(database: u32, query: Option<String>, out: u64) -> u32 {
    let Some(bytes) = database_bytes(database) else {
        return error::INVALID_HANDLE;
    };
    let Some(query) = query else {
        return error::INVALID_PARAMETER;
    };
    let q = match sql::parse(&query) {
        Ok(q) => q,
        Err(e) => return e,
    };
    // Check the tables and columns now, as Windows does.
    if let Ok(db) = door_msi::Database::open(&bytes)
        && let Err(e) = sql::run(&db, &q, &vec![Field::Null; q.parameters])
    {
        return e;
    }
    let h = insert(Object::View {
        package: bytes,
        query: q,
        columns: Vec::new(),
        rows: Vec::new(),
        next: 0,
        executed: false,
    });
    if write_u32(out, h).is_err() {
        close(h);
        return error::INVALID_PARAMETER;
    }
    error::SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiDatabaseOpenViewW(database: u32, query: usize, out: usize) -> u32 {
    let query = door_windows_abi::widen(query);
    let out = door_windows_abi::widen(out);
    {
        match wide(query) {
            Ok(q) => open_view(database, q, out),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiDatabaseOpenViewA(database: u32, query: usize, out: usize) -> u32 {
    let query = door_windows_abi::widen(query);
    let out = door_windows_abi::widen(out);
    {
        match ansi(query) {
            Ok(q) => open_view(database, q, out),
            Err(e) => e,
        }
    }
}
fn record_fields(handle: u32) -> Option<Vec<Field>> {
    with_object(handle, |o| match o {
        Object::Record(f, _) => Some(f.clone()),
        _ => None,
    })
    .flatten()
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiViewExecute(view: u32, parameters: u32) -> u32 {
    let params: Vec<Field> = if parameters == 0 {
        Vec::new()
    } else {
        match record_fields(parameters) {
            Some(f) => f.into_iter().skip(1).collect(),
            None => return error::INVALID_HANDLE,
        }
    };
    let Some((bytes, query)) = with_object(view, |o| match o {
        Object::View { package, query, .. } => Some((package.clone(), query.clone())),
        _ => None,
    })
    .flatten() else {
        return error::INVALID_HANDLE;
    };
    let Ok(db) = door_msi::Database::open(&bytes) else {
        return error::FUNCTION_FAILED;
    };
    let (columns, rows) = match sql::run(&db, &query, &params) {
        Ok(r) => r,
        Err(e) => return e,
    };
    with_object(view, |o| {
        if let Object::View {
            columns: c,
            rows: r,
            next,
            executed,
            ..
        } = o
        {
            *c = columns;
            *r = rows;
            *next = 0;
            *executed = true;
        }
    });
    error::SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiViewFetch(view: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let row = with_object(view, |o| match o {
            Object::View {
                rows,
                next,
                executed,
                package,
                ..
            } => {
                if !*executed {
                    return Err(error::FUNCTION_FAILED);
                }
                let r = rows.get(*next).cloned();
                *next += 1;
                Ok(r.map(|r| (r, package.clone())))
            }
            _ => Err(error::INVALID_HANDLE),
        });
        match row {
            None => error::INVALID_HANDLE,
            Some(Err(e)) => e,
            Some(Ok(None)) => error::NO_MORE_ITEMS,
            Some(Ok(Some((fields, package)))) => {
                let mut all = vec![Field::Null];
                all.extend(fields);
                let h = insert(Object::Record(all, Some(package)));
                if write_u32(out, h).is_err() {
                    close(h);
                    return error::INVALID_PARAMETER;
                }
                error::SUCCESS
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiViewClose(view: u32) -> u32 {
    with_object(view, |o| match o {
        Object::View { executed, next, .. } => {
            *executed = false;
            *next = 0;
            error::SUCCESS
        }
        _ => error::INVALID_HANDLE,
    })
    .unwrap_or(error::INVALID_HANDLE)
}
/// MsiViewGetColumnInfo: MSICOLINFO_NAMES (0) or MSICOLINFO_TYPES (1).
#[unsafe(no_mangle)]
pub extern "system" fn MsiViewGetColumnInfo(view: u32, kind: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let columns = with_object(view, |o| match o {
            Object::View {
                columns,
                executed,
                query,
                package,
                ..
            } => {
                if *executed {
                    Some(columns.clone())
                } else {
                    let db = door_msi::Database::open(package).ok()?;
                    sql::run(&db, query, &vec![Field::Null; query.parameters])
                        .ok()
                        .map(|r| r.0)
                }
            }
            _ => None,
        })
        .flatten();
        let Some(columns) = columns else {
            return error::INVALID_HANDLE;
        };
        if kind > 1 {
            return error::INVALID_PARAMETER;
        }
        let mut fields = vec![Field::Null];
        for c in columns {
            fields.push(Field::Str(if kind == 0 { c.name } else { c.kind }));
        }
        let h = insert(Object::Record(fields, None));
        if write_u32(out, h).is_err() {
            close(h);
            return error::INVALID_PARAMETER;
        }
        error::SUCCESS
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiCreateRecord(count: u32) -> u32 {
    if count > 65535 {
        return 0;
    }
    insert(Object::Record(vec![Field::Null; count as usize + 1], None))
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordGetFieldCount(record: u32) -> u32 {
    record_fields(record).map_or(u32::MAX, |f| f.len() as u32 - 1)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordIsNull(record: u32, field: u32) -> i32 {
    record_fields(record)
        .and_then(|f| f.get(field as usize).cloned())
        .map_or(1, |f| i32::from(matches!(f, Field::Null)))
}
/// MSI_NULL_INTEGER for a null or non-integer field.
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordGetInteger(record: u32, field: u32) -> i32 {
    const NULL: i32 = i32::MIN;
    match record_fields(record).and_then(|f| f.get(field as usize).cloned()) {
        Some(Field::Int(i)) => i,
        Some(Field::Str(s)) => s.trim().parse().unwrap_or(NULL),
        _ => NULL,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordGetStringW(
    record: u32,
    field: u32,
    buffer: usize,
    length: usize,
) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Some(fields) = record_fields(record) else {
            return error::INVALID_HANDLE;
        };
        let text = fields
            .get(field as usize)
            .map(Field::text)
            .unwrap_or_default();
        out_wide(&text, buffer, length)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordGetStringA(
    record: u32,
    field: u32,
    buffer: usize,
    length: usize,
) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Some(fields) = record_fields(record) else {
            return error::INVALID_HANDLE;
        };
        let text = fields
            .get(field as usize)
            .map(Field::text)
            .unwrap_or_default();
        out_ansi(&text, buffer, length)
    }
}
fn set_field(record: u32, field: u32, value: Field) -> u32 {
    with_object(record, |o| match o {
        Object::Record(f, _) => match f.get_mut(field as usize) {
            Some(slot) => {
                *slot = value;
                error::SUCCESS
            }
            None => error::INVALID_PARAMETER,
        },
        _ => error::INVALID_HANDLE,
    })
    .unwrap_or(error::INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordSetStringW(record: u32, field: u32, value: usize) -> u32 {
    let value = door_windows_abi::widen(value);
    {
        match wide(value) {
            Ok(Some(s)) if !s.is_empty() => set_field(record, field, Field::Str(s)),
            Ok(_) => set_field(record, field, Field::Null),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordSetStringA(record: u32, field: u32, value: usize) -> u32 {
    let value = door_windows_abi::widen(value);
    {
        match ansi(value) {
            Ok(Some(s)) if !s.is_empty() => set_field(record, field, Field::Str(s)),
            Ok(_) => set_field(record, field, Field::Null),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordSetInteger(record: u32, field: u32, value: i32) -> u32 {
    set_field(
        record,
        field,
        if value == i32::MIN {
            Field::Null
        } else {
            Field::Int(value)
        },
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordClearData(record: u32) -> u32 {
    with_object(record, |o| match o {
        Object::Record(f, _) => {
            for x in f.iter_mut() {
                *x = Field::Null;
            }
            error::SUCCESS
        }
        _ => error::INVALID_HANDLE,
    })
    .unwrap_or(error::INVALID_HANDLE)
}
fn stream_bytes(record: u32, field: u32) -> Option<Vec<u8>> {
    let (name, package) = with_object(record, |o| match o {
        Object::Record(f, p) => match f.get(field as usize) {
            Some(Field::Stream(n)) => Some((n.clone(), p.clone()?)),
            _ => None,
        },
        _ => None,
    })??;
    let db = door_msi::Database::open(&package).ok()?;
    db.stream(&name).ok()
}
/// MsiRecordDataSize: a string's length, 4 for an integer, a stream's
/// size.
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordDataSize(record: u32, field: u32) -> u32 {
    match record_fields(record).and_then(|f| f.get(field as usize).cloned()) {
        Some(Field::Int(_)) => 4,
        Some(Field::Str(s)) => s.encode_utf16().count() as u32,
        Some(Field::Stream(_)) => stream_bytes(record, field).map_or(0, |b| b.len() as u32),
        _ => 0,
    }
}
/// MsiRecordReadStream: reads the stream from the start on each call
/// after the last; `*length` bytes at most.
#[unsafe(no_mangle)]
pub extern "system" fn MsiRecordReadStream(
    record: u32,
    field: u32,
    buffer: usize,
    length: usize,
) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        static OFFSETS: Lock<Vec<(u32, u32, usize)>> = Lock::new(Vec::new());
        let Some(bytes) = stream_bytes(record, field) else {
            return error::INVALID_DATATYPE;
        };
        if buffer == 0 {
            return write_u32(length, bytes.len() as u32)
                .map_or(error::INVALID_PARAMETER, |()| error::SUCCESS);
        }
        let Ok(capacity) = read_u32(length) else {
            return error::INVALID_PARAMETER;
        };
        let offset = OFFSETS.with(|o| {
            o.iter()
                .find(|x| x.0 == record && x.1 == field)
                .map_or(0, |x| x.2)
        });
        let end = (offset + capacity as usize).min(bytes.len());
        if write(buffer, &bytes[offset..end]).is_err() {
            return error::INVALID_PARAMETER;
        }
        let _ = write_u32(length, (end - offset) as u32);
        OFFSETS.with(|o| {
            o.retain(|x| !(x.0 == record && x.1 == field));
            if end < bytes.len() {
                o.push((record, field, end));
            }
        });
        error::SUCCESS
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiCloseHandle(handle: u32) -> u32 {
    if handle == 0 || close(handle) {
        error::SUCCESS
    } else {
        error::INVALID_HANDLE
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiCloseAllHandles() -> u32 {
    HANDLES.with(|h| {
        let n = h.objects.len() as u32;
        h.objects.retain(|o| matches!(o.1, Object::Action(_)));
        n
    })
}
/// MsiDatabaseGetPrimaryKeysW: a record of the table's key column names
/// (field 0: the table).
#[unsafe(no_mangle)]
pub extern "system" fn MsiDatabaseGetPrimaryKeysW(database: u32, table: usize, out: usize) -> u32 {
    let table = door_windows_abi::widen(table);
    let out = door_windows_abi::widen(out);
    {
        let Some(bytes) = database_bytes(database) else {
            return error::INVALID_HANDLE;
        };
        let Ok(Some(table)) = wide(table) else {
            return error::INVALID_PARAMETER;
        };
        let Ok(db) = door_msi::Database::open(&bytes) else {
            return error::FUNCTION_FAILED;
        };
        let Some(columns) = db.tables.iter().find(|t| t.0 == table).map(|t| t.1.clone()) else {
            return error::INVALID_TABLE;
        };
        let mut fields = vec![Field::Str(table)];
        fields.extend(
            columns
                .iter()
                .filter(|c| c.is_key())
                .map(|c| Field::Str(c.name.clone())),
        );
        let h = insert(Object::Record(fields, None));
        if write_u32(out, h).is_err() {
            close(h);
            return error::INVALID_PARAMETER;
        }
        error::SUCCESS
    }
}
mod api;
