//! Named kernel objects and mutexes (P18).
//!
//! Names: an event, semaphore or mutex created with a name is found again
//! by Create* (the existing object, ERROR_ALREADY_EXISTS) and by Open*,
//! within this process. Names compare without ASCII case, and the
//! `Global\` and `Local\` prefixes name the same session space; sharing a
//! name between processes waits for M0k's object broker. The table keeps
//! its own handle to each named object, so a name outlives the creator's
//! handle, as long as the process runs.
//!
//! Mutexes are kernel semaphores of one count whose ownership lives here:
//! the owning thread acquires again without waiting (a recursion count),
//! ReleaseMutex from another thread fails with ERROR_NOT_OWNER, and a
//! thread that exits holding one abandons it, so its next owner's wait
//! returns WAIT_ABANDONED.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) const EVENT: u8 = 1;
pub(super) const SEMAPHORE: u8 = 2;
pub(super) const MUTEX: u8 = 3;
const NAME_UNITS: usize = 260;
const SLOTS: usize = 64;
/// Mutex records (kept until the process ends: a closed handle is not seen).
const MUTEXES: usize = 256;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOACCESS: u32 = 998;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_NOT_OWNER: u32 = 288;
const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
/// WAIT_ABANDONED_0.
pub(super) const ABANDONED: u32 = 0x80;

#[derive(Clone, Copy)]
struct Entry {
    kind: u8,
    len: usize,
    name: [u16; NAME_UNITS],
    /// The table's own handle.
    handle: u64,
}
#[derive(Clone, Copy)]
struct Mutex {
    /// The kernel object's identity (the same for every duplicate).
    object: u64,
    /// The table's own handle (abandonment releases through it).
    handle: u64,
    owner: u32,
    depth: u32,
    abandoned: bool,
}
struct Table {
    locked: AtomicBool,
    names: UnsafeCell<[Option<Entry>; SLOTS]>,
    mutexes: UnsafeCell<[Option<Mutex>; MUTEXES]>,
}
// SAFETY: `locked` gives one thread the cells at a time.
unsafe impl Sync for Table {}
static TABLE: Table = Table {
    locked: AtomicBool::new(false),
    names: UnsafeCell::new([None; SLOTS]),
    mutexes: UnsafeCell::new([None; MUTEXES]),
};
fn with<R>(f: impl FnOnce(&mut [Option<Entry>; SLOTS], &mut [Option<Mutex>; MUTEXES]) -> R) -> R {
    while TABLE
        .locked
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        let _ = door_sys::yield_now();
    }
    // SAFETY: the flag gives this closure the only access.
    let result = f(unsafe { &mut *TABLE.names.get() }, unsafe {
        &mut *TABLE.mutexes.get()
    });
    TABLE.locked.store(false, Ordering::Release);
    result
}
fn thread() -> u32 {
    super::threads::DoorGetCurrentThreadId()
}
/// A second handle to the object `handle` names, with the same rights.
fn duplicate(handle: u64) -> Result<u64, u32> {
    let rights = door_sys::inspect(handle).map_err(|_| ERROR_INVALID_HANDLE)?[1];
    door_sys::duplicate(handle, rights).map_err(|_| ERROR_INVALID_HANDLE)
}
fn object_of(handle: u64) -> Option<u64> {
    door_sys::inspect(handle).ok().map(|info| info[2])
}

/// The NUL-terminated name at `pointer`, its namespace prefix dropped.
fn read_name(pointer: u64) -> Result<([u16; NAME_UNITS], usize), u32> {
    let mut units = [0u16; NAME_UNITS];
    let mut len = 0;
    'read: loop {
        let at = pointer + 2 * len as u64;
        let n = ((4096 - (at % 4096)) as usize / 2).clamp(1, 128);
        let mut chunk = [0u16; 128];
        super::guest::copy(chunk.as_mut_ptr() as u64, at, 2 * n).map_err(|_| ERROR_NOACCESS)?;
        for &unit in &chunk[..n] {
            if unit == 0 {
                break 'read;
            }
            if len == NAME_UNITS {
                return Err(ERROR_FILENAME_EXCED_RANGE);
            }
            units[len] = unit;
            len += 1;
        }
    }
    for prefix in ["Global\\", "Local\\"] {
        let p: [u16; 8] =
            core::array::from_fn(|i| prefix.as_bytes().get(i).map_or(0, |b| u16::from(*b)));
        let plen = prefix.len();
        if len >= plen && same(&units[..plen], &p[..plen]) {
            units.copy_within(plen..len, 0);
            len -= plen;
            break;
        }
    }
    Ok((units, len))
}
fn same(a: &[u16], b: &[u16]) -> bool {
    let fold = |u: u16| if (65..=90).contains(&u) { u + 32 } else { u };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| fold(*x) == fold(*y))
}

/// Create* with a name (`name` non-zero): the existing object of `kind`
/// (a new handle, `true`), or the one `create` makes, now named (`false`).
/// An existing name of another kind is ERROR_INVALID_HANDLE.
pub(super) fn create_or_open(
    kind: u8,
    name: u64,
    create: impl FnOnce() -> Result<u64, u32>,
) -> Result<(u64, bool), u32> {
    let (units, len) = read_name(name)?;
    let find = |names: &[Option<Entry>; SLOTS]| -> Option<Result<u64, u32>> {
        let entry = names
            .iter()
            .flatten()
            .find(|e| same(&e.name[..e.len], &units[..len]))?;
        Some(if entry.kind != kind {
            Err(ERROR_INVALID_HANDLE)
        } else {
            duplicate(entry.handle)
        })
    };
    if let Some(existing) = with(|names, _| find(names)) {
        return existing.map(|handle| (handle, true));
    }
    // Made outside the lock (a mutex records itself in this table).
    let handle = create()?;
    let own = duplicate(handle)?;
    let raced = with(|names, _| {
        // Another thread named one first: that object is the name's.
        if let Some(existing) = find(names) {
            return Some(existing);
        }
        if let Some(slot) = names.iter_mut().find(|e| e.is_none()) {
            *slot = Some(Entry {
                kind,
                len,
                name: units,
                handle: own,
            });
        }
        None
    });
    match raced {
        None => Ok((handle, false)),
        Some(existing) => {
            let _ = door_sys::close(own);
            let _ = door_sys::close(handle);
            existing.map(|handle| (handle, true))
        }
    }
}
/// Open* by name: a new handle to the named object of `kind`.
pub(super) fn open(kind: u8, name: u64) -> Result<u64, u32> {
    if name == 0 {
        return Err(87);
    }
    let (units, len) = read_name(name)?;
    with(|names, _| {
        let entry = names
            .iter()
            .flatten()
            .find(|e| same(&e.name[..e.len], &units[..len]))
            .ok_or(ERROR_FILE_NOT_FOUND)?;
        if entry.kind != kind {
            return Err(ERROR_INVALID_HANDLE);
        }
        duplicate(entry.handle)
    })
}

/// A new mutex (owned by the calling thread when `owned`).
pub(super) fn mutex_create(owned: bool) -> Result<u64, u32> {
    let handle = door_sys::wait_create(4, u64::from(!owned), 1).map_err(|_| 8u32)?;
    let object = object_of(handle).ok_or(ERROR_INVALID_HANDLE)?;
    let own = duplicate(handle)?;
    let me = thread();
    let stored = with(|_, mutexes| {
        let slot = mutexes.iter_mut().find(|m| m.is_none())?;
        *slot = Some(Mutex {
            object,
            handle: own,
            owner: if owned { me } else { 0 },
            depth: u32::from(owned),
            abandoned: false,
        });
        Some(())
    });
    if stored.is_none() {
        let _ = door_sys::close(own);
        let _ = door_sys::close(handle);
        return Err(8);
    }
    Ok(handle)
}
/// A wait on `handle` by the mutex's owner: it holds it once more
/// (WAIT_OBJECT_0 without waiting). None for anything else.
pub(super) fn owned_again(handle: u64) -> Option<u32> {
    let object = object_of(handle)?;
    let me = thread();
    with(|_, mutexes| {
        let m = mutexes.iter_mut().flatten().find(|m| m.object == object)?;
        if m.owner != me || me == 0 {
            return None;
        }
        m.depth = m.depth.checked_add(1)?;
        Some(0)
    })
}
/// After a wait acquired `handle`: a mutex becomes the caller's (and
/// reports an abandonment once). 0 (WAIT_OBJECT_0) or ABANDONED.
pub(super) fn acquired(handle: u64) -> u32 {
    let Some(object) = object_of(handle) else {
        return 0;
    };
    let me = thread();
    with(|_, mutexes| {
        let Some(m) = mutexes.iter_mut().flatten().find(|m| m.object == object) else {
            return 0;
        };
        m.owner = me;
        m.depth = 1;
        if core::mem::take(&mut m.abandoned) {
            ABANDONED
        } else {
            0
        }
    })
}
/// ReleaseMutex: one level of the caller's ownership; the last frees it.
pub(super) fn release(handle: u64) -> Result<(), u32> {
    let object = object_of(handle).ok_or(ERROR_INVALID_HANDLE)?;
    let me = thread();
    let free = with(|_, mutexes| {
        let m = mutexes
            .iter_mut()
            .flatten()
            .find(|m| m.object == object)
            .ok_or(ERROR_INVALID_HANDLE)?;
        if m.owner != me || m.depth == 0 {
            return Err(ERROR_NOT_OWNER);
        }
        m.depth -= 1;
        if m.depth == 0 {
            m.owner = 0;
            return Ok(Some(m.handle));
        }
        Ok(None)
    })?;
    if let Some(own) = free {
        door_sys::signal(own, 1).map_err(|_| ERROR_INVALID_HANDLE)?;
    }
    Ok(())
}
/// A thread ends: every mutex it holds is abandoned and freed.
pub(super) fn abandon(thread: u32) {
    if thread == 0 {
        return;
    }
    let mut freed = [0u64; MUTEXES];
    let mut count = 0;
    with(|_, mutexes| {
        for m in mutexes.iter_mut().flatten() {
            if m.owner == thread {
                m.owner = 0;
                m.depth = 0;
                m.abandoned = true;
                freed[count] = m.handle;
                count += 1;
            }
        }
    });
    for &own in &freed[..count] {
        let _ = door_sys::signal(own, 1);
    }
}
