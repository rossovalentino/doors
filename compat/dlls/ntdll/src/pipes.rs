//! Anonymous pipes (CreatePipe) and the standard handles a launcher's pipes
//! give its child (STARTF_USESTDHANDLES). A pipe is one kernel section: a
//! 64-byte header (byte counters, open ends of each kind, a writers' lock)
//! and a ring buffer. A process maps a pipe once and shares the mapping
//! between its ends; nothing else crosses into a child but the section, so
//! readers and writers poll with a short backoff. ReadFile returns what is
//! there (at least one byte) and ERROR_BROKEN_PIPE once every write end has
//! closed; WriteFile fails with ERROR_NO_DATA once no read end is left. An
//! exiting process closes its ends, so a child's exit ends its parent's
//! read.
use super::guest::{RtlSetLastWin32Error, copy};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use door_abi::rights;
use door_windows_abi::pipe as handles;

const SECTION_BYTES: u64 = 64 * 1024;
const HEADER: u64 = 64;
const CAPACITY: u64 = SECTION_BYTES - HEADER;
const MAGIC: u32 = 0x4550_4950;
const CHUNK: usize = 1024;
const BROKEN_PIPE: u32 = 109;
const NO_DATA: u32 = 232;

#[repr(C)]
struct Header {
    magic: AtomicU32,
    lock: AtomicU32,
    read: AtomicU64,
    written: AtomicU64,
    readers: AtomicU32,
    writers: AtomicU32,
}
#[derive(Clone, Copy)]
struct Mapping {
    object: u64,
    section: u64,
    address: u64,
    users: u32,
}
#[derive(Clone, Copy)]
struct End {
    mapping: usize,
    write: bool,
}
struct Table {
    maps: [Option<Mapping>; handles::SLOTS],
    ends: [Option<End>; handles::SLOTS],
    generations: [u32; handles::SLOTS],
}
struct State {
    owner: AtomicU32,
    table: UnsafeCell<Table>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    owner: AtomicU32::new(0),
    table: UnsafeCell::new(Table {
        maps: [None; handles::SLOTS],
        ends: [None; handles::SLOTS],
        generations: [0; handles::SLOTS],
    }),
};
/// The standard handles a launcher's pipes became (0: the console), and
/// SetStdHandle's choices (u64::MAX: none set).
static STANDARD: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static CHOSEN: [AtomicU64; 3] = [const { AtomicU64::new(u64::MAX) }; 3];

/// The table, held by one thread at a time; never while waiting for data.
fn with<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    while STATE
        .owner
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        let _ = door_sys::yield_now();
    }
    // SAFETY: the owner flag above serializes every access.
    let result = f(unsafe { &mut *STATE.table.get() });
    STATE.owner.store(0, Ordering::Release);
    result
}
fn header(address: u64) -> &'static Header {
    // SAFETY: the mapping lives while an end uses it; the header's fields
    // are atomics shared with the other processes mapping the section.
    unsafe { &*(address as *const Header) }
}
fn pause(step: &mut u64) {
    let ticks = door_abi::platform::ticks_from_millis(*step).unwrap_or(1);
    if let Ok(now) = door_sys::time() {
        let _ = door_sys::sleep(now.saturating_add(ticks));
    }
    *step = (*step * 2).min(16);
}
/// Map `section` (taking it), or share this process's mapping of the same
/// section object.
fn map(table: &mut Table, section: u64) -> Result<usize, u32> {
    let object = door_sys::inspect(section).map_err(|_| 6u32)?[2];
    if let Some(i) = table
        .maps
        .iter()
        .position(|m| m.is_some_and(|m| m.object == object))
    {
        let _ = door_sys::close(section);
        table.maps[i].as_mut().unwrap().users += 1;
        return Ok(i);
    }
    let slot = table.maps.iter().position(Option::is_none).ok_or(4u32)?;
    let address = super::memory::DoorReserveSectionRange(SECTION_BYTES);
    if address == 0 {
        return Err(8);
    }
    // SAFETY: the reserved range is this mapping's alone.
    if unsafe { door_sys::section_map(section, address, rights::READ | rights::WRITE) }.is_err() {
        super::memory::DoorReleaseSectionRange(address, SECTION_BYTES);
        return Err(5);
    }
    table.maps[slot] = Some(Mapping {
        object,
        section,
        address,
        users: 1,
    });
    Ok(slot)
}
fn unmap(table: &mut Table, index: usize) {
    let Some(m) = table.maps[index].as_mut() else {
        return;
    };
    m.users -= 1;
    if m.users == 0 {
        let m = table.maps[index].take().unwrap();
        // SAFETY: no end uses the mapping any more.
        let _ = unsafe { door_sys::section_unmap(m.address, SECTION_BYTES) };
        super::memory::DoorReleaseSectionRange(m.address, SECTION_BYTES);
        let _ = door_sys::close(m.section);
    }
}
fn new_end(table: &mut Table, end: End) -> Result<u64, u32> {
    let slot = table.ends.iter().position(Option::is_none).ok_or(4u32)?;
    let generation = table.generations[slot].wrapping_add(1).max(1);
    table.generations[slot] = generation;
    table.ends[slot] = Some(end);
    Ok(handles::encode(slot, generation).unwrap())
}
fn end(table: &Table, handle: u64) -> Result<(End, u64), u32> {
    let (slot, generation) = handles::decode(handle).ok_or(6u32)?;
    if !door_windows_abi::tagged::same(table.generations[slot], generation) {
        return Err(6);
    }
    let end = table.ends[slot].ok_or(6u32)?;
    Ok((end, table.maps[end.mapping].ok_or(6u32)?.address))
}
fn failed(error: u32) -> i32 {
    RtlSetLastWin32Error(error);
    0
}
fn put(pointer: u64, value: u32) -> Result<(), u32> {
    if pointer == 0 {
        return Ok(());
    }
    copy(pointer, value.to_le_bytes().as_ptr() as u64, 4).map_err(|_| 998u32)
}

/// `CreatePipe`: a new pipe's read and write ends.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreatePipe(read: u64, write: u64) -> i32 {
    let result = (|| {
        if read == 0 || write == 0 {
            return Err(87);
        }
        let section = door_sys::section_create(SECTION_BYTES, rights::ALL & !rights::WAIT)
            .map_err(|_| 8u32)?;
        let handles = with(|table| -> Result<(u64, u64), u32> {
            let mapping = map(table, section)?;
            let address = table.maps[mapping].unwrap().address;
            let h = header(address);
            h.magic.store(MAGIC, Ordering::Relaxed);
            h.readers.store(1, Ordering::Relaxed);
            h.writers.store(1, Ordering::Release);
            table.maps[mapping].as_mut().unwrap().users = 2;
            let r = new_end(
                table,
                End {
                    mapping,
                    write: false,
                },
            );
            let w = new_end(
                table,
                End {
                    mapping,
                    write: true,
                },
            );
            match (r, w) {
                (Ok(r), Ok(w)) => Ok((r, w)),
                (r, w) => {
                    for h in [r, w].into_iter().flatten() {
                        let (slot, _) = handles::decode(h).unwrap();
                        table.ends[slot] = None;
                    }
                    table.maps[mapping].as_mut().unwrap().users = 1;
                    unmap(table, mapping);
                    Err(4)
                }
            }
        })?;
        let pair = [handles.0, handles.1];
        // PHANDLE outs at the program's width (P17).
        super::shapes::write_pointer(read, pair[0]).map_err(|_| 998u32)?;
        super::shapes::write_pointer(write, pair[1]).map_err(|_| 998u32)
    })();
    result.map_or_else(failed, |()| 1)
}
/// ReadFile on a read end: at least one byte, or ERROR_BROKEN_PIPE once no
/// writer is left and the ring is empty.
#[unsafe(no_mangle)]
pub extern "system" fn DoorReadPipe(
    handle: u64,
    destination: u64,
    bytes: u32,
    read: u64,
    overlapped: u64,
) -> i32 {
    let result = (|| {
        put(read, 0)?;
        if overlapped != 0 {
            return Err(87);
        }
        let (end, address) = with(|t| end(t, handle))?;
        if end.write {
            return Err(5);
        }
        if bytes == 0 {
            return Ok(());
        }
        let h = header(address);
        let data = address + HEADER;
        let mut step = 1;
        loop {
            let written = h.written.load(Ordering::Acquire);
            let at = h.read.load(Ordering::Acquire);
            let available = written - at;
            if available > 0 {
                let n = available.min(u64::from(bytes));
                let mut done = 0;
                while done < n {
                    let start = (at + done) % CAPACITY;
                    let piece = (n - done).min(CAPACITY - start).min(CHUNK as u64);
                    copy(destination + done, data + start, piece as usize).map_err(|_| 998u32)?;
                    done += piece;
                }
                if h.read
                    .compare_exchange(at, at + n, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    // Another reader took these bytes first.
                    continue;
                }
                return put(read, n as u32);
            }
            if h.writers.load(Ordering::Acquire) == 0 {
                if h.written.load(Ordering::Acquire) == written {
                    return Err(BROKEN_PIPE);
                }
                continue;
            }
            pause(&mut step);
        }
    })();
    result.map_or_else(failed, |()| 1)
}
/// WriteFile on a write end: every byte, waiting for room; ERROR_NO_DATA
/// once no reader is left.
#[unsafe(no_mangle)]
pub extern "system" fn DoorWritePipe(
    handle: u64,
    source: u64,
    bytes: u32,
    written: u64,
    overlapped: u64,
) -> i32 {
    let result = (|| {
        put(written, 0)?;
        if overlapped != 0 {
            return Err(87);
        }
        let (end, address) = with(|t| end(t, handle))?;
        if !end.write {
            return Err(5);
        }
        let h = header(address);
        let data = address + HEADER;
        let mut done = 0u64;
        let mut step = 1;
        while done < u64::from(bytes) {
            if h.readers.load(Ordering::Acquire) == 0 {
                put(written, done as u32)?;
                return if done == 0 { Err(NO_DATA) } else { Ok(()) };
            }
            // Writers of one pipe (a parent and its child) take turns.
            if h.lock
                .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                let _ = door_sys::yield_now();
                continue;
            }
            let at = h.written.load(Ordering::Acquire);
            let space = CAPACITY - (at - h.read.load(Ordering::Acquire));
            let n = space.min(u64::from(bytes) - done).min(CHUNK as u64);
            let copied = if n == 0 {
                Ok(())
            } else {
                let start = at % CAPACITY;
                let first = n.min(CAPACITY - start);
                copy(data + start, source + done, first as usize)
                    .and_then(|()| {
                        if n > first {
                            copy(data, source + done + first, (n - first) as usize)
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|_| 998u32)
            };
            if copied.is_ok() && n != 0 {
                h.written.store(at + n, Ordering::Release);
            }
            h.lock.store(0, Ordering::Release);
            copied?;
            if n == 0 {
                pause(&mut step);
            } else {
                done += n;
                step = 1;
            }
        }
        put(written, done as u32)
    })();
    result.map_or_else(failed, |()| 1)
}
/// PeekNamedPipe: copies what is there without taking it; the counts of
/// bytes available (and left in the "message": a byte pipe has none).
#[unsafe(no_mangle)]
pub extern "system" fn DoorPeekNamedPipe(
    handle: u64,
    destination: u64,
    bytes: u32,
    read: u64,
    available: u64,
    left: u64,
) -> i32 {
    let result = (|| {
        let (end, address) = with(|t| end(t, handle))?;
        if end.write {
            return Err(5);
        }
        let h = header(address);
        let written = h.written.load(Ordering::Acquire);
        let at = h.read.load(Ordering::Acquire);
        let count = written - at;
        if count == 0 && h.writers.load(Ordering::Acquire) == 0 {
            return Err(BROKEN_PIPE);
        }
        let n = if destination == 0 {
            0
        } else {
            count.min(u64::from(bytes))
        };
        let mut done = 0;
        while done < n {
            let start = (at + done) % CAPACITY;
            let piece = (n - done).min(CAPACITY - start).min(CHUNK as u64);
            copy(destination + done, address + HEADER + start, piece as usize)
                .map_err(|_| 998u32)?;
            done += piece;
        }
        put(read, n as u32)?;
        put(available, count.min(u64::from(u32::MAX)) as u32)?;
        put(left, 0)
    })();
    result.map_or_else(failed, |()| 1)
}
fn close(table: &mut Table, handle: u64) -> Result<(), u32> {
    let (slot, generation) = handles::decode(handle).ok_or(6u32)?;
    if !door_windows_abi::tagged::same(table.generations[slot], generation) {
        return Err(6);
    }
    let end = table.ends[slot].take().ok_or(6u32)?;
    let address = table.maps[end.mapping].ok_or(6u32)?.address;
    let h = header(address);
    if end.write {
        h.writers.fetch_sub(1, Ordering::AcqRel);
    } else {
        h.readers.fetch_sub(1, Ordering::AcqRel);
    }
    unmap(table, end.mapping);
    for (standard, chosen) in STANDARD.iter().zip(&CHOSEN) {
        let _ = standard.compare_exchange(handle, 0, Ordering::AcqRel, Ordering::Relaxed);
        let _ = chosen.compare_exchange(handle, 0, Ordering::AcqRel, Ordering::Relaxed);
    }
    Ok(())
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorClosePipe(handle: u64) -> i32 {
    with(|t| close(t, handle)).map_or_else(failed, |()| 1)
}
/// DuplicateHandle of a pipe end: another end of the same kind.
#[unsafe(no_mangle)]
pub extern "system" fn DoorDuplicatePipe(handle: u64, target: u64) -> i32 {
    let result = with(|table| {
        let (e, address) = end(table, handle)?;
        let copy_handle = new_end(table, e)?;
        table.maps[e.mapping].as_mut().unwrap().users += 1;
        let h = header(address);
        if e.write {
            h.writers.fetch_add(1, Ordering::AcqRel);
        } else {
            h.readers.fetch_add(1, Ordering::AcqRel);
        }
        Ok(copy_handle)
    })
    .and_then(|h| super::shapes::write_pointer(target, h).map_err(|_| 998u32));
    result.map_or_else(failed, |()| 1)
}

/// DuplicateHandle of a kernel handle (event, process, thread, section)
/// with the same rights.
#[unsafe(no_mangle)]
pub extern "system" fn DoorDuplicateKernelHandle(handle: u64, target: u64) -> i32 {
    let result = door_sys::inspect(handle)
        .map_err(|_| 6u32)
        .and_then(|info| door_sys::duplicate(handle, info[1]).map_err(|_| 5u32))
        .and_then(|copied| {
            super::shapes::write_pointer(target, copied).map_err(|_| {
                let _ = door_sys::close(copied);
                998u32
            })
        });
    result.map_or_else(failed, |()| 1)
}
/// Bootstrap: the pipe sections a launcher gave this process become its
/// standard handles (input reads, output and error write). The launcher
/// counted these ends when it passed them.
pub(super) fn adopt(sections: [u64; 3]) {
    with(|table| {
        for (slot, section) in sections.into_iter().enumerate() {
            if section == 0 {
                continue;
            }
            let Ok(mapping) = map(table, section) else {
                continue;
            };
            let address = table.maps[mapping].unwrap().address;
            if header(address).magic.load(Ordering::Acquire) != MAGIC {
                unmap(table, mapping);
                continue;
            }
            match new_end(
                table,
                End {
                    mapping,
                    write: slot != 0,
                },
            ) {
                Ok(handle) => STANDARD[slot].store(handle, Ordering::Release),
                Err(_) => unmap(table, mapping),
            }
        }
    });
}
/// GetStdHandle's pipe for `slot` (0 input, 1 output, 2 error): SetStdHandle's
/// choice, else the launcher's pipe; None for the console.
pub(super) fn standard(slot: usize) -> Option<u64> {
    let chosen = CHOSEN[slot].load(Ordering::Acquire);
    if chosen != u64::MAX {
        return Some(chosen);
    }
    Some(STANDARD[slot].load(Ordering::Acquire)).filter(|h| *h != 0)
}
/// `SetStdHandle`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetStandardHandle(selector: u32, handle: u64) -> i32 {
    let slot = match selector {
        0xFFFF_FFF6 => 0,
        0xFFFF_FFF5 => 1,
        0xFFFF_FFF4 => 2,
        _ => return failed(6),
    };
    CHOSEN[slot].store(handle, Ordering::Release);
    1
}
/// CreateProcess with STARTF_USESTDHANDLES: for each standard handle that
/// is a pipe end of the right kind (input a read end, output and error
/// write ends), a section handle of its own for the child, with the end
/// counted for the child. `undo` gives the counts back when the launch
/// fails; either way the caller closes the returned sections.
pub(super) fn for_child(standard: [u64; 3]) -> [u64; 3] {
    with(|table| {
        let mut out = [0; 3];
        for (slot, handle) in standard.into_iter().enumerate() {
            let Ok((e, address)) = end(table, handle) else {
                continue;
            };
            if e.write != (slot != 0) {
                continue;
            }
            let section = table.maps[e.mapping].unwrap().section;
            let Ok(copy_handle) = door_sys::duplicate(
                section,
                rights::READ
                    | rights::WRITE
                    | rights::MAP
                    | rights::QUERY
                    | rights::DUPLICATE
                    | rights::TRANSFER,
            ) else {
                continue;
            };
            let h = header(address);
            if e.write {
                h.writers.fetch_add(1, Ordering::AcqRel);
            } else {
                h.readers.fetch_add(1, Ordering::AcqRel);
            }
            out[slot] = copy_handle;
        }
        out
    })
}
pub(super) fn undo(standard: [u64; 3], sections: [u64; 3]) {
    with(|table| {
        for (slot, handle) in standard.into_iter().enumerate() {
            if sections[slot] == 0 {
                continue;
            }
            if let Ok((e, address)) = end(table, handle) {
                let h = header(address);
                if e.write {
                    h.writers.fetch_sub(1, Ordering::AcqRel);
                } else {
                    h.readers.fetch_sub(1, Ordering::AcqRel);
                }
            }
        }
    });
}
/// ExitProcess: every end this process holds closes, as Windows closes a
/// process's handles.
pub(super) fn close_all() {
    with(|table| {
        for slot in 0..handles::SLOTS {
            if table.ends[slot].is_some() {
                let handle = handles::encode(slot, table.generations[slot]).unwrap();
                let _ = close(table, handle);
            }
        }
    });
}
