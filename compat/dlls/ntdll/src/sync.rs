//! User-mode synchronization over the kernel's address waits (P18 M0c):
//! WaitOnAddress/WakeByAddress, SRW locks, critical sections, condition
//! variables and interlocked singly linked lists, with Windows' layouts.
//! Uncontended paths are one atomic operation; a contended lock sleeps in
//! the kernel (WAIT_ADDRESS) instead of polling, and costs no kernel
//! object, so a process may hold as many locks as it likes.
use super::guest::RtlSetLastWin32Error;
use core::sync::atomic::{AtomicU32, Ordering};
use door_abi::{Status, operation as op};

const TIMEOUT: u32 = 1460;
const INFINITE: u32 = u32::MAX;

fn thread_id() -> u32 {
    super::threads::DoorGetCurrentThreadId()
}
fn deadline(millis: u32) -> u64 {
    if millis == INFINITE {
        return u64::MAX;
    }
    door_sys::time().unwrap_or(0).saturating_add(
        door_abi::platform::ticks_from_millis(u64::from(millis)).expect("u32 duration"),
    )
}
/// Sleep while the `size`-byte word at `address` holds `expected`, until
/// woken or `deadline` (ticks). Ok(true) when woken or the word differed,
/// Ok(false) on timeout.
fn wait_word(address: usize, expected: u64, size: u64, deadline: u64) -> Result<bool, Status> {
    match unsafe {
        door_sys::raw(
            op::WAIT_ADDRESS,
            [address as u64, expected, size, deadline, 0, 0],
        )
    }
    .result()
    {
        Ok(_) => Ok(true),
        Err(Status::Timeout) => Ok(false),
        Err(error) => Err(error),
    }
}
fn wake_word(address: usize, count: u64) {
    let _ = unsafe { door_sys::raw(op::WAKE_ADDRESS, [address as u64, count, 0, 0, 0, 0]) };
}
fn word(address: usize) -> &'static AtomicU32 {
    // SAFETY: callers pass the address of a live, aligned lock object.
    unsafe { &*(address as *const AtomicU32) }
}

// ---- WaitOnAddress ----------------------------------------------------

/// RtlWaitOnAddress / WaitOnAddress: TRUE once woken (or the value at
/// `address` differs from `compare`), FALSE with ERROR_TIMEOUT.
#[unsafe(no_mangle)]
pub extern "system" fn DoorWaitOnAddress(
    address: usize,
    compare: usize,
    size: usize,
    millis: u32,
) -> i32 {
    if !matches!(size, 1 | 2 | 4 | 8) || !address.is_multiple_of(size) || compare == 0 {
        RtlSetLastWin32Error(87);
        return 0;
    }
    // SAFETY: the caller's compare buffer holds `size` readable bytes.
    let expected = unsafe {
        match size {
            1 => u64::from((compare as *const u8).read_unaligned()),
            2 => u64::from((compare as *const u16).read_unaligned()),
            4 => u64::from((compare as *const u32).read_unaligned()),
            _ => (compare as *const u64).read_unaligned(),
        }
    };
    match wait_word(address, expected, size as u64, deadline(millis)) {
        Ok(true) => 1,
        Ok(false) => {
            RtlSetLastWin32Error(TIMEOUT);
            0
        }
        Err(_) => {
            RtlSetLastWin32Error(998);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorWakeByAddressSingle(address: usize) {
    wake_word(address, 1);
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorWakeByAddressAll(address: usize) {
    wake_word(address, 0);
}

// ---- SRW locks (one pointer; the low 32 bits are the state) ------------

const WRITER: u32 = 1 << 31;
const WAITING: u32 = 1 << 30;
const READERS: u32 = WAITING - 1;

#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeSRWLock(lock: usize) {
    // SAFETY: SRWLOCK is pointer-sized and pointer-aligned.
    unsafe { (lock as *mut usize).write(0) };
}
fn srw_wait(lock: usize, observed: u32) {
    let state = word(lock);
    // Announce the waiter, then sleep while nothing changed.
    if observed & WAITING == 0
        && state
            .compare_exchange(
                observed,
                observed | WAITING,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
    {
        return;
    }
    let _ = wait_word(lock, u64::from(observed | WAITING), 4, u64::MAX);
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlAcquireSRWLockExclusive(lock: usize) {
    let state = word(lock);
    loop {
        let s = state.load(Ordering::Relaxed);
        if s & (WRITER | READERS) == 0 {
            if state
                .compare_exchange_weak(s, s | WRITER, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
            continue;
        }
        srw_wait(lock, s);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlTryAcquireSRWLockExclusive(lock: usize) -> u8 {
    let state = word(lock);
    let s = state.load(Ordering::Relaxed);
    u8::from(
        s & (WRITER | READERS) == 0
            && state
                .compare_exchange(s, s | WRITER, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlReleaseSRWLockExclusive(lock: usize) {
    if word(lock).swap(0, Ordering::Release) & WAITING != 0 {
        wake_word(lock, 0);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlAcquireSRWLockShared(lock: usize) {
    let state = word(lock);
    loop {
        let s = state.load(Ordering::Relaxed);
        // New readers queue behind a waiting writer, so writers progress.
        if s & (WRITER | WAITING) == 0 && s & READERS != READERS {
            if state
                .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
            continue;
        }
        srw_wait(lock, s);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlTryAcquireSRWLockShared(lock: usize) -> u8 {
    let state = word(lock);
    let s = state.load(Ordering::Relaxed);
    u8::from(
        s & (WRITER | WAITING) == 0
            && s & READERS != READERS
            && state
                .compare_exchange(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok(),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlReleaseSRWLockShared(lock: usize) {
    let state = word(lock);
    let left = state.fetch_sub(1, Ordering::Release) - 1;
    // The last reader out with waiters behind it lets them all retry.
    if left == WAITING
        && state
            .compare_exchange(WAITING, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        wake_word(lock, 0);
    }
}

// ---- Critical sections -------------------------------------------------
// RTL_CRITICAL_SECTION: DebugInfo, LockCount, RecursionCount, OwningThread,
// LockSemaphore, SpinCount (pointer-sized except the two counts). The lock
// word (0 free, 1 held, 2 held with waiters) lives in LockSemaphore;
// LockCount mirrors Windows (-1 free, -2 held) for code that peeks at it.

const PTR: usize = core::mem::size_of::<usize>();
const LOCK_COUNT: usize = PTR;
const RECURSION: usize = PTR + 4;
const OWNER: usize = PTR + 8;
const SEMAPHORE: usize = 2 * PTR + 8;
const SPIN: usize = 3 * PTR + 8;
/// RTL_CRITICAL_SECTION_FLAG_NO_DEBUG_INFO: no debug record is kept.
const NO_DEBUG_INFO: usize = usize::MAX;

fn field<T>(section: usize, offset: usize) -> *mut T {
    (section + offset) as *mut T
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeCriticalSectionEx(
    section: usize,
    spin: u32,
    _flags: u32,
) -> u32 {
    // SAFETY: the caller passes storage for one RTL_CRITICAL_SECTION.
    unsafe {
        field::<usize>(section, 0).write(NO_DEBUG_INFO);
        field::<i32>(section, LOCK_COUNT).write(-1);
        field::<i32>(section, RECURSION).write(0);
        field::<usize>(section, OWNER).write(0);
        field::<usize>(section, SEMAPHORE).write(0);
        field::<usize>(section, SPIN).write((spin & 0x00ff_ffff) as usize);
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeCriticalSection(section: usize) -> u32 {
    RtlInitializeCriticalSectionEx(section, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeCriticalSectionAndSpinCount(section: usize, spin: u32) -> u32 {
    RtlInitializeCriticalSectionEx(section, spin, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlSetCriticalSectionSpinCount(section: usize, spin: u32) -> u32 {
    // SAFETY: an initialized critical section.
    unsafe {
        let old = field::<usize>(section, SPIN).read() as u32;
        field::<usize>(section, SPIN).write(spin as usize);
        old
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlDeleteCriticalSection(section: usize) -> u32 {
    // Nothing is held outside the structure itself.
    unsafe { field::<usize>(section, OWNER).write(0) };
    0
}
fn owned(section: usize) {
    // SAFETY: the lock word is held by this thread.
    unsafe {
        field::<usize>(section, OWNER).write(thread_id() as usize);
        field::<i32>(section, RECURSION).write(1);
        field::<i32>(section, LOCK_COUNT).write(-2);
    }
}
fn recursed(section: usize) -> bool {
    // SAFETY: OwningThread is only ever this thread's id while it holds
    // the lock, so reading it racily can only match for the owner.
    unsafe {
        if field::<usize>(section, OWNER).read_volatile() == thread_id() as usize {
            let count = field::<i32>(section, RECURSION);
            count.write(count.read() + 1);
            return true;
        }
    }
    false
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlEnterCriticalSection(section: usize) -> u32 {
    if recursed(section) {
        return 0;
    }
    let lock = word(section + SEMAPHORE);
    if lock
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while lock.swap(2, Ordering::Acquire) != 0 {
            let _ = wait_word(section + SEMAPHORE, 2, 4, u64::MAX);
        }
    }
    owned(section);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlTryEnterCriticalSection(section: usize) -> u8 {
    if recursed(section) {
        return 1;
    }
    if word(section + SEMAPHORE)
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        owned(section);
        1
    } else {
        0
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlLeaveCriticalSection(section: usize) -> u32 {
    // SAFETY: called by the owner of an entered critical section.
    unsafe {
        let count = field::<i32>(section, RECURSION);
        let left = count.read() - 1;
        count.write(left);
        if left > 0 {
            return 0;
        }
        field::<usize>(section, OWNER).write(0);
        field::<i32>(section, LOCK_COUNT).write(-1);
    }
    if word(section + SEMAPHORE).swap(0, Ordering::Release) == 2 {
        wake_word(section + SEMAPHORE, 1);
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlIsCriticalSectionLockedByThread(section: usize) -> u32 {
    // SAFETY: an initialized critical section.
    u32::from(unsafe { field::<usize>(section, OWNER).read_volatile() } == thread_id() as usize)
}

// ---- Condition variables (one pointer; the low 32 bits count wakes) ----

#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeConditionVariable(variable: usize) {
    unsafe { (variable as *mut usize).write(0) };
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlWakeConditionVariable(variable: usize) {
    word(variable).fetch_add(1, Ordering::Release);
    wake_word(variable, 1);
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlWakeAllConditionVariable(variable: usize) {
    word(variable).fetch_add(1, Ordering::Release);
    wake_word(variable, 0);
}
/// Sleep on `variable` with `release` done and `reacquire` after; Windows'
/// status (STATUS_SUCCESS or STATUS_TIMEOUT 0x102).
fn sleep_condition(
    variable: usize,
    millis: u32,
    release: impl FnOnce(),
    reacquire: impl FnOnce(),
) -> u32 {
    let seen = word(variable).load(Ordering::Acquire);
    release();
    let woken = wait_word(variable, u64::from(seen), 4, deadline(millis)).unwrap_or(true);
    reacquire();
    if woken { 0 } else { 0x102 }
}
/// CONDITION_VARIABLE_LOCKMODE_SHARED.
const LOCKMODE_SHARED: u32 = 1;
#[unsafe(no_mangle)]
pub extern "system" fn RtlSleepConditionVariableSRW(
    variable: usize,
    lock: usize,
    millis: u32,
    flags: u32,
) -> u32 {
    if flags & LOCKMODE_SHARED != 0 {
        sleep_condition(
            variable,
            millis,
            || RtlReleaseSRWLockShared(lock),
            || RtlAcquireSRWLockShared(lock),
        )
    } else {
        sleep_condition(
            variable,
            millis,
            || RtlReleaseSRWLockExclusive(lock),
            || RtlAcquireSRWLockExclusive(lock),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlSleepConditionVariableCS(
    variable: usize,
    section: usize,
    millis: u32,
) -> u32 {
    // The whole recursion is released and restored, as on Windows.
    let depth = unsafe { field::<i32>(section, RECURSION).read() }.max(1);
    sleep_condition(
        variable,
        millis,
        || {
            unsafe { field::<i32>(section, RECURSION).write(1) };
            RtlLeaveCriticalSection(section);
        },
        || {
            RtlEnterCriticalSection(section);
            unsafe { field::<i32>(section, RECURSION).write(depth) };
        },
    )
}

// ---- Interlocked singly linked lists ------------------------------------
// x64 SLIST_HEADER: Depth:16 | Sequence:48 in the first quadword, the next
// entry >> 4 in the top 60 bits of the second (entries are 16-byte aligned).
// x86: Next, Depth:16, Sequence:16 in one quadword.

#[cfg(target_arch = "x86_64")]
mod slist {
    use core::arch::asm;
    /// cmpxchg16b on `header`: (succeeded, observed).
    pub unsafe fn exchange(header: usize, old: (u64, u64), new: (u64, u64)) -> (bool, (u64, u64)) {
        let (mut low, mut high) = old;
        let ok: u8;
        // RBX is reserved by LLVM: swap the new low quadword through it.
        unsafe {
            asm!(
                "xchg {new_low}, rbx",
                "lock cmpxchg16b [{header}]",
                "sete {ok}",
                "mov rbx, {new_low}",
                header = in(reg) header,
                new_low = inout(reg) new.0 => _,
                ok = out(reg_byte) ok,
                inout("rax") low,
                inout("rdx") high,
                in("rcx") new.1,
            );
        }
        (ok != 0, (low, high))
    }
    pub unsafe fn load(header: usize) -> (u64, u64) {
        // A compare-exchange of (0,0) with (0,0) reads atomically.
        unsafe { exchange(header, (0, 0), (0, 0)).1 }
    }
    pub fn next(h: (u64, u64)) -> usize {
        ((h.1 >> 4) << 4) as usize
    }
    pub fn depth(h: (u64, u64)) -> u16 {
        h.0 as u16
    }
    pub fn make(next: usize, depth: u16, sequence: u64) -> (u64, u64) {
        (
            u64::from(depth) | (sequence & 0xffff_ffff_ffff) << 16,
            (next as u64 >> 4) << 4,
        )
    }
    pub fn sequence(h: (u64, u64)) -> u64 {
        h.0 >> 16
    }
}
#[cfg(target_arch = "x86")]
mod slist {
    use core::sync::atomic::{AtomicU64, Ordering};
    fn cell(header: usize) -> &'static AtomicU64 {
        unsafe { &*(header as *const AtomicU64) }
    }
    pub unsafe fn exchange(header: usize, old: (u64, u64), new: (u64, u64)) -> (bool, (u64, u64)) {
        match cell(header).compare_exchange(old.0, new.0, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => (true, old),
            Err(seen) => (false, (seen, 0)),
        }
    }
    pub unsafe fn load(header: usize) -> (u64, u64) {
        (cell(header).load(Ordering::Acquire), 0)
    }
    pub fn next(h: (u64, u64)) -> usize {
        h.0 as u32 as usize
    }
    pub fn depth(h: (u64, u64)) -> u16 {
        (h.0 >> 32) as u16
    }
    pub fn make(next: usize, depth: u16, sequence: u64) -> (u64, u64) {
        (
            next as u64 | u64::from(depth) << 32 | (sequence & 0xffff) << 48,
            0,
        )
    }
    pub fn sequence(h: (u64, u64)) -> u64 {
        h.0 >> 48
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitializeSListHead(header: usize) {
    unsafe { core::ptr::write_bytes(header as *mut u8, 0, 2 * core::mem::size_of::<usize>()) };
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlQueryDepthSList(header: usize) -> u16 {
    slist::depth(unsafe { slist::load(header) })
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInterlockedPushEntrySList(header: usize, entry: usize) -> usize {
    let mut seen = unsafe { slist::load(header) };
    loop {
        // SAFETY: the caller owns `entry` until it is on the list.
        unsafe { (entry as *mut usize).write(slist::next(seen)) };
        let new = slist::make(
            entry,
            slist::depth(seen).wrapping_add(1),
            slist::sequence(seen).wrapping_add(1),
        );
        match unsafe { slist::exchange(header, seen, new) } {
            (true, _) => return slist::next(seen),
            (false, now) => seen = now,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInterlockedPopEntrySList(header: usize) -> usize {
    let mut seen = unsafe { slist::load(header) };
    loop {
        let first = slist::next(seen);
        if first == 0 {
            return 0;
        }
        // SAFETY: entries on the list stay readable (Windows' contract).
        let after = unsafe { (first as *const usize).read_volatile() };
        let new = slist::make(
            after,
            slist::depth(seen).wrapping_sub(1),
            slist::sequence(seen).wrapping_add(1),
        );
        match unsafe { slist::exchange(header, seen, new) } {
            (true, _) => return first,
            (false, now) => seen = now,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlInterlockedFlushSList(header: usize) -> usize {
    let mut seen = unsafe { slist::load(header) };
    loop {
        if slist::next(seen) == 0 {
            return 0;
        }
        let new = slist::make(0, 0, slist::sequence(seen).wrapping_add(1));
        match unsafe { slist::exchange(header, seen, new) } {
            (true, _) => return slist::next(seen),
            (false, now) => seen = now,
        }
    }
}
