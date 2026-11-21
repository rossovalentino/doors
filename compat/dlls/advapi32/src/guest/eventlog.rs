//! The classic Event Log API over Init's Event Log: RegisterEventSource,
//! ReportEvent and DeregisterEventSource for writing; OpenEventLog,
//! GetNumberOfEventLogRecords, GetOldestEventLogRecord, ReadEventLog,
//! ClearEventLog and CloseEventLog for reading. An event's strings become
//! its message (joined by spaces, as Event Viewer shows the inserts when a
//! source registers no message file).
use super::*;
use door_service_client::EventLog;
use door_service_protocol::eventlog::{Event, Log, kind};

const EVENTLOG_SEQUENTIAL_READ: u32 = 0x1;
const EVENTLOG_SEEK_READ: u32 = 0x2;
const EVENTLOG_FORWARDS_READ: u32 = 0x4;
const EVENTLOG_BACKWARDS_READ: u32 = 0x8;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

#[derive(Clone)]
enum Source {
    /// RegisterEventSource: writes to Application under `name`.
    Writer { name: String },
    /// OpenEventLog: reads `log`, `next` is the sequential position.
    Reader { log: Log, next: u32, started: bool },
}
static SOURCES: Lock<Vec<Option<Source>>> = Lock::new(Vec::new());
const SOURCE_BASE: u64 = 0x0000_0000_00e0_0000;
fn new_source(s: Source) -> u64 {
    SOURCES.with(|t| {
        let i = t.iter().position(Option::is_none).unwrap_or_else(|| {
            t.push(None);
            t.len() - 1
        });
        t[i] = Some(s);
        SOURCE_BASE + 8 * i as u64
    })
}
fn source(h: u64) -> Result<Source, u32> {
    SOURCES.with(|t| {
        h.checked_sub(SOURCE_BASE)
            .filter(|o| o % 8 == 0)
            .and_then(|o| t.get((o / 8) as usize).cloned().flatten())
            .ok_or(error::INVALID_HANDLE)
    })
}
fn set_source(h: u64, s: Source) {
    SOURCES.with(|t| {
        if let Some(slot) = h
            .checked_sub(SOURCE_BASE)
            .and_then(|o| t.get_mut((o / 8) as usize))
        {
            *slot = Some(s);
        }
    });
}
fn log_client() -> Result<EventLog, u32> {
    let registry =
        unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
    EventLog::connect(registry).map_err(|_| 1722) // RPC_S_SERVER_UNAVAILABLE
}
fn utf16_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

fn register(server: u64, name: u64, wide: bool) -> u64 {
    let r = (|| {
        if server != 0 {
            // Only this computer.
            return Err(1722);
        }
        let name = utf16_string(&text(name, wide)?);
        if name.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        Ok(new_source(Source::Writer { name }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterEventSourceW(server: usize, name: usize) -> usize {
    let server = door_windows_abi::widen(server);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { register(server, name, true) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterEventSourceA(server: usize, name: usize) -> usize {
    let server = door_windows_abi::widen(server);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { register(server, name, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DeregisterEventSource(h: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    close_source(h)
}
fn close_source(h: u64) -> i32 {
    let found = SOURCES.with(|t| {
        h.checked_sub(SOURCE_BASE)
            .filter(|o| o % 8 == 0)
            .and_then(|o| t.get_mut((o / 8) as usize))
            .and_then(Option::take)
            .is_some()
    });
    last_error(if found {
        error::SUCCESS
    } else {
        error::INVALID_HANDLE
    })
}
#[allow(clippy::too_many_arguments)]
fn report(
    h: u64,
    event_type: u16,
    category: u16,
    id: u32,
    count: u16,
    strings: u64,
    wide: bool,
) -> i32 {
    let r = (|| {
        let Source::Writer { name } = source(h)? else {
            return Err(error::INVALID_HANDLE);
        };
        let mut message = String::new();
        for i in 0..u64::from(count) {
            let p = read_pointer(strings + POINTER as u64 * i)?;
            if p == 0 {
                continue;
            }
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&utf16_string(&text(p, wide)?));
        }
        let kind = match event_type {
            kind::ERROR | kind::WARNING | kind::AUDIT_SUCCESS | kind::AUDIT_FAILURE => event_type,
            _ => kind::INFORMATION,
        };
        // Windows logs the low 16 bits as the Event ID.
        let event = Event::new(
            Log::Application,
            kind,
            category,
            id & 0xffff,
            &name,
            &message,
        );
        log_client()?
            .report(&event)
            .map_err(|_| error::ACCESS_DENIED)?;
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn ReportEventW(
    h: usize,
    event_type: u16,
    category: u16,
    id: u32,
    _sid: usize,
    count: u16,
    _data_size: u32,
    strings: usize,
    _data: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let strings = door_windows_abi::widen(strings);
    report(h, event_type, category, id, count, strings, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn ReportEventA(
    h: usize,
    event_type: u16,
    category: u16,
    id: u32,
    _sid: usize,
    count: u16,
    _data_size: u32,
    strings: usize,
    _data: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let strings = door_windows_abi::widen(strings);
    report(h, event_type, category, id, count, strings, false)
}

fn open(server: u64, name: u64, wide: bool) -> u64 {
    let r = (|| {
        if server != 0 {
            return Err(1722);
        }
        let name = utf16_string(&text(name, wide)?);
        // An unknown log name opens Application, as Windows does.
        let log = Log::named(&name).unwrap_or(Log::Application);
        Ok(new_source(Source::Reader {
            log,
            next: 0,
            started: false,
        }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenEventLogW(server: usize, name: usize) -> usize {
    let server = door_windows_abi::widen(server);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { open(server, name, true) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenEventLogA(server: usize, name: usize) -> usize {
    let server = door_windows_abi::widen(server);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { open(server, name, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseEventLog(h: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    close_source(h)
}
fn reader(h: u64) -> Result<Log, u32> {
    match source(h)? {
        Source::Reader { log, .. } => Ok(log),
        Source::Writer { .. } => Err(error::INVALID_HANDLE),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumberOfEventLogRecords(h: usize, out: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let log = reader(h)?;
            let (_, count) = log_client()?.info(log).map_err(|_| error::ACCESS_DENIED)?;
            write_u32(out, count)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetOldestEventLogRecord(h: usize, out: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let log = reader(h)?;
            let (oldest, _) = log_client()?.info(log).map_err(|_| error::ACCESS_DENIED)?;
            write_u32(out, oldest)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// EVENTLOGRECORD: Length, Reserved 'LfLe', RecordNumber, TimeGenerated,
/// TimeWritten (seconds since 1970), EventID, EventType, NumStrings,
/// EventCategory, ReservedFlags, ClosingRecordNumber, StringOffset,
/// UserSidLength, UserSidOffset, DataLength, DataOffset (56 bytes), then
/// SourceName and Computername (NUL-terminated), the strings, padding to
/// 4 bytes and the trailing Length.
fn record_bytes(e: &Event, computer: &str, wide: bool) -> Vec<u8> {
    let encode = |s: &str| -> Vec<u8> {
        if wide {
            s.encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect()
        } else {
            s.bytes().chain([0]).collect()
        }
    };
    let source = encode(e.source());
    let computer = encode(computer);
    let message = encode(e.message());
    let string_offset = 56 + source.len() + computer.len();
    let mut length = string_offset + message.len() + 4;
    length = length.div_ceil(4) * 4;
    let seconds = (e.time / 10_000_000).saturating_sub(11_644_473_600) as u32;
    let mut b = vec![0u8; length];
    let put = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut b, 0, length as u32);
    put(&mut b, 4, 0x654c_664c);
    put(&mut b, 8, e.record);
    put(&mut b, 12, seconds);
    put(&mut b, 16, seconds);
    put(&mut b, 20, e.id);
    b[24..26].copy_from_slice(&e.kind.to_le_bytes());
    b[26..28].copy_from_slice(&1u16.to_le_bytes());
    b[28..30].copy_from_slice(&e.category.to_le_bytes());
    put(&mut b, 36, string_offset as u32);
    put(&mut b, 48, 0);
    put(&mut b, 52, length as u32 - 4);
    b[56..56 + source.len()].copy_from_slice(&source);
    b[56 + source.len()..string_offset].copy_from_slice(&computer);
    b[string_offset..string_offset + message.len()].copy_from_slice(&message);
    put(&mut b, length - 4, length as u32);
    b
}
#[allow(clippy::too_many_arguments)]
fn read_log(
    h: u64,
    flags: u32,
    offset: u32,
    out: u64,
    size: u32,
    read: u64,
    needed: u64,
    wide: bool,
) -> i32 {
    let r = (|| {
        let Source::Reader { log, next, started } = source(h)? else {
            return Err(error::INVALID_HANDLE);
        };
        let forwards = flags & EVENTLOG_FORWARDS_READ != 0;
        let backwards = flags & EVENTLOG_BACKWARDS_READ != 0;
        let seek = flags & EVENTLOG_SEEK_READ != 0;
        if forwards == backwards || seek == (flags & EVENTLOG_SEQUENTIAL_READ != 0) {
            return Err(error::INVALID_PARAMETER);
        }
        let mut client = log_client()?;
        let (oldest, count) = client.info(log).map_err(|_| error::ACCESS_DENIED)?;
        if count == 0 {
            return Err(ERROR_HANDLE_EOF);
        }
        let newest = oldest + count - 1;
        let mut record = if seek {
            offset
        } else if started {
            next
        } else if forwards {
            oldest
        } else {
            newest
        };
        let computer = String::from_utf16_lossy(&computer_name());
        let mut buffer = Vec::new();
        while (oldest..=newest).contains(&record) {
            let Some(event) = client.read(log, record).map_err(|_| error::ACCESS_DENIED)? else {
                break;
            };
            let bytes = record_bytes(&event, &computer, wide);
            if buffer.len() + bytes.len() > size as usize {
                if buffer.is_empty() {
                    write_u32(read, 0)?;
                    write_u32(needed, bytes.len() as u32)?;
                    return Err(ERROR_INSUFFICIENT_BUFFER);
                }
                break;
            }
            buffer.extend(bytes);
            record = if forwards {
                record + 1
            } else {
                record.wrapping_sub(1)
            };
        }
        if buffer.is_empty() {
            return Err(ERROR_HANDLE_EOF);
        }
        write(out, &buffer)?;
        write_u32(read, buffer.len() as u32)?;
        write_u32(needed, 0)?;
        set_source(
            h,
            Source::Reader {
                log,
                next: record,
                started: true,
            },
        );
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn ReadEventLogW(
    h: usize,
    flags: u32,
    offset: u32,
    out: usize,
    size: u32,
    read: usize,
    needed: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let read = door_windows_abi::widen(read);
    let needed = door_windows_abi::widen(needed);
    read_log(h, flags, offset, out, size, read, needed, true)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn ReadEventLogA(
    h: usize,
    flags: u32,
    offset: u32,
    out: usize,
    size: u32,
    read: usize,
    needed: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let read = door_windows_abi::widen(read);
    let needed = door_windows_abi::widen(needed);
    read_log(h, flags, offset, out, size, read, needed, false)
}
fn clear_log(h: u64, backup: u64) -> i32 {
    let r = (|| {
        let log = reader(h)?;
        if backup != 0 {
            // No .evt backups.
            return Err(50);
        }
        log_client()?.clear(log).map_err(|_| error::ACCESS_DENIED)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ClearEventLogW(h: usize, backup: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let backup = door_windows_abi::widen(backup);
    clear_log(h, backup)
}
#[unsafe(no_mangle)]
pub extern "system" fn ClearEventLogA(h: usize, backup: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let backup = door_windows_abi::widen(backup);
    clear_log(h, backup)
}
